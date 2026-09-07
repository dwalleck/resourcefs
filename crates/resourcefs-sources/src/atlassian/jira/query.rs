use std::collections::HashSet;

use resourcefs_core::{
    ErrorCategory, JiraAddress, JiraQuery, PathReference, ProjectionSelector, ResourceError,
    SourceResource,
};

use crate::http::BoundedRead;

use super::super::{render::collections, wire::collections as wire};
use super::{
    AtlassianSite, AtlassianSource, cursor::CursorOwner, malformed_upstream, transport::JiraRead,
};

impl AtlassianSource {
    /// Assemble one atomic page in native order under the shared physical-attempt budget.
    pub(super) async fn read_query(
        &self,
        reference: &PathReference,
        address: &JiraAddress,
        query: &JiraQuery,
        site: &AtlassianSite,
        operation: BoundedRead<'_>,
    ) -> Result<SourceResource, ResourceError> {
        let owner = CursorOwner::new(address, site)?;
        let mut token = reference
            .projection()
            .and_then(ProjectionSelector::source_cursor)
            .map(|cursor| owner.decode(cursor))
            .transpose()?;
        let limits = self.browse_limits;
        let mut read = JiraRead::collection(self, operation, limits.attempts)?;
        let mut native = limits.native;
        let mut rows = Vec::new();
        let mut identities = HashSet::new();
        let mut visited_tokens = HashSet::new();
        let continuation = loop {
            let requested = native.min(limits.records - rows.len());
            let fetched = read
                .fetch_query(site, query, token.as_ref(), requested)
                .await?;
            if let Some(current) = token.take() {
                visited_tokens.insert(current.into_string());
            }
            let page = wire::decode_issue_page(&fetched.body, site.origin())?;
            if page.values.len() > requested {
                return Err(ResourceError::new(
                    ErrorCategory::LimitExceeded,
                    "Jira query page exceeds the requested record bound",
                ));
            }
            if let Some(maximum) = page.max_results {
                let wire::NativePageMaximum::Valid(effective) = maximum else {
                    return Err(malformed_upstream(
                        "Jira query effective maximum must be a positive integer",
                    ));
                };
                let effective = usize::try_from(effective.get()).map_err(|_| {
                    malformed_upstream("Jira effective query maximum is not representable")
                })?;
                if effective > requested || page.values.len() > effective {
                    return Err(malformed_upstream(
                        "Jira query page contradicts its effective maximum",
                    ));
                }
                native = native.min(effective);
            }
            for issue in page.values {
                if !identities.insert(issue.id.clone()) {
                    return Err(malformed_upstream(
                        "Jira query page contains duplicate stable identities",
                    ));
                }
                rows.push(issue);
            }
            let Some(next) = page.next_token else {
                break None;
            };
            if visited_tokens.contains(next.as_str()) {
                return Err(malformed_upstream(
                    "Jira query continuation token does not advance",
                ));
            }
            // Even internal continuations must fit the owner-bound public reference. Never
            // follow an upstream token that cannot be represented if this page is interrupted.
            owner.validate_continuation(&next)?;
            if rows.len() == limits.records || read.remaining_attempts() == Some(0) {
                break Some(owner.continuation(next)?);
            }
            token = Some(next);
        };
        let resource = self.markdown_resource(
            PathReference::jira(address.clone(), None)?,
            collections::render_issues(address, &rows)?,
            reference
                .projection()
                .filter(|selector| selector.source_cursor().is_none()),
        )?;
        match continuation {
            Some(next) => Ok(resource.with_continuation(&next)),
            None => Ok(resource),
        }
    }
}
