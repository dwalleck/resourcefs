//! Review-submission decoding and the owned record projection.
use std::fmt;

use resourcefs_core::{
    ErrorReason, GithubRepositoryIdentity, PullRequestNumber, ResourceError, ReviewId,
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

native!(NativeReview {
    id: NativeId,
    node_id: String,
    url: String,
    html_url: String,
    pull_request_url: String,
    body: String,
    user: Actor,
    state: String,
    commit_id: String,
    submitted_at: String,
});

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewLinks<'a> {
    #[serde(skip_serializing_if = "Presence::omitted")]
    api_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    html_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    pull_request_url: &'a Presence<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewRecord<'a> {
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
    state: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    commit_sha: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    submitted_at: &'a Presence<String>,
    links: ReviewLinks<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewRequest<'a> {
    repository: RepositoryName<'a>,
    number: &'a NativeId,
    review_id: &'a NativeId,
}

#[derive(Serialize)]
struct ReviewObserved<'a> {
    parent: parent::ParentFacts<'a>,
}

#[derive(Serialize)]
struct ReviewBody<'a> {
    request: ReviewRequest<'a>,
    observed: ReviewObserved<'a>,
    upstream: Upstream<'a>,
    data: ReviewRecord<'a>,
}

/// A validated review owns exactly the decoded native object and its
/// availability observations. Projection only borrows this value and cannot
/// fail, so collection admission never repeats identity checks or availability
/// bookkeeping.
pub(super) struct ValidatedReview {
    review: NativeReview,
    unavailable: Vec<Unavailable>,
}

impl ValidatedReview {
    pub(super) fn review_id(&self) -> u64 {
        self.review
            .id
            .value()
            .expect("validated review has an id")
            .0
    }

    pub(super) fn review_unavailable(&self) -> &[Unavailable] {
        &self.unavailable
    }

    pub(super) fn project_review<'a>(
        &'a self,
        pull: &'a pull::NativePull,
        identity: &'a identity::ValidatedIdentity<'a>,
    ) -> ReviewRecord<'a> {
        ReviewRecord {
            kind: "github.review_submission",
            id: self.review.id.value().expect("validated review has an id"),
            node_id: &self.review.node_id,
            parent: parent::parent_facts(pull, identity),
            body: &self.review.body,
            author: &self.review.user,
            state: &self.review.state,
            commit_sha: &self.review.commit_id,
            submitted_at: &self.review.submitted_at,
            links: ReviewLinks {
                api_url: &self.review.url,
                html_url: &self.review.html_url,
                pull_request_url: &self.review.pull_request_url,
            },
        }
    }
}

/// Validate and own one review submission. The parent/link checks happen
/// before availability is recorded, so a contradictory identity can never be
/// softened into an ordinary later-page prefix.
pub(super) fn validate_review(
    review: NativeReview,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    review_id: Option<ReviewId>,
    api: &Url,
    web: &Url,
) -> Result<ValidatedReview, ResourceError> {
    let observed = identity::require_observed_id(
        review_id.map(ReviewId::get),
        review.id.value().map(|id| id.0),
    )?;
    identity::require_object_link(
        &review.pull_request_url,
        &identity::expected_object_url(api, repository, &format!("pulls/{}", number.get()))?,
    )?;
    identity::validate_optional_object_link(
        &review.url,
        &identity::expected_object_url(
            api,
            repository,
            &format!("pulls/{}/reviews/{observed}", number.get()),
        )?,
    )?;
    if let Some(fragment) =
        identity::validate_optional_web_link(&review.html_url, repository, number, web)?
        && !matches!(fragment, identity::WebFragment::Review(id) if id.get() == observed)
    {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    let mut unavailable = Vec::new();
    review.node_id.unavailable("nodeId", &mut unavailable);
    review.body.unavailable("body", &mut unavailable);
    review.user.unavailable("author", &mut unavailable);
    review.state.unavailable("state", &mut unavailable);
    review.commit_id.unavailable("commitSha", &mut unavailable);
    review
        .submitted_at
        .unavailable("submittedAt", &mut unavailable);
    review.url.unavailable("links.apiUrl", &mut unavailable);
    review
        .html_url
        .unavailable("links.htmlUrl", &mut unavailable);
    review
        .pull_request_url
        .unavailable("links.pullRequestUrl", &mut unavailable);
    Ok(ValidatedReview {
        review,
        unavailable,
    })
}

/// Reads one review submission beneath a verified pull request.
pub(super) async fn read_review_item(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    review_id: ReviewId,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let (pull, parent_endpoint, parent_generation) =
        parent::fetch_parent(source, repository, number, ctx).await?;
    let web = Url::parse(ctx.web_origin).map_err(|_| failure(ErrorReason::UpstreamUnavailable))?;
    let identity =
        parent::validate_parent(&pull, repository, number, &parent_endpoint, source, ctx)?;
    let endpoint = source.endpoint(
        repository,
        &format!("pulls/{}/reviews/{}", number.get(), review_id.get()),
    )?;
    let response = source
        .fetch_controlled(endpoint, GITHUB_JSON, ctx.read, Some(&mut ctx.budget))
        .await?;
    if response.cache_generation != parent_generation {
        // A concurrent mutation invalidated the parent's body: the same
        // condition the shared generation guards report, so the caller is not
        // told a different story depending on where the invalidation landed.
        return Err(failure(ErrorReason::CacheGenerationChanged));
    }
    let review: NativeReview = serde_json::from_slice(response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let validated = validate_review(
        review,
        repository,
        number,
        Some(review_id),
        &source.api_base,
        &web,
    )?;
    let requested_number = NativeId::from_positive(number.get());
    let requested_review_id = NativeId::from_positive(review_id.get());
    let data = validated.project_review(&pull, &identity);
    let facts = Facts {
        schema_version: super::SchemaVersion { major: 1, minor: 0 },
        kind: "github.review_submission",
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
        body: ReviewBody {
            request: ReviewRequest {
                repository: RepositoryName {
                    owner: repository.owner(),
                    name: repository.repository(),
                },
                number: &requested_number,
                review_id: &requested_review_id,
            },
            observed: ReviewObserved {
                parent: parent::parent_facts(&pull, &identity),
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
