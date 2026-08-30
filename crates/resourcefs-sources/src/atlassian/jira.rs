use std::{io::Cursor, sync::Arc};

use resourcefs_core::{
    DiscoveryAdapter, ErrorCategory, GlobOptions, GlobTarget, JiraAddress, JiraIssueResource,
    OperationGuard, PathReference, ProjectionSelector, ResourceAddress, ResourceError,
    SearchOptions, SearchSourceResult, SearchTarget, SessionCacheEntry, SessionCacheKey,
    SourceAdapter, SourceGlobResult, SourceResource, Utf8ContentType, select_utf8,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    HttpRequest,
    catalog::{SourceCatalogEntry, SourceCatalogMetadata},
    http::BoundedRead,
    pattern::search_document,
};

use super::{AtlassianSite, AtlassianSource, render, wire};

const ACCEPT: &str = "application/json";
const USER_AGENT: &str = "resourcefs/1.0 jira-read";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheMetadata {
    etag: String,
}

struct FetchedResponse {
    body: Arc<[u8]>,
}

impl AtlassianSource {
    pub(crate) async fn read_resource(
        &self,
        reference: &PathReference,
        operation: BoundedRead<'_>,
    ) -> Result<SourceResource, ResourceError> {
        let ResourceAddress::Jira(address) = reference.address() else {
            return Err(unsupported_jira_projection());
        };
        let site = self.site(address.site())?;
        let projection = reference.projection();
        if projection.is_some_and(|selector| selector.page_offset().is_some()) {
            return Err(unsupported_jira_projection());
        }

        let (identifier, lookup, cacheable, requested_resource) = match address {
            JiraAddress::Issue {
                issue_id, resource, ..
            } => {
                if matches!(resource, JiraIssueResource::Field(_)) && projection.is_some() {
                    return Err(unsupported_jira_field_projection());
                }
                (
                    issue_id.as_str(),
                    wire::JiraLookup::StableId(issue_id),
                    true,
                    resource,
                )
            }
            JiraAddress::IssueKeyAlias { issue_key, .. } => (
                issue_key.as_str(),
                wire::JiraLookup::IssueKeyAlias,
                false,
                &JiraIssueResource::Aggregate,
            ),
        };
        let url = Self::issue_endpoint(site, identifier)?;
        let fetched = if cacheable {
            self.fetch_cached(site, url, operation).await?
        } else {
            self.fetch_uncached(url, operation).await?
        };
        let issue = wire::decode_issue(&fetched.body, site.origin(), lookup)?;

        match requested_resource {
            JiraIssueResource::Aggregate => {
                let rendered = render::render_issue(site.id(), &issue)?;
                self.markdown_resource(
                    PathReference::jira(
                        JiraAddress::Issue {
                            site: site.id().clone(),
                            issue_id: issue.id.clone(),
                            resource: JiraIssueResource::Aggregate,
                        },
                        None,
                    )?,
                    rendered.aggregate,
                    projection,
                )
            }
            JiraIssueResource::Fields => {
                let field_index = render::render_field_index(site.id(), &issue)?;
                self.markdown_resource(
                    PathReference::jira(
                        JiraAddress::Issue {
                            site: site.id().clone(),
                            issue_id: issue.id.clone(),
                            resource: JiraIssueResource::Fields,
                        },
                        None,
                    )?,
                    field_index,
                    projection,
                )
            }
            JiraIssueResource::Field(field_id) => {
                let field = issue.fields.get(field_id).ok_or_else(|| {
                    ResourceError::new(
                        ErrorCategory::NotFound,
                        "Jira Field was not present on the visible issue",
                    )
                })?;
                let canonical = PathReference::jira(
                    JiraAddress::Issue {
                        site: site.id().clone(),
                        issue_id: issue.id.clone(),
                        resource: JiraIssueResource::Field(field_id.clone()),
                    },
                    None,
                )?;
                SourceResource::utf8(
                    canonical,
                    field.canonical_json.clone(),
                    Utf8ContentType::new(render::field_content_type(field)?)?,
                )
            }
        }
    }

    fn markdown_resource(
        &self,
        canonical: PathReference,
        content: String,
        projection: Option<&ProjectionSelector>,
    ) -> Result<SourceResource, ResourceError> {
        match projection.and_then(ProjectionSelector::line_selection) {
            Some(_) => {
                let selected = select_utf8(Cursor::new(content.as_bytes()), projection)?;
                SourceResource::selected_utf8(canonical, selected, Utf8ContentType::MARKDOWN)
            }
            None => SourceResource::utf8(canonical, content, Utf8ContentType::MARKDOWN),
        }
    }

    fn issue_endpoint(site: &AtlassianSite, identifier: &str) -> Result<Url, ResourceError> {
        let mut endpoint = site
            .origin()
            .base_url()
            .join("rest/api/3/issue/")
            .map_err(|_| malformed_upstream("Jira issue endpoint could not be constructed"))?;
        endpoint
            .path_segments_mut()
            .map_err(|_| malformed_upstream("Jira issue endpoint cannot carry path segments"))?
            .pop_if_empty()
            .push(identifier);
        endpoint
            .query_pairs_mut()
            .append_pair("fields", "*all")
            .append_pair("fieldsByKeys", "false")
            .append_pair("expand", "names,schema");
        Ok(endpoint)
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

    async fn fetch_cached(
        &self,
        site: &AtlassianSite,
        url: Url,
        operation: BoundedRead<'_>,
    ) -> Result<FetchedResponse, ResourceError> {
        let namespace = Self::cache_namespace(site);
        let key = Self::cache_key(site, &url)?;
        let generation = self.session.cache_generation(&namespace).await?;
        let mut cached = self.session.cache_get(&key).await?;
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
            self.session
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
        let response = operation
            .fetch(request)
            .await
            .map_err(Self::sanitize_fetch_error)?;
        if response.final_url() != &expected_url {
            return Err(malformed_upstream(
                "Jira issue endpoint returned an unexpected redirect",
            ));
        }
        if response.status() == 304 {
            if self.session.cache_generation(&namespace).await? != generation {
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
        let etag = response.etag().map(str::to_owned);
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
            self.session
                .cache_remove_if_generation(&key, generation)
                .await?;
        }
        Ok(FetchedResponse {
            body: Arc::clone(entry.content_arc()),
        })
    }

    async fn fetch_uncached(
        &self,
        url: Url,
        operation: BoundedRead<'_>,
    ) -> Result<FetchedResponse, ResourceError> {
        let expected_url = url.clone();
        let response = operation
            .fetch(Self::request(url, None)?)
            .await
            .map_err(Self::sanitize_fetch_error)?;
        if response.final_url() != &expected_url {
            return Err(malformed_upstream(
                "Jira issue endpoint returned an unexpected redirect",
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

#[async_trait::async_trait]
impl SourceAdapter for AtlassianSource {
    async fn read(
        &self,
        reference: &PathReference,
        operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        let timeout = self.substrate.ceilings().timeout();
        let operation = self.substrate.begin_read(operation)?;
        tokio::time::timeout(timeout, self.read_resource(reference, operation))
            .await
            .map_err(|_| {
                ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "Jira operation exceeded the configured logical deadline",
                )
            })?
    }
}

impl SourceCatalogMetadata for AtlassianSource {
    fn catalog_entries(&self) -> Result<Vec<SourceCatalogEntry>, ResourceError> {
        Ok(vec![SourceCatalogEntry::new(
            "jira://",
            "jira://<site>/issues/<issue-id>[/fields[/<field-id>]][:selector] | jira://<site>/issue-keys/<issue-key>[:selector]",
            "jira://site/issues/10001",
            None,
        )?])
    }
}

#[async_trait::async_trait]
impl DiscoveryAdapter for AtlassianSource {
    async fn search(
        &self,
        target: &SearchTarget,
        pattern: &str,
        options: SearchOptions,
        operation: &OperationGuard,
    ) -> Result<SearchSourceResult, ResourceError> {
        let reference = target.reference().ok_or_else(unsupported_jira_projection)?;
        let ResourceAddress::Jira(address) = reference.address() else {
            return Err(unsupported_jira_projection());
        };
        let requested = PathReference::jira(address.clone(), None)?;
        let source = self.read(&requested, operation).await?;
        let searched = PathReference::parse(source.canonical_reference().to_owned())?;
        let content = source.content().to_owned();
        let pattern = pattern.to_owned();
        let case_sensitive = options.case_sensitive();
        tokio::task::spawn_blocking(move || {
            search_document(&content, &searched, &pattern, case_sensitive)
        })
        .await
        .map_err(|error| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("Jira discovery worker failed: {error}"),
            )
        })?
    }

    async fn glob(
        &self,
        _target: &GlobTarget,
        _options: GlobOptions,
        _operation: &OperationGuard,
    ) -> Result<SourceGlobResult, ResourceError> {
        Err(ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            "Jira Resources cannot be enumerated by glob; read an explicit Resource",
        ))
    }
}

fn unsupported_jira_projection() -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        "Jira Source Adapter supports direct issue Aggregates, Field indexes, and Fields only",
    )
}

fn unsupported_jira_field_projection() -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        "Jira Field Resources require complete canonical JSON without selectors",
    )
}

fn malformed_upstream(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::SourceUnavailable, message)
}
