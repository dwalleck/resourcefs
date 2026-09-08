use resourcefs_core::{GithubRepositoryIdentity, ReadAcquisitionLimits};
use std::collections::HashSet;

use super::{
    ConfigurationError, MAX_CONFIGURATION_ENTRIES, MutationGrants, MutationSupport,
    SecretReference, https::validate_https_url, validate_configuration_id,
};

const DEFAULT_API_BASE_URL: &str = "https://api.github.com/";

/// Validated API authority and, when known, its machine-readable web identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubDeployment {
    api_base_url: String,
    identity: DeploymentIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeploymentIdentity {
    Public,
    EnterpriseCloud { web_origin: String },
    Custom { web_origin: Option<String> },
}

impl Default for GithubDeployment {
    fn default() -> Self {
        Self {
            api_base_url: DEFAULT_API_BASE_URL.to_owned(),
            identity: DeploymentIdentity::Public,
        }
    }
}

impl GithubDeployment {
    /// Public GitHub and `api.<tenant>.ghe.com` derive their documented web
    /// origin. Custom API bases retain legacy paths/ports but never guess one.
    pub fn new(
        api_base_url: Option<String>,
        web_origin: Option<String>,
    ) -> Result<Self, ConfigurationError> {
        let api_base_url = api_base_url.unwrap_or_else(|| DEFAULT_API_BASE_URL.to_owned());
        let api = validate_https_url(&api_base_url)?;
        let host = api.host_str().expect("validated HTTPS host");
        let web_origin = web_origin
            .map(|origin| validate_web_origin(&origin))
            .transpose()?;
        let identity = if host == "api.github.com" {
            validate_documented_origin(&api_base_url, &api)?;
            require_matching_web(web_origin.as_deref(), "https://github.com")?;
            DeploymentIdentity::Public
        } else if claims_enterprise_cloud(host) {
            validate_documented_origin(&api_base_url, &api)?;
            let tenant = host
                .strip_prefix("api.")
                .and_then(|host| host.strip_suffix(".ghe.com"))
                .filter(|tenant| valid_tenant(tenant))
                .ok_or_else(|| {
                    ConfigurationError::new(
                        "GitHub Enterprise Cloud API requires one valid tenant label",
                    )
                })?;
            let expected = format!("https://{tenant}.ghe.com");
            require_matching_web(web_origin.as_deref(), &expected)?;
            DeploymentIdentity::EnterpriseCloud {
                web_origin: expected,
            }
        } else {
            DeploymentIdentity::Custom { web_origin }
        };
        Ok(Self {
            api_base_url,
            identity,
        })
    }

    pub fn api_base_url(&self) -> &str {
        &self.api_base_url
    }

    /// None means custom legacy reads remain available without machine identity.
    pub fn web_origin(&self) -> Option<&str> {
        match &self.identity {
            DeploymentIdentity::Public => Some("https://github.com"),
            DeploymentIdentity::EnterpriseCloud { web_origin } => Some(web_origin),
            DeploymentIdentity::Custom { web_origin } => web_origin.as_deref(),
        }
    }
}

/// Only the documented `api.<tenant>.ghe.com` authority claims Enterprise
/// Cloud identity, and a claim that then fails its shape is an error rather
/// than a guess. Any other `.ghe.com` host never made the claim: it is a
/// legacy custom base whose reads keep working without a derived identity.
fn claims_enterprise_cloud(host: &str) -> bool {
    host.starts_with("api.") && host.ends_with(".ghe.com")
}

fn valid_tenant(tenant: &str) -> bool {
    !tenant.is_empty()
        && tenant.len() <= 63
        && !tenant.starts_with('-')
        && !tenant.ends_with('-')
        && tenant
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn validate_web_origin(input: &str) -> Result<String, ConfigurationError> {
    let url = validate_https_url(input)?;
    let origin = url.origin().ascii_serialization();
    if url.path() != "/" || (input != origin && input != format!("{origin}/")) {
        return Err(ConfigurationError::new(
            "GitHub web origin must be a standard HTTPS origin",
        ));
    }
    Ok(origin)
}

fn validate_documented_origin(input: &str, url: &url::Url) -> Result<(), ConfigurationError> {
    let expected = format!("https://{}", url.host_str().expect("validated HTTPS host"));
    if input != expected && input != format!("{expected}/") {
        return Err(ConfigurationError::new(
            "documented GitHub API origin cannot contain a path, port, or nonstandard authority",
        ));
    }
    Ok(())
}

fn require_matching_web(actual: Option<&str>, expected: &str) -> Result<(), ConfigurationError> {
    if actual.is_some_and(|actual| actual != expected) {
        return Err(ConfigurationError::new(
            "GitHub web and API origins do not identify the same deployment",
        ));
    }
    Ok(())
}

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
    deployment: GithubDeployment,
    allow_private_network: bool,
    credential: SecretReference,
    repositories: Vec<GithubRepository>,
    acquisition_limits: ReadAcquisitionLimits,
}

impl GithubConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        deployment: GithubDeployment,
        allow_private_network: bool,
        credential: SecretReference,
        repositories: Vec<GithubRepository>,
        acquisition_limits: ReadAcquisitionLimits,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::GITHUB.validate(grants)?;
        if repositories.is_empty() || repositories.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "GitHub repositories must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }

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
            deployment,
            allow_private_network,
            credential,
            repositories,
            acquisition_limits,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn required(&self) -> bool {
        self.required
    }

    pub const fn grants(&self) -> MutationGrants {
        self.grants
    }

    pub fn api_base_url(&self) -> &str {
        self.deployment.api_base_url()
    }

    pub const fn deployment(&self) -> &GithubDeployment {
        &self.deployment
    }

    pub const fn acquisition_limits(&self) -> ReadAcquisitionLimits {
        self.acquisition_limits
    }

    pub const fn allow_private_network(&self) -> bool {
        self.allow_private_network
    }

    pub const fn credential(&self) -> &SecretReference {
        &self.credential
    }

    pub fn repositories(&self) -> &[GithubRepository] {
        &self.repositories
    }

    pub fn repository(&self, identity: &GithubRepositoryIdentity) -> Option<&GithubRepository> {
        self.repositories
            .iter()
            .find(|repository| repository.identity() == identity)
    }
}
