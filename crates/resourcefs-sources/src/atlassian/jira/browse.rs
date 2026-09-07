use std::collections::HashSet;

use resourcefs_core::{
    ErrorCategory, JiraAddress, PathReference, ProjectionSelector, ResourceError, SourceResource,
};
use url::Url;

use crate::http::BoundedRead;

use super::super::{render::collections, wire::collections as wire};
use super::{AtlassianSite, AtlassianSource, malformed_upstream, transport::JiraRead};

#[derive(Clone, Copy)]
pub(crate) struct BrowseLimits {
    native: usize,
    records: usize,
    attempts: usize,
}

impl Default for BrowseLimits {
    fn default() -> Self {
        Self {
            native: 100,
            records: 1_000,
            attempts: 10,
        }
    }
}

impl AtlassianSource {
    /// Lowers fixed production bounds for real-provider and deterministic fixture coverage.
    #[cfg(feature = "test-support")]
    pub fn with_jira_browse_limits_for_test(
        mut self,
        native: usize,
        records: usize,
        attempts: usize,
    ) -> Result<Self, ResourceError> {
        if native == 0
            || native > 100
            || records == 0
            || records > 1_000
            || attempts == 0
            || attempts > 10
        {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "Jira fixture limits must be positive and cannot raise production ceilings",
            ));
        }
        self.browse_limits = BrowseLimits {
            native,
            records,
            attempts,
        };
        Ok(self)
    }

    pub(super) async fn read_browse(
        &self,
        reference: &PathReference,
        address: &JiraAddress,
        site: &AtlassianSite,
        operation: BoundedRead<'_>,
    ) -> Result<SourceResource, ResourceError> {
        match address {
            JiraAddress::Projects { .. } => self.read_projects(reference, site, operation).await,
            JiraAddress::Project { project_id, .. } => {
                let mut read = JiraRead::direct(self, operation);
                let fetched = read
                    .fetch_cached(site, project_endpoint(site, project_id.as_str())?)
                    .await?;
                let project = wire::decode_project(
                    &fetched.body,
                    site.origin(),
                    wire::ProjectLookup::StableId(project_id),
                )?;
                self.project_resource(reference, site, &project)
            }
            JiraAddress::ProjectKeyAlias { project_key, .. } => {
                let mut read = JiraRead::direct(self, operation);
                let fetched = read
                    .fetch_uncached(project_endpoint(site, project_key.as_str())?)
                    .await?;
                let project = wire::decode_project(
                    &fetched.body,
                    site.origin(),
                    wire::ProjectLookup::ProjectKeyAlias,
                )?;
                self.project_resource(reference, site, &project)
            }
            JiraAddress::Issue { .. } | JiraAddress::IssueKeyAlias { .. } => {
                Err(super::unsupported_jira_projection())
            }
        }
    }

    fn project_resource(
        &self,
        reference: &PathReference,
        site: &AtlassianSite,
        project: &wire::JiraProject,
    ) -> Result<SourceResource, ResourceError> {
        self.markdown_resource(
            PathReference::jira(
                JiraAddress::Project {
                    site: site.id().clone(),
                    project_id: project.id.clone(),
                },
                None,
            )?,
            collections::render_project(site.id(), project)?,
            reference.projection(),
        )
    }

    async fn read_projects(
        &self,
        reference: &PathReference,
        site: &AtlassianSite,
        operation: BoundedRead<'_>,
    ) -> Result<SourceResource, ResourceError> {
        let limits = self.browse_limits;
        let mut read = JiraRead::collection(self, operation, limits.attempts)?;
        let mut offset = reference
            .projection()
            .and_then(|selector| selector.source_offset())
            .map_or(0, |offset| offset.get());
        let mut native = limits.native;
        let mut rows = Vec::new();
        let mut identities = HashSet::new();
        let continuation = loop {
            let requested = native.min(limits.records - rows.len());
            let mut endpoint = site
                .origin()
                .base_url()
                .join("rest/api/3/project/search")
                .map_err(|_| malformed_upstream("Jira project search endpoint is invalid"))?;
            endpoint
                .query_pairs_mut()
                .append_pair("startAt", &offset.to_string())
                .append_pair("maxResults", &requested.to_string());
            let fetched = read.fetch_cached(site, endpoint.clone()).await?;
            let page = wire::decode_project_page(&fetched.body, site.origin(), &endpoint)?;
            if page.values.len() > requested {
                return Err(ResourceError::new(
                    ErrorCategory::LimitExceeded,
                    "Jira project page exceeds the requested record bound",
                ));
            }
            native = native.min(usize::try_from(page.max_results).map_err(|_| {
                malformed_upstream("Jira effective page size is not representable")
            })?);
            for project in page.values {
                if !identities.insert(project.id.clone()) {
                    return Err(malformed_upstream(
                        "Jira project page contains duplicate stable identities",
                    ));
                }
                rows.push(project);
            }
            let Some(next) = page.next_offset else {
                break None;
            };
            if rows.len() == limits.records || read.remaining_attempts() == Some(0) {
                break Some(PathReference::jira(
                    JiraAddress::Projects {
                        site: site.id().clone(),
                    },
                    Some(ProjectionSelector::parse(format!("offset:{}", next.get()))?),
                )?);
            }
            offset = next.get();
        };
        rows.sort_unstable_by(|left, right| {
            left.id
                .as_str()
                .len()
                .cmp(&right.id.as_str().len())
                .then_with(|| left.id.as_str().cmp(right.id.as_str()))
        });
        let canonical = PathReference::jira(
            JiraAddress::Projects {
                site: site.id().clone(),
            },
            None,
        )?;
        let projection = reference
            .projection()
            .filter(|selector| selector.source_offset().is_none());
        let resource = self.markdown_resource(
            canonical,
            collections::render_projects(site.id(), &rows)?,
            projection,
        )?;
        match continuation {
            Some(next) => Ok(resource.with_continuation(&next)),
            None => Ok(resource),
        }
    }
}

pub(super) fn project_endpoint(
    site: &AtlassianSite,
    identifier: &str,
) -> Result<Url, ResourceError> {
    let mut endpoint = site
        .origin()
        .base_url()
        .join("rest/api/3/project/")
        .map_err(|_| malformed_upstream("Jira project endpoint is invalid"))?;
    endpoint
        .path_segments_mut()
        .map_err(|_| malformed_upstream("Jira project endpoint cannot carry path segments"))?
        .pop_if_empty()
        .push(identifier);
    Ok(endpoint)
}
