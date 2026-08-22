use std::collections::HashSet;
use std::path::PathBuf;

use super::{
    ConfigurationDirectory, ConfigurationError, ConfigurationTargetKind, MAX_CONFIGURATION_ENTRIES,
    MutationGrants, MutationSupport, validate_configuration_id,
};

/// One candidate vault, validated by [`VaultConfig::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRoot {
    name: String,
    path: String,
    grants: MutationGrants,
}

impl VaultRoot {
    /// Records one operator-supplied `{name, path, grants}` triple without validation.
    pub fn new(name: impl Into<String>, path: impl Into<String>, grants: MutationGrants) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            grants,
        }
    }
}

/// One validated vault beneath the configuration directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultTarget {
    /// Stable first Path Reference segment, using configuration-ID syntax.
    name: String,
    /// **Canonical absolute path** of the contained vault directory.
    path: PathBuf,
    /// Per-vault grants, a validated subset of the source grants.
    grants: MutationGrants,
}

/// Validated static authority for the Vault Source Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultConfig {
    id: String,
    required: bool,
    grants: MutationGrants,
    vaults: Vec<VaultTarget>,
}

impl VaultConfig {
    /// Validates names, source and per-vault grants, and contained directories.
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        configuration_directory: &ConfigurationDirectory,
        vaults: Vec<VaultRoot>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::FULL.validate(grants)?;
        if vaults.is_empty() || vaults.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "vaults must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }

        let mut names = HashSet::with_capacity(vaults.len());
        let mut identities = HashSet::with_capacity(vaults.len());
        let mut targets = Vec::with_capacity(vaults.len());
        for vault in &vaults {
            validate_configuration_id(&vault.name).map_err(|_| {
                ConfigurationError::new("vault names must use configuration-ID syntax")
            })?;
            if !names.insert(vault.name.as_str()) {
                return Err(ConfigurationError::new("vault names must be distinct"));
            }
            MutationSupport::FULL.validate_nested(grants, vault.grants)?;
            let path =
                configuration_directory.resolve(&vault.path, ConfigurationTargetKind::Directory)?;
            if !identities.insert(path.clone()) {
                return Err(ConfigurationError::new(
                    "vault directories must have distinct canonical paths",
                ));
            }
            targets.push(VaultTarget {
                name: vault.name.clone(),
                path,
                grants: vault.grants,
            });
        }

        Ok(Self {
            id,
            required,
            grants,
            vaults: targets,
        })
    }
}
