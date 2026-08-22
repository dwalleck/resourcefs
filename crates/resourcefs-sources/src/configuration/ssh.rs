use std::collections::HashSet;

use super::{
    CommandSpec, ConfigurationError, MAX_CONFIGURATION_ENTRIES, MutationGrants, MutationSupport,
    validate_configuration_id,
};

/// One candidate SSH host alias and its remote roots, validated by [`SshConfig::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    alias: String,
    remote_roots: Vec<String>,
}

impl SshHost {
    pub fn new(alias: impl Into<String>, remote_roots: Vec<String>) -> Self {
        Self {
            alias: alias.into(),
            remote_roots,
        }
    }
}

/// Validated static authority for the read-only SSH Source Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshConfig {
    id: String,
    required: bool,
    command: CommandSpec,
    hosts: Vec<SshHost>,
}

impl SshConfig {
    pub fn new(
        id: impl Into<String>,
        required: bool,
        grants: MutationGrants,
        command: CommandSpec,
        hosts: Vec<SshHost>,
    ) -> Result<Self, ConfigurationError> {
        let id = id.into();
        validate_configuration_id(&id)?;
        MutationSupport::READ_ONLY.validate(grants)?;
        validate_size("SSH hosts", hosts.len())?;

        let mut aliases = HashSet::with_capacity(hosts.len());
        for host in &hosts {
            validate_configuration_id(&host.alias).map_err(|_| {
                ConfigurationError::new("SSH aliases must use exact configuration-ID syntax")
            })?;
            if !aliases.insert(host.alias.as_str()) {
                return Err(ConfigurationError::new("SSH host aliases must be distinct"));
            }
            validate_size("SSH remote roots", host.remote_roots.len())?;
            for root in &host.remote_roots {
                validate_remote_root(root)?;
            }
            let mut roots: Vec<Vec<&str>> = host
                .remote_roots
                .iter()
                .map(|root| remote_components(root))
                .collect();
            roots.sort_unstable();
            if roots
                .windows(2)
                .any(|pair| pair[0].starts_with(&pair[1]) || pair[1].starts_with(&pair[0]))
            {
                return Err(ConfigurationError::new(
                    "SSH remote roots must be distinct and non-overlapping per host",
                ));
            }
        }

        Ok(Self {
            id,
            required,
            command,
            hosts,
        })
    }
}

fn validate_remote_root(root: &str) -> Result<(), ConfigurationError> {
    if root == "/" {
        return Ok(());
    }
    if !root.starts_with('/')
        || root.ends_with('/')
        || root.contains("//")
        || root.contains('\\')
        || root.contains('\0')
        || root
            .split('/')
            .skip(1)
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(ConfigurationError::new(
            "SSH remote roots must be canonical absolute POSIX paths",
        ));
    }
    Ok(())
}

fn remote_components(root: &str) -> Vec<&str> {
    if root == "/" {
        Vec::new()
    } else {
        root.split('/').skip(1).collect()
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
