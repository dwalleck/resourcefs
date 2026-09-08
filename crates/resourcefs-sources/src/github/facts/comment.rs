//! Conversation-comment decoding and the owned record projection.
use std::fmt;

use resourcefs_core::{ErrorReason, GithubRepositoryIdentity, ResourceError, SourceResource};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor},
};
use url::Url;

use super::super::{GITHUB_JSON, GithubSource, malformed_upstream};
use super::{
    Actor, Facts, FactsRead, NativeId, Presence, RepositoryName, Unavailable, Upstream,
    acquisition, failure, finish_facts, identity, pull,
};

native!(NativeComment {
    id: NativeId,
    node_id: String,
    url: String,
    html_url: String,
    issue_url: String,
    body: String,
    user: Actor,
    created_at: String,
    updated_at: String,
});

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ParentLinks<'a> {
    #[serde(skip_serializing_if = "Presence::omitted")]
    api_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    html_url: &'a Presence<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ParentFacts<'a> {
    kind: &'static str,
    id: &'a NativeId,
    number: &'a NativeId,
    #[serde(skip_serializing_if = "Presence::omitted")]
    node_id: &'a Presence<String>,
    links: ParentLinks<'a>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommentLinks<'a> {
    #[serde(skip_serializing_if = "Presence::omitted")]
    api_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    html_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    issue_url: &'a Presence<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommentRecord<'a> {
    kind: &'static str,
    id: &'a NativeId,
    #[serde(skip_serializing_if = "Presence::omitted")]
    node_id: &'a Presence<String>,
    parent: ParentFacts<'a>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    body: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    author: &'a Presence<Actor>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    created_at: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    updated_at: &'a Presence<String>,
    links: CommentLinks<'a>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommentRequest<'a> {
    repository: RepositoryName<'a>,
    number: &'a NativeId,
    comment_id: &'a NativeId,
}
#[derive(Serialize)]
struct CommentObserved<'a> {
    parent: ParentFacts<'a>,
}
#[derive(Serialize)]
struct CommentBody<'a> {
    request: CommentRequest<'a>,
    observed: CommentObserved<'a>,
    upstream: Upstream<'a>,
    data: CommentRecord<'a>,
}

/// Required native identity from a presence-aware decode.
///
/// Distinct from `identity::required`, which reports one fixed reason: this one
/// names the missing field, and the identity module owns identity validation.
fn native_field<'a, T>(value: &'a Presence<T>, field: &str) -> Result<&'a T, ResourceError> {
    value
        .value()
        .ok_or_else(|| malformed_upstream(&format!("GitHub {field} is required")))
}

fn parent_facts<'a>(
    pull: &'a pull::NativePull,
    identity: &identity::ValidatedIdentity<'a>,
) -> ParentFacts<'a> {
    ParentFacts {
        kind: "github.pull_request",
        id: identity.id,
        number: identity.number,
        node_id: &pull.node_id,
        links: ParentLinks {
            api_url: &pull.url,
            html_url: &pull.html_url,
        },
    }
}

/// The owned record for one conversation comment beneath a verified parent.
pub(super) fn record<'a>(
    comment: &'a NativeComment,
    pull: &'a pull::NativePull,
    identity: &'a identity::ValidatedIdentity<'a>,
    repository: &GithubRepositoryIdentity,
    number: u64,
    unavailable: &mut Vec<Unavailable>,
) -> Result<CommentRecord<'a>, ResourceError> {
    super::super::validate_parent(
        native_field(&comment.issue_url, "conversation comment issue_url")?,
        repository,
        "issues",
        number,
    )?;
    comment.node_id.unavailable("nodeId", unavailable);
    comment.body.unavailable("body", unavailable);
    comment.user.unavailable("author", unavailable);
    comment.created_at.unavailable("createdAt", unavailable);
    comment.updated_at.unavailable("updatedAt", unavailable);
    comment.url.unavailable("links.apiUrl", unavailable);
    comment.html_url.unavailable("links.htmlUrl", unavailable);
    comment.issue_url.unavailable("links.issueUrl", unavailable);
    Ok(CommentRecord {
        kind: "github.conversation_comment",
        id: native_field(&comment.id, "conversation comment id")?,
        node_id: &comment.node_id,
        parent: parent_facts(pull, identity),
        body: &comment.body,
        author: &comment.user,
        created_at: &comment.created_at,
        updated_at: &comment.updated_at,
        links: CommentLinks {
            api_url: &comment.url,
            html_url: &comment.html_url,
            issue_url: &comment.issue_url,
        },
    })
}

/// Reads one conversation comment beneath a verified pull request.
pub(super) async fn read_item(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    number: u64,
    comment_id: u64,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let (pull, parent_endpoint) = fetch_parent(source, repository, number, ctx).await?;
    let web = Url::parse(ctx.web_origin).map_err(|_| failure(ErrorReason::UpstreamUnavailable))?;
    let identity = identity::validate(
        &pull,
        repository,
        number,
        &parent_endpoint,
        &source.api_base,
        &web,
    )?;
    let endpoint = source.endpoint(repository, &format!("issues/comments/{comment_id}"))?;
    let response = source
        .fetch_controlled(endpoint, GITHUB_JSON, ctx.read, Some(&mut ctx.budget))
        .await?;
    let comment: NativeComment = serde_json::from_slice(response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let mut unavailable_facts = Vec::new();
    let data = record(
        &comment,
        &pull,
        &identity,
        repository,
        number,
        &mut unavailable_facts,
    )?;
    let facts = Facts {
        schema_version: super::SchemaVersion { major: 1, minor: 0 },
        kind: "github.conversation_comment",
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
        body: CommentBody {
            request: CommentRequest {
                repository: RepositoryName {
                    owner: repository.owner(),
                    name: repository.repository(),
                },
                number: identity.number,
                comment_id: native_field(&comment.id, "conversation comment id")?,
            },
            observed: CommentObserved {
                parent: parent_facts(&pull, &identity),
            },
            upstream: Upstream {
                body: &response.observation,
                revalidation: &response.revalidation,
            },
            data,
        },
        unavailable_facts,
    };
    let resource = finish_facts(
        &facts,
        ctx.limits.max_representation_bytes(),
        ctx.canonical.clone(),
        ctx.read,
    )?;
    if source
        .session
        .cache_generation(super::super::fetch::GITHUB_CACHE_NAMESPACE)
        .await?
        != response.cache_generation
    {
        return Err(failure(ErrorReason::UpstreamUnavailable));
    }
    ctx.read.check_acceptance()?;
    Ok(resource)
}

/// Fetches the pull-request parent that owns the comment, with the endpoint it
/// was read from so the caller can validate the observed identity against it.
pub(super) async fn fetch_parent(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    number: u64,
    ctx: &mut FactsRead<'_>,
) -> Result<(pull::NativePull, Url), ResourceError> {
    let endpoint = source.endpoint(repository, &format!("pulls/{number}"))?;
    let response = source
        .fetch_controlled(
            endpoint.clone(),
            GITHUB_JSON,
            ctx.read,
            Some(&mut ctx.budget),
        )
        .await?;
    let pull: pull::NativePull = serde_json::from_slice(response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    Ok((pull, endpoint))
}
