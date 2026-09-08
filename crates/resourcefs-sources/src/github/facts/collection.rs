//! Bounded conversation-comment collection acquisition and honest coverage.
//!
//! One logical read owns a single attempt/deadline/byte budget. A page is
//! admitted only when its records and the resulting final representation both
//! fit; otherwise earlier verified pages survive with explicit coverage.
use std::io;

use resourcefs_core::{
    ErrorCategory, ErrorReason, MAX_COLLECTION_RECORDS, ResourceError, SourceResource,
};
use serde::Serialize;
use url::Url;

use super::super::GithubSource;
use super::super::fetch::BodyObservation;
use super::{
    Acquisition, Facts, FactsRead, NativeId, RepositoryName, Unavailable, Upstream, acquisition,
    comment, failure, finish_facts, identity,
};

/// Bounded reserve for coverage facts, acquisition digit growth and one page
/// observation entry, on top of the measured envelope. A continuation is not
/// yet issued by this slice; when it is, its exact encoded length joins this
/// reservation before the page that would name it is admitted.
const OUTCOME_RESERVE_BYTES: usize = 512;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalLimit {
    kind: &'static str,
    bound: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed: Option<u64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureFacts {
    category: &'static str,
    reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    access_ambiguity: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_limit_reset_unix: Option<u64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Inconsistency {
    reason: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Collection {
    state: &'static str,
    accepted_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_limit: Option<LocalLimit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<FailureFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inconsistency: Option<Inconsistency>,
}
#[derive(Serialize)]
struct Pages<'a> {
    pages: Vec<Upstream<'a>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CollectionRequest<'a> {
    repository: RepositoryName<'a>,
    number: &'a NativeId,
}
#[derive(Serialize)]
struct CollectionObserved<'a> {
    parent: comment::ParentFacts<'a>,
}
#[derive(Serialize)]
struct Records<'a> {
    records: Vec<comment::CommentRecord<'a>>,
}
#[derive(Serialize)]
struct CollectionBody<'a> {
    request: CollectionRequest<'a>,
    observed: CollectionObserved<'a>,
    upstream: Pages<'a>,
    data: Records<'a>,
    collection: Collection,
}

/// Effective acquisition metadata with zeroed observations, used only to
/// measure the fixed envelope bytes before any page is admitted.
fn probe_acquisition(ctx: &FactsRead<'_>) -> Acquisition {
    Acquisition {
        started_at_unix_ms: 0,
        completed_at_unix_ms: 0,
        elapsed_ms: 0,
        rest_api_version: super::GITHUB_API_VERSION,
        limits: super::Limits {
            max_attempts: ctx.limits.max_attempts(),
            timeout_ms: ctx.limits.timeout().as_nanos() as f64 / 1_000_000.0,
            max_response_bytes: ctx.limits.max_response_bytes(),
            max_accepted_body_bytes: ctx.limits.max_accepted_body_bytes(),
            max_representation_bytes: ctx.limits.max_representation_bytes(),
        },
        usage: super::Usage {
            attempted_requests: 0,
            accepted_body_bytes: 0,
        },
    }
}

/// Bounded sanitized failure facts derived from one operational error.
fn failure_facts(error: &ResourceError) -> FailureFacts {
    let details = error.details();
    FailureFacts {
        category: error.category().as_str(),
        reason: details.map_or("upstream_unavailable", |details| details.reason().as_str()),
        http_status: details
            .and_then(|details| details.http_status())
            .map(|s| s.get()),
        access_ambiguity: details
            .and_then(|details| details.access_ambiguity())
            .map(|a| a.as_str()),
        retry_after_seconds: details
            .and_then(|details| details.retry_guidance())
            .map(|guidance| match guidance {
                resourcefs_core::RetryGuidance::DelaySeconds(seconds) => seconds,
                resourcefs_core::RetryGuidance::AtUnixSeconds(seconds) => seconds,
            }),
        rate_limit_reset_unix: details.and_then(|details| details.rate_limit_reset()),
    }
}

/// Exact serialized byte length without retaining the encoded document.
fn serialized_len<T: Serialize>(value: &T) -> Result<usize, ResourceError> {
    struct Counter(usize);
    impl io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    Ok(counter.0)
}

/// Whether an error rejects the whole read even after earlier pages were
/// admitted: explicit cancellation and authority failures are never softened
/// into a partial result.
fn rejects_every_page(error: &ResourceError) -> bool {
    matches!(
        error.category(),
        ErrorCategory::Cancelled | ErrorCategory::PermissionDenied
    )
}

/// A page the local ceilings refuse before any record is admitted is a typed
/// failure, not an empty or partial collection.
fn local_limit_error(kind: &'static str, bound: usize, observed: usize) -> ResourceError {
    use resourcefs_core::{AcquisitionLimitKind, ErrorCategory as Category, LimitDetail};

    let (kind, category_kind) = match kind {
        "records" => ("records", AcquisitionLimitKind::CollectionRecords),
        _ => (
            "representation_bytes",
            AcquisitionLimitKind::RepresentationBytes,
        ),
    };
    let detail = LimitDetail::new(category_kind, bound as u64, Some(observed as u64));
    let mut error = ResourceError::new(
        Category::LimitExceeded,
        "GitHub collection exceeds its local admission limit",
    );
    if let Ok(detail) = detail {
        error = error.with_details(
            resourcefs_core::ResourceErrorDetails::new(ErrorReason::LimitExceeded)
                .with_limit(detail),
        );
    }
    let _ = kind;
    error
}

/// Merges a record's unavailable facts into the collection-level list without
/// duplicates, so the list stays bounded by the finite field vocabulary.
fn note_unavailable(into: &mut Vec<Unavailable>, from: Vec<Unavailable>) {
    for entry in from {
        if !into
            .iter()
            .any(|existing| existing.field == entry.field && existing.reason == entry.reason)
        {
            into.push(entry);
        }
    }
}

pub(super) async fn read(
    source: &GithubSource,
    repository: &resourcefs_core::GithubRepositoryIdentity,
    number: u64,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let (pull, parent_endpoint) = comment::fetch_parent(source, repository, number, ctx).await?;
    let web = Url::parse(ctx.web_origin).map_err(|_| failure(ErrorReason::UpstreamUnavailable))?;
    let identity = identity::validate(
        &pull,
        repository,
        number,
        &parent_endpoint,
        &source.api_base,
        &web,
    )?;
    let suffix = format!("issues/{number}/comments");
    // Measure the fixed part of the document once: envelope, request, observed
    // parent and an empty data/collection pair. Admission then compares the
    // real record bytes against the remaining ceiling.
    let envelope_bytes = serialized_len(&Facts {
        schema_version: super::SchemaVersion { major: 1, minor: 0 },
        kind: "github.conversation_comment_collection",
        resource: ctx.canonical.requested(),
        source: super::Source {
            source_id: source.config.id(),
            deployment: super::Deployment {
                web_origin: ctx.web_origin,
                api_origin: source.api_base.origin().ascii_serialization(),
            },
        },
        repository: super::RequestedRepository {
            owner: repository.owner(),
            name: repository.repository(),
            observed: &identity.base.repo,
        },
        acquisition: probe_acquisition(ctx),
        body: CollectionBody {
            request: CollectionRequest {
                repository: RepositoryName {
                    owner: repository.owner(),
                    name: repository.repository(),
                },
                number: identity.number,
            },
            observed: CollectionObserved {
                parent: comment::parent_facts(&pull, &identity),
            },
            upstream: Pages { pages: Vec::new() },
            data: Records {
                records: Vec::new(),
            },
            collection: Collection {
                state: "complete",
                accepted_count: 0,
                local_limit: None,
                failure: None,
                inconsistency: None,
            },
        },
        unavailable_facts: Vec::new(),
    })?;
    let mut next = Some(source.page_url(repository, &suffix, &[], 1)?);
    let mut seen: Vec<String> = Vec::new();
    let mut observations: Vec<BodyObservation> = Vec::new();
    let mut revalidations: Vec<Option<BodyObservation>> = Vec::new();
    let mut comments: Vec<comment::NativeComment> = Vec::new();
    let mut unavailable_facts: Vec<Unavailable> = Vec::new();
    let mut accepted_bytes = 0_usize;
    let mut state = "complete";
    let mut local_limit = None;
    let mut recorded_failure = None;
    let mut inconsistency = None;
    while let Some(url) = next.take() {
        if seen.iter().any(|seen| seen == url.as_str()) {
            // Observed pagination does not progress; never loop and never
            // claim the collection is complete or a snapshot.
            state = "unknown";
            inconsistency = Some(Inconsistency {
                reason: "repeated_pagination",
            });
            break;
        }
        if let Err(error) = ctx.read.check_acceptance() {
            if comments.is_empty() || rejects_every_page(&error) {
                return Err(error);
            }
            state = "incomplete";
            recorded_failure = Some(failure_facts(&error));
            break;
        }
        let page = match source
            .facts_page(url.clone(), repository, &suffix, ctx.read, &mut ctx.budget)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                if comments.is_empty() || rejects_every_page(&error) {
                    return Err(error);
                }
                state = "incomplete";
                recorded_failure = Some(failure_facts(&error));
                break;
            }
        };
        seen.push(url.to_string());
        if source
            .session
            .cache_generation(super::super::fetch::GITHUB_CACHE_NAMESPACE)
            .await?
            != page.cache_generation
        {
            let error = failure(ErrorReason::UpstreamUnavailable);
            if comments.is_empty() || rejects_every_page(&error) {
                return Err(error);
            }
            state = "incomplete";
            recorded_failure = Some(failure_facts(&error));
            break;
        }
        let decoded: Vec<comment::NativeComment> = match serde_json::from_slice(&page.body) {
            Ok(decoded) => decoded,
            Err(_) => {
                let error = failure(ErrorReason::UpstreamMalformed);
                if comments.is_empty() || rejects_every_page(&error) {
                    return Err(error);
                }
                state = "incomplete";
                recorded_failure = Some(failure_facts(&error));
                break;
            }
        };
        // Validate the whole page before admitting any of it.
        let mut page_unavailable = Vec::new();
        let mut page_records = Vec::with_capacity(decoded.len());
        for record in &decoded {
            match comment::record(
                record,
                &pull,
                &identity,
                repository,
                number,
                &mut page_unavailable,
            ) {
                Ok(projected) => page_records.push(projected),
                Err(error) => return Err(error),
            }
        }
        // `page_bytes` includes the array brackets, which are re-added once in
        // `array_bytes`; subtract them here.
        let page_bytes = serialized_len(&page_records)?;
        let candidate_records = comments.len() + decoded.len();
        if candidate_records > MAX_COLLECTION_RECORDS {
            if comments.is_empty() {
                return Err(local_limit_error(
                    "records",
                    MAX_COLLECTION_RECORDS,
                    candidate_records,
                ));
            }
            state = "incomplete";
            local_limit = Some(LocalLimit {
                kind: "records",
                bound: MAX_COLLECTION_RECORDS as u64,
                observed: Some(candidate_records as u64),
            });
            break;
        }
        let separators = candidate_records.saturating_sub(1);
        let array_bytes = accepted_bytes + page_bytes.saturating_sub(2) + separators + 2;
        let pages_reserve = (observations.len() + 1) * OUTCOME_RESERVE_BYTES;
        let candidate_bytes = envelope_bytes + OUTCOME_RESERVE_BYTES + pages_reserve + array_bytes;
        if candidate_bytes > ctx.limits.max_representation_bytes() {
            if comments.is_empty() {
                return Err(local_limit_error(
                    "representation_bytes",
                    ctx.limits.max_representation_bytes(),
                    candidate_bytes,
                ));
            }
            state = "incomplete";
            local_limit = Some(LocalLimit {
                kind: "representation_bytes",
                bound: ctx.limits.max_representation_bytes() as u64,
                observed: Some(candidate_bytes as u64),
            });
            break;
        }
        accepted_bytes = array_bytes;
        note_unavailable(&mut unavailable_facts, page_unavailable);
        observations.push(page.observation);
        revalidations.push(page.revalidation);
        comments.extend(decoded);
        next = page.next;
    }
    let mut projection_unavailable = Vec::new();
    let records = comments
        .iter()
        .map(|record| {
            comment::record(
                record,
                &pull,
                &identity,
                repository,
                number,
                &mut projection_unavailable,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let facts = Facts {
        schema_version: super::SchemaVersion { major: 1, minor: 0 },
        kind: "github.conversation_comment_collection",
        resource: ctx.canonical.requested(),
        source: super::Source {
            source_id: source.config.id(),
            deployment: super::Deployment {
                web_origin: ctx.web_origin,
                api_origin: source.api_base.origin().ascii_serialization(),
            },
        },
        repository: super::RequestedRepository {
            owner: repository.owner(),
            name: repository.repository(),
            observed: &identity.base.repo,
        },
        acquisition: acquisition(ctx)?,
        body: CollectionBody {
            request: CollectionRequest {
                repository: RepositoryName {
                    owner: repository.owner(),
                    name: repository.repository(),
                },
                number: identity.number,
            },
            observed: CollectionObserved {
                parent: comment::parent_facts(&pull, &identity),
            },
            upstream: Pages {
                pages: observations
                    .iter()
                    .zip(&revalidations)
                    .map(|(body, revalidation)| Upstream { body, revalidation })
                    .collect(),
            },
            data: Records { records },
            collection: Collection {
                state,
                accepted_count: comments.len(),
                local_limit,
                failure: recorded_failure,
                inconsistency,
            },
        },
        unavailable_facts,
    };
    finish_facts(
        &facts,
        ctx.limits.max_representation_bytes(),
        ctx.canonical.clone(),
        ctx.read,
    )
}
