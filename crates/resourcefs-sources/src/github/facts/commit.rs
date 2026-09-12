//! Immutable GitHub commit metadata acquisition and facts projection.
use std::fmt;

use resourcefs_core::{
    ErrorReason, GithubCommitId, GithubRepositoryIdentity, ResourceError, SourceResource,
};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, MapAccess, Visitor},
    ser::SerializeMap,
};
use url::Url;

use super::super::fetch::BodyObservation;
use super::super::{GITHUB_JSON, GithubSource};
use super::{
    Actor, Facts, FactsRead, Presence, Repository, RepositoryName, RequestedRepository, Source,
    Upstream, acquisition, failure, finish_facts, identity,
};

pub(super) struct AcquiredCommit {
    pub(super) repository: Presence<Repository>,
    pub(super) commit: NativeCommit,
    pub(super) parent_indices: Vec<usize>,
    pub(super) repository_observation: BodyObservation,
    pub(super) repository_revalidation: Option<BodyObservation>,
    pub(super) commit_observation: BodyObservation,
    pub(super) commit_revalidation: Option<BodyObservation>,
}

native!(NativePerson {
    name: String,
    email: String,
    date: String
});
native!(NativeTree {
    sha: String,
    url: String
});
native!(NativeParent {
    sha: String,
    url: String,
    html_url: String
});
native!(NativeCommitData {
    author: NativePerson,
    committer: NativePerson,
    message: String,
    tree: NativeTree,
    url: String
});
native!(NativeCommit {
    sha: String,
    url: String,
    html_url: String,
    comments_url: String,
    commit: NativeCommitData,
    author: Actor,
    committer: Actor,
    parents: Vec<NativeParent>
});

impl Serialize for NativePerson {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if !self.name.omitted() {
            map.serialize_entry("name", &self.name)?;
        }
        if !self.email.omitted() {
            map.serialize_entry("email", &self.email)?;
        }
        if !self.date.omitted() {
            map.serialize_entry("date", &self.date)?;
        }
        map.end()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitLinks<'a> {
    #[serde(skip_serializing_if = "Presence::omitted")]
    api_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    html_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    comments_url: &'a Presence<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitParentFacts<'a> {
    sha: &'a str,
    links: super::Links<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Data<'a> {
    sha: &'a str,
    tree_sha: &'a str,
    parents: Vec<CommitParentFacts<'a>>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    message: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    author: &'a Presence<NativePerson>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    committer: &'a Presence<NativePerson>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    author_account: &'a Presence<Actor>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    committer_account: &'a Presence<Actor>,
    links: CommitLinks<'a>,
}

#[derive(Serialize)]
struct Request<'a> {
    repository: RepositoryName<'a>,
    #[serde(rename = "commitSha")]
    commit_sha: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observed<'a> {
    commit_sha: &'a str,
    tree_sha: &'a str,
}

#[derive(Serialize)]
struct CommitUpstream<'a> {
    repository: Upstream<'a>,
    commit: Upstream<'a>,
}

#[derive(Serialize)]
struct CommitBody<'a> {
    request: Request<'a>,
    observed: Observed<'a>,
    upstream: CommitUpstream<'a>,
    data: Data<'a>,
}

pub(super) async fn acquire(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    commit_id: &GithubCommitId,
    ctx: &mut FactsRead<'_>,
) -> Result<AcquiredCommit, ResourceError> {
    let web = Url::parse(ctx.web_origin).map_err(|_| failure(ErrorReason::UpstreamUnavailable))?;
    let repository_endpoint = source
        .api_base
        .join(&format!(
            "repos/{}/{}",
            repository.owner(),
            repository.repository()
        ))
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let repository_response = source
        .fetch_controlled(
            repository_endpoint.clone(),
            GITHUB_JSON,
            ctx.read,
            Some(&mut ctx.budget),
        )
        .await?;
    accept_generation(ctx, repository_response.cache_generation)?;
    let repository_value: Repository = serde_json::from_slice(repository_response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    require_repository(&repository_value)?;
    let observed_repository = Presence::Present(repository_value);
    let observed_repository_ref = identity::required(&observed_repository)?;
    identity::require_object_link(&observed_repository_ref.url, &repository_endpoint)?;
    let expected_web_repository = web
        .join(repository.as_str())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    identity::require_object_link(&observed_repository_ref.html_url, &expected_web_repository)?;
    identity::validate_repository(
        &observed_repository,
        Some(repository),
        &source.api_base,
        &web,
    )?;
    validate_account(&observed_repository_ref.owner, &source.api_base, &web)?;

    let commit_endpoint =
        source.endpoint(repository, &format!("commits/{}", commit_id.as_str()))?;
    let commit_response = source
        .fetch_controlled(
            commit_endpoint.clone(),
            GITHUB_JSON,
            ctx.read,
            Some(&mut ctx.budget),
        )
        .await?;
    accept_generation(ctx, commit_response.cache_generation)?;
    let commit: NativeCommit = serde_json::from_slice(commit_response.body())
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let (_, _, _, parent_indices) = validate_commit(
        &commit,
        repository,
        commit_id,
        &commit_endpoint,
        &source.api_base,
        &web,
    )?;
    Ok(AcquiredCommit {
        repository: observed_repository,
        commit,
        parent_indices,
        repository_observation: repository_response.observation,
        repository_revalidation: repository_response.revalidation,
        commit_observation: commit_response.observation,
        commit_revalidation: commit_response.revalidation,
    })
}

pub(super) async fn read(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    commit_id: &GithubCommitId,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
    let acquired = acquire(source, repository, commit_id, ctx).await?;
    let observed_repository = &acquired.repository;
    let commit = &acquired.commit;
    let details = identity::required(&commit.commit)?;
    let observed_sha = identity::sha(&commit.sha)?;
    let tree_sha = identity::sha(&identity::required(&details.tree)?.sha)?;
    let mut unavailable_facts = Vec::new();
    missing!(unavailable_facts;
        commit.html_url => "links.htmlUrl",
        commit.comments_url => "links.commentsUrl",
        details.message => "message",
        details.author => "author",
        details.committer => "committer",
        commit.author => "authorAccount",
        commit.committer => "committerAccount"
    );
    let parent_values = identity::required(&commit.parents)?;
    let parents = acquired
        .parent_indices
        .iter()
        .map(|&index| {
            let parent = parent_values
                .get(index)
                .ok_or_else(|| failure(ErrorReason::UpstreamMalformed))?;
            Ok(CommitParentFacts {
                sha: identity::sha(&parent.sha)?,
                links: super::Links {
                    api_url: &parent.url,
                    html_url: &parent.html_url,
                },
            })
        })
        .collect::<Result<Vec<_>, ResourceError>>()?;
    let body = CommitBody {
        request: Request {
            repository: RepositoryName {
                owner: repository.owner(),
                name: repository.repository(),
            },
            commit_sha: commit_id.as_str(),
        },
        observed: Observed {
            commit_sha: observed_sha,
            tree_sha,
        },
        upstream: CommitUpstream {
            repository: Upstream {
                body: &acquired.repository_observation,
                revalidation: &acquired.repository_revalidation,
            },
            commit: Upstream {
                body: &acquired.commit_observation,
                revalidation: &acquired.commit_revalidation,
            },
        },
        data: Data {
            sha: observed_sha,
            tree_sha,
            parents,
            message: &details.message,
            author: &details.author,
            committer: &details.committer,
            author_account: &commit.author,
            committer_account: &commit.committer,
            links: CommitLinks {
                api_url: &commit.url,
                html_url: &commit.html_url,
                comments_url: &commit.comments_url,
            },
        },
    };
    let facts = Facts {
        schema_version: super::SchemaVersion { major: 1, minor: 0 },
        kind: "github.commit",
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
            observed: observed_repository,
        },
        acquisition: acquisition(ctx)?,
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

pub(super) fn accept_generation(
    ctx: &mut FactsRead<'_>,
    generation: u64,
) -> Result<(), ResourceError> {
    if ctx
        .cache_generation
        .is_some_and(|expected| expected != generation)
    {
        return Err(failure(ErrorReason::UpstreamUnavailable));
    }
    super::establish_generation(ctx, generation);
    Ok(())
}

fn require_repository(repository: &Repository) -> Result<(), ResourceError> {
    identity::required(&repository.id)?;
    identity::required(&repository.name)?;
    identity::required(&repository.full_name)?;
    let owner = identity::required(&repository.owner)?;
    identity::required(&owner.login)?;
    identity::required(&repository.url)?;
    identity::required(&repository.html_url)?;
    Ok(())
}

fn validate_commit<'a>(
    commit: &'a NativeCommit,
    repository: &GithubRepositoryIdentity,
    requested: &GithubCommitId,
    endpoint: &Url,
    api: &Url,
    web: &Url,
) -> Result<(&'a NativeCommitData, &'a str, &'a str, Vec<usize>), ResourceError> {
    let observed_sha = identity::sha(&commit.sha)?;
    if observed_sha != requested.as_str() {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    // Presence and identity answer different questions: an absent link is a
    // malformed document, while a present link naming a foreign object is a
    // contradiction. The shared guard answers both with the contradiction, so
    // presence is required here to keep this read's verdicts consistent — the
    // repository's own required links are already presence-checked above.
    identity::required(&commit.url)?;
    identity::require_object_link(&commit.url, endpoint)?;
    let expected_web = expected_web_commit_url(web, repository, observed_sha)?;
    identity::validate_optional_object_link(&commit.html_url, &expected_web)?;
    let expected_comments = identity::expected_object_url(
        api,
        repository,
        &format!("commits/{observed_sha}/comments"),
    )?;
    identity::validate_optional_object_link(&commit.comments_url, &expected_comments)?;
    validate_account(&commit.author, api, web)?;
    validate_account(&commit.committer, api, web)?;

    let details = identity::required(&commit.commit)?;
    let expected_git_commit =
        identity::expected_object_url(api, repository, &format!("git/commits/{observed_sha}"))?;
    identity::validate_optional_object_link(&details.url, &expected_git_commit)?;
    let tree = identity::required(&details.tree)?;
    let tree_sha = identity::sha(&tree.sha)?;
    let expected_tree =
        identity::expected_object_url(api, repository, &format!("git/trees/{tree_sha}"))?;
    identity::validate_optional_object_link(&tree.url, &expected_tree)?;
    let parent_values = identity::required(&commit.parents)?;
    let mut parent_indices = Vec::with_capacity(parent_values.len());
    for (index, parent) in parent_values.iter().enumerate() {
        let sha = identity::sha(&parent.sha)?;
        let expected_api =
            identity::expected_object_url(api, repository, &format!("commits/{sha}"))?;
        let expected_html = expected_web_commit_url(web, repository, sha)?;
        identity::validate_optional_object_link(&parent.url, &expected_api)?;
        identity::validate_optional_object_link(&parent.html_url, &expected_html)?;
        parent_indices.push(index);
    }
    Ok((details, observed_sha, tree_sha, parent_indices))
}

fn validate_account(account: &Presence<Actor>, api: &Url, web: &Url) -> Result<(), ResourceError> {
    let Some(account) = account.value() else {
        return Ok(());
    };
    let login = identity::required(&account.login)?;
    require_login(login)?;
    validate_account_link(&account.url, api, &[&["users", login]])?;
    // A GitHub App's account is spelled `<slug>[bot]` and served from
    // `/apps/<slug>`, so its profile link names the slug rather than the login;
    // both routes spell the one account the login identifies.
    match login.strip_suffix("[bot]").filter(|slug| !slug.is_empty()) {
        Some(slug) => validate_account_link(&account.html_url, web, &[&[login], &["apps", slug]]),
        None => validate_account_link(&account.html_url, web, &[&[login]]),
    }
}

/// A login names exactly one account. An empty login resolves to the user
/// *collection* and a dot segment never survives URL parsing, so neither can
/// be the account the login claims to identify.
fn require_login(login: &str) -> Result<(), ResourceError> {
    if login.is_empty() || matches!(login, "." | "..") {
        return Err(failure(ErrorReason::UpstreamMalformed));
    }
    Ok(())
}

/// A supplied account link must belong to this deployment and, when its path
/// is one of the account's own routes, must name that account.
///
/// Compared on decoded segments, never on serialized paths: the provider
/// percent-encodes an app account's login in the API route
/// (`/users/dependabot%5Bbot%5D`) while a constructed path renders those bytes
/// literally, so a path comparison reads the provider's own escaping as a
/// contradiction. Segments are split before decoding, so an escaped separator
/// cannot merge two segments into one.
fn validate_account_link(
    link: &Presence<String>,
    authority: &Url,
    routes: &[&[&str]],
) -> Result<(), ResourceError> {
    let Some(value) = link.value() else {
        return Ok(());
    };
    let parsed = Url::parse(value).map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    if !identity::clean_authority(&parsed, authority)
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    for route in routes {
        if route_matches(&parsed, authority, route)? {
            return Ok(());
        }
    }
    Err(failure(ErrorReason::UpstreamIdentityMismatch))
}

/// Whether `url` is `authority`'s own path prefix followed by exactly `route`.
///
/// The prefix is skipped rather than assumed empty: a custom `apiBaseUrl` may
/// carry one (`https://host/api/v3/`), and a route is checked below it.
fn route_matches(url: &Url, authority: &Url, route: &[&str]) -> Result<bool, ResourceError> {
    let (Some(observed), Some(base)) = (url.path_segments(), authority.path_segments()) else {
        return Ok(false);
    };
    let mut observed = observed;
    // The deployment's own prefix is compared as spelled. Both sides come from
    // `Url::path_segments`, which yields the serialized segment, so an escaped
    // byte in a custom `apiBaseUrl` matches the provider's escaping of it.
    for segment in base.filter(|segment| !segment.is_empty()) {
        match observed.next() {
            Some(observed) if observed.eq_ignore_ascii_case(segment) => {}
            _ => return Ok(false),
        }
    }
    // A route is compared decoded, because its expected segments are the
    // account's own login and app slug as the provider publishes them while the
    // link spells them percent-encoded.
    for segment in route {
        match observed.next() {
            Some(observed) if decode_segment(observed)?.eq_ignore_ascii_case(segment) => {}
            _ => return Ok(false),
        }
    }
    Ok(observed.next().is_none())
}

/// Percent-decode one supplied path segment for comparison only.
///
/// A malformed escape is a contradiction rather than a recovery: the link is a
/// provider observation that is compared and never published, so there is no
/// lossless reading of it to preserve.
fn decode_segment(segment: &str) -> Result<String, ResourceError> {
    if !segment.contains('%') {
        return Ok(segment.to_owned());
    }
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let (Some(high), Some(low)) = (
                bytes.get(index + 1).and_then(|byte| hex_nibble(*byte)),
                bytes.get(index + 2).and_then(|byte| hex_nibble(*byte)),
            ) else {
                return Err(failure(ErrorReason::UpstreamMalformed));
            };
            decoded.push(high << 4 | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| failure(ErrorReason::UpstreamMalformed))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

fn expected_web_commit_url(
    web: &Url,
    repository: &GithubRepositoryIdentity,
    sha: &str,
) -> Result<Url, ResourceError> {
    web.join(&format!("{}/commit/{sha}", repository.as_str()))
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))
}
