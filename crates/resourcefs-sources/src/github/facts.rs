//! Presence-aware native PR decoding and the bounded owned machine representation.
mod identity;

use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Write},
};

use resourcefs_core::{
    AcquisitionLimitKind, ErrorCategory, ErrorReason, LimitDetail, OperationGuard, PathReference,
    PullRequestAddress, ReadAcquisitionLimits, ResourceAddress, ResourceError,
    ResourceErrorDetails, SourceResource, Utf8ContentType,
};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, MapAccess, Visitor},
    ser::SerializeMap,
};
use url::Url;

use super::{
    GITHUB_API_VERSION, GITHUB_JSON, GithubSource,
    fetch::{BodyObservation, unix_ms},
};
use crate::http::{BoundedRead, HttpReadBudget};

#[derive(Debug, Default)]
enum Presence<T> {
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
struct NativeId(u64);
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
        struct $name { $($field: Presence<$ty>,)* }
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
native!(NativePull {
    id: NativeId,
    node_id: String,
    number: NativeId,
    url: String,
    html_url: String,
    diff_url: String,
    patch_url: String,
    issue_url: String,
    _links: Relations,
    title: String,
    body: String,
    state: String,
    user: Actor,
    created_at: String,
    updated_at: String,
    closed_at: String,
    merged_at: String,
    draft: bool,
    merged: bool,
    base: Branch,
    head: Branch,
});
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
native!(Branch {
    sha: String,
    r#ref: String,
    repo: Repository
});
native!(Relation { href: String });

#[derive(Debug)]
struct Relations(BTreeMap<String, Presence<Relation>>);
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
struct BranchFacts<'a> {
    #[serde(skip_serializing_if = "Presence::omitted")]
    ref_name: &'a Presence<String>,
    commit_sha: &'a str,
    #[serde(skip_serializing_if = "Presence::omitted")]
    repository: &'a Presence<Repository>,
    repository_availability: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservedBranch<'a> {
    commit_sha: &'a str,
    #[serde(skip_serializing_if = "Presence::omitted")]
    repository: &'a Presence<Repository>,
    repository_availability: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PullLinks<'a> {
    #[serde(skip_serializing_if = "Presence::omitted")]
    api_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    html_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    diff_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    patch_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    issue_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    relations: &'a Presence<Relations>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Data<'a> {
    id: &'a NativeId,
    number: &'a NativeId,
    #[serde(skip_serializing_if = "Presence::omitted")]
    node_id: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    title: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    body: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    state: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    author: &'a Presence<Actor>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    created_at: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    updated_at: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    closed_at: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    merged_at: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    draft: &'a Presence<bool>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    merged: &'a Presence<bool>,
    links: PullLinks<'a>,
    base: BranchFacts<'a>,
    head: BranchFacts<'a>,
}
#[derive(Serialize)]
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
struct Request<'a> {
    repository: RepositoryName<'a>,
    number: &'a NativeId,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observed<'a> {
    id: &'a NativeId,
    number: &'a NativeId,
    #[serde(skip_serializing_if = "Presence::omitted")]
    node_id: &'a Presence<String>,
    base: ObservedBranch<'a>,
    head: ObservedBranch<'a>,
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
struct Facts<'a> {
    schema_version: SchemaVersion,
    kind: &'static str,
    resource: &'a str,
    source: Source<'a>,
    repository: RequestedRepository<'a>,
    request: Request<'a>,
    observed: Observed<'a>,
    acquisition: Acquisition,
    upstream: Upstream<'a>,
    data: Data<'a>,
    unavailable_facts: Vec<Unavailable>,
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
        operation: &OperationGuard,
        acquisition: Option<&ReadAcquisitionLimits>,
    ) -> Result<SourceResource, ResourceError> {
        self.facts_resource(reference, operation, acquisition)
            .await
            .map_err(sanitize)
    }

    async fn facts_resource(
        &self,
        reference: &PathReference,
        operation: &OperationGuard,
        acquisition: Option<&ReadAcquisitionLimits>,
    ) -> Result<SourceResource, ResourceError> {
        if reference.projection().is_some() {
            return Err(super::unsupported_github_projection());
        }
        let ResourceAddress::PullRequest(address) = reference.address() else {
            return Err(super::unsupported_github_projection());
        };
        let PullRequestAddress::Item {
            repository, number, ..
        } = address
        else {
            return Err(super::unsupported_github_projection());
        };
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
        let mut budget = HttpReadBudget::with_limits(&limits);
        let endpoint = self.endpoint(repository, &format!("pulls/{}", number.get()))?;
        let response = self
            .fetch_controlled(endpoint.clone(), GITHUB_JSON, read, Some(&mut budget))
            .await?;
        let pull: NativePull = serde_json::from_slice(response.body())
            .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
        let web = Url::parse(web_origin).map_err(|_| failure(ErrorReason::UpstreamUnavailable))?;
        let identity::ValidatedIdentity {
            id,
            number: observed_number,
            base,
            head,
            base_sha,
            head_sha,
        } = identity::validate(
            &pull,
            repository,
            number.get(),
            &endpoint,
            &self.api_base,
            &web,
        )?;
        let mut unavailable_facts = Vec::new();
        macro_rules! missing { ($($field:expr => $name:literal),* $(,)?) => { $(
            $field.unavailable($name, &mut unavailable_facts);
        )* }; }
        missing!(pull.node_id => "nodeId", pull.title => "title", pull.body => "body", pull.state => "state",
            pull.user => "author", pull.created_at => "createdAt", pull.updated_at => "updatedAt",
            pull.closed_at => "closedAt", pull.merged_at => "mergedAt", pull.draft => "draft", pull.merged => "merged",
            pull.html_url => "links.htmlUrl", pull.diff_url => "links.diffUrl", pull.patch_url => "links.patchUrl",
            pull.issue_url => "links.issueUrl", pull._links => "links.relations", base.r#ref => "base.refName",
            base.repo => "base.repository", head.r#ref => "head.refName", head.repo => "head.repository");
        let completed_at_unix_ms = unix_ms()?;
        let facts = Facts {
            schema_version: SchemaVersion { major: 1, minor: 0 },
            kind: "github.pull_request",
            resource: canonical.requested(),
            source: Source {
                source_id: self.config.id(),
                deployment: Deployment {
                    web_origin,
                    api_origin: self.api_base.origin().ascii_serialization(),
                },
            },
            repository: RequestedRepository {
                owner: repository.owner(),
                name: repository.repository(),
                observed: &base.repo,
            },
            request: Request {
                repository: RepositoryName {
                    owner: repository.owner(),
                    name: repository.repository(),
                },
                number: observed_number,
            },
            observed: Observed {
                id,
                number: observed_number,
                node_id: &pull.node_id,
                base: ObservedBranch {
                    commit_sha: base_sha,
                    repository: &base.repo,
                    repository_availability: base.repo.availability(),
                },
                head: ObservedBranch {
                    commit_sha: head_sha,
                    repository: &head.repo,
                    repository_availability: head.repo.availability(),
                },
            },
            acquisition: Acquisition {
                started_at_unix_ms,
                completed_at_unix_ms,
                elapsed_ms: started.elapsed().as_millis() as u64,
                rest_api_version: GITHUB_API_VERSION,
                limits: Limits {
                    max_attempts: limits.max_attempts(),
                    timeout_ms: u64::try_from(limits.timeout().as_millis())
                        .expect("bounded deadline fits in milliseconds"),
                    max_response_bytes: limits.max_response_bytes(),
                    max_accepted_body_bytes: limits.max_accepted_body_bytes(),
                    max_representation_bytes: limits.max_representation_bytes(),
                },
                usage: Usage {
                    attempted_requests: budget.used_attempts(),
                    accepted_body_bytes: budget.accepted_body_bytes(),
                },
            },
            upstream: Upstream {
                body: &response.observation,
                revalidation: &response.revalidation,
            },
            data: Data {
                id,
                number: observed_number,
                node_id: &pull.node_id,
                title: &pull.title,
                body: &pull.body,
                state: &pull.state,
                author: &pull.user,
                created_at: &pull.created_at,
                updated_at: &pull.updated_at,
                closed_at: &pull.closed_at,
                merged_at: &pull.merged_at,
                draft: &pull.draft,
                merged: &pull.merged,
                links: PullLinks {
                    api_url: &pull.url,
                    html_url: &pull.html_url,
                    diff_url: &pull.diff_url,
                    patch_url: &pull.patch_url,
                    issue_url: &pull.issue_url,
                    relations: &pull._links,
                },
                base: BranchFacts {
                    ref_name: &base.r#ref,
                    commit_sha: base_sha,
                    repository: &base.repo,
                    repository_availability: base.repo.availability(),
                },
                head: BranchFacts {
                    ref_name: &head.r#ref,
                    commit_sha: head_sha,
                    repository: &head.repo,
                    repository_availability: head.repo.availability(),
                },
            },
            unavailable_facts,
        };
        let resource = finish_facts(
            &facts,
            limits.max_representation_bytes(),
            canonical.clone(),
            read,
        )?;
        if self
            .session
            .cache_generation(super::fetch::GITHUB_CACHE_NAMESPACE)
            .await?
            != response.cache_generation
        {
            return Err(failure(ErrorReason::UpstreamUnavailable));
        }
        read.check_acceptance()?;
        Ok(resource)
    }
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
