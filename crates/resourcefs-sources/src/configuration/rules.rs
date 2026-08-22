use std::collections::HashSet;
use std::path::PathBuf;

use super::{
    ConfigurationDirectory, ConfigurationError, ConfigurationTargetKind, MAX_CONFIGURATION_ENTRIES,
    MutationGrants, MutationSupport, validate_configuration_id,
};

/// Validated static authority for the Rule Source Adapter.
///
/// Every manifest is a distinct canonical file contained by the
/// configuration directory. Native rule manifest content validation and
/// mutation are intended work in rfs-cdlp; this type validates path
/// authority only and performs no manifest content parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesConfig {
    id: String,
    required: bool,
    grants: MutationGrants,
    manifests: Vec<PathBuf>,
}

impl RulesConfig {
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        base: &ConfigurationDirectory,
        manifests: Vec<String>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::FULL.validate(grants)?;
        if manifests.is_empty() || manifests.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "rule manifests must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }

        let mut identities = HashSet::with_capacity(manifests.len());
        let mut canonical_manifests = Vec::with_capacity(manifests.len());
        for manifest in &manifests {
            let canonical = base.resolve(manifest, ConfigurationTargetKind::File)?;
            if !identities.insert(canonical.clone()) {
                return Err(ConfigurationError::new(
                    "rule manifests must have distinct canonical identities",
                ));
            }
            canonical_manifests.push(canonical);
        }

        Ok(Self {
            id,
            required,
            grants,
            manifests: canonical_manifests,
        })
    }
}
