//! Exact immutable GitHub source acquisition and facts projection.
use std::collections::BTreeMap;

use resourcefs_core::{
    AcquisitionLimitKind, ErrorCategory, ErrorReason, GithubCommitId, GithubRepositoryIdentity,
    GithubSourcePath, ResourceError, ResourceErrorDetails, SourceResource,
};
use serde::{Serialize, Serializer, ser::SerializeMap};

use super::super::fetch::BodyObservation;
use super::super::{GITHUB_JSON, GithubSource};
use super::{
    Facts, FactsRead, Presence, RepositoryName, RequestedRepository, Source, Upstream,
    acquisition_at, collection, commit, failure, finish_facts, identity, object,
};

#[derive(Serialize)]
struct SourceRequest<'a> {
    repository: RepositoryName<'a>,
    #[serde(rename = "commitSha")]
    commit_sha: &'a str,
    path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceObserved<'a> {
    commit_sha: &'a str,
    tree_sha: &'a str,
    containing_tree_sha: &'a str,
    path: &'a str,
    mode: &'a str,
    object_type: &'a str,
    object_sha: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceData<'a> {
    #[serde(flatten)]
    identity: &'a SourceObserved<'a>,
    /// Kept presence-aware: an upstream `null` and an omitted key are different
    /// observations, and this family's purpose is byte-exact fidelity.
    #[serde(skip_serializing_if = "Presence::omitted")]
    size_bytes: &'a Presence<u64>,
    content: SourceContent,
}

enum SourceContent {
    Available {
        decoded_size_bytes: usize,
        bytes_base64: String,
    },
    DecodedSizeLimit(collection::LocalLimit),
    AcquisitionFailed(collection::FailureFacts),
    NonRegularObject,
    UnsupportedObjectMode,
}

impl Serialize for SourceContent {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // One variable-shape map, the same form the sibling native records use:
        // the arm split below is what makes a new variant a non-exhaustive-match
        // error instead of a silent fallback to another variant's reason.
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Available {
                decoded_size_bytes,
                bytes_base64,
            } => {
                map.serialize_entry("state", "available")?;
                map.serialize_entry("encoding", "base64")?;
                map.serialize_entry("decodedSizeBytes", decoded_size_bytes)?;
                map.serialize_entry("bytesBase64", bytes_base64)?;
            }
            Self::DecodedSizeLimit(limit) => {
                map.serialize_entry("state", "unavailable")?;
                map.serialize_entry("reason", "decoded_size_limit")?;
                map.serialize_entry("limit", limit)?;
            }
            Self::AcquisitionFailed(failure) => {
                map.serialize_entry("state", "unavailable")?;
                map.serialize_entry("reason", "acquisition_failed")?;
                map.serialize_entry("failure", failure)?;
            }
            Self::NonRegularObject => {
                map.serialize_entry("state", "unsupported")?;
                map.serialize_entry("reason", "non_regular_object")?;
            }
            Self::UnsupportedObjectMode => {
                map.serialize_entry("state", "unsupported")?;
                map.serialize_entry("reason", "unsupported_object_mode")?;
            }
        }
        map.end()
    }
}

#[derive(Serialize)]
struct ObjectUpstream {
    body: BodyObservation,
    #[serde(skip_serializing_if = "Option::is_none")]
    revalidation: Option<BodyObservation>,
}

#[derive(Serialize)]
struct SourceUpstream<'a> {
    repository: Upstream<'a>,
    commit: Upstream<'a>,
    trees: BTreeMap<String, ObjectUpstream>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    blobs: BTreeMap<String, ObjectUpstream>,
}

#[derive(Serialize)]
struct SourceBody<'a> {
    request: SourceRequest<'a>,
    observed: &'a SourceObserved<'a>,
    upstream: SourceUpstream<'a>,
    data: SourceData<'a>,
}

struct Terminal {
    containing_tree_sha: String,
    mode: String,
    object_type: String,
    object_sha: String,
    size_bytes: Presence<u64>,
    disposition: object::EntryDisposition,
}

pub(super) async fn read(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    commit_id: &GithubCommitId,
    path: &GithubSourcePath,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let acquired = commit::acquire(source, repository, commit_id, ctx).await?;
    let details = identity::required(&acquired.commit.commit)?;
    let root_tree_sha = identity::sha(&identity::required(&details.tree)?.sha)?;
    let requested_path = path.segments().join("/");
    // A validated `GithubSourcePath` holds at least one component, so the last
    // segment is the terminal one and every earlier segment must descend. The
    // split keeps the terminal entry out of the loop entirely: there is no
    // "did the loop run" question and no failure arm that could blame the
    // provider for a local invariant.
    let (leaf, ancestors) = path
        .segments()
        .split_last()
        .expect("a validated source path has at least one component");
    let mut current_tree_sha = root_tree_sha.to_owned();
    let mut observations = BTreeMap::new();
    for segment in ancestors {
        let entry = read_tree_entry(
            source,
            repository,
            ctx,
            &mut observations,
            &current_tree_sha,
            segment,
        )
        .await?;
        if entry.disposition != object::EntryDisposition::Tree {
            // Nothing upstream is malformed: a verified tree proved the caller's
            // path does not name this object.
            return Err(path_absent());
        }
        current_tree_sha = entry.object_sha;
    }
    let entry = read_tree_entry(
        source,
        repository,
        ctx,
        &mut observations,
        &current_tree_sha,
        leaf,
    )
    .await?;
    let terminal = Terminal {
        containing_tree_sha: current_tree_sha,
        mode: entry.mode,
        object_type: entry.object_type,
        object_sha: entry.object_sha,
        size_bytes: entry.size,
        disposition: entry.disposition,
    };
    let (content, blob) = match terminal.disposition {
        object::EntryDisposition::Blob => read_blob(source, repository, &terminal, ctx).await?,
        object::EntryDisposition::Link
        | object::EntryDisposition::Commit
        | object::EntryDisposition::Tree => (SourceContent::NonRegularObject, None),
        object::EntryDisposition::UnsupportedMode => (SourceContent::UnsupportedObjectMode, None),
    };
    let observed = SourceObserved {
        commit_sha: identity::sha(&acquired.commit.sha)?,
        tree_sha: root_tree_sha,
        containing_tree_sha: &terminal.containing_tree_sha,
        path: &requested_path,
        mode: &terminal.mode,
        object_type: &terminal.object_type,
        object_sha: &terminal.object_sha,
    };
    let mut unavailable_facts = Vec::new();
    missing!(unavailable_facts; terminal.size_bytes => "sizeBytes");
    // The decoded bound governs this read only because this family decodes
    // content, so the caller names it here rather than patching the shared
    // envelope afterwards.
    let acquisition = acquisition_at(
        ctx.limits,
        ctx.budget.used_attempts(),
        ctx.budget.accepted_body_bytes(),
        ctx.started_at_unix_ms,
        ctx.started,
        Some(ctx.limits.max_decoded_bytes()),
    )?;
    let body = SourceBody {
        request: SourceRequest {
            repository: RepositoryName {
                owner: repository.owner(),
                name: repository.repository(),
            },
            commit_sha: commit_id.as_str(),
            path: &requested_path,
        },
        observed: &observed,
        upstream: SourceUpstream {
            repository: Upstream {
                body: &acquired.repository_observation,
                revalidation: &acquired.repository_revalidation,
            },
            commit: Upstream {
                body: &acquired.commit_observation,
                revalidation: &acquired.commit_revalidation,
            },
            trees: observations,
            blobs: blob
                .into_iter()
                .map(|value| (terminal.object_sha.clone(), value))
                .collect(),
        },
        data: SourceData {
            identity: &observed,
            size_bytes: &terminal.size_bytes,
            content,
        },
    };
    let facts = Facts {
        schema_version: super::SchemaVersion { major: 1, minor: 0 },
        kind: "github.source",
        resource: ctx.canonical.requested(),
        source: Source {
            source_id: source.config.id(),
            deployment: super::Deployment {
                web_origin: ctx.web_origin,
                api_origin: source.api_base.origin().ascii_serialization(),
            },
        },
        repository: RequestedRepository {
            owner: repository.owner(),
            name: repository.repository(),
            observed: &acquired.repository,
        },
        acquisition,
        body,
        unavailable_facts,
    };
    let resource = finish_facts(
        &facts,
        ctx.limits.max_representation_bytes(),
        ctx.canonical.clone(),
        ctx.read,
    )?;
    super::check_generation(source, ctx).await?;
    ctx.read.check_acceptance()?;
    Ok(resource)
}

/// Fetch one non-recursive tree, verify it against its native Git object hash,
/// and return the entry its directory listing selects.
///
/// The hash reconstruction is synchronous CPU work over the whole tree, so it
/// runs on a blocking worker rather than inside the polled read future.
async fn read_tree_entry(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    ctx: &mut FactsRead<'_>,
    observations: &mut BTreeMap<String, ObjectUpstream>,
    tree_sha: &str,
    segment: &str,
) -> Result<object::VerifiedEntry, ResourceError> {
    ctx.read.check_acceptance()?;
    // Checked before the attempt is charged: a body invalidated by a concurrent
    // mutation fails the whole read either way, and refusing here spends none of
    // the shared attempt budget on it.
    super::check_generation(source, ctx).await?;
    let endpoint = source.endpoint(repository, &format!("git/trees/{tree_sha}"))?;
    let response = source
        .fetch_controlled(
            endpoint.clone(),
            GITHUB_JSON,
            ctx.read,
            Some(&mut ctx.budget),
        )
        .await?;
    commit::accept_generation(ctx, response.cache_generation)?;
    let native: object::NativeGitTree = serde_json::from_slice(response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let requested = tree_sha.to_owned();
    let tree = tokio::task::spawn_blocking(move || object::validate_tree(native, &requested))
        .await
        .map_err(verification_worker_error)??;
    // Presence is checked before identity: an absent or null link is a
    // malformed document, while one naming a foreign object is a contradiction.
    identity::required(&tree.url)?;
    identity::require_object_link(&tree.url, &endpoint)?;
    let entry = tree
        .entries
        .into_iter()
        .find(|entry| entry.name == segment)
        .ok_or_else(path_absent)?;
    validate_selected_link(source, repository, &entry)?;
    observations.insert(
        tree.sha,
        ObjectUpstream {
            body: response.observation,
            revalidation: response.revalidation,
        },
    );
    Ok(entry)
}

/// A path a complete verified tree proved is not there. The caller asked for
/// something that does not exist; nothing upstream contradicted itself.
fn path_absent() -> ResourceError {
    ResourceError::new(
        ErrorCategory::NotFound,
        "GitHub source path was not present in a complete verified tree",
    )
    .with_details(ResourceErrorDetails::new(
        ErrorReason::UpstreamNotFoundOrHidden,
    ))
}

async fn read_blob(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    terminal: &Terminal,
    ctx: &mut FactsRead<'_>,
) -> Result<(SourceContent, Option<ObjectUpstream>), ResourceError> {
    if terminal
        .size_bytes
        .value()
        .is_some_and(|size| *size > ctx.limits.max_decoded_bytes() as u64)
    {
        // The supplied tree-entry size is upstream metadata, and Git tree
        // objects carry no size the reconstructed tree hash could cover, so the
        // refusal names its bound without presenting that number as an
        // observation of bytes this read measured.
        return Ok((
            SourceContent::DecodedSizeLimit(decoded_limit(ctx, None)),
            None,
        ));
    }
    let endpoint = source.endpoint(repository, &format!("git/blobs/{}", terminal.object_sha))?;
    let response = match source
        .fetch_controlled(
            endpoint.clone(),
            GITHUB_JSON,
            ctx.read,
            Some(&mut ctx.budget),
        )
        .await
    {
        Ok(response) => response,
        Err(error) if retains_verified_metadata(&error) => {
            return Ok((
                SourceContent::AcquisitionFailed(collection::failure_facts(&error)),
                None,
            ));
        }
        Err(error) => return Err(error),
    };
    commit::accept_generation(ctx, response.cache_generation)?;
    let native: object::NativeBlob = serde_json::from_slice(response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    identity::required(&native.url)?;
    identity::require_object_link(&native.url, &endpoint)?;
    let requested_sha = terminal.object_sha.clone();
    let expected_size = terminal.size_bytes.value().copied();
    let decoded_cap = ctx.limits.max_decoded_bytes();
    let content = match tokio::task::spawn_blocking(move || {
        object::validate_blob(native, &requested_sha, expected_size, decoded_cap)
    })
    .await
    .map_err(verification_worker_error)??
    {
        object::BlobResult::Available(blob) => SourceContent::Available {
            decoded_size_bytes: blob.decoded_size,
            bytes_base64: blob.bytes_base64,
        },
        object::BlobResult::Oversized(blob) => {
            SourceContent::DecodedSizeLimit(decoded_limit(ctx, Some(blob.decoded_size as u64)))
        }
    };
    Ok((
        content,
        Some(ObjectUpstream {
            body: response.observation,
            revalidation: response.revalidation,
        }),
    ))
}

fn verification_worker_error(error: tokio::task::JoinError) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!("GitHub object verification worker failed: {error}"),
    )
}

/// Confine the selected entry's supplied link to this deployment's own object.
///
/// Only the selected entry is checked. The tree hash reconstructed immediately
/// before authenticates the whole entry set, this read publishes exactly one
/// entry, and every other entry's link is never read, retained or followed.
fn validate_selected_link(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    entry: &object::VerifiedEntry,
) -> Result<(), ResourceError> {
    let suffix = match entry.object_type.as_str() {
        "tree" => format!("git/trees/{}", entry.object_sha),
        "blob" => format!("git/blobs/{}", entry.object_sha),
        // A submodule's commit is an object of the submodule's own repository,
        // so there is no containing-repository URL to expect for it — and
        // GitHub omits the field on exactly those entries.
        "commit" => return Ok(()),
        _ => return Err(failure(ErrorReason::UpstreamMalformed)),
    };
    let expected = identity::expected_object_url(&source.api_base, repository, &suffix)?;
    identity::validate_optional_object_link(&entry.url, &expected)
}

/// Whether a blob-stage failure may still publish the verified terminal identity.
///
/// The source contract names its whole-read refusals: integrity, identity and
/// link contradictions, cancellation, deadline expiration, artifact overflow and
/// cache-generation invalidation. Everything else that can go wrong while
/// fetching the one blob of an already-verified path — an outage, a rate limit,
/// a lapsed credential, a hidden object, a dropped connection — is an ordinary
/// acquisition failure of that single request, so its sanitized facts are
/// published beside the identity the trees already proved. The allowlist is
/// deliberate: a new error kind must be classified here rather than retained by
/// a catch-all.
fn retains_verified_metadata(error: &ResourceError) -> bool {
    let reason = error.details().map(|details| details.reason());
    match error.category() {
        ErrorCategory::SourceUnavailable => matches!(
            reason,
            Some(
                ErrorReason::UpstreamUnavailable
                    | ErrorReason::UpstreamRateLimited
                    | ErrorReason::TransportFailure
            )
        ),
        ErrorCategory::PermissionDenied => matches!(reason, Some(ErrorReason::UpstreamDenied)),
        ErrorCategory::NotFound => matches!(reason, Some(ErrorReason::UpstreamNotFoundOrHidden)),
        _ => false,
    }
}

fn decoded_limit(ctx: &FactsRead<'_>, observed: Option<u64>) -> collection::LocalLimit {
    collection::LocalLimit::new(
        AcquisitionLimitKind::DecodedContentBytes,
        ctx.limits.max_decoded_bytes() as u64,
        observed,
    )
}
