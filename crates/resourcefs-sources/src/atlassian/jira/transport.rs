use std::sync::Arc;

use resourcefs_core::{ErrorCategory, ResourceError, SessionCacheEntry, SessionCacheKey};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    HttpRequest,
    http::{BoundedRead, HttpReadBudget},
};

use super::{AtlassianSite, AtlassianSource, malformed_upstream};

const ACCEPT: &str = "application/json";
const USER_AGENT: &str = "resourcefs/1.0 jira-read";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheMetadata {
    etag: String,
}

pub(super) struct FetchedResponse {
    pub(super) body: Arc<[u8]>,
}

/// GET/cache/status state for one Jira read with an optional collection budget.
pub(super) struct JiraRead<'a> {
    source: &'a AtlassianSource,
    operation: BoundedRead<'a>,
    budget: Option<HttpReadBudget>,
}

impl<'a> JiraRead<'a> {
    pub(super) fn direct(source: &'a AtlassianSource, operation: BoundedRead<'a>) -> Self {
        Self {
            source,
            operation,
            budget: None,
        }
    }

    pub(super) fn collection(
        source: &'a AtlassianSource,
        operation: BoundedRead<'a>,
        max_attempts: usize,
    ) -> Result<Self, ResourceError> {
        Ok(Self {
            source,
            operation,
            budget: Some(HttpReadBudget::new(max_attempts)?),
        })
    }

    pub(super) fn remaining_attempts(&self) -> Option<usize> {
        self.budget.as_ref().map(HttpReadBudget::remaining_attempts)
    }

    async fn fetch(
        &mut self,
        request: HttpRequest,
    ) -> Result<crate::BoundedHttpResponse, ResourceError> {
        match self.budget.as_mut() {
            Some(budget) => self.operation.fetch_with_budget(request, budget).await,
            None => self.operation.fetch(request).await,
        }
        .map_err(Self::sanitize_fetch_error)
    }

    fn request(url: Url, etag: Option<&str>) -> Result<HttpRequest, ResourceError> {
        let request = HttpRequest::get(url)
            .with_header("Accept", ACCEPT)
            .and_then(|request| request.with_header("User-Agent", USER_AGENT))?;
        match etag {
            Some(etag) => request.with_header("If-None-Match", etag),
            None => Ok(request),
        }
    }

    fn cache_namespace(site: &AtlassianSite) -> String {
        format!("atlassian.jira.{}", site.id().as_str())
    }

    fn cache_key(site: &AtlassianSite, url: &Url) -> Result<SessionCacheKey, ResourceError> {
        SessionCacheKey::new(Self::cache_namespace(site), format!("{ACCEPT}\n{url}"))
    }

    pub(super) async fn fetch_cached(
        &mut self,
        site: &AtlassianSite,
        url: Url,
    ) -> Result<FetchedResponse, ResourceError> {
        let namespace = Self::cache_namespace(site);
        let key = Self::cache_key(site, &url)?;
        let generation = self.source.session.cache_generation(&namespace).await?;
        let mut cached = self.source.session.cache_get(&key).await?;
        let mut metadata = cached
            .as_ref()
            .map(|entry| serde_json::from_slice::<CacheMetadata>(entry.metadata()))
            .transpose()
            .map_err(|_| malformed_upstream("Jira session cache metadata is corrupt"))?;
        if metadata
            .as_ref()
            .is_some_and(|metadata| metadata.etag.is_empty())
        {
            return Err(malformed_upstream("Jira session cache ETag is empty"));
        }
        if let Some(entry) = metadata.as_ref()
            && let Err(error) = Self::request(url.clone(), Some(&entry.etag))
        {
            if error.category() != ErrorCategory::LimitExceeded {
                return Err(error);
            }
            self.source
                .session
                .cache_remove_if_generation(&key, generation)
                .await?;
            cached = None;
            metadata = None;
        }
        let expected_url = url.clone();
        let request = Self::request(
            url,
            metadata.as_ref().map(|metadata| metadata.etag.as_str()),
        )?;
        let response = self.fetch(request).await?;
        if response.final_url() != &expected_url {
            return Err(malformed_upstream(
                "Jira endpoint returned an unexpected redirect",
            ));
        }
        if response.status() == 304 {
            if self.source.session.cache_generation(&namespace).await? != generation {
                return Err(malformed_upstream(
                    "Jira read was invalidated during cache revalidation",
                ));
            }
            let (Some(entry), Some(_)) = (cached.as_ref(), metadata.as_ref()) else {
                return Err(malformed_upstream(
                    "Jira returned 304 without a matching session cache entry",
                ));
            };
            return Ok(FetchedResponse {
                body: Arc::clone(entry.content_arc()),
            });
        }
        Self::classify_status(&response)?;
        if response.truncated() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                "Jira response exceeds the bounded HTTP body ceiling",
            ));
        }
        // A zero-byte header cannot revalidate; the quoted opaque tag `""` can.
        let etag = response
            .etag()
            .filter(|etag| !etag.is_empty())
            .map(str::to_owned);
        let body = response.into_body();
        let entry = SessionCacheEntry::new(
            etag.as_ref().map_or_else(Vec::new, |etag| {
                serde_json::to_vec(&CacheMetadata { etag: etag.clone() })
                    .expect("CacheMetadata serialization cannot fail")
            }),
            body,
        )?;
        if etag.is_some() {
            self.cache(key, &entry, generation).await?;
        } else {
            self.source
                .session
                .cache_remove_if_generation(&key, generation)
                .await?;
        }
        Ok(FetchedResponse {
            body: Arc::clone(entry.content_arc()),
        })
    }

    pub(super) async fn fetch_uncached(
        &mut self,
        url: Url,
    ) -> Result<FetchedResponse, ResourceError> {
        let expected_url = url.clone();
        let response = self.fetch(Self::request(url, None)?).await?;
        if response.final_url() != &expected_url {
            return Err(malformed_upstream(
                "Jira endpoint returned an unexpected redirect",
            ));
        }
        Self::classify_status(&response)?;
        if response.truncated() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                "Jira response exceeds the bounded HTTP body ceiling",
            ));
        }
        Ok(FetchedResponse {
            body: Arc::from(response.into_body()),
        })
    }

    // A ceiling refusal returns the new body uncached and evicts any old entry,
    // so a later 304 cannot revive the content that this response replaced.
    async fn cache(
        &self,
        key: SessionCacheKey,
        entry: &SessionCacheEntry,
        generation: u64,
    ) -> Result<(), ResourceError> {
        match self
            .source
            .session
            .cache_put_if_generation(key.clone(), entry.clone(), generation)
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if error.category() == ErrorCategory::LimitExceeded => {
                self.source
                    .session
                    .cache_remove_if_generation(&key, generation)
                    .await?;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn classify_status(response: &crate::BoundedHttpResponse) -> Result<(), ResourceError> {
        match response.status() {
            200 => Ok(()),
            401 => Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "Jira rejected the configured credential",
            )),
            403 if response.retry_after().is_some() => Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "Jira rate limit is exhausted",
            )),
            403 => Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "Jira denied the requested operation",
            )),
            404 | 410 => Err(ResourceError::new(
                ErrorCategory::NotFound,
                "Jira Resource was not found or is access-hidden",
            )),
            413 => Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                "Jira response exceeds the accepted body limit",
            )),
            429 => Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "Jira rate limit is exhausted",
            )),
            500..=599 => Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "Jira upstream is unavailable",
            )),
            status => Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("Jira upstream returned HTTP status {status}"),
            )),
        }
    }

    fn sanitize_fetch_error(error: ResourceError) -> ResourceError {
        match error.category() {
            ErrorCategory::SourceUnavailable => ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "Jira upstream request failed",
            ),
            ErrorCategory::PermissionDenied => ResourceError::new(
                ErrorCategory::PermissionDenied,
                "Jira request was refused by egress policy",
            ),
            _ => error,
        }
    }
}
