use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use resourcefs_core::{AllowedOrigin, AtlassianSiteId, ErrorCategory, PathSession, ResourceError};

use crate::{HttpSubstrate, MAX_CONFIGURATION_ENTRIES};

#[derive(Debug, Clone)]
pub struct AtlassianSite {
    id: AtlassianSiteId,
    origin: AllowedOrigin,
}

impl AtlassianSite {
    pub fn new(id: AtlassianSiteId, origin: AllowedOrigin) -> Result<Self, ResourceError> {
        let url = origin.base_url();
        if !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "Atlassian Site Mount URL must be one canonical HTTPS origin without credentials, path, query, or fragment",
            ));
        }
        Ok(Self { id, origin })
    }

    pub const fn id(&self) -> &AtlassianSiteId {
        &self.id
    }

    pub const fn origin(&self) -> &AllowedOrigin {
        &self.origin
    }
}

#[derive(Clone)]
pub struct AtlassianSourceMount {
    sites: Arc<HashMap<AtlassianSiteId, AtlassianSite>>,
    substrate: Arc<HttpSubstrate>,
}

impl std::fmt::Debug for AtlassianSourceMount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AtlassianSourceMount")
            .field("sites", &self.sites.len())
            .finish_non_exhaustive()
    }
}

impl AtlassianSourceMount {
    pub fn new(
        sites: Vec<AtlassianSite>,
        substrate: Arc<HttpSubstrate>,
    ) -> Result<Self, ResourceError> {
        if sites.is_empty() || sites.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                format!(
                    "Atlassian Source Mount must contain 1–{MAX_CONFIGURATION_ENTRIES} Site Mounts"
                ),
            ));
        }
        let mut by_id = HashMap::with_capacity(sites.len());
        let mut origins = HashSet::with_capacity(sites.len());
        for site in sites {
            let authority = site.origin.base_url().origin().ascii_serialization();
            if !origins.insert(authority) {
                return Err(ResourceError::new(
                    ErrorCategory::InvalidReference,
                    "Atlassian Site Mount origins must be unique",
                ));
            }
            if by_id.insert(site.id.clone(), site).is_some() {
                return Err(ResourceError::new(
                    ErrorCategory::InvalidReference,
                    "Atlassian Site Mount IDs must be unique",
                ));
            }
        }
        Ok(Self {
            sites: Arc::new(by_id),
            substrate,
        })
    }

    pub fn bind(self, session: PathSession) -> AtlassianSource {
        AtlassianSource {
            sites: self.sites,
            substrate: self.substrate,
            session,
        }
    }
}

#[derive(Clone)]
pub struct AtlassianSource {
    pub(crate) sites: Arc<HashMap<AtlassianSiteId, AtlassianSite>>,
    pub(crate) substrate: Arc<HttpSubstrate>,
    pub(crate) session: PathSession,
}

impl std::fmt::Debug for AtlassianSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AtlassianSource")
            .field("sites", &self.sites.len())
            .finish_non_exhaustive()
    }
}

impl AtlassianSource {
    pub(crate) fn site(&self, id: &AtlassianSiteId) -> Result<&AtlassianSite, ResourceError> {
        self.sites.get(id).ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::PermissionDenied,
                "Atlassian Site Mount is not configured",
            )
        })
    }
}

mod jira;
mod render;
mod wire;

#[cfg(feature = "test-support")]
pub use render::{
    JiraProjectionWarningObservation, JiraRenderFieldObservation, JiraRenderObservation,
    inspect_jira_render_for_test,
};

#[cfg(feature = "test-support")]
pub use wire::{
    JiraWireFieldObservation, JiraWireLookupForTest, JiraWireObservation,
    inspect_jira_wire_for_test,
};
