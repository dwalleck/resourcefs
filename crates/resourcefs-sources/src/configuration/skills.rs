use std::collections::HashSet;
use std::path::PathBuf;

use super::{
    ConfigurationDirectory, ConfigurationError, ConfigurationTargetKind, MAX_CONFIGURATION_ENTRIES,
    MutationGrants, MutationSupport, validate_configuration_id,
};

/// Validated static authority for the Skill Source Adapter.
///
/// Every root is a distinct canonical directory contained by the
/// configuration directory. Skill content projection and mutation are
/// intended work in rfs-cdlp; this type validates path authority only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillsConfig {
    id: String,
    required: bool,
    grants: MutationGrants,
    roots: Vec<PathBuf>,
}

impl SkillsConfig {
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        base: &ConfigurationDirectory,
        roots: Vec<String>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::FULL.validate(grants)?;
        if roots.is_empty() || roots.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "skill roots must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }

        let mut identities = HashSet::with_capacity(roots.len());
        let mut canonical_roots = Vec::with_capacity(roots.len());
        for root in &roots {
            let canonical = base.resolve(root, ConfigurationTargetKind::Directory)?;
            if !identities.insert(canonical.clone()) {
                return Err(ConfigurationError::new(
                    "skill roots must have distinct canonical identities",
                ));
            }
            canonical_roots.push(canonical);
        }

        Ok(Self {
            id,
            required,
            grants,
            roots: canonical_roots,
        })
    }
}
