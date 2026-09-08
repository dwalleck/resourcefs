//! Conditional GitHub acquisition, session caching, and confined pagination.

use std::sync::Arc;

use resourcefs_core::{
    ErrorCategory, ErrorReason, GithubRepositoryIdentity, ResourceError, ResourceErrorDetails,
    SessionCacheEntry, SessionCacheKey,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::{
    HttpRequest,
    http::{BoundedRead, HttpReadBudget},
};

use super::{
    GITHUB_API_VERSION, GITHUB_JSON, GithubSource, PAGE_SIZE, USER_AGENT, github_error,
    malformed_upstream, wire,
};

pub(super) const GITHUB_CACHE_NAMESPACE: &str = "github-http";
const MAX_PAGES_PER_OPERATION: u64 = 10;

#[derive(Debug, Serialize, Deserialize)]
struct CacheMetadata {
    etag: String,
    link: Option<String>,
    #[serde(default)]
    observation: Option<BodyObservation>,
}

pub(super) struct FetchedResponse {
    pub(super) body: Arc<[u8]>,
    pub(super) link: Option<String>,
    pub(super) observation: BodyObservation,
    pub(super) revalidation: Option<BodyObservation>,
    pub(super) cache_generation: u64,
}

/// One controlled collection page: the accepted body, its provenance and the
/// confined target of the next page, if the provider names one.
pub(super) struct PageResponse {
    pub(super) body: Arc<[u8]>,
    pub(super) observation: BodyObservation,
    pub(super) revalidation: Option<BodyObservation>,
    pub(super) cache_generation: u64,
    pub(super) next: Option<Url>,
}

impl FetchedResponse {
    pub(super) fn body(&self) -> &[u8] {
        &self.body
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BodyObservation {
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_api_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at_unix_ms: Option<u64>,
}

impl BodyObservation {
    fn from_response(response: &crate::BoundedHttpResponse) -> Result<Self, ResourceError> {
        Ok(Self {
            status: Some(response.status()),
            etag: response.etag().map(str::to_owned),
            last_modified: response.last_modified().map(str::to_owned),
            date: response.date().map(str::to_owned),
            selected_api_version: response.selected_api_version().map(str::to_owned),
            observed_at_unix_ms: Some(unix_ms()?),
        })
    }

    fn legacy(etag: &str) -> Self {
        Self {
            status: None,
            etag: Some(etag.to_owned()),
            last_modified: None,
            date: None,
            selected_api_version: None,
            observed_at_unix_ms: None,
        }
    }
}

pub(super) fn unix_ms() -> Result<u64, ResourceError> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| {
            github_error(
                ErrorCategory::SourceUnavailable,
                "Observation clock is unavailable",
            )
        })?;
    u64::try_from(duration.as_millis()).map_err(|_| {
        github_error(
            ErrorCategory::SourceUnavailable,
            "Observation clock is out of range",
        )
    })
}

/// GitHub signals exhaustion on a 403 through the rate-limit headers; without
/// them a 403 is an authorization refusal.
///
/// Both classifiers below triage the same upstream response, so they share one
/// predicate rather than restating it in opposite polarities that must be kept
/// in agreement by hand.
fn is_rate_limited(response: &crate::BoundedHttpResponse) -> bool {
    response.rate_limit_remaining() == Some(0) || response.retry_after().is_some()
}

fn classify_facts_status(response: &crate::BoundedHttpResponse) -> Result<(), ResourceError> {
    use resourcefs_core::{
        AccessAmbiguity, ErrorReason, HttpStatus, ResourceErrorDetails, RetryGuidance,
    };
    let (category, reason) = match response.status() {
        // The same success range the legacy classifier accepts: a 2xx that is
        // not 200 is not an outage on one path and a success on the other.
        200..=299 => return Ok(()),
        401 => (ErrorCategory::PermissionDenied, ErrorReason::UpstreamDenied),
        403 if !is_rate_limited(response) => {
            (ErrorCategory::PermissionDenied, ErrorReason::UpstreamDenied)
        }
        403 | 429 => (
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamRateLimited,
        ),
        404 => (
            ErrorCategory::NotFound,
            ErrorReason::UpstreamNotFoundOrHidden,
        ),
        _ => (
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamUnavailable,
        ),
    };
    let mut details =
        ResourceErrorDetails::new(reason).with_http_status(HttpStatus::new(response.status())?);
    if response.status() == 404 {
        details = details.with_access_ambiguity(AccessAmbiguity::MissingOrAccessHidden);
    }
    if let Some(delay) = response.retry_after() {
        details = details.with_retry_guidance(RetryGuidance::DelaySeconds(delay.as_secs()));
    }
    if let Some(reset) = response.rate_limit_reset() {
        details = details.with_rate_limit_reset(reset);
    }
    Err(ResourceError::new(category, "GitHub facts acquisition failed").with_details(details))
}

fn observation_error(error: ResourceError, controlled: bool, reason: ErrorReason) -> ResourceError {
    if controlled {
        error.with_details(ResourceErrorDetails::new(reason))
    } else {
        error
    }
}

impl GithubSource {
    /// The URL of one upstream page of a collection, at the fixed page size.
    pub(super) fn page_url(
        &self,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
        parameters: &[(&str, &str)],
        page: u64,
    ) -> Result<Url, ResourceError> {
        let mut url = self.endpoint(repository, suffix)?;
        {
            let mut query = url.query_pairs_mut();
            for (name, value) in parameters {
                query.append_pair(name, value);
            }
            query.append_pair("per_page", &PAGE_SIZE.to_string());
            query.append_pair("page", &page.to_string());
        }
        Ok(url)
    }

    fn request(url: Url, accept: &str, etag: Option<&str>) -> Result<HttpRequest, ResourceError> {
        let request = HttpRequest::get(url)
            .with_header("Accept", accept)
            .and_then(|request| request.with_header("User-Agent", USER_AGENT))
            .and_then(|request| request.with_header("X-GitHub-Api-Version", GITHUB_API_VERSION))?;
        match etag {
            Some(etag) => request.with_header("If-None-Match", etag),
            None => Ok(request),
        }
    }

    fn cache_key(url: &Url, accept: &str) -> Result<SessionCacheKey, ResourceError> {
        SessionCacheKey::new(GITHUB_CACHE_NAMESPACE, format!("{accept}\n{url}"))
    }

    pub(super) async fn fetch(
        &self,
        url: Url,
        accept: &str,
        operation: BoundedRead<'_>,
    ) -> Result<FetchedResponse, ResourceError> {
        self.fetch_controlled(url, accept, operation, None).await
    }

    pub(super) async fn fetch_controlled(
        &self,
        url: Url,
        accept: &str,
        operation: BoundedRead<'_>,
        mut budget: Option<&mut HttpReadBudget>,
    ) -> Result<FetchedResponse, ResourceError> {
        let controlled = budget.is_some();
        let response_ceiling = operation.response_ceiling;
        let key = Self::cache_key(&url, accept)?;
        let cache_generation = self
            .session
            .cache_generation(GITHUB_CACHE_NAMESPACE)
            .await?;
        let mut cached = self.session.cache_get(&key).await?;
        let mut cached_metadata = cached
            .as_ref()
            .map(|entry| serde_json::from_slice::<CacheMetadata>(entry.metadata()))
            .transpose()
            .map_err(|_| {
                observation_error(
                    malformed_upstream("GitHub session cache metadata is corrupt"),
                    controlled,
                    ErrorReason::UpstreamMalformed,
                )
            })?;
        // A validator the request header ceiling cannot carry is no validator:
        // drop the entry and fetch unconditionally rather than leave a URL
        // unreadable for the rest of the session behind a retained ETag.
        if let Some(metadata) = cached_metadata.as_ref()
            && let Err(error) = Self::request(url.clone(), accept, Some(&metadata.etag))
        {
            if error.category() != ErrorCategory::LimitExceeded {
                return Err(error);
            }
            self.session
                .cache_remove_if_generation(&key, cache_generation)
                .await?;
            cached = None;
            cached_metadata = None;
        }
        let validator = cached_metadata
            .as_ref()
            .map(|metadata| metadata.etag.as_str());
        let request = Self::request(url, accept, validator)?;
        let response = match budget.as_deref_mut() {
            Some(budget) => operation.fetch_with_budget(request, budget).await?,
            None => operation.fetch(request).await?,
        };
        let observation = BodyObservation::from_response(&response)?;
        if response.status() == 304 {
            if self
                .session
                .cache_generation(GITHUB_CACHE_NAMESPACE)
                .await?
                != cache_generation
            {
                return Err(observation_error(
                    github_error(
                        ErrorCategory::SourceUnavailable,
                        "GitHub read was invalidated by a concurrent mutation; retry the read",
                    ),
                    controlled,
                    ErrorReason::UpstreamUnavailable,
                ));
            }
            let (Some(entry), Some(metadata)) = (cached.as_ref(), cached_metadata.as_ref()) else {
                return Err(observation_error(
                    malformed_upstream("GitHub returned 304 without a session cache entry"),
                    controlled,
                    ErrorReason::UpstreamMalformed,
                ));
            };
            if let Some(budget) = budget {
                budget.admit_body(entry.content_arc().len())?;
            }
            return Ok(FetchedResponse {
                body: Arc::clone(entry.content_arc()),
                link: metadata.link.clone(),
                observation: metadata
                    .observation
                    .clone()
                    .unwrap_or_else(|| BodyObservation::legacy(&metadata.etag)),
                revalidation: Some(observation),
                cache_generation,
            });
        }
        if budget.is_some() {
            classify_facts_status(&response)?;
        } else {
            self.classify_status(&response)?;
        }
        if response.truncated() {
            if let Some(budget) = budget.as_deref_mut() {
                budget.admit_body(response.body().len().saturating_add(1))?;
            }
            let refusal = ResourceError::new(
                ErrorCategory::LimitExceeded,
                "GitHub response exceeds the bounded HTTP body ceiling",
            );
            // A ceiling refusal that does not name the ceiling leaves the
            // caller nothing to lower; every other controlled failure here
            // carries its reason.
            return Err(if controlled {
                refusal.with_details(
                    ResourceErrorDetails::new(ErrorReason::LimitExceeded).with_limit(
                        resourcefs_core::LimitDetail::new(
                            resourcefs_core::AcquisitionLimitKind::ResponseBodyBytes,
                            response_ceiling as u64,
                            None,
                        )?,
                    ),
                )
            } else {
                refusal
            });
        }
        let etag = response.etag().map(str::to_owned);
        // An unreadable Link is a corrupt continuation, not the last page.
        let link = response
            .link()
            .map_err(|error| observation_error(error, controlled, ErrorReason::UpstreamMalformed))?
            .map(str::to_owned);
        if let Some(budget) = budget {
            budget.admit_body(response.body().len())?;
        }
        let body = response.into_body();
        let entry = SessionCacheEntry::new(
            etag.as_ref().map_or_else(Vec::new, |etag| {
                serde_json::to_vec(&CacheMetadata {
                    etag: etag.clone(),
                    link: link.clone(),
                    observation: Some(observation.clone()),
                })
                .expect("CacheMetadata serialization cannot fail")
            }),
            body,
        )?;
        if etag.is_some() {
            self.cache(key, &entry, cache_generation).await?;
        } else {
            self.session
                .cache_remove_if_generation(&key, cache_generation)
                .await?;
        }
        Ok(FetchedResponse {
            body: Arc::clone(entry.content_arc()),
            link,
            observation,
            revalidation: None,
            cache_generation,
        })
    }

    /// Fetches one controlled collection page and resolves its next target
    /// inside the same origin and repository endpoint family.
    pub(super) async fn facts_page(
        &self,
        url: Url,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
        operation: BoundedRead<'_>,
        budget: &mut HttpReadBudget,
    ) -> Result<PageResponse, ResourceError> {
        let response = self
            .fetch_controlled(url, GITHUB_JSON, operation, Some(budget))
            .await?;
        let next = self.next_link(response.link.as_deref(), repository, suffix)?;
        Ok(PageResponse {
            body: response.body,
            observation: response.observation,
            revalidation: response.revalidation,
            cache_generation: response.cache_generation,
            next,
        })
    }

    /// Caches a validated response for conditional revalidation.
    ///
    /// A ceiling refusal is not a read failure: the substrate accepted the
    /// bytes and they are returned to the caller uncached. Any previous entry
    /// under the key is dropped so a later `304` can never revive content the
    /// refused response replaced. Every other cache failure is propagated.
    async fn cache(
        &self,
        key: SessionCacheKey,
        entry: &SessionCacheEntry,
        generation: u64,
    ) -> Result<(), ResourceError> {
        match self
            .session
            .cache_put_if_generation(key.clone(), entry.clone(), generation)
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if error.category() == ErrorCategory::LimitExceeded => {
                self.session
                    .cache_remove_if_generation(&key, generation)
                    .await?;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn classify_status(&self, response: &crate::BoundedHttpResponse) -> Result<(), ResourceError> {
        match response.status() {
            200..=299 => Ok(()),
            401 => Err(github_error(
                ErrorCategory::PermissionDenied,
                "GitHub rejected the configured credential",
            )),
            403 if is_rate_limited(response) => Err(github_error(
                ErrorCategory::SourceUnavailable,
                "GitHub rate limit is exhausted",
            )),
            403 => Err(github_error(
                ErrorCategory::PermissionDenied,
                "GitHub denied the requested operation",
            )),
            404 => Err(github_error(
                ErrorCategory::NotFound,
                "GitHub Resource was not found or is access-hidden",
            )),
            429 => Err(github_error(
                ErrorCategory::SourceUnavailable,
                "GitHub rate limit is exhausted",
            )),
            500..=599 => Err(github_error(
                ErrorCategory::SourceUnavailable,
                "GitHub upstream is unavailable",
            )),
            status => Err(github_error(
                ErrorCategory::SourceUnavailable,
                &format!("GitHub upstream returned HTTP status {status}"),
            )),
        }
    }

    /// Resolves the `rel="next"` target of a page's Link header, confined to
    /// the same API origin and the same repository endpoint family.
    ///
    /// Two path spellings are accepted for the family: the `/repos/{owner}/
    /// {repo}/{suffix}` form this adapter requests, and `/repositories/{id}/
    /// {suffix}`, which is what the live API actually returns (evidence P4).
    /// The numeric id is the upstream's own name for the repository it was
    /// just asked about, so following it discloses nothing new to the origin.
    fn next_link(
        &self,
        link: Option<&str>,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
    ) -> Result<Option<Url>, ResourceError> {
        let Some(link) = link else {
            return Ok(None);
        };
        let Some(target) = next_link_target(link)? else {
            return Ok(None);
        };
        let target = Url::parse(target).map_err(|_| malformed_link())?;
        self.confine_next(target, repository, suffix).map(Some)
    }

    /// Confines a next target to the configured API origin and the same
    /// repository endpoint family, whether it came from a Link header or from
    /// an opaque continuation handle.
    pub(super) fn confine_next(
        &self,
        target: Url,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
    ) -> Result<Url, ResourceError> {
        let repository_path = self.endpoint(repository, suffix)?.path().to_owned();
        let same_origin = target.scheme() == self.api_base.scheme()
            && target.host_str() == self.api_base.host_str()
            && target.port_or_known_default() == self.api_base.port_or_known_default();
        let same_family = target.path().eq_ignore_ascii_case(&repository_path)
            || self.is_repository_id_path(target.path(), suffix);
        if !same_origin || !same_family {
            return Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "GitHub pagination Link leaves its repository endpoint authority",
            ));
        }
        Ok(target)
    }

    fn is_repository_id_path(&self, path: &str, suffix: &str) -> bool {
        let Some(rest) = path.strip_prefix(self.api_base.path()) else {
            return false;
        };
        let Some(rest) = rest.strip_prefix("repositories/") else {
            return false;
        };
        let Some((id, rest)) = rest.split_once('/') else {
            return false;
        };
        !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()) && rest == suffix
    }

    /// Fetches one logical collection from `start_page`, following at most
    /// [`MAX_PAGES_PER_OPERATION`] Link targets, and returns the objects with
    /// the number of the first page not fetched.
    pub(super) async fn collection<T: DeserializeOwned>(
        &self,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
        parameters: &[(&str, &str)],
        start_page: u64,
        operation: BoundedRead<'_>,
    ) -> Result<(Vec<T>, Option<u64>), ResourceError> {
        let mut next = self.page_url(repository, suffix, parameters, start_page)?;
        let mut values = Vec::new();
        for _ in 0..MAX_PAGES_PER_OPERATION {
            let response = self.fetch(next, GITHUB_JSON, operation).await?;
            let following = self.next_link(response.link.as_deref(), repository, suffix)?;
            let mut page: Vec<T> = wire::decode(response.body())?;
            values.append(&mut page);
            let Some(link) = following else {
                return Ok((values, None));
            };
            next = link;
        }
        let continuation = next
            .query_pairs()
            .find_map(|(name, value)| (name == "page").then(|| value.parse::<u64>().ok()))
            .flatten()
            .ok_or_else(|| malformed_upstream("GitHub next Link does not name a numeric page"))?;
        Ok((values, Some(continuation)))
    }

    pub(super) async fn json<T: DeserializeOwned>(
        &self,
        url: Url,
        operation: BoundedRead<'_>,
    ) -> Result<T, ResourceError> {
        let response = self.fetch(url, GITHUB_JSON, operation).await?;
        wire::decode(response.body())
    }
}

/// Extracts the `rel="next"` target of a Link header (RFC 8288), or `None`
/// when the header names no next relation.
///
/// Strict on shape: a link-value that does not open with `<`, never closes
/// its `>`, or carries an unparseable parameter is an error, never "no next
/// page" — an unlabeled partial listing is exactly what a lenient parse would
/// produce. Lenient on spelling: `rel=next`, `rel="next"`, `rel="next last"`,
/// and any parameter-name case all name the same relation.
fn next_link_target(link: &str) -> Result<Option<&str>, ResourceError> {
    let mut rest = link.trim_start();
    let mut next = None;
    while !rest.is_empty() {
        let Some(after_open) = rest.strip_prefix('<') else {
            return Err(malformed_link());
        };
        let Some((target, after_target)) = after_open.split_once('>') else {
            return Err(malformed_link());
        };
        let (parameters, remainder) = split_first_outside_quotes(after_target, b',');
        if link_value_is_next(parameters)? && next.replace(target).is_some() {
            return Err(malformed_link());
        }
        rest = remainder.unwrap_or("").trim_start();
    }
    Ok(next)
}

fn link_value_is_next(parameters: &str) -> Result<bool, ResourceError> {
    let mut is_next = false;
    let mut rest = Some(parameters);
    while let Some(remaining) = rest {
        let (parameter, remainder) = split_first_outside_quotes(remaining, b';');
        rest = remainder;
        let parameter = parameter.trim();
        if parameter.is_empty() {
            continue;
        }
        let Some((name, value)) = parameter.split_once('=') else {
            return Err(malformed_link());
        };
        if name.trim().eq_ignore_ascii_case("rel")
            && value
                .trim()
                .trim_matches('"')
                .split_ascii_whitespace()
                .any(|relation| relation.eq_ignore_ascii_case("next"))
        {
            is_next = true;
        }
    }
    Ok(is_next)
}

/// Splits `value` at the first `delimiter` that sits outside a quoted-string,
/// returning the head and the remainder after the delimiter, if any.
fn split_first_outside_quotes(value: &str, delimiter: u8) -> (&str, Option<&str>) {
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in value.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            _ if byte == delimiter && !quoted => {
                return (&value[..index], Some(&value[index + 1..]));
            }
            _ => {}
        }
    }
    (value, None)
}

fn malformed_link() -> ResourceError {
    malformed_upstream("GitHub pagination Link is malformed")
}
