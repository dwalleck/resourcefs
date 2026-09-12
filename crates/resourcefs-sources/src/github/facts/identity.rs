//! Validating native PR, repository and conversation-comment identities without
//! acquiring linked resources.
use resourcefs_core::{
    ConversationCommentId, ErrorCategory, ErrorReason, GithubRepositoryIdentity, PullRequestNumber,
    ResourceError, ResourceErrorDetails, ReviewCommentId, ReviewId,
};
use url::Url;

use super::comment::NativeComment;
use super::pull::{Branch, NativePull};
use super::{NativeId, Presence, Repository, failure};

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
    number: PullRequestNumber,
    endpoint: &Url,
    api: &Url,
    web: &Url,
) -> Result<ValidatedIdentity<'a>, ResourceError> {
    let id = required(&pull.id)?;
    let observed_number = required(&pull.number)?;
    if observed_number.0 != number.get() {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    same_url(required(&pull.url)?, endpoint)?;
    for link in [
        &pull.html_url,
        &pull.diff_url,
        &pull.patch_url,
        &pull.issue_url,
    ] {
        validate_observed_link(link, repository, number.get(), api, web)?;
    }
    if let Some(relations) = pull._links.value() {
        for name in ["self", "html", "issue", "comments", "review_comments"] {
            if let Some(Presence::Present(relation)) = relations.0.get(name) {
                validate_observed_link(&relation.href, repository, number.get(), api, web)?;
            }
        }
    }
    let base = required(&pull.base)?;
    let head = required(&pull.head)?;
    let base_sha = sha(&base.sha)?;
    let head_sha = sha(&head.sha)?;
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

/// Validate the authority and recognizable identity of comment links. Empty
/// and non-URL provider values remain lossless opaque observations, while a
/// parsed URL must belong to this deployment and cannot carry credentials.
pub(super) fn validate_comment_links(
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    comment_id: Option<ConversationCommentId>,
    comment: &NativeComment,
    api: &Url,
    web: &Url,
) -> Result<(), ResourceError> {
    let observed_id = comment
        .id
        .value()
        .ok_or_else(|| failure(ErrorReason::UpstreamMalformed))?;
    if comment_id.is_some_and(|comment_id| observed_id.0 != comment_id.get()) {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    let comment_id = comment_id.unwrap_or_else(|| {
        ConversationCommentId::new(observed_id.0).expect("decoded native IDs are positive")
    });
    let issue = comment
        .issue_url
        .value()
        .ok_or_else(|| failure(ErrorReason::UpstreamIdentityMismatch))?;
    let issue = Url::parse(issue).map_err(|_| failure(ErrorReason::UpstreamIdentityMismatch))?;
    validate_api_authority(&issue, api)?;
    let Some((owner, name, family, observed_number)) = api_route(&issue, api)? else {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    };
    if !owner.eq_ignore_ascii_case(repository.owner())
        || !name.eq_ignore_ascii_case(repository.repository())
        || family != "issues"
        || observed_number != number.get()
    {
        return Err(ResourceError::new(
            ErrorCategory::NotFound,
            "GitHub comment does not belong to the addressed pull request",
        )
        .with_details(ResourceErrorDetails::new(
            ErrorReason::UpstreamIdentityMismatch,
        )));
    }
    validate_optional_comment_link(&comment.url, repository, number, comment_id, api, web, true)?;
    validate_optional_comment_link(
        &comment.html_url,
        repository,
        number,
        comment_id,
        api,
        web,
        false,
    )?;
    Ok(())
}

fn validate_optional_comment_link(
    link: &Presence<String>,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    comment_id: ConversationCommentId,
    api: &Url,
    web: &Url,
    api_link: bool,
) -> Result<(), ResourceError> {
    let Some(value) = link.value() else {
        return Ok(());
    };
    let Ok(parsed) = Url::parse(value) else {
        return Ok(());
    };
    let expected = if api_link { api } else { web };
    if !clean_authority(&parsed, expected) {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    if api_link {
        if let Some((owner, name, family, observed_id)) = api_route(&parsed, api)?
            && (!owner.eq_ignore_ascii_case(repository.owner())
                || !name.eq_ignore_ascii_case(repository.repository())
                || family != "issues/comments"
                || observed_id != comment_id.get())
        {
            return Err(failure(ErrorReason::UpstreamIdentityMismatch));
        }
    } else if let Some(route) = web_route(&parsed, web)?
        && (!route.owner.eq_ignore_ascii_case(repository.owner())
            || !route.name.eq_ignore_ascii_case(repository.repository())
            || route.number != number
            || route
                .fragment
                .is_some_and(|fragment| !matches!(fragment, WebFragment::ConversationComment(id) if id == comment_id)))
    {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    Ok(())
}

/// The deployment's own object URL for `suffix` below `repos/<owner>/<name>/`.
pub(super) fn expected_object_url(
    api: &Url,
    repository: &GithubRepositoryIdentity,
    suffix: &str,
) -> Result<Url, ResourceError> {
    api.join(&format!(
        "repos/{}/{}/{suffix}",
        repository.owner(),
        repository.repository()
    ))
    .map_err(|_| failure(ErrorReason::UpstreamMalformed))
}

/// A supplied URL names this deployment's own authority only when its origin
/// matches and it carries no credential. `Url::origin()` ignores userinfo, so
/// every authority comparison must use this rule rather than origin alone:
/// a credential-bearing URL is a contradiction, never an opaque value to
/// publish.
pub(super) fn clean_authority(url: &Url, expected: &Url) -> bool {
    url.origin() == expected.origin() && url.username().is_empty() && url.password().is_none()
}

fn matches_object_url(value: &str, expected: &Url) -> bool {
    Url::parse(value).is_ok_and(|actual| {
        clean_authority(&actual, expected)
            && actual.path().eq_ignore_ascii_case(expected.path())
            && actual.query().is_none()
            && actual.fragment().is_none()
    })
}

/// A required link must name exactly this deployment's own object. A
/// contradiction is `not_found`, matching the conversation-comment family's
/// verdict for a record that does not belong to the addressed pull request.
pub(super) fn require_object_link(
    link: &Presence<String>,
    expected: &Url,
) -> Result<(), ResourceError> {
    match link.value() {
        Some(value) if matches_object_url(value, expected) => Ok(()),
        _ => Err(ResourceError::new(
            ErrorCategory::NotFound,
            "GitHub record does not belong to the addressed pull request",
        )
        .with_details(ResourceErrorDetails::new(
            ErrorReason::UpstreamIdentityMismatch,
        ))),
    }
}

/// A supplied optional link, when it is a URL at all, must name this
/// deployment's own object; opaque provider values stay lossless.
pub(super) fn validate_optional_object_link(
    link: &Presence<String>,
    expected: &Url,
) -> Result<(), ResourceError> {
    match link.value() {
        Some(value) if !matches_object_url(value, expected) => {
            Err(failure(ErrorReason::UpstreamIdentityMismatch))
        }
        _ => Ok(()),
    }
}

/// A supplied optional web link, when it is a recognizable route, must name
/// this repository and pull request. Returns the recognized fragment so the
/// caller can require the family's own object id.
pub(super) fn validate_optional_web_link(
    link: &Presence<String>,
    repository: &GithubRepositoryIdentity,
    number: PullRequestNumber,
    web: &Url,
) -> Result<Option<WebFragment>, ResourceError> {
    let Some(value) = link.value() else {
        return Ok(None);
    };
    let Ok(parsed) = Url::parse(value) else {
        return Ok(None);
    };
    if !clean_authority(&parsed, web) {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    let Some(route) = web_route(&parsed, web)? else {
        return Ok(None);
    };
    if !route.owner.eq_ignore_ascii_case(repository.owner())
        || !route.name.eq_ignore_ascii_case(repository.repository())
        || route.number != number
    {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    Ok(route.fragment)
}

/// The native id every family record must carry, checked against a requested
/// id when the read addressed one object.
pub(super) fn require_observed_id(
    expected: Option<u64>,
    observed: Option<u64>,
) -> Result<u64, ResourceError> {
    let observed = observed.ok_or_else(|| failure(ErrorReason::UpstreamMalformed))?;
    if expected.is_some_and(|expected| expected != observed) {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    Ok(observed)
}

fn validate_api_authority(url: &Url, api: &Url) -> Result<(), ResourceError> {
    if !clean_authority(url, api) {
        return Err(failure(ErrorReason::UpstreamIdentityMismatch));
    }
    Ok(())
}

fn api_route<'a>(
    url: &'a Url,
    api: &Url,
) -> Result<Option<(&'a str, &'a str, &'a str, u64)>, ResourceError> {
    let prefix = api.path().trim_end_matches('/');
    let Some(path) = url.path().strip_prefix(prefix) else {
        return Ok(None);
    };
    let Some(path) = path.strip_prefix("/repos/") else {
        return Ok(None);
    };
    let mut parts = path.trim_end_matches('/').split('/');
    let (Some(owner), Some(name), Some(first), Some(second)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Ok(None);
    };
    let (family, number) = if first == "issues" && second == "comments" {
        let Some(comment_id) = parts.next() else {
            return Ok(None);
        };
        if parts.next().is_some() {
            return Ok(None);
        }
        ("issues/comments", comment_id)
    } else {
        if parts.next().is_some() {
            return Ok(None);
        }
        (first, second)
    };
    let number = number
        .parse::<u64>()
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    Ok(Some((owner, name, family, number)))
}

/// A web link's recognized object fragment, when it carries one.
pub(super) enum WebFragment {
    ConversationComment(ConversationCommentId),
    Review(ReviewId),
    ReviewComment(ReviewCommentId),
}

struct WebObjectRoute<'a> {
    owner: &'a str,
    name: &'a str,
    number: PullRequestNumber,
    fragment: Option<WebFragment>,
}

fn web_route<'a>(url: &'a Url, web: &Url) -> Result<Option<WebObjectRoute<'a>>, ResourceError> {
    if url.origin() != web.origin() {
        return Ok(None);
    }
    let mut parts = url.path().trim_matches('/').split('/');
    let (Some(owner), Some(name), Some(family), Some(number)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Ok(None);
    };
    if parts.next().is_some() || !matches!(family, "pull" | "issues") {
        return Ok(None);
    }
    let number = number
        .parse::<u64>()
        .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
    let number = PullRequestNumber::new(number)
        .map_err(|_| failure(ErrorReason::UpstreamIdentityMismatch))?;
    let fragment = url
        .fragment()
        .map(|fragment| {
            let (kind, value) = if let Some(value) = fragment.strip_prefix("issuecomment-") {
                ("issuecomment", value)
            } else if let Some(value) = fragment.strip_prefix("pullrequestreview-") {
                ("pullrequestreview", value)
            } else if let Some(value) = fragment.strip_prefix("discussion_r") {
                ("discussion_r", value)
            } else {
                return Ok(None);
            };
            let id = value
                .parse::<u64>()
                .map_err(|_| failure(ErrorReason::UpstreamMalformed))?;
            let mismatch = || failure(ErrorReason::UpstreamIdentityMismatch);
            match kind {
                "issuecomment" => {
                    ConversationCommentId::new(id).map(WebFragment::ConversationComment)
                }
                "pullrequestreview" => ReviewId::new(id).map(WebFragment::Review),
                _ => ReviewCommentId::new(id).map(WebFragment::ReviewComment),
            }
            .map_err(|_| mismatch())
            .map(Some)
        })
        .transpose()?
        .flatten();
    Ok(Some(WebObjectRoute {
        owner,
        name,
        number,
        fragment,
    }))
}

pub(super) fn required<T>(field: &Presence<T>) -> Result<&T, ResourceError> {
    field
        .value()
        .ok_or_else(|| failure(ErrorReason::UpstreamMalformed))
}
pub(super) fn sha(field: &Presence<String>) -> Result<&str, ResourceError> {
    let sha = required(field)?;
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

pub(super) fn validate_repository(
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
