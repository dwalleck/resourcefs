//! Inline review-comment decoding and the owned native-anchor projection.
use std::fmt;

use resourcefs_core::{
    ErrorReason, GithubRepositoryIdentity, PullRequestNumber, ResourceError, ReviewCommentId,
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

native!(NativeInlineComment {
    id: NativeId,
    node_id: String,
    url: String,
    html_url: String,
    pull_request_url: String,
    pull_request_review_id: NativeId,
    in_reply_to_id: NativeId,
    body: String,
    user: Actor,
    created_at: String,
    updated_at: String,
    path: String,
    diff_hunk: String,
    commit_id: String,
    original_commit_id: String,
    side: String,
    line: i64,
    start_side: String,
    start_line: i64,
    original_line: i64,
    original_start_line: i64,
    position: i64,
    original_position: i64,
    subject_type: String,
});

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineLinks<'a> {
    #[serde(skip_serializing_if = "Presence::omitted")]
    api_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    html_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    pull_request_url: &'a Presence<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InlineCommentRecord<'a> {
    kind: &'static str,
    id: &'a NativeId,
    #[serde(skip_serializing_if = "Presence::omitted")]
    node_id: &'a Presence<String>,
    parent: parent::ParentFacts<'a>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    review_id: &'a Presence<NativeId>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    reply_to_id: &'a Presence<NativeId>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    body: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    author: &'a Presence<Actor>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    created_at: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    updated_at: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    path: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    diff_hunk: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    commit_sha: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    original_commit_sha: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    side: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    line: &'a Presence<i64>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    start_side: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    start_line: &'a Presence<i64>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    original_line: &'a Presence<i64>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    original_start_line: &'a Presence<i64>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    position: &'a Presence<i64>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    original_position: &'a Presence<i64>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    subject_type: &'a Presence<String>,
    links: InlineLinks<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineRequest<'a> {
    repository: RepositoryName<'a>,
    number: &'a NativeId,
    comment_id: &'a NativeId,
}

#[derive(Serialize)]
struct InlineObserved<'a> {
    parent: parent::ParentFacts<'a>,
}

#[derive(Serialize)]
struct InlineBody<'a> {
    request: InlineRequest<'a>,
    observed: InlineObserved<'a>,
    upstream: Upstream<'a>,
    data: InlineCommentRecord<'a>,
}

/// A validated inline comment owns exactly the decoded native object and its
/// availability observations. Projection only borrows this value and cannot
/// fail, so collection admission never repeats identity checks or availability
/// bookkeeping.
pub(super) struct ValidatedRecord {
    comment: NativeInlineComment,
    unavailable: Vec<Unavailable>,
}

impl ValidatedRecord {
    pub(super) fn id(&self) -> u64 {
        self.comment
            .id
            .value()
            .expect("validated inline comment has an id")
            .0
    }

    pub(super) fn unavailable(&self) -> &[Unavailable] {
        &self.unavailable
    }

    pub(super) fn project<'a>(
        &'a self,
        pull: &'a pull::NativePull,
        identity: &'a identity::ValidatedIdentity<'a>,
    ) -> InlineCommentRecord<'a> {
        InlineCommentRecord {
            kind: "github.review_comment",
            id: self
                .comment
                .id
                .value()
                .expect("validated inline comment has an id"),
            node_id: &self.comment.node_id,
            parent: parent::facts(pull, identity),
            review_id: &self.comment.pull_request_review_id,
            reply_to_id: &self.comment.in_reply_to_id,
            body: &self.comment.body,
            author: &self.comment.user,
            created_at: &self.comment.created_at,
            updated_at: &self.comment.updated_at,
            path: &self.comment.path,
            diff_hunk: &self.comment.diff_hunk,
            commit_sha: &self.comment.commit_id,
            original_commit_sha: &self.comment.original_commit_id,
            side: &self.comment.side,
            line: &self.comment.line,
            start_side: &self.comment.start_side,
            start_line: &self.comment.start_line,
            original_line: &self.comment.original_line,
            original_start_line: &self.comment.original_start_line,
            position: &self.comment.position,
            original_position: &self.comment.original_position,
            subject_type: &self.comment.subject_type,
            links: InlineLinks {
                api_url: &self.comment.url,
                html_url: &self.comment.html_url,
                pull_request_url: &self.comment.pull_request_url,
            },
        }
    }
}

/// Validate and own one inline review comment. The parent/link checks happen
/// before availability is recorded, so a contradictory identity can never be
/// softened into an ordinary later-page prefix.
pub(super) fn validate_record(
    comment: NativeInlineComment,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    comment_id: Option<ReviewCommentId>,
    api: &Url,
    web: &Url,
) -> Result<ValidatedRecord, ResourceError> {
    let observed = identity::require_observed_id(
        comment_id.map(ReviewCommentId::get),
        comment.id.value().map(|id| id.0),
    )?;
    identity::require_object_link(
        &comment.pull_request_url,
        &identity::expected_object_url(api, repository, &format!("pulls/{}", number.get()))?,
    )?;
    identity::validate_optional_object_link(
        &comment.url,
        &identity::expected_object_url(api, repository, &format!("pulls/comments/{observed}"))?,
    )?;
    if let Some(fragment) =
        identity::validate_optional_web_link(&comment.html_url, repository, number, web)?
        && !matches!(fragment, identity::WebFragment::ReviewComment(id) if id.get() == observed)
    {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    let mut unavailable = Vec::new();
    comment.node_id.unavailable("nodeId", &mut unavailable);
    comment
        .pull_request_review_id
        .unavailable("reviewId", &mut unavailable);
    comment
        .in_reply_to_id
        .unavailable("replyToId", &mut unavailable);
    comment.body.unavailable("body", &mut unavailable);
    comment.user.unavailable("author", &mut unavailable);
    comment
        .created_at
        .unavailable("createdAt", &mut unavailable);
    comment
        .updated_at
        .unavailable("updatedAt", &mut unavailable);
    comment.path.unavailable("path", &mut unavailable);
    comment.diff_hunk.unavailable("diffHunk", &mut unavailable);
    comment.commit_id.unavailable("commitSha", &mut unavailable);
    comment
        .original_commit_id
        .unavailable("originalCommitSha", &mut unavailable);
    comment.side.unavailable("side", &mut unavailable);
    comment.line.unavailable("line", &mut unavailable);
    comment
        .start_side
        .unavailable("startSide", &mut unavailable);
    comment
        .start_line
        .unavailable("startLine", &mut unavailable);
    comment
        .original_line
        .unavailable("originalLine", &mut unavailable);
    comment
        .original_start_line
        .unavailable("originalStartLine", &mut unavailable);
    comment.position.unavailable("position", &mut unavailable);
    comment
        .original_position
        .unavailable("originalPosition", &mut unavailable);
    comment
        .subject_type
        .unavailable("subjectType", &mut unavailable);
    comment.url.unavailable("links.apiUrl", &mut unavailable);
    comment
        .html_url
        .unavailable("links.htmlUrl", &mut unavailable);
    comment
        .pull_request_url
        .unavailable("links.pullRequestUrl", &mut unavailable);
    Ok(ValidatedRecord {
        comment,
        unavailable,
    })
}

/// Reads one inline review comment beneath a verified pull request. The
/// provider serves this object by repository-wide id, so the parent link is
/// the only proof that it belongs to the addressed pull request.
pub(super) async fn read_item(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    comment_id: ReviewCommentId,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let (pull, parent_endpoint, parent_generation) =
        parent::fetch(source, repository, number, ctx).await?;
    let web = Url::parse(ctx.web_origin).map_err(|_| failure(ErrorReason::UpstreamUnavailable))?;
    let identity = parent::validate(&pull, repository, number, &parent_endpoint, source, ctx)?;
    let endpoint = source.endpoint(repository, &format!("pulls/comments/{}", comment_id.get()))?;
    let response = source
        .fetch_controlled(endpoint, GITHUB_JSON, ctx.read, Some(&mut ctx.budget))
        .await?;
    if response.cache_generation != parent_generation {
        return Err(failure(ErrorReason::UpstreamUnavailable));
    }
    let comment: NativeInlineComment = serde_json::from_slice(response.body())
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
        kind: "github.review_comment",
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
        body: InlineBody {
            request: InlineRequest {
                repository: RepositoryName {
                    owner: repository.owner(),
                    name: repository.repository(),
                },
                number: &requested_number,
                comment_id: &requested_comment_id,
            },
            observed: InlineObserved {
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
