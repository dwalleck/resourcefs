//! Conditional GitHub acquisition, session caching, and confined pagination.

use std::sync::Arc;

use resourcefs_core::{
    ErrorCategory, GithubRepositoryIdentity, ResourceError, SessionCacheEntry, SessionCacheKey,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::{HttpRequest, http::BoundedRead};

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
}

pub(super) struct FetchedResponse {
    body: Arc<[u8]>,
    link: Option<String>,
}

impl FetchedResponse {
    pub(super) fn body(&self) -> &[u8] {
        &self.body
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
            .map_err(|_| malformed_upstream("GitHub session cache metadata is corrupt"))?;
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
        let response = operation.fetch(request).await?;
        if response.status() == 304 {
            if self
                .session
                .cache_generation(GITHUB_CACHE_NAMESPACE)
                .await?
                != cache_generation
            {
                return Err(github_error(
                    ErrorCategory::SourceUnavailable,
                    "GitHub read was invalidated by a concurrent mutation; retry the read",
                ));
            }
            let (Some(entry), Some(metadata)) = (cached.as_ref(), cached_metadata.as_ref()) else {
                return Err(malformed_upstream(
                    "GitHub returned 304 without a session cache entry",
                ));
            };
            return Ok(FetchedResponse {
                body: Arc::clone(entry.content_arc()),
                link: metadata.link.clone(),
            });
        }
        self.classify_status(&response)?;
        if response.truncated() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                "GitHub response exceeds the bounded HTTP body ceiling",
            ));
        }
        let etag = response.etag().map(str::to_owned);
        // An unreadable Link is a corrupt continuation, not the last page.
        let link = response.link()?.map(str::to_owned);
        let body = response.into_body();
        let entry = SessionCacheEntry::new(
            etag.as_ref().map_or_else(Vec::new, |etag| {
                serde_json::to_vec(&CacheMetadata {
                    etag: etag.clone(),
                    link: link.clone(),
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
            403 if response.rate_limit_remaining() == Some(0)
                || response.retry_after().is_some() =>
            {
                Err(github_error(
                    ErrorCategory::SourceUnavailable,
                    "GitHub rate limit is exhausted",
                ))
            }
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
        Ok(Some(target))
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
