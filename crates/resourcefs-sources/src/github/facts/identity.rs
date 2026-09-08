//! Validating native PR and repository identities without acquiring linked resources.
use resourcefs_core::{ErrorReason, GithubRepositoryIdentity, ResourceError};
use url::Url;

use super::{Branch, NativeId, NativePull, Presence, Repository, failure};

pub(super) struct ValidatedIdentity<'a> {
    pub(super) id: &'a NativeId,
    pub(super) number: &'a NativeId,
    pub(super) base: &'a Branch,
    pub(super) head: &'a Branch,
    pub(super) base_sha: &'a str,
    pub(super) head_sha: &'a str,
}

pub(super) fn validate<'a>(
    pull: &'a NativePull,
    repository: &GithubRepositoryIdentity,
    number: u64,
    endpoint: &Url,
    api: &Url,
    web: &Url,
) -> Result<ValidatedIdentity<'a>, ResourceError> {
    let id = required(&pull.id)?;
    let observed_number = required(&pull.number)?;
    if observed_number.0 != number {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    same_url(required(&pull.url)?, endpoint)?;
    for link in [
        &pull.html_url,
        &pull.diff_url,
        &pull.patch_url,
        &pull.issue_url,
    ] {
        validate_observed_link(link, repository, number, api, web)?;
    }
    if let Some(relations) = pull._links.value() {
        for name in ["self", "html", "issue", "comments", "review_comments"] {
            if let Some(Presence::Present(relation)) = relations.0.get(name) {
                validate_observed_link(&relation.href, repository, number, api, web)?;
            }
        }
    }
    let base = required(&pull.base)?;
    let head = required(&pull.head)?;
    let base_sha = sha(base)?;
    let head_sha = sha(head)?;
    validate_repository(&base.repo, Some(repository), api, web)?;
    validate_repository(&head.repo, None, api, web)?;
    Ok(ValidatedIdentity {
        id,
        number: observed_number,
        base,
        head,
        base_sha,
        head_sha,
    })
}

fn required<T>(field: &Presence<T>) -> Result<&T, ResourceError> {
    field
        .value()
        .ok_or_else(|| failure(ErrorReason::UpstreamMalformed))
}
fn sha(branch: &Branch) -> Result<&str, ResourceError> {
    let sha = required(&branch.sha)?;
    if sha.len() != 40
        || !sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(failure(ErrorReason::UpstreamMalformed));
    }
    Ok(sha)
}
fn same_url(actual: &str, expected: &Url) -> Result<(), ResourceError> {
    let actual = Url::parse(actual).map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    if actual.origin() != expected.origin()
        || !actual.path().eq_ignore_ascii_case(expected.path())
        || !actual.username().is_empty()
        || actual.password().is_some()
        || actual.query().is_some()
        || actual.fragment().is_some()
    {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    Ok(())
}

// Only recognizable object routes carry identity. Opaque observations and URI
// templates stay untouched; query/fragment spellings do not change the object.
fn validate_observed_link(
    link: &Presence<String>,
    repository: &GithubRepositoryIdentity,
    number: u64,
    api: &Url,
    web: &Url,
) -> Result<(), ResourceError> {
    let Some(link) = link.value() else {
        return Ok(());
    };
    if link.contains('{') || link.contains('}') {
        return Ok(());
    }
    let actual = match Url::parse(link) {
        Ok(url) => url,
        // Not a URL at all: an opaque provider observation, published as-is.
        Err(_) => return Ok(()),
    };
    // A value that *is* a URL is offered as one of this deployment's own
    // links. One whose origin is neither the API nor the web authority is not
    // an unrecognized shape to wave through — it is a contradiction, and
    // publishing it puts an arbitrary URL (`file:///…`, another host) in a
    // field an agent reads as this deployment's.
    let api_origin = actual.origin() == api.origin();
    if !api_origin && actual.origin() != web.origin() {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    let (owner, name, native_number, authority_matches) =
        // Selected by origin, not by path content: a web URL whose first
        // segment happens to be `repos` is a repository named `repos`, not an
        // API URL, and must not skip validation by taking the API branch.
        if let Some((prefix, path)) = actual
            .path()
            .split_once("/repos/")
            .filter(|_| api_origin)
        {
            let mut parts = path.split('/');
            let (Some(owner), Some(name), Some(kind), Some(number)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return Ok(());
            };
            if !matches!(kind, "pulls" | "issues") {
                return Ok(());
            }
            (
                owner,
                name,
                number,
                actual.origin() == api.origin() && prefix == api.path().trim_end_matches('/'),
            )
        } else {
            let mut parts = actual.path().trim_start_matches('/').split('/');
            let (Some(owner), Some(name), Some(kind), Some(number)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return Ok(());
            };
            if !matches!(kind, "pull" | "issues") {
                return Ok(());
            }
            let number = number
                .strip_suffix(".diff")
                .or_else(|| number.strip_suffix(".patch"))
                .unwrap_or(number);
            (owner, name, number, actual.origin() == web.origin())
        };
    if native_number.is_empty() || !native_number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(());
    }
    let native_number = native_number
        .parse::<u64>()
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let identity = GithubRepositoryIdentity::parse(&format!("{owner}/{name}"))
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    if !authority_matches
        || identity != *repository
        || native_number != number
        || !actual.username().is_empty()
        || actual.password().is_some()
    {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    Ok(())
}
fn repository_link_identity(
    link: &Presence<String>,
    origin: &Url,
    api: bool,
) -> Result<Option<GithubRepositoryIdentity>, ResourceError> {
    let Some(link) = link.value() else {
        return Ok(None);
    };
    if link.contains('{') || link.contains('}') {
        return Ok(None);
    }
    let url = match Url::parse(link) {
        Ok(url) => url,
        Err(_) => return Ok(None),
    };
    let (path, prefix_matches) = if api {
        let Some((prefix, path)) = url.path().split_once("/repos/") else {
            return Ok(None);
        };
        (path, prefix == origin.path().trim_end_matches('/'))
    } else {
        (url.path().trim_start_matches('/'), true)
    };
    let mut parts = path.trim_end_matches('/').split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return Ok(None);
    };
    if owner.is_empty() || name.is_empty() {
        return Ok(None);
    }
    let identity = GithubRepositoryIdentity::parse(&format!("{owner}/{name}"))
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    if !prefix_matches
        || url.origin() != origin.origin()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    Ok(Some(identity))
}

fn validate_repository(
    repository: &Presence<Repository>,
    expected: Option<&GithubRepositoryIdentity>,
    api: &Url,
    web: &Url,
) -> Result<(), ResourceError> {
    let Some(repository) = repository.value() else {
        return Ok(());
    };
    let full = repository
        .full_name
        .value()
        .map(|full| {
            GithubRepositoryIdentity::parse(full)
                .map_err(|_| failure(ErrorReason::UpstreamMalformed))
        })
        .transpose()?;
    let owner = repository
        .owner
        .value()
        .and_then(|actor| actor.login.value());
    let name = repository.name.value();
    let composed = match (owner, name) {
        (Some(owner), Some(name)) => Some(
            GithubRepositoryIdentity::parse(&format!("{owner}/{name}"))
                .map_err(|_| failure(ErrorReason::UpstreamMalformed))?,
        ),
        _ => None,
    };
    let api_identity = repository_link_identity(&repository.url, api, true)?;
    let web_identity = repository_link_identity(&repository.html_url, web, false)?;
    let identities = [
        expected,
        full.as_ref(),
        composed.as_ref(),
        api_identity.as_ref(),
        web_identity.as_ref(),
    ];
    let mut identities = identities.into_iter().flatten();
    if let Some(first) = identities.next()
        && (owner.is_some_and(|owner| !owner.eq_ignore_ascii_case(first.owner()))
            || name.is_some_and(|name| !name.eq_ignore_ascii_case(first.repository()))
            || identities.any(|identity| identity != first))
    {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    Ok(())
}
