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

use super::super::{GITHUB_JSON, GithubSource};
use super::{
    Actor, Facts, FactsRead, Presence, Repository, RepositoryName, RequestedRepository, Source,
    Upstream, acquisition, failure, finish_facts, identity,
};

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
struct ParentLinks<'a> {
    #[serde(skip_serializing_if = "Presence::omitted")]
    api_url: &'a Presence<String>,
    #[serde(skip_serializing_if = "Presence::omitted")]
    html_url: &'a Presence<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitParentFacts<'a> {
    sha: &'a str,
    links: ParentLinks<'a>,
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

pub(super) async fn read(
    source: &GithubSource,
    repository: &GithubRepositoryIdentity,
    commit_id: &GithubCommitId,
    ctx: &mut FactsRead<'_>,
) -> Result<SourceResource, ResourceError> {
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
    let (details, observed_sha, tree_sha, parents) = validate_commit(
        &commit,
        repository,
        commit_id,
        &commit_endpoint,
        &source.api_base,
        &web,
    )?;

    let mut unavailable_facts = Vec::new();
    macro_rules! missing {
        ($($field:expr => $name:literal),* $(,)?) => { $(
            $field.unavailable($name, &mut unavailable_facts);
        )* };
    }
    missing!(
        commit.html_url => "links.htmlUrl",
        commit.comments_url => "links.commentsUrl",
        details.message => "message",
        details.author => "author",
        details.committer => "committer",
        commit.author => "authorAccount",
        commit.committer => "committerAccount"
    );
    // Nested native-person fields retain their own absent/null/value
    // distinction through Presence serialization. Dynamic parent indexes are
    // intentionally not flattened into the stable unavailable vocabulary.

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
                body: &repository_response.observation,
                revalidation: &repository_response.revalidation,
            },
            commit: Upstream {
                body: &commit_response.observation,
                revalidation: &commit_response.revalidation,
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
            observed: &observed_repository,
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

fn accept_generation(ctx: &mut FactsRead<'_>, generation: u64) -> Result<(), ResourceError> {
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
) -> Result<
    (
        &'a NativeCommitData,
        &'a str,
        &'a str,
        Vec<CommitParentFacts<'a>>,
    ),
    ResourceError,
> {
    let observed_sha = identity::sha(&commit.sha)?;
    if observed_sha != requested.as_str() {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
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
    let mut parents = Vec::with_capacity(parent_values.len());
    for parent in parent_values {
        let sha = identity::sha(&parent.sha)?;
        let expected_api =
            identity::expected_object_url(api, repository, &format!("commits/{sha}"))?;
        let expected_html = expected_web_commit_url(web, repository, sha)?;
        identity::validate_optional_object_link(&parent.url, &expected_api)?;
        identity::validate_optional_object_link(&parent.html_url, &expected_html)?;
        parents.push(CommitParentFacts {
            sha,
            links: ParentLinks {
                api_url: &parent.url,
                html_url: &parent.html_url,
            },
        });
    }
    Ok((details, observed_sha, tree_sha, parents))
}

fn validate_account(account: &Presence<Actor>, api: &Url, web: &Url) -> Result<(), ResourceError> {
    let Some(account) = account.value() else {
        return Ok(());
    };
    let login = identity::required(&account.login)?;
    let expected_api = append_path_segments(api, &["users", login])?;
    let expected_web = append_path_segments(web, &[login])?;
    identity::validate_optional_object_link(&account.url, &expected_api)?;
    identity::validate_optional_object_link(&account.html_url, &expected_web)?;
    Ok(())
}

fn append_path_segments(base: &Url, segments: &[&str]) -> Result<Url, ResourceError> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

fn expected_web_commit_url(
    web: &Url,
    repository: &GithubRepositoryIdentity,
    sha: &str,
) -> Result<Url, ResourceError> {
    web.join(&format!("{}/commit/{sha}", repository.as_str()))
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))
}
