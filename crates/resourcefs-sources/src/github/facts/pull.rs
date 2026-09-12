//! Singular pull-request fact projection.
use std::fmt;

use resourcefs_core::{ErrorReason, GithubRepositoryIdentity, ResourceError, SourceResource};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, MapAccess, Visitor},
};
use url::Url;

use super::super::{GITHUB_JSON, GithubSource};
use super::{
    Actor, Facts, FactsRead, Presence, Repository, RepositoryName, RequestedRepository, Source,
    Upstream, acquisition, failure, finish_facts, identity,
};

native!(NativePull {
    id: super::NativeId,
    node_id: String,
    number: super::NativeId,
    url: String,
    html_url: String,
    diff_url: String,
    patch_url: String,
    issue_url: String,
    _links: super::Relations,
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
native!(Branch {
    sha: String,
    r#ref: String,
    repo: Repository
});

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
    relations: &'a Presence<super::Relations>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Data<'a> {
    id: &'a super::NativeId,
    number: &'a super::NativeId,
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
struct Request<'a> {
    repository: RepositoryName<'a>,
    number: &'a super::NativeId,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observed<'a> {
    id: &'a super::NativeId,
    number: &'a super::NativeId,
    #[serde(skip_serializing_if = "Presence::omitted")]
    node_id: &'a Presence<String>,
    base: ObservedBranch<'a>,
    head: ObservedBranch<'a>,
}

#[derive(Serialize)]
struct PullBody<'a> {
    request: Request<'a>,
    observed: Observed<'a>,
    upstream: Upstream<'a>,
    data: Data<'a>,
}

pub(super) async fn read(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    number: resourcefs_core::PullRequestNumber,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let endpoint = source.endpoint(repository, &format!("pulls/{}", number.get()))?;
    let response = source
        .fetch_controlled(
            endpoint.clone(),
            GITHUB_JSON,
            ctx.read,
            Some(&mut ctx.budget),
        )
        .await?;
    super::establish_generation(ctx, response.cache_generation);
    let pull: NativePull = serde_json::from_slice(response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let web = Url::parse(ctx.web_origin).map_err(|_| failure(ErrorReason::UpstreamUnavailable))?;
    let identity::ValidatedIdentity {
        id,
        number: observed_number,
        base,
        head,
        base_sha,
        head_sha,
    } = identity::validate(&pull, repository, number, &endpoint, &source.api_base, &web)?;
    let requested_number = super::NativeId::from_positive(number.get());
    let mut unavailable_facts = Vec::new();
    missing!(unavailable_facts; pull.node_id => "nodeId", pull.title => "title", pull.body => "body", pull.state => "state",
        pull.user => "author", pull.created_at => "createdAt", pull.updated_at => "updatedAt",
        pull.closed_at => "closedAt", pull.merged_at => "mergedAt", pull.draft => "draft", pull.merged => "merged",
        pull.html_url => "links.htmlUrl", pull.diff_url => "links.diffUrl", pull.patch_url => "links.patchUrl",
        pull.issue_url => "links.issueUrl", pull._links => "links.relations", base.r#ref => "base.refName",
        base.repo => "base.repository", head.r#ref => "head.refName", head.repo => "head.repository");
    let facts = Facts {
        schema_version: super::SchemaVersion { major: 1, minor: 0 },
        kind: "github.pull_request",
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
            observed: &base.repo,
        },
        acquisition: acquisition(ctx)?,
        body: PullBody {
            request: Request {
                repository: RepositoryName {
                    owner: repository.owner(),
                    name: repository.repository(),
                },
                number: &requested_number,
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
        },
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
