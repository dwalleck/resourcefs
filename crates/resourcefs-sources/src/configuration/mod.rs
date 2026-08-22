mod command;
mod downstream_mcp;
mod github;
mod grants;
mod https;
mod secret_reference;
mod ssh;

use std::fmt;

pub use command::{ChildEnvironment, CommandSpec, EnvironmentValue};
pub use downstream_mcp::{
    DownstreamMcpConfig, DownstreamServer, DownstreamTransport, MAX_SCHEME_CLAIM_BYTES, SchemeClaim,
};
pub use github::{GithubConfig, GithubRepository};
pub use grants::{MutationGrants, MutationSupport};
pub use https::{CredentialHeader, HttpsConfig, HttpsOrigin};
pub use secret_reference::SecretReference;
pub use ssh::{SshConfig, SshHost};

/// Maximum encoded length of an operator-controlled configuration identifier.
pub const MAX_CONFIGURATION_ID_BYTES: usize = 128;
/// Maximum entries accepted by one source-owned allowlist.
pub const MAX_CONFIGURATION_ENTRIES: usize = 4_096;
/// Maximum number of arguments in one direct-argv command.
pub const MAX_COMMAND_ARGUMENTS: usize = 256;
/// Maximum aggregate UTF-8 bytes across one command's arguments.
pub const MAX_COMMAND_ARGUMENT_BYTES: usize = 65_536;
/// Maximum explicit mappings in one cleared child environment.
pub const MAX_COMMAND_ENVIRONMENT_ENTRIES: usize = 256;

/// Stable failure returned while constructing source configuration authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationError {
    message: String,
}

impl ConfigurationError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConfigurationError {}

/// Validates the shared grammar for operator-controlled configuration IDs.
pub fn validate_configuration_id(id: &str) -> Result<(), ConfigurationError> {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_CONFIGURATION_ID_BYTES {
        return Err(ConfigurationError::new(format!(
            "configuration ID must contain 1–{MAX_CONFIGURATION_ID_BYTES} ASCII bytes"
        )));
    }
    if !bytes[0].is_ascii_alphanumeric() {
        return Err(ConfigurationError::new(
            "configuration ID must start with an ASCII letter or digit",
        ));
    }
    if !bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ConfigurationError::new(
            "configuration ID may contain only ASCII letters, digits, '.', '_', or '-'",
        ));
    }
    Ok(())
}
