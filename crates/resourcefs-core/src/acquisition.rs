use std::{num::NonZeroUsize, time::Duration};

use crate::{
    AcquisitionLimitKind, ErrorCategory, ErrorReason, LimitDetail, ResourceError,
    ResourceErrorDetails,
};

/// Source-neutral acquisition ceilings. Every dimension is positive and at most its hard cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadAcquisitionLimits {
    max_attempts: NonZeroUsize,
    timeout: Duration,
    max_response_bytes: NonZeroUsize,
    max_accepted_body_bytes: NonZeroUsize,
    max_representation_bytes: NonZeroUsize,
}

impl ReadAcquisitionLimits {
    pub fn new(
        max_attempts: Option<usize>,
        timeout: Option<Duration>,
        max_response_bytes: Option<usize>,
        max_accepted_body_bytes: Option<usize>,
        max_representation_bytes: Option<usize>,
    ) -> Result<Self, ResourceError> {
        let hard = Self::default();
        let timeout = timeout.unwrap_or(hard.timeout);
        if timeout.is_zero() || timeout > hard.timeout {
            // Extremely large Durations cannot be represented by the bounded
            // numeric detail; rejection is unchanged and the observation is absent.
            let observed = u64::try_from(timeout.as_nanos()).ok();
            return Err(invalid_limit(
                AcquisitionLimitKind::ElapsedNanoseconds,
                u64::try_from(hard.timeout.as_nanos()).expect("hard deadline fits in nanoseconds"),
                observed,
            ));
        }
        Ok(Self {
            max_attempts: validate(
                max_attempts,
                hard.max_attempts,
                AcquisitionLimitKind::Attempts,
            )?,
            timeout,
            max_response_bytes: validate(
                max_response_bytes,
                hard.max_response_bytes,
                AcquisitionLimitKind::ResponseBodyBytes,
            )?,
            max_accepted_body_bytes: validate(
                max_accepted_body_bytes,
                hard.max_accepted_body_bytes,
                AcquisitionLimitKind::AcceptedBodyBytes,
            )?,
            max_representation_bytes: validate(
                max_representation_bytes,
                hard.max_representation_bytes,
                AcquisitionLimitKind::RepresentationBytes,
            )?,
        })
    }

    pub const fn max_attempts(self) -> usize {
        self.max_attempts.get()
    }
    pub const fn timeout(self) -> Duration {
        self.timeout
    }
    pub const fn max_response_bytes(self) -> usize {
        self.max_response_bytes.get()
    }
    pub const fn max_accepted_body_bytes(self) -> usize {
        self.max_accepted_body_bytes.get()
    }
    pub const fn max_representation_bytes(self) -> usize {
        self.max_representation_bytes.get()
    }

    /// Applies both policies without raising any dimension of either one.
    pub fn intersect(self, other: Self) -> Self {
        Self {
            max_attempts: self.max_attempts.min(other.max_attempts),
            timeout: self.timeout.min(other.timeout),
            max_response_bytes: self.max_response_bytes.min(other.max_response_bytes),
            max_accepted_body_bytes: self
                .max_accepted_body_bytes
                .min(other.max_accepted_body_bytes),
            max_representation_bytes: self
                .max_representation_bytes
                .min(other.max_representation_bytes),
        }
    }
}

impl Default for ReadAcquisitionLimits {
    fn default() -> Self {
        Self {
            max_attempts: NonZeroUsize::new(10).expect("positive hard attempt cap"),
            timeout: Duration::from_secs(30),
            max_response_bytes: NonZeroUsize::new(8_388_608).expect("positive hard response cap"),
            max_accepted_body_bytes: NonZeroUsize::new(16_777_216)
                .expect("positive hard admitted cap"),
            max_representation_bytes: NonZeroUsize::new(16_777_216)
                .expect("positive hard representation cap"),
        }
    }
}

fn validate(
    value: Option<usize>,
    hard: NonZeroUsize,
    kind: AcquisitionLimitKind,
) -> Result<NonZeroUsize, ResourceError> {
    let value = value.unwrap_or(hard.get());
    match NonZeroUsize::new(value) {
        Some(value) if value <= hard => Ok(value),
        _ => Err(invalid_limit(kind, hard.get() as u64, Some(value as u64))),
    }
}

fn invalid_limit(kind: AcquisitionLimitKind, bound: u64, observed: Option<u64>) -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        "acquisition limit must be positive and no greater than its hard ceiling",
    )
    .with_details(
        ResourceErrorDetails::new(ErrorReason::InvalidAcquisitionLimit).with_limit(
            LimitDetail::new(kind, bound, observed)
                .expect("hard acquisition ceilings are positive"),
        ),
    )
}
