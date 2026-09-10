//! Bounded discussion-collection acquisition and honest coverage.
//!
//! Conversation comments, review submissions and inline review comments share
//! this one engine; `family` supplies the native decoding, validation and
//! projection for whichever collection a read addressed.
//!
//! One logical read owns a single attempt/deadline/byte budget. A page is
//! admitted only when its records and the resulting final representation both
//! fit; otherwise earlier verified pages survive with explicit coverage.
use std::collections::HashSet;

use resourcefs_core::{
    AcquisitionLimitKind, ErrorCategory, ErrorReason, GithubRepositoryIdentity, LimitDetail,
    MAX_COLLECTION_RECORDS, PullRequestNumber, ResourceError, ResourceErrorDetails, SourceResource,
};
use serde::Serialize;
use url::Url;

use super::super::GithubSource;
use super::super::fetch::BodyObservation;
use super::{
    Acquisition, Facts, FactsRead, NativeId, RepositoryName, Unavailable, Upstream, acquisition,
    continuation, failure,
    family::{self, CollectionFamily, Record, RecordView},
    finish_facts, identity, parent, pull, serialized_len,
};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum CollectionScope {
    Initial,
    Continuation,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum CoverageState {
    Complete,
    Incomplete,
    Unknown,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalLimit {
    kind: &'static str,
    bound: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
enum RetryGuidanceFacts {
    DelaySeconds(u64),
    AtUnixSeconds(u64),
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureFacts {
    category: &'static str,
    reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    access_ambiguity: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_guidance: Option<RetryGuidanceFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_limit_reset_unix: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<LocalLimit>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Inconsistency {
    reason: &'static str,
}

/// Typed policy for the terminal collection outcome. The wire fields remain
/// stable, but impossible state/stop combinations are not constructible by
/// callers.
enum CollectionStop {
    Complete,
    Incomplete {
        local_limit: Option<LocalLimit>,
        failure: Option<FailureFacts>,
        continuation: Option<String>,
    },
    Unknown {
        inconsistency: Inconsistency,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Collection<'a> {
    scope: CollectionScope,
    state: CoverageState,
    accepted_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_limit: Option<LocalLimit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure: Option<FailureFacts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    continuation: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inconsistency: Option<Inconsistency>,
}

fn collection<'a>(
    scope: CollectionScope,
    accepted_count: usize,
    stop: &'a CollectionStop,
) -> Collection<'a> {
    match stop {
        CollectionStop::Complete => Collection {
            scope,
            state: CoverageState::Complete,
            accepted_count,
            local_limit: None,
            failure: None,
            continuation: None,
            inconsistency: None,
        },
        CollectionStop::Incomplete {
            local_limit,
            failure,
            continuation,
        } => Collection {
            scope,
            state: CoverageState::Incomplete,
            accepted_count,
            local_limit: local_limit.clone(),
            failure: failure.clone(),
            continuation: continuation.as_deref(),
            inconsistency: None,
        },
        CollectionStop::Unknown { inconsistency } => Collection {
            scope,
            state: CoverageState::Unknown,
            accepted_count,
            local_limit: None,
            failure: None,
            continuation: None,
            inconsistency: Some(inconsistency.clone()),
        },
    }
}

struct PageObservation {
    request_url: Url,
    record_count: usize,
    body: BodyObservation,
    revalidation: Option<BodyObservation>,
}

#[derive(Serialize)]
struct Pages<'a> {
    pages: Vec<Upstream<'a>>,
}

#[derive(Serialize)]
struct CollectionRequest<'a> {
    repository: RepositoryName<'a>,
    number: NativeId,
}

#[derive(Serialize)]
struct CollectionObserved<'a> {
    parent: parent::ParentFacts<'a>,
}

#[derive(Serialize)]
struct Records<'a> {
    records: Vec<RecordView<'a>>,
}

#[derive(Serialize)]
struct CollectionBody<'a> {
    request: CollectionRequest<'a>,
    observed: CollectionObserved<'a>,
    upstream: Pages<'a>,
    data: Records<'a>,
    collection: Collection<'a>,
}
fn failure_facts(error: &ResourceError) -> FailureFacts {
    let details = error.details();
    FailureFacts {
        category: error.category().as_str(),
        reason: details.map_or("upstream_unavailable", |details| details.reason().as_str()),
        http_status: details
            .and_then(|details| details.http_status())
            .map(|status| status.get()),
        access_ambiguity: details
            .and_then(|details| details.access_ambiguity())
            .map(|ambiguity| ambiguity.as_str()),
        retry_guidance: details
            .and_then(|details| details.retry_guidance())
            .map(|guidance| match guidance {
                resourcefs_core::RetryGuidance::DelaySeconds(seconds) => {
                    RetryGuidanceFacts::DelaySeconds(seconds)
                }
                resourcefs_core::RetryGuidance::AtUnixSeconds(seconds) => {
                    RetryGuidanceFacts::AtUnixSeconds(seconds)
                }
            }),
        rate_limit_reset_unix: details.and_then(|details| details.rate_limit_reset()),
        limit: details
            .and_then(|details| details.limit())
            .map(|limit| LocalLimit {
                kind: limit.kind().as_str(),
                bound: limit.bound(),
                observed: limit.observed(),
            }),
    }
}

fn retryable(error: &ResourceError, ctx: &FactsRead<'_>, parent_body_bytes: usize) -> bool {
    if error
        .details()
        .is_some_and(|details| details.reason() == ErrorReason::UpstreamMalformed)
    {
        return false;
    }
    if error.category() == ErrorCategory::LimitExceeded {
        let Some(limit) = error.details().and_then(|details| details.limit()) else {
            return false;
        };
        return match limit.kind() {
            AcquisitionLimitKind::Attempts | AcquisitionLimitKind::ElapsedNanoseconds => true,
            AcquisitionLimitKind::AcceptedBodyBytes => limit
                .observed()
                .and_then(|observed| observed.checked_sub(ctx.budget.accepted_body_bytes() as u64))
                .and_then(|page_bytes| page_bytes.checked_add(parent_body_bytes as u64))
                .is_some_and(|fresh_read_bytes| fresh_read_bytes <= limit.bound()),
            AcquisitionLimitKind::ResponseBodyBytes
            | AcquisitionLimitKind::RepresentationBytes
            | AcquisitionLimitKind::DecodedContentBytes
            | AcquisitionLimitKind::CollectionRecords => false,
        };
    }
    error.category() == ErrorCategory::SourceUnavailable
}

fn rejects_every_page(error: &ResourceError) -> bool {
    matches!(
        error.category(),
        ErrorCategory::Cancelled | ErrorCategory::PermissionDenied
    ) || error
        .details()
        .is_some_and(|details| details.reason() == ErrorReason::UpstreamIdentityMismatch)
}

fn local_limit_error(kind: AcquisitionLimitKind, bound: usize, observed: usize) -> ResourceError {
    let detail = LimitDetail::new(kind, bound as u64, Some(observed as u64))
        .expect("effective local limit bounds are positive");
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        "GitHub collection exceeds its local admission limit",
    )
    .with_details(ResourceErrorDetails::new(ErrorReason::LimitExceeded).with_limit(detail))
}
fn is_representation_limit(error: &ResourceError) -> bool {
    error.category() == ErrorCategory::LimitExceeded
        && error
            .details()
            .and_then(|details| details.limit())
            .is_some_and(|limit| limit.kind() == AcquisitionLimitKind::RepresentationBytes)
}

fn unavailable_for(records: &[Record]) -> Vec<Unavailable> {
    let mut unavailable = Vec::new();
    for record in records {
        note_unavailable(&mut unavailable, family::record_unavailable(record));
    }
    unavailable
}
fn rollback_allowed(stop: &CollectionStop) -> bool {
    match stop {
        CollectionStop::Complete => true,
        CollectionStop::Incomplete { failure, .. } => failure
            .as_ref()
            .is_none_or(|failure| failure.reason != ErrorReason::UpstreamMalformed.as_str()),
        CollectionStop::Unknown { .. } => false,
    }
}

fn note_unavailable(into: &mut Vec<Unavailable>, from: &[Unavailable]) {
    for entry in from {
        if !into
            .iter()
            .any(|existing| existing.field == entry.field && existing.reason == entry.reason)
        {
            into.push(*entry);
        }
    }
}

/// Candidate records cannot be measured without their native observation.
#[derive(Clone, Copy)]
struct CandidatePage<'a> {
    records: &'a [Record],
    observation: &'a PageObservation,
}

struct CollectionView<'a> {
    source: &'a GithubSource,
    repository: &'a GithubRepositoryIdentity,
    number: PullRequestNumber,
    family: CollectionFamily,
    pull: &'a pull::NativePull,
    identity: &'a identity::ValidatedIdentity<'a>,
    scope: CollectionScope,
    resource: &'a str,
    web_origin: &'a str,
}

fn build_facts<'a>(
    view: &CollectionView<'a>,
    records: &'a [Record],
    pages: &'a [PageObservation],
    candidate: Option<CandidatePage<'a>>,
    unavailable: &[Unavailable],
    acquisition: Acquisition,
    stop: &'a CollectionStop,
) -> Facts<'a, CollectionBody<'a>> {
    let extra_records = candidate.map_or(&[][..], |page| page.records);
    let extra_page = candidate.map(|page| page.observation);
    let mut projected = Vec::with_capacity(records.len() + extra_records.len());
    projected.extend(
        records
            .iter()
            .map(|record| family::project_record(record, view.pull, view.identity)),
    );
    projected.extend(
        extra_records
            .iter()
            .map(|record| family::project_record(record, view.pull, view.identity)),
    );
    let mut upstream = Vec::with_capacity(pages.len() + usize::from(extra_page.is_some()));
    upstream.extend(pages.iter().map(|page| Upstream {
        body: &page.body,
        revalidation: &page.revalidation,
    }));
    if let Some(page) = extra_page {
        upstream.push(Upstream {
            body: &page.body,
            revalidation: &page.revalidation,
        });
    }
    let requested_number = NativeId::from_positive(view.number.get());
    Facts {
        schema_version: super::SchemaVersion { major: 1, minor: 1 },
        kind: view.family.kind(),
        resource: view.resource,
        source: super::Source {
            source_id: view.source.config.id(),
            deployment: super::Deployment {
                web_origin: view.web_origin,
                api_origin: view.source.api_base.origin().ascii_serialization(),
            },
        },
        repository: super::RequestedRepository {
            owner: view.repository.owner(),
            name: view.repository.repository(),
            observed: &view.identity.base.repo,
        },
        acquisition,
        body: CollectionBody {
            request: CollectionRequest {
                repository: RepositoryName {
                    owner: view.repository.owner(),
                    name: view.repository.repository(),
                },
                number: requested_number,
            },
            observed: CollectionObserved {
                parent: parent::parent_facts(view.pull, view.identity),
            },
            upstream: Pages { pages: upstream },
            data: Records { records: projected },
            collection: collection(view.scope, records.len() + extra_records.len(), stop),
        },
        unavailable_facts: unavailable.to_vec(),
    }
}

fn measure_candidate<'a>(
    view: &CollectionView<'a>,
    records: &'a [Record],
    pages: &'a [PageObservation],
    candidate: Option<CandidatePage<'a>>,
    unavailable: &[Unavailable],
    acquisition: Acquisition,
    stop: &'a CollectionStop,
) -> Result<usize, ResourceError> {
    serialized_len(&build_facts(
        view,
        records,
        pages,
        candidate,
        unavailable,
        acquisition,
        stop,
    ))
}

fn observed_representation<'a>(
    view: &CollectionView<'a>,
    records: &'a [Record],
    pages: &'a [PageObservation],
    candidate: Option<CandidatePage<'a>>,
    unavailable: &[Unavailable],
    acquisition: Acquisition,
    stop: &'a mut CollectionStop,
) -> Result<usize, ResourceError> {
    let mut observed = 0;
    for _ in 0..4 {
        let length = measure_candidate(
            view,
            records,
            pages,
            candidate,
            unavailable,
            acquisition,
            stop,
        )?;
        if length == observed {
            return Ok(length);
        }
        observed = length;
        if let CollectionStop::Incomplete {
            local_limit: Some(limit),
            ..
        } = stop
            && limit.kind == AcquisitionLimitKind::RepresentationBytes.as_str()
        {
            limit.observed = Some(observed as u64);
        }
    }
    Ok(observed)
}

fn actual_stop(
    owner: &continuation::CursorOwner,
    next: Option<&Url>,
) -> Result<(CollectionStop, Option<resourcefs_core::PathReference>), ResourceError> {
    let Some(next) = next else {
        return Ok((CollectionStop::Complete, None));
    };
    match owner.continuation(next)? {
        Some(reference) => Ok((
            CollectionStop::Incomplete {
                local_limit: None,
                failure: None,
                continuation: Some(reference.requested().to_owned()),
            },
            Some(reference),
        )),
        None => Ok((
            CollectionStop::Unknown {
                inconsistency: Inconsistency {
                    reason: "continuation_unrepresentable",
                },
            },
            None,
        )),
    }
}

pub(super) async fn read(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    family: CollectionFamily,
    cursor: Option<&resourcefs_core::SourceCursor>,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let suffix = family.suffix(number);
    let owner = continuation::CursorOwner::new(ctx.canonical.clone(), source);
    let resumed = cursor
        .map(|cursor| {
            source
                .confine_next(owner.decode(cursor)?, repository, &suffix)
                .map(Some)
        })
        .transpose()?
        .flatten();
    let scope = if resumed.is_some() {
        CollectionScope::Continuation
    } else {
        CollectionScope::Initial
    };
    let (pull, parent_endpoint, parent_generation) =
        parent::fetch_parent(source, repository, number, ctx).await?;
    let parent_body_bytes = ctx.budget.accepted_body_bytes();
    let web = Url::parse(ctx.web_origin).map_err(|_| failure(ErrorReason::UpstreamUnavailable))?;
    let identity =
        parent::validate_parent(&pull, repository, number, &parent_endpoint, source, ctx)?;
    let resource = ctx.canonical.requested().to_owned();
    let web_origin = ctx.web_origin.to_owned();
    let view = CollectionView {
        source,
        repository,
        number,
        family,
        pull: &pull,
        identity: &identity,
        scope,
        resource: &resource,
        web_origin: &web_origin,
    };
    let mut pending = Some(match resumed {
        Some(target) => target,
        None => source.page_url(repository, &suffix, &[], 1)?,
    });
    let mut seen_urls = HashSet::new();
    let mut seen_ids = HashSet::new();
    let mut pages = Vec::new();
    let mut records = Vec::new();
    let mut unavailable = Vec::new();
    let mut terminal = None;
    let mut resume = None;
    while let Some(url) = pending.take() {
        if !seen_urls.insert(url.to_string()) {
            if records.is_empty() {
                return Err(failure(ErrorReason::UpstreamMalformed));
            }
            terminal = Some(CollectionStop::Unknown {
                inconsistency: Inconsistency {
                    reason: "repeated_pagination",
                },
            });
            break;
        }
        ctx.read.check_acceptance()?;
        super::check_generation(source, ctx).await?;
        let page = match source
            .facts_page(url.clone(), repository, &suffix, ctx.read, &mut ctx.budget)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                if records.is_empty() || rejects_every_page(&error) {
                    return Err(error);
                }
                let continuation = if retryable(&error, ctx, parent_body_bytes) {
                    owner.continuation(&url)?
                } else {
                    None
                };
                resume = continuation.clone();
                terminal = Some(CollectionStop::Incomplete {
                    local_limit: None,
                    failure: Some(failure_facts(&error)),
                    continuation: continuation.map(|reference| reference.requested().to_owned()),
                });
                break;
            }
        };
        if page.response.cache_generation != parent_generation {
            return Err(failure(ErrorReason::UpstreamUnavailable));
        }
        let response = page.response;
        let next = page.next;
        let decoded = match family.decode(response.body()) {
            Ok(decoded) => decoded,
            Err(_) => {
                let error = failure(ErrorReason::UpstreamMalformed);
                if records.is_empty() {
                    return Err(error);
                }
                terminal = Some(CollectionStop::Incomplete {
                    local_limit: None,
                    failure: Some(failure_facts(&error)),
                    continuation: None,
                });
                break;
            }
        };
        let mut page_observation = PageObservation {
            request_url: url.clone(),
            record_count: 0,
            body: response.observation,
            revalidation: response.revalidation,
        };
        let mut page_records = Vec::with_capacity(decoded.len());
        for native in decoded {
            match family::validate_native(native, repository, number, &source.api_base, &web) {
                Ok(record) => page_records.push(record),
                Err(error) if rejects_every_page(&error) => return Err(error),
                Err(error) => {
                    if records.is_empty() {
                        return Err(error);
                    }
                    terminal = Some(CollectionStop::Incomplete {
                        local_limit: None,
                        failure: Some(failure_facts(&error)),
                        continuation: None,
                    });
                    break;
                }
            }
        }
        let mut candidate_unavailable = unavailable.clone();
        for record in &page_records {
            note_unavailable(
                &mut candidate_unavailable,
                family::record_unavailable(record),
            );
        }
        if terminal.is_some() {
            break;
        }
        let mut page_ids = HashSet::new();
        if page_records.iter().any(|record| {
            !page_ids.insert(family::record_id(record))
                || seen_ids.contains(&family::record_id(record))
        }) {
            if records.is_empty() {
                return Err(failure(ErrorReason::UpstreamMalformed));
            }
            terminal = Some(CollectionStop::Unknown {
                inconsistency: Inconsistency {
                    reason: "duplicate_record_id",
                },
            });
            break;
        }
        let candidate_count = records.len() + page_records.len();
        if candidate_count > MAX_COLLECTION_RECORDS {
            if records.is_empty() {
                return Err(local_limit_error(
                    AcquisitionLimitKind::CollectionRecords,
                    MAX_COLLECTION_RECORDS,
                    candidate_count,
                ));
            }
            let continuation = owner.continuation(&url)?;
            resume = continuation.clone();
            terminal = Some(CollectionStop::Incomplete {
                local_limit: Some(LocalLimit {
                    kind: AcquisitionLimitKind::CollectionRecords.as_str(),
                    bound: MAX_COLLECTION_RECORDS as u64,
                    observed: Some(candidate_count as u64),
                }),
                failure: None,
                continuation: continuation.map(|reference| reference.requested().to_owned()),
            });
            break;
        }
        let (candidate_stop, _) = actual_stop(&owner, next.as_ref())?;
        let candidate_length = measure_candidate(
            &view,
            &records,
            &pages,
            Some(CandidatePage {
                records: &page_records,
                observation: &page_observation,
            }),
            &candidate_unavailable,
            acquisition(ctx)?,
            &candidate_stop,
        )?;
        let admitted_length = candidate_length;
        if admitted_length > ctx.limits.max_representation_bytes() {
            if records.is_empty() {
                return Err(local_limit_error(
                    AcquisitionLimitKind::RepresentationBytes,
                    ctx.limits.max_representation_bytes(),
                    candidate_length,
                ));
            }
            let continuation = owner.continuation(&url)?;
            resume = continuation.clone();
            let mut stop = CollectionStop::Incomplete {
                local_limit: Some(LocalLimit {
                    kind: AcquisitionLimitKind::RepresentationBytes.as_str(),
                    bound: ctx.limits.max_representation_bytes() as u64,
                    observed: Some(0),
                }),
                failure: None,
                continuation: continuation.map(|reference| reference.requested().to_owned()),
            };
            let observed = observed_representation(
                &view,
                &records,
                &pages,
                Some(CandidatePage {
                    records: &page_records,
                    observation: &page_observation,
                }),
                &candidate_unavailable,
                acquisition(ctx)?,
                &mut stop,
            )?;
            if let CollectionStop::Incomplete { local_limit, .. } = &mut stop
                && let Some(local_limit) = local_limit
            {
                local_limit.observed = Some(observed as u64);
            }
            terminal = Some(stop);
            break;
        }
        for record in &page_records {
            seen_ids.insert(family::record_id(record));
        }
        unavailable = candidate_unavailable;
        page_observation.record_count = page_records.len();
        records.extend(page_records);
        pages.push(page_observation);
        pending = next;
    }
    let mut terminal = terminal.unwrap_or(CollectionStop::Complete);
    let resource = loop {
        let facts = build_facts(
            &view,
            &records,
            &pages,
            None,
            &unavailable,
            acquisition(ctx)?,
            &terminal,
        );
        match finish_facts(
            &facts,
            ctx.limits.max_representation_bytes(),
            ctx.canonical.clone(),
            ctx.read,
        ) {
            Ok(resource) => break resource,
            Err(error) if is_representation_limit(&error) && rollback_allowed(&terminal) => {
                let Some(page) = pages.pop() else {
                    return Err(error);
                };
                let retained = records
                    .len()
                    .checked_sub(page.record_count)
                    .expect("retained page records belong to this collection");
                if retained == 0 {
                    return Err(error);
                }
                records.truncate(retained);
                unavailable = unavailable_for(&records);
                let continuation = owner.continuation(&page.request_url)?;
                resume = continuation.clone();
                terminal = CollectionStop::Incomplete {
                    local_limit: Some(LocalLimit {
                        kind: AcquisitionLimitKind::RepresentationBytes.as_str(),
                        bound: ctx.limits.max_representation_bytes() as u64,
                        observed: error
                            .details()
                            .and_then(|details| details.limit())
                            .and_then(|limit| limit.observed()),
                    }),
                    failure: None,
                    continuation: continuation.map(|reference| reference.requested().to_owned()),
                };
            }
            Err(error) => return Err(error),
        }
    };
    super::check_generation(source, ctx).await?;
    ctx.read.check_acceptance()?;
    Ok(match resume {
        Some(reference) => resource.with_continuation(&reference),
        None => resource,
    })
}
