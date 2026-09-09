//! Conversation-comment decoding and the owned record projection.
use std::fmt;

use resourcefs_core::{
    ConversationCommentId, ErrorReason, GithubRepositoryIdentity, PullRequestNumber, ResourceError,
    SourceResource,
};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor},
};
use url::Url;

use super::super::{GITHUB_JSON, GithubSource};
use super::{
    Actor, Facts, FactsRead, NativeId, Presence, RepositoryName, Unavailable, Upstream,
    acquisition, failure, finish_facts, identity, parent, pull,
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
    parent: parent::ParentFacts<'a>,
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
    parent: parent::ParentFacts<'a>,
}
#[derive(Serialize)]
struct CommentBody<'a> {
    request: CommentRequest<'a>,
    observed: CommentObserved<'a>,
    upstream: Upstream<'a>,
    data: CommentRecord<'a>,
}

/// A validated comment owns exactly the decoded native object and its
/// availability observations. Projection only borrows this value and cannot
/// fail, so collection admission never repeats identity checks or availability
/// bookkeeping.
pub(super) struct ValidatedRecord {
    comment: NativeComment,
    unavailable: Vec<Unavailable>,
}

impl ValidatedRecord {
    pub(super) fn id(&self) -> u64 {
        self.comment
            .id
            .value()
            .expect("validated comment has an id")
            .0
    }

    pub(super) fn unavailable(&self) -> &[Unavailable] {
        &self.unavailable
    }

    pub(super) fn project<'a>(
        &'a self,
        pull: &'a pull::NativePull,
        identity: &'a identity::ValidatedIdentity<'a>,
    ) -> CommentRecord<'a> {
        CommentRecord {
            kind: "github.conversation_comment",
            id: self
                .comment
                .id
                .value()
                .expect("validated comment has an id"),
            node_id: &self.comment.node_id,
            parent: parent::facts(pull, identity),
            body: &self.comment.body,
            author: &self.comment.user,
            created_at: &self.comment.created_at,
            updated_at: &self.comment.updated_at,
            links: CommentLinks {
                api_url: &self.comment.url,
                html_url: &self.comment.html_url,
                issue_url: &self.comment.issue_url,
            },
        }
    }
}

/// Validate and own one conversation comment. The parent/link checks happen
/// before availability is recorded, so a contradictory identity can never be
/// softened into an ordinary later-page prefix.
pub(super) fn validate_record(
    comment: NativeComment,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    comment_id: Option<ConversationCommentId>,
    api: &Url,
    web: &Url,
) -> Result<ValidatedRecord, ResourceError> {
    identity::validate_comment_links(repository, number, comment_id, &comment, api, web)?;
    let mut unavailable = Vec::new();
    comment.node_id.unavailable("nodeId", &mut unavailable);
    comment.body.unavailable("body", &mut unavailable);
    comment.user.unavailable("author", &mut unavailable);
    comment
        .created_at
        .unavailable("createdAt", &mut unavailable);
    comment
        .updated_at
        .unavailable("updatedAt", &mut unavailable);
    comment.url.unavailable("links.apiUrl", &mut unavailable);
    comment
        .html_url
        .unavailable("links.htmlUrl", &mut unavailable);
    comment
        .issue_url
        .unavailable("links.issueUrl", &mut unavailable);
    Ok(ValidatedRecord {
        comment,
        unavailable,
    })
}

/// Reads one conversation comment beneath a verified pull request.
pub(super) async fn read_item(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    comment_id: ConversationCommentId,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let (pull, parent_endpoint, parent_generation) =
        parent::fetch(source, repository, number, ctx).await?;
    let web = Url::parse(ctx.web_origin).map_err(|_| failure(ErrorReason::UpstreamUnavailable))?;
    let identity = parent::validate(&pull, repository, number, &parent_endpoint, source, ctx)?;
    let endpoint = source.endpoint(repository, &format!("issues/comments/{}", comment_id.get()))?;
    let response = source
        .fetch_controlled(endpoint, GITHUB_JSON, ctx.read, Some(&mut ctx.budget))
        .await?;
    if response.cache_generation != parent_generation {
        return Err(failure(ErrorReason::UpstreamUnavailable));
    }
    let comment: NativeComment = serde_json::from_slice(response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let validated = validate_record(
        comment,
        repository,
        number,
        Some(comment_id),
        &source.api_base,
        &web,
    )?;
    let requested_number = NativeId::from_positive(number.get());
    let requested_comment_id = NativeId::from_positive(comment_id.get());
    let data = validated.project(&pull, &identity);
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
                number: &requested_number,
                comment_id: &requested_comment_id,
            },
            observed: CommentObserved {
                parent: parent::facts(&pull, &identity),
            },
            upstream: Upstream {
                body: &response.observation,
                revalidation: &response.revalidation,
            },
            data,
        },
        unavailable_facts: validated.unavailable.clone(),
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
