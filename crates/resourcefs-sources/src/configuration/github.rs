use resourcefs_core::GithubRepositoryIdentity;
use std::collections::HashSet;

use super::{
    ConfigurationError, MAX_CONFIGURATION_ENTRIES, MutationGrants, MutationSupport,
    SecretReference, https::validate_https_url, validate_configuration_id,
};

const DEFAULT_API_BASE_URL: &str = "https://api.github.com/";

/// One validated repository authority and its nested mutation grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRepository {
    identity: GithubRepositoryIdentity,
    grants: MutationGrants,
}

impl GithubRepository {
    pub fn new(
        name: impl Into<String>,
        grants: MutationGrants,
    ) -> Result<Self, ConfigurationError> {
        let name = name.into();
        let identity = GithubRepositoryIdentity::parse(&name)
            .map_err(|error| ConfigurationError::new(error.message()))?;
        Ok(Self { identity, grants })
    }

    pub fn identity(&self) -> &GithubRepositoryIdentity {
        &self.identity
    }

    pub const fn grants(&self) -> MutationGrants {
        self.grants
    }
}

/// Validated static authority for the GitHub Source Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubConfig {
    id: String,
    required: bool,
    grants: MutationGrants,
    api_base_url: String,
    allow_private_network: bool,
    credential: SecretReference,
    repositories: Vec<GithubRepository>,
}

impl GithubConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        api_base_url: Option<String>,
        allow_private_network: bool,
        credential: SecretReference,
        repositories: Vec<GithubRepository>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::GITHUB.validate(grants)?;
        if repositories.is_empty() || repositories.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "GitHub repositories must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }

        let api_base_url = api_base_url.unwrap_or_else(|| DEFAULT_API_BASE_URL.to_owned());
        validate_https_url(&api_base_url)?;

        let mut identities = HashSet::with_capacity(repositories.len());
        for repository in &repositories {
            MutationSupport::GITHUB.validate_nested(grants, repository.grants)?;
            if !identities.insert(repository.identity.clone()) {
                return Err(ConfigurationError::new(
                    "GitHub repository identities must be unique ignoring ASCII case",
                ));
            }
        }

        Ok(Self {
            id,
            required,
            grants,
            api_base_url,
            allow_private_network,
            credential,
            repositories,
        })
    }
}
