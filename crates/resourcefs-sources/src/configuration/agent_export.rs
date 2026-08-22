use std::collections::HashSet;
use std::path::PathBuf;

use super::{
    ConfigurationDirectory, ConfigurationError, ConfigurationTargetKind, MAX_CONFIGURATION_ENTRIES,
    MutationGrants, MutationSupport, validate_configuration_id,
};

/// Validated static authority for the read-only Agent Export Source Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExportConfig {
    id: String,
    required: bool,
    /// **Canonical absolute paths** of contained manifest files.
    manifests: Vec<PathBuf>,
}

impl AgentExportConfig {
    /// Validates grants and the contained manifest-path list.
    ///
    /// Manifest content, versioning, and global agent-ID reconciliation are
    /// validated by the compiled Agent Export adapter, not here.
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        configuration_directory: &ConfigurationDirectory,
        manifests: Vec<String>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::READ_ONLY.validate(grants)?;
        if manifests.is_empty() || manifests.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "agent export manifests must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }

        let mut identities = HashSet::with_capacity(manifests.len());
        let mut resolved = Vec::with_capacity(manifests.len());
        for manifest in &manifests {
            let path = configuration_directory.resolve(manifest, ConfigurationTargetKind::File)?;
            if !identities.insert(path.clone()) {
                return Err(ConfigurationError::new(
                    "agent export manifests must have distinct canonical paths",
                ));
            }
            resolved.push(path);
        }

        Ok(Self {
            id,
            required,
            manifests: resolved,
        })
    }
}
