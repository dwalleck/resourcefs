use std::{fmt, time::Duration};

use resourcefs_core::{AcquisitionLimitKind, ReadAcquisitionLimits, ResourceError};
use schemars::JsonSchema;
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};

/// Shared, closed read/profile syntax; core owns the numerical policy.
#[derive(Debug, Clone, Default, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AcquisitionInput {
    #[serde(default)]
    #[schemars(with = "usize", range(min = 1, max = 10))]
    max_attempts: Option<usize>,
    #[serde(default)]
    #[schemars(with = "u64", range(min = 1, max = 30_000))]
    timeout_ms: Option<u64>,
    #[serde(default)]
    #[schemars(with = "usize", range(min = 1, max = 8_388_608))]
    max_response_bytes: Option<usize>,
    #[serde(default)]
    #[schemars(with = "usize", range(min = 1, max = 16_777_216))]
    max_accepted_body_bytes: Option<usize>,
    #[serde(default)]
    #[schemars(with = "usize", range(min = 1, max = 16_777_216))]
    max_representation_bytes: Option<usize>,
    #[serde(default)]
    #[schemars(with = "usize", range(min = 1, max = 4_194_304))]
    max_decoded_bytes: Option<usize>,
}

const NANOSECONDS_PER_MILLISECOND: u64 = 1_000_000;

impl AcquisitionInput {
    pub(crate) fn into_limits(self) -> Result<ReadAcquisitionLimits, ResourceError> {
        ReadAcquisitionLimits::new(
            self.max_attempts,
            self.timeout_ms.map(Duration::from_millis),
            self.max_response_bytes,
            self.max_accepted_body_bytes,
            self.max_representation_bytes,
            self.max_decoded_bytes,
        )
    }
}

/// Renders an acquisition rejection in the syntax the caller actually wrote.
///
/// Core names the failing dimension only in the machine-readable detail, in
/// its own vocabulary and — for the deadline — in nanoseconds. Its message is
/// identical for all six dimensions, so relaying that alone tells an operator
/// a limit is wrong without telling them which one or what the ceiling is.
pub(crate) fn describe_limit_rejection(error: &ResourceError) -> String {
    let Some(limit) = error.details().and_then(|details| details.limit()) else {
        return error.message().to_owned();
    };
    let scale = match limit.kind() {
        AcquisitionLimitKind::ElapsedNanoseconds => NANOSECONDS_PER_MILLISECOND,
        _ => 1,
    };
    let field = match limit.kind() {
        AcquisitionLimitKind::Attempts => "maxAttempts",
        AcquisitionLimitKind::ElapsedNanoseconds => "timeoutMs",
        AcquisitionLimitKind::ResponseBodyBytes => "maxResponseBytes",
        AcquisitionLimitKind::AcceptedBodyBytes => "maxAcceptedBodyBytes",
        AcquisitionLimitKind::RepresentationBytes => "maxRepresentationBytes",
        AcquisitionLimitKind::DecodedContentBytes => "maxDecodedBytes",
        // A collection record ceiling is local policy, not a control the
        // caller wrote, so there is no caller-facing field name to report.
        AcquisitionLimitKind::CollectionRecords => return error.message().to_owned(),
    };
    let bound = limit.bound() / scale;
    match limit.observed() {
        Some(observed) => {
            format!(
                "{field} must be between 1 and {bound}, but is {}",
                observed / scale
            )
        }
        None => format!("{field} must be between 1 and {bound}"),
    }
}

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "camelCase")]
enum AcquisitionField {
    MaxAttempts,
    TimeoutMs,
    MaxResponseBytes,
    MaxAcceptedBodyBytes,
    MaxRepresentationBytes,
    MaxDecodedBytes,
}

struct AcquisitionObject;

impl<'de> Visitor<'de> for AcquisitionObject {
    type Value = AcquisitionInput;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an acquisition object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut input = AcquisitionInput::default();
        while let Some(field) = map.next_key::<AcquisitionField>()? {
            match field {
                AcquisitionField::MaxAttempts => {
                    read_field(&mut input.max_attempts, &mut map, "maxAttempts")?
                }
                AcquisitionField::TimeoutMs => {
                    read_field(&mut input.timeout_ms, &mut map, "timeoutMs")?
                }
                AcquisitionField::MaxResponseBytes => {
                    read_field(&mut input.max_response_bytes, &mut map, "maxResponseBytes")?
                }
                AcquisitionField::MaxAcceptedBodyBytes => read_field(
                    &mut input.max_accepted_body_bytes,
                    &mut map,
                    "maxAcceptedBodyBytes",
                )?,
                AcquisitionField::MaxRepresentationBytes => read_field(
                    &mut input.max_representation_bytes,
                    &mut map,
                    "maxRepresentationBytes",
                )?,
                AcquisitionField::MaxDecodedBytes => {
                    read_field(&mut input.max_decoded_bytes, &mut map, "maxDecodedBytes")?
                }
            }
        }
        Ok(input)
    }
}

impl<'de> Deserialize<'de> for AcquisitionInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Derived struct deserialization also accepts sequences. This syntax is
        // an object in both the protocol and profile, including when it is empty.
        deserializer.deserialize_map(AcquisitionObject)
    }
}

fn read_field<'de, A, T>(
    slot: &mut Option<T>,
    map: &mut A,
    name: &'static str,
) -> Result<(), A::Error>
where
    A: MapAccess<'de>,
    T: Deserialize<'de>,
{
    if slot.is_some() {
        return Err(de::Error::duplicate_field(name));
    }
    *slot = Some(map.next_value()?);
    Ok(())
}
