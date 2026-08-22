use std::collections::HashSet;
use std::path::PathBuf;

use super::{
    ConfigurationDirectory, ConfigurationError, ConfigurationTargetKind, MAX_CONFIGURATION_ENTRIES,
    MutationGrants, MutationSupport, validate_configuration_id,
};

/// One candidate memory root, validated by [`MemoryConfig::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRoot {
    name: String,
    path: String,
}

impl MemoryRoot {
    /// Records one operator-supplied `{name, path}` pair without validation.
    pub fn new(name: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
        }
    }
}

/// Whether one validated memory target is a file or a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTargetKind {
    File,
    Directory,
}

/// One validated memory target beneath the configuration directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryTarget {
    /// Stable first Path Reference segment, using configuration-ID syntax.
    name: String,
    /// **Canonical absolute path** beneath the configuration directory.
    path: PathBuf,
    kind: MemoryTargetKind,
}

/// Validated static authority for the read-only Memory Source Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryConfig {
    id: String,
    required: bool,
    roots: Vec<MemoryTarget>,
}

impl MemoryConfig {
    /// Validates names, grants, and contained file-or-directory targets.
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        configuration_directory: &ConfigurationDirectory,
        roots: Vec<MemoryRoot>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::READ_ONLY.validate(grants)?;
        if roots.is_empty() || roots.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "memory roots must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }

        let mut names = HashSet::with_capacity(roots.len());
        let mut identities = HashSet::with_capacity(roots.len());
        let mut targets = Vec::with_capacity(roots.len());
        for root in &roots {
            validate_configuration_id(&root.name).map_err(|_| {
                ConfigurationError::new("memory root names must use configuration-ID syntax")
            })?;
            if !names.insert(root.name.as_str()) {
                return Err(ConfigurationError::new(
                    "memory root names must be distinct",
                ));
            }
            let path = configuration_directory
                .resolve(&root.path, ConfigurationTargetKind::FileOrDirectory)?;
            let kind = match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_dir() => MemoryTargetKind::Directory,
                Ok(_) => MemoryTargetKind::File,
                Err(_) => {
                    return Err(ConfigurationError::new(
                        "memory root targets must remain readable during validation",
                    ));
                }
            };
            if !identities.insert(path.clone()) {
                return Err(ConfigurationError::new(
                    "memory root targets must have distinct canonical paths",
                ));
            }
            targets.push(MemoryTarget {
                name: root.name.clone(),
                path,
                kind,
            });
        }

        Ok(Self {
            id,
            required,
            roots: targets,
        })
    }
}
