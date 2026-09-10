//! Exact immutable GitHub source acquisition and facts projection.
use std::collections::BTreeMap;

use resourcefs_core::{
    AcquisitionLimitKind, ErrorCategory, ErrorReason, GithubCommitId, GithubRepositoryIdentity,
    GithubSourcePath, ResourceError, ResourceErrorDetails, SourceResource,
};
use serde::{Serialize, Serializer, ser::SerializeStruct};

use super::super::fetch::BodyObservation;
use super::super::{GITHUB_JSON, GithubSource};
use super::{
    Facts, FactsRead, RepositoryName, RequestedRepository, Source, Upstream, acquisition,
    collection, commit, failure, finish_facts, identity, object,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
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
        let fields = match self {
            Self::Available { .. } => 4,
            Self::DecodedSizeLimit(_) | Self::AcquisitionFailed(_) => 3,
            Self::NonRegularObject | Self::UnsupportedObjectMode => 2,
        };
        let mut map = serializer.serialize_struct("SourceContent", fields)?;
        match self {
            Self::Available {
                decoded_size_bytes,
                bytes_base64,
            } => {
                map.serialize_field("state", "available")?;
                map.serialize_field("encoding", "base64")?;
                map.serialize_field("decodedSizeBytes", decoded_size_bytes)?;
                map.serialize_field("bytesBase64", bytes_base64)?;
            }
            Self::DecodedSizeLimit(limit) => {
                map.serialize_field("state", "unavailable")?;
                map.serialize_field("reason", "decoded_size_limit")?;
                map.serialize_field("limit", limit)?;
            }
            Self::AcquisitionFailed(failure) => {
                map.serialize_field("state", "unavailable")?;
                map.serialize_field("reason", "acquisition_failed")?;
                map.serialize_field("failure", failure)?;
            }
            Self::NonRegularObject | Self::UnsupportedObjectMode => {
                map.serialize_field("state", "unsupported")?;
                let reason = match self {
                    Self::NonRegularObject => "non_regular_object",
                    _ => "unsupported_object_mode",
                };
                map.serialize_field("reason", reason)?;
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
    size_bytes: Option<u64>,
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
    let mut current_tree_sha = root_tree_sha.to_owned();
    let mut observations = BTreeMap::new();
    let mut terminal = None;
    for (index, segment) in path.segments().iter().enumerate() {
        ctx.read.check_acceptance()?;
        super::check_generation(source, ctx).await?;
        let endpoint = source.endpoint(repository, &format!("git/trees/{current_tree_sha}"))?;
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
        let tree = object::validate_tree(native, &current_tree_sha)?;
        identity::require_object_link(&tree.url, &endpoint)?;
        validate_entry_links(source, repository, &tree)?;
        observations.insert(
            tree.sha,
            ObjectUpstream {
                body: response.observation,
                revalidation: response.revalidation,
            },
        );
        let entry = tree
            .entries
            .into_iter()
            .find(|entry| entry.name == *segment)
            .ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::NotFound,
                    "GitHub source path was not present in a complete verified tree",
                )
                .with_details(ResourceErrorDetails::new(
                    ErrorReason::UpstreamNotFoundOrHidden,
                ))
            })?;
        if index + 1 != path.segments().len() {
            if entry.disposition != object::EntryDisposition::Tree {
                return Err(failure(ErrorReason::UpstreamMalformed));
            }
            current_tree_sha = entry.object_sha;
            continue;
        }
        terminal = Some(Terminal {
            containing_tree_sha: current_tree_sha.clone(),
            mode: entry.mode,
            object_type: entry.object_type,
            object_sha: entry.object_sha,
            size_bytes: entry.size.value().copied(),
            disposition: entry.disposition,
        });
    }
    let terminal = terminal.ok_or_else(|| failure(ErrorReason::UpstreamMalformed))?;
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
    let mut acquisition = acquisition(ctx)?;
    acquisition.limits.max_decoded_bytes = Some(ctx.limits.max_decoded_bytes());
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
            size_bytes: terminal.size_bytes,
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
        unavailable_facts: Vec::new(),
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

async fn read_blob(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    terminal: &Terminal,
    ctx: &mut FactsRead<'_>,
) -> Result<(SourceContent, Option<ObjectUpstream>), ResourceError> {
    if terminal
        .size_bytes
        .is_some_and(|size| size > ctx.limits.max_decoded_bytes() as u64)
    {
        return Ok((
            SourceContent::DecodedSizeLimit(decoded_limit(ctx, terminal.size_bytes)),
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
        Err(error) if ordinary_blob_failure(&error) => {
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
    identity::require_object_link(&native.url, &endpoint)?;
    let content = match object::validate_blob(
        native,
        &terminal.object_sha,
        terminal.size_bytes,
        ctx.limits.max_decoded_bytes(),
    )? {
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

fn validate_entry_links(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    tree: &object::VerifiedTree,
) -> Result<(), ResourceError> {
    for entry in &tree.entries {
        let segment = match entry.object_type.as_str() {
            "tree" => "trees",
            "blob" => "blobs",
            "commit" => "commits",
            _ => return Err(failure(ErrorReason::UpstreamMalformed)),
        };
        let expected =
            source.endpoint(repository, &format!("git/{segment}/{}", entry.object_sha))?;
        identity::validate_optional_object_link(&entry.url, &expected)?;
    }
    Ok(())
}

fn ordinary_blob_failure(error: &ResourceError) -> bool {
    !collection::invalidates_retention(error)
        && !error
            .details()
            .is_some_and(|details| details.reason() == ErrorReason::UpstreamMalformed)
}

fn decoded_limit(ctx: &FactsRead<'_>, observed: Option<u64>) -> collection::LocalLimit {
    collection::LocalLimit {
        kind: AcquisitionLimitKind::DecodedContentBytes.as_str(),
        bound: ctx.limits.max_decoded_bytes() as u64,
        observed,
    }
}
