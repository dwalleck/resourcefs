pub(super) mod browse;
mod cursor;
mod query;
mod transport;

use std::io::Cursor;

use resourcefs_core::{
    DiscoveryAdapter, ErrorCategory, GlobOptions, GlobTarget, JiraAddress, JiraIssueResource,
    OperationGuard, PathReference, ProjectionSelector, ResourceAddress, ResourceError,
    SearchOptions, SearchSourceResult, SearchTarget, SourceAdapter, SourceGlobResult,
    SourceResource, Utf8ContentType, select_utf8,
};
use url::Url;

use crate::{
    catalog::{SourceCatalogEntry, SourceCatalogMetadata},
    http::BoundedRead,
    pattern::search_document,
};

use super::{AtlassianSite, AtlassianSource, render, wire};
use transport::JiraRead;

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
        if let JiraAddress::Query { query, .. } = address {
            return self
                .read_query(reference, address, query, site, operation)
                .await;
        }
        if matches!(
            address,
            JiraAddress::Projects { .. }
                | JiraAddress::Project { .. }
                | JiraAddress::ProjectKeyAlias { .. }
                | JiraAddress::Issues { .. }
                | JiraAddress::ProjectIssues { .. }
        ) {
            return self.read_browse(reference, address, site, operation).await;
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
            JiraAddress::Projects { .. }
            | JiraAddress::Issues { .. }
            | JiraAddress::Query { .. }
            | JiraAddress::ProjectIssues { .. }
            | JiraAddress::Project { .. }
            | JiraAddress::ProjectKeyAlias { .. } => {
                return Err(unsupported_jira_projection());
            }
        };
        let url = Self::issue_endpoint(site, identifier)?;
        let mut read = JiraRead::direct(self, operation);
        let fetched = if cacheable {
            read.fetch_cached(site, url).await?
        } else {
            read.fetch_uncached(url).await?
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
}

#[async_trait::async_trait]
impl SourceAdapter for AtlassianSource {
    async fn read(
        &self,
        reference: &PathReference,
        operation: &OperationGuard,
        acquisition: Option<&resourcefs_core::ReadAcquisitionLimits>,
    ) -> Result<SourceResource, ResourceError> {
        if acquisition.is_some() {
            return Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "acquisition controls are not supported for this resource",
            )
            .with_details(resourcefs_core::ResourceErrorDetails::new(
                resourcefs_core::ErrorReason::AcquisitionControlsUnsupported,
            )));
        }
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
            "jira://<site>/issues/<issue-id>[/fields[/<field-id>]][:selector] | jira://<site>/issue-keys/<issue-key>[:selector] | jira://<site>/projects[:offset:N] | jira://<site>/projects/<project-id>[:selector] | jira://<site>/project-keys/<project-key>[:selector] | jira://<site>/issues[:cursor:C] | jira://<site>/projects/<project-id>/issues[:cursor:C] | jira://<site>/search/<percent-encoded-jql>[:cursor:C]",
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
        let source_selector = reference
            .projection()
            .filter(|selector| {
                selector.source_offset().is_some() || selector.source_cursor().is_some()
            })
            .cloned();
        let requested = PathReference::jira(address.clone(), source_selector.clone())?;
        let source = self.read(&requested, operation, None).await?;
        let canonical = PathReference::parse(source.canonical_reference().to_owned())?;
        let ResourceAddress::Jira(canonical_address) = canonical.address() else {
            return Err(unsupported_jira_projection());
        };
        let searched = PathReference::jira(canonical_address.clone(), source_selector)?;
        let continuation = source
            .continuation()
            .map(PathReference::parse)
            .transpose()?;
        let content = source.content().to_owned();
        let pattern = pattern.to_owned();
        let case_sensitive = options.case_sensitive();
        let result = tokio::task::spawn_blocking(move || {
            search_document(&content, &searched, &pattern, case_sensitive)
        })
        .await
        .map_err(|error| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("Jira discovery worker failed: {error}"),
            )
        })??;
        Ok(match continuation {
            Some(next) => result.with_source_continuation(next),
            None => result,
        })
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
        "Jira Source Adapter supports explicit issue and project Resources, collections and queries",
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
