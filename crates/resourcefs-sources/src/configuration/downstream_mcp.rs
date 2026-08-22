use std::collections::HashSet;

use super::{
    CommandSpec, ConfigurationError, CredentialHeader, MAX_CONFIGURATION_ENTRIES, MutationGrants,
    MutationSupport, https::validate_https_url, validate_configuration_id,
};

pub const MAX_SCHEME_CLAIM_BYTES: usize = 64;

const BUILT_IN_SCHEMES: &[&str] = &[
    "rfs", "file", "artifact", "local", "https", "github", "issue", "pr", "ssh", "skill", "rule",
    "memory", "vault", "agent", "history",
];

/// One lowercase-normalized native URI scheme claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemeClaim(String);

impl SchemeClaim {
    pub fn new(value: String) -> Result<Self, ConfigurationError> {
        let bytes = value.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_SCHEME_CLAIM_BYTES {
            return Err(ConfigurationError::new(format!(
                "native scheme claims must contain 1–{MAX_SCHEME_CLAIM_BYTES} ASCII bytes"
            )));
        }
        if !bytes[0].is_ascii_alphabetic()
            || !bytes[1..]
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        {
            return Err(ConfigurationError::new(
                "native scheme claims must use RFC URI-scheme syntax",
            ));
        }
        Ok(Self(value.to_ascii_lowercase()))
    }
}

/// One shape-validated downstream MCP transport definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownstreamTransport(DownstreamTransportKind);

#[derive(Debug, Clone, PartialEq, Eq)]
enum DownstreamTransportKind {
    Stdio {
        command: CommandSpec,
    },
    Http {
        endpoint: String,
        allow_private_network: bool,
        credential: Option<CredentialHeader>,
    },
}

impl DownstreamTransport {
    pub const fn stdio(command: CommandSpec) -> Self {
        Self(DownstreamTransportKind::Stdio { command })
    }

    pub fn http(
        endpoint: impl Into<String>,
        allow_private_network: bool,
        credential: Option<CredentialHeader>,
    ) -> Self {
        Self(DownstreamTransportKind::Http {
            endpoint: endpoint.into(),
            allow_private_network,
            credential,
        })
    }

    fn validate(&self) -> Result<(), ConfigurationError> {
        match &self.0 {
            DownstreamTransportKind::Stdio { .. } => Ok(()),
            DownstreamTransportKind::Http { endpoint, .. } => {
                validate_https_url(endpoint).map(|_| ())
            }
        }
    }
}

/// One candidate downstream server, validated by [`DownstreamMcpConfig::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownstreamServer {
    id: String,
    schemes: Vec<SchemeClaim>,
    transport: DownstreamTransport,
}

impl DownstreamServer {
    pub fn new(
        id: impl Into<String>,
        schemes: Vec<SchemeClaim>,
        transport: DownstreamTransport,
    ) -> Self {
        Self {
            id: id.into(),
            schemes,
            transport,
        }
    }
}

/// Validated static authority for downstream MCP Resource servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownstreamMcpConfig {
    id: String,
    required: bool,
    servers: Vec<DownstreamServer>,
}

impl DownstreamMcpConfig {
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        servers: Vec<DownstreamServer>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::READ_ONLY.validate(grants)?;
        validate_size("downstream MCP servers", servers.len())?;

        let mut server_ids = HashSet::with_capacity(servers.len());
        let mut schemes = HashSet::new();
        for server in &servers {
            validate_configuration_id(&server.id).map_err(|_| {
                ConfigurationError::new(
                    "downstream MCP server IDs must use configuration-ID syntax",
                )
            })?;
            if !server_ids.insert(server.id.as_str()) {
                return Err(ConfigurationError::new(
                    "downstream MCP server IDs must be unique",
                ));
            }
            validate_size("downstream MCP scheme claims", server.schemes.len())?;
            for scheme in &server.schemes {
                if BUILT_IN_SCHEMES.contains(&scheme.0.as_str()) {
                    return Err(ConfigurationError::new(
                        "downstream MCP scheme claims cannot shadow built-in schemes",
                    ));
                }
                if !schemes.insert(scheme.0.as_str()) {
                    return Err(ConfigurationError::new(
                        "downstream MCP scheme claims must be globally unique",
                    ));
                }
            }
            server.transport.validate()?;
        }

        Ok(Self {
            id,
            required,
            servers,
        })
    }
}

fn validate_size(name: &str, size: usize) -> Result<(), ConfigurationError> {
    if size == 0 || size > MAX_CONFIGURATION_ENTRIES {
        return Err(ConfigurationError::new(format!(
            "{name} must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
        )));
    }
    Ok(())
}
