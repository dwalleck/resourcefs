use std::collections::HashSet;

use resourcefs_sources::{MutationSupport, validate_configuration_id};

use super::model::{ProfileDocument, ProfileError};

const MAX_CATALOG_ENTRIES: usize = 256;

pub(super) fn validate_profile(profile: &ProfileDocument) -> Result<(), ProfileError> {
    validate_workspace(profile)?;
    validate_sources(profile.sources())
}

fn validate_workspace(profile: &ProfileDocument) -> Result<(), ProfileError> {
    let Some(workspace) = profile.workspace() else {
        return Ok(());
    };
    let roots = workspace.roots();
    if roots.len() > MAX_CATALOG_ENTRIES {
        return Err(ProfileError::invalid(format!(
            "workspace.roots has {} entries; maximum is {MAX_CATALOG_ENTRIES}",
            roots.len()
        )));
    }
    for root in roots {
        validate_configuration_id(root.id())
            .map_err(|error| ProfileError::invalid(format!("workspace root ID: {error}")))?;
        MutationSupport::FULL
            .validate(root.grants())
            .map_err(|error| ProfileError::invalid(format!("workspace root grants: {error}")))?;
    }
    Ok(())
}

fn validate_sources(sources: &[super::model::SourceProfile]) -> Result<(), ProfileError> {
    if sources.len() > MAX_CATALOG_ENTRIES {
        return Err(ProfileError::invalid(format!(
            "sources has {} entries; maximum is {MAX_CATALOG_ENTRIES}",
            sources.len()
        )));
    }

    let mut seen_kinds = HashSet::with_capacity(sources.len());
    let mut seen_ids = HashSet::with_capacity(sources.len());
    let mut seen_claims = HashSet::new();
    for source in sources {
        validate_configuration_id(source.id())
            .map_err(|error| ProfileError::invalid(format!("source ID: {error}")))?;
        if !seen_kinds.insert(source.kind()) {
            return Err(ProfileError::invalid(
                "each configured source kind may appear at most once",
            ));
        }
        if !seen_ids.insert(source.id()) {
            return Err(ProfileError::invalid(
                "configured source IDs must be globally unique",
            ));
        }
        source
            .validate_grants()
            .map_err(|error| ProfileError::invalid(format!("source grants: {error}")))?;

        let mut duplicate_claim = false;
        source.visit_scheme_claims(|claim| {
            duplicate_claim |= !seen_claims.insert(claim.to_ascii_lowercase());
        });
        if duplicate_claim {
            return Err(ProfileError::invalid(
                "native scheme claims must be globally unique after lowercase normalization",
            ));
        }
    }
    Ok(())
}
