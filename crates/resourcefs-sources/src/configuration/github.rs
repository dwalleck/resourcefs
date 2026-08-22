use std::collections::HashSet;

use super::{
    ConfigurationError, MAX_CONFIGURATION_ENTRIES, MutationGrants, MutationSupport,
    SecretReference, https::validate_https_url, validate_configuration_id,
};

const DEFAULT_API_BASE_URL: &str = "https://api.github.com/";
const MAX_GITHUB_OWNER_BYTES: usize = 39;
const MAX_GITHUB_REPOSITORY_BYTES: usize = 100;

/// One candidate repository authority, validated by [`GithubConfig::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRepository {
    name: String,
    grants: MutationGrants,
}

impl GithubRepository {
    pub fn new(name: impl Into<String>, grants: MutationGrants) -> Self {
        Self {
            name: name.into(),
            grants,
        }
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
            validate_repository_name(&repository.name)?;
            MutationSupport::GITHUB.validate_nested(grants, repository.grants)?;
            if !identities.insert(repository.name.to_ascii_lowercase()) {
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

fn validate_repository_name(name: &str) -> Result<(), ConfigurationError> {
    let Some((owner, repository)) = name.split_once('/') else {
        return Err(invalid_repository_name());
    };
    if repository.contains('/')
        || owner.is_empty()
        || repository.is_empty()
        || owner.len() > MAX_GITHUB_OWNER_BYTES
        || repository.len() > MAX_GITHUB_REPOSITORY_BYTES
        || matches!(owner, "." | "..")
        || matches!(repository, "." | "..")
        || !owner.bytes().all(is_repository_byte)
        || !repository.bytes().all(is_repository_byte)
    {
        return Err(invalid_repository_name());
    }
    Ok(())
}

fn is_repository_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}

fn invalid_repository_name() -> ConfigurationError {
    ConfigurationError::new(
        "GitHub repository must be one canonical ASCII owner/repository identity",
    )
}
