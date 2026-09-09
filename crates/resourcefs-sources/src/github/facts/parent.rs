//! The pull-request parent every discussion fact family is scoped by.
//!
//! One acquisition path and one projection are shared by conversation
//! comments, review submissions and inline review comments, so a family can
//! never disagree with its siblings about which pull request owns a record.
use resourcefs_core::{ErrorReason, GithubRepositoryIdentity, PullRequestNumber, ResourceError};
use serde::Serialize;
use url::Url;

use super::super::{GITHUB_JSON, GithubSource};
use super::{FactsRead, Links, NativeId, Presence, failure, identity, pull};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ParentFacts<'a> {
    kind: &'static str,
    id: &'a NativeId,
    number: &'a NativeId,
    #[serde(skip_serializing_if = "Presence::omitted")]
    node_id: &'a Presence<String>,
    links: Links<'a>,
}

pub(super) fn facts<'a>(
    pull: &'a pull::NativePull,
    identity: &'a identity::ValidatedIdentity<'a>,
) -> ParentFacts<'a> {
    ParentFacts {
        kind: "github.pull_request",
        id: identity.id,
        number: identity.number,
        node_id: &pull.node_id,
        links: Links {
            api_url: &pull.url,
            html_url: &pull.html_url,
        },
    }
}

/// Fetches the pull-request parent that owns a fact family, with the endpoint
/// it was read from and its generation baseline.
pub(super) async fn fetch(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    ctx: &mut FactsRead<'_>,
) -> Result<(pull::NativePull, Url, u64), ResourceError> {
    let endpoint = source.endpoint(repository, &format!("pulls/{}", number.get()))?;
    let response = source
        .fetch_controlled(
            endpoint.clone(),
            GITHUB_JSON,
            ctx.read,
            Some(&mut ctx.budget),
        )
        .await?;
    let generation = response.cache_generation;
    let pull: pull::NativePull = serde_json::from_slice(response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    Ok((pull, endpoint, generation))
}
