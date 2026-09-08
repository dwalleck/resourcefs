//! Owned machine representation: the shared facts envelope, presence-aware
//! decoding primitives, the bounded serialization boundary and family dispatch.
mod identity;

use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Write},
};

use resourcefs_core::{
    AcquisitionLimitKind, ErrorCategory, ErrorReason, LimitDetail, OperationGuard, PathReference,
    PullRequestAddress, PullRequestFact, PullRequestResource, ReadAcquisitionLimits,
    ResourceAddress, ResourceError, ResourceErrorDetails, SourceResource, Utf8ContentType,
};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, MapAccess, Visitor},
    ser::SerializeMap,
};

use super::{
    GITHUB_API_VERSION, GithubSource,
    fetch::{BodyObservation, unix_ms},
};
use crate::http::{BoundedRead, HttpReadBudget};

#[derive(Debug, Default)]
pub(super) enum Presence<T> {
    #[default]
    Omitted,
    Null,
    Present(T),
}
impl<T> Presence<T> {
    fn omitted(&self) -> bool {
        matches!(self, Self::Omitted)
    }
    fn value(&self) -> Option<&T> {
        match self {
            Self::Present(value) => Some(value),
            _ => None,
        }
    }
    fn availability(&self) -> &'static str {
        match self {
            Self::Omitted => "omitted",
            Self::Null => "null",
            Self::Present(_) => "present",
        }
    }
    fn unavailable(&self, field: &'static str, output: &mut Vec<Unavailable>) {
        if !matches!(self, Self::Present(_)) {
            output.push(Unavailable {
                field,
                reason: self.availability(),
            });
        }
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Presence<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Present(value),
            None => Self::Null,
        })
    }
}
impl<T: Serialize> Serialize for Presence<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Present(value) => value.serialize(serializer),
            _ => serializer.serialize_none(),
        }
    }
}

#[derive(Debug)]
pub(super) struct NativeId(u64);
impl<'de> Deserialize<'de> for NativeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u64::deserialize(deserializer)?;
        if value == 0 {
            return Err(de::Error::custom("identity must be positive"));
        }
        Ok(Self(value))
    }
}
impl Serialize for NativeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

// Object-only visitors reject serde's positional struct representation. Keys
// borrow from the parser, and unrecognized values are skipped without a tree.
macro_rules! native {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug)]
        pub(super) struct $name { $(pub(super) $field: Presence<$ty>,)* }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct Field(&'static str);
                impl<'de> Deserialize<'de> for Field {
                    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                        struct FieldVisitor;
                        impl Visitor<'_> for FieldVisitor {
                            type Value = Field;
                            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                                f.write_str("object field")
                            }
                            fn visit_str<E: de::Error>(self, value: &str) -> Result<Field, E> {
                                $(if value == stringify!($field).strip_prefix("r#").unwrap_or(stringify!($field)) {
                                    return Ok(Field(stringify!($field)));
                                })*
                                Ok(Field(""))
                            }
                        }
                        deserializer.deserialize_identifier(FieldVisitor)
                    }
                }
                struct ObjectVisitor;
                impl<'de> Visitor<'de> for ObjectVisitor {
                    type Value = $name;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        f.write_str(concat!(stringify!($name), " object"))
                    }
                    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<$name, M::Error> {
                        $(let mut $field: Presence<$ty> = Presence::Omitted;)*
                        while let Some(key) = map.next_key::<Field>()? {
                            match key.0 {
                                $(stringify!($field) => {
                                    if !$field.omitted() { return Err(de::Error::custom("duplicate native field")); }
                                    $field = map.next_value()?;
                                })*
                                _ => { map.next_value::<de::IgnoredAny>()?; }
                            }
                        }
                        Ok($name { $($field,)* })
                    }
                }
                deserializer.deserialize_map(ObjectVisitor)
            }
        }
    };
}
native!(Actor {
    id: NativeId,
    node_id: String,
    login: String,
    url: String,
    html_url: String
});
native!(Repository {
    id: NativeId,
    node_id: String,
    name: String,
    full_name: String,
    owner: Actor,
    url: String,
    html_url: String
});
native!(Relation { href: String });

#[derive(Debug)]
pub(super) struct Relations(BTreeMap<String, Presence<Relation>>);
impl<'de> Deserialize<'de> for Relations {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RelationsVisitor;
        impl<'de> Visitor<'de> for RelationsVisitor {
            type Value = Relations;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("link relations object")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Relations, M::Error> {
                let mut relations = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if relations.contains_key(&key) {
                        return Err(de::Error::custom("duplicate relation"));
                    }
                    relations.insert(key, map.next_value()?);
                }
                Ok(Relations(relations))
            }
        }
        deserializer.deserialize_map(RelationsVisitor)
    }
}
impl Serialize for Relation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if !self.href.omitted() {
            map.serialize_entry("href", &self.href)?;
        }
        map.end()
    }
}
impl Serialize for Relations {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Links<'a> {
    #[serde(skip_serializing_if = "Presence::omitted")]
    api_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    html_url: &'a Presence<String>,
}
impl Serialize for Actor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if !self.id.omitted() {
            map.serialize_entry("id", &self.id)?;
        }
        if !self.node_id.omitted() {
            map.serialize_entry("nodeId", &self.node_id)?;
        }
        if !self.login.omitted() {
            map.serialize_entry("login", &self.login)?;
        }
        if !self.url.omitted() || !self.html_url.omitted() {
            map.serialize_entry(
                "links",
                &Links {
                    api_url: &self.url,
                    html_url: &self.html_url,
                },
            )?;
        }
        map.end()
    }
}
impl Serialize for Repository {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if !self.id.omitted() {
            map.serialize_entry("id", &self.id)?;
        }
        if !self.node_id.omitted() {
            map.serialize_entry("nodeId", &self.node_id)?;
        }
        if !self.name.omitted() {
            map.serialize_entry("name", &self.name)?;
        }
        if !self.full_name.omitted() {
            map.serialize_entry("fullName", &self.full_name)?;
        }
        if !self.owner.omitted() {
            map.serialize_entry("owner", &self.owner)?;
        }
        if !self.url.omitted() || !self.html_url.omitted() {
            map.serialize_entry(
                "links",
                &Links {
                    api_url: &self.url,
                    html_url: &self.html_url,
                },
            )?;
        }
        map.end()
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SchemaVersion {
    major: u8,
    minor: u8,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Deployment<'a> {
    web_origin: &'a str,
    api_origin: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Source<'a> {
    source_id: &'a str,
    deployment: Deployment<'a>,
}
#[derive(Serialize)]
struct RepositoryName<'a> {
    owner: &'a str,
    name: &'a str,
}
#[derive(Serialize)]
struct RequestedRepository<'a> {
    owner: &'a str,
    name: &'a str,
    #[serde(skip_serializing_if = "Presence::omitted")]
    observed: &'a Presence<Repository>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Limits {
    max_attempts: usize,
    /// Whole milliseconds: `timeoutMs` is declared `"type": "integer"` in the
    /// profile and tool schemas, so echoing `30000.0` would not round-trip.
    timeout_ms: u64,
    max_response_bytes: usize,
    max_accepted_body_bytes: usize,
    max_representation_bytes: usize,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Usage {
    attempted_requests: usize,
    accepted_body_bytes: usize,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Acquisition {
    started_at_unix_ms: u64,
    completed_at_unix_ms: u64,
    elapsed_ms: u64,
    rest_api_version: &'static str,
    limits: Limits,
    usage: Usage,
}
#[derive(Serialize)]
struct Upstream<'a> {
    body: &'a BodyObservation,
    #[serde(skip_serializing_if = "Option::is_none")]
    revalidation: &'a Option<BodyObservation>,
}
#[derive(Serialize)]
struct Unavailable {
    field: &'static str,
    reason: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Facts<'a, B: Serialize> {
    schema_version: SchemaVersion,
    kind: &'static str,
    resource: &'a str,
    source: Source<'a>,
    repository: RequestedRepository<'a>,
    acquisition: Acquisition,
    #[serde(flatten)]
    body: B,
    unavailable_facts: Vec<Unavailable>,
}

/// One bounded facts acquisition shared by every fact family in this read.
struct FactsRead<'a> {
    canonical: PathReference,
    web_origin: &'a str,
    limits: ReadAcquisitionLimits,
    read: BoundedRead<'a>,
    budget: HttpReadBudget,
    started_at_unix_ms: u64,
    started: tokio::time::Instant,
}

mod collection;
mod comment;
mod pull;

fn acquisition(ctx: &FactsRead<'_>) -> Result<Acquisition, ResourceError> {
    Ok(Acquisition {
        started_at_unix_ms: ctx.started_at_unix_ms,
        completed_at_unix_ms: unix_ms()?,
        elapsed_ms: ctx.started.elapsed().as_millis() as u64,
        rest_api_version: GITHUB_API_VERSION,
        limits: Limits {
            max_attempts: ctx.limits.max_attempts(),
            timeout_ms: u64::try_from(ctx.limits.timeout().as_millis())
                .expect("bounded deadline fits in milliseconds"),
            max_response_bytes: ctx.limits.max_response_bytes(),
            max_accepted_body_bytes: ctx.limits.max_accepted_body_bytes(),
            max_representation_bytes: ctx.limits.max_representation_bytes(),
        },
        usage: Usage {
            attempted_requests: ctx.budget.used_attempts(),
            accepted_body_bytes: ctx.budget.accepted_body_bytes(),
        },
    })
}

fn failure(reason: ErrorReason) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        "GitHub facts could not be accepted",
    )
    .with_details(ResourceErrorDetails::new(reason))
}

struct CappedWriter {
    bytes: Vec<u8>,
    cap: usize,
    exceeded: bool,
}
impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.cap - self.bytes.len() {
            self.exceeded = true;
            return Err(io::Error::other("representation limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn finish_facts<T: Serialize>(
    facts: &T,
    cap: usize,
    reference: PathReference,
    read: BoundedRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let mut writer = CappedWriter {
        bytes: Vec::new(),
        cap,
        exceeded: false,
    };
    if serde_json::to_writer_pretty(&mut writer, facts).is_err() {
        if writer.exceeded {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                "GitHub facts representation exceeds its limit",
            )
            .with_details(
                ResourceErrorDetails::new(ErrorReason::LimitExceeded).with_limit(LimitDetail::new(
                    AcquisitionLimitKind::RepresentationBytes,
                    cap as u64,
                    None,
                )?),
            ));
        }
        return Err(failure(ErrorReason::UpstreamMalformed));
    }
    let content =
        String::from_utf8(writer.bytes).map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let resource = SourceResource::utf8(
        reference,
        content,
        Utf8ContentType::new("application/json; charset=utf-8")?,
    )?;
    read.check_acceptance()?;
    Ok(resource)
}

/// Withholds anything the upstream said, while leaving the caller a cause.
/// Only categories carrying an upstream answer are rewritten, and each ends
/// with a machine-readable reason rather than a bare message.
fn sanitize(error: ResourceError) -> ResourceError {
    let reason = match error.category() {
        ErrorCategory::Cancelled => ErrorReason::Cancelled,
        ErrorCategory::PermissionDenied => ErrorReason::UpstreamDenied,
        ErrorCategory::LimitExceeded => ErrorReason::LimitExceeded,
        ErrorCategory::NotFound => ErrorReason::UpstreamNotFoundOrHidden,
        ErrorCategory::SourceUnavailable => ErrorReason::UpstreamUnavailable,
        // Every other category describes the request, not the upstream: this
        // adapter's own sentence, with nothing in it to withhold. Blanking it
        // costs the caller their only description of what they got wrong.
        _ => return error,
    };
    let sanitized = ResourceError::new(error.category(), "GitHub facts read failed");
    match error.details() {
        Some(details) => sanitized.with_details(*details),
        None => sanitized.with_details(ResourceErrorDetails::new(reason)),
    }
}

impl GithubSource {
    pub(super) async fn read_facts(
        &self,
        reference: &PathReference,
        fact: PullRequestFact,
        operation: &OperationGuard,
        acquisition: Option<&ReadAcquisitionLimits>,
    ) -> Result<SourceResource, ResourceError> {
        self.facts_resource(reference, fact, operation, acquisition)
            .await
            .map_err(sanitize)
    }

    async fn facts_resource(
        &self,
        reference: &PathReference,
        fact: PullRequestFact,
        operation: &OperationGuard,
        acquisition: Option<&ReadAcquisitionLimits>,
    ) -> Result<SourceResource, ResourceError> {
        let ResourceAddress::PullRequest(address) = reference.address() else {
            return Err(super::unsupported_github_projection());
        };
        let PullRequestAddress::Item {
            repository, number, ..
        } = address
        else {
            return Err(super::unsupported_github_projection());
        };
        if reference.projection().is_some() {
            return Err(super::unsupported_github_projection());
        }
        let canonical = PathReference::pull_request(address.clone(), None)?;
        self.authorize_repository(repository).map_err(|error| {
            error.with_details(ResourceErrorDetails::new(
                ErrorReason::RepositoryNotAuthorized,
            ))
        })?;
        let web_origin = self.config.deployment().web_origin().ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "GitHub deployment identity is unavailable",
            )
            .with_details(ResourceErrorDetails::new(
                ErrorReason::DeploymentIdentityUnavailable,
            ))
        })?;
        let limits = self
            .config
            .acquisition_limits()
            .intersect(acquisition.copied().unwrap_or_default());
        let (read, limits) = self.substrate.begin_read_with_limits(operation, &limits)?;
        read.check_acceptance()?;
        let started_at_unix_ms = unix_ms()?;
        let started = tokio::time::Instant::now();
        let mut ctx = FactsRead {
            canonical,
            web_origin,
            limits,
            read,
            budget: HttpReadBudget::with_limits(&limits),
            started_at_unix_ms,
            started,
        };
        let number = number.get();
        match fact {
            PullRequestFact::Pull => pull::read(self, repository, number, &mut ctx).await,
            PullRequestFact::Comment(id) => {
                comment::read_item(self, repository, number, id.get(), &mut ctx).await
            }
            PullRequestFact::Comments => collection::read(self, repository, number, &mut ctx).await,
        }
    }
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
