#![expect(
    dead_code,
    reason = "validated source configurations are consumed by the later launch-composition slice"
)]

use std::collections::BTreeMap;

use resourcefs_sources::{
    AgentExportConfig, ChildEnvironment, CommandSpec, ConfigurationDirectory, ConverterInput,
    CredentialHeader, DocumentConverter, DocumentsConfig, DownstreamMcpConfig, DownstreamServer,
    DownstreamTransport, EnvironmentValue, GithubConfig, GithubRepository, HttpsConfig,
    HttpsOrigin, MemoryConfig, MemoryRoot, RulesConfig, SchemeClaim, SecretReference, SkillsConfig,
    SshConfig, SshHost, VaultConfig, VaultRoot,
};

use super::ProfileError;
use super::model::{
    CommandProfile, ConverterInputProfile, CredentialHeaderProfile, DownstreamTransportProfile,
    EnvironmentValueProfile, SecretReferenceProfile, SourceProfile,
};

#[derive(Debug, Clone)]
pub(super) enum ConfiguredSource {
    Https(HttpsConfig),
    Github(GithubConfig),
    Ssh(SshConfig),
    Documents(DocumentsConfig),
    Skills(SkillsConfig),
    Rules(RulesConfig),
    Memory(MemoryConfig),
    Vault(VaultConfig),
    AgentExport(AgentExportConfig),
    DownstreamMcp(DownstreamMcpConfig),
}

pub(super) fn convert_sources(
    sources: Vec<SourceProfile>,
    base: &ConfigurationDirectory,
) -> Result<Vec<ConfiguredSource>, ProfileError> {
    sources
        .into_iter()
        .enumerate()
        .map(|(index, source)| {
            convert_source(source, base).map_err(|error| {
                ProfileError::invalid(format!("sources[{index}] is invalid: {error}"))
            })
        })
        .collect()
}

fn convert_source(
    source: SourceProfile,
    base: &ConfigurationDirectory,
) -> Result<ConfiguredSource, ProfileError> {
    match source {
        SourceProfile::Https(source) => {
            let (id, required, grants, origins) = source.into_parts();
            let origins = origins
                .into_iter()
                .map(|origin| {
                    let (base_url, allow_private_network, credential) = origin.into_parts();
                    Ok(HttpsOrigin::new(
                        base_url,
                        allow_private_network,
                        credential.map(convert_credential).transpose()?,
                    ))
                })
                .collect::<Result<Vec<_>, ProfileError>>()?;
            HttpsConfig::new(id, required, grants, origins)
                .map(ConfiguredSource::Https)
                .map_err(source_configuration_error)
        }
        SourceProfile::Github(source) => {
            let (
                id,
                required,
                grants,
                api_base_url,
                allow_private_network,
                credential,
                repositories,
            ) = source.into_parts();
            let repositories = repositories
                .into_iter()
                .map(|repository| {
                    let (name, grants) = repository.into_parts();
                    GithubRepository::new(name, grants)
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(source_configuration_error)?;
            GithubConfig::new(
                id,
                required,
                grants,
                api_base_url,
                allow_private_network,
                convert_secret(credential)?,
                repositories,
            )
            .map(ConfiguredSource::Github)
            .map_err(source_configuration_error)
        }
        SourceProfile::Ssh(source) => {
            let (id, required, grants, command, hosts) = source.into_parts();
            let hosts = hosts
                .into_iter()
                .map(|host| {
                    let (alias, remote_roots) = host.into_parts();
                    SshHost::new(alias, remote_roots)
                })
                .collect();
            SshConfig::new(id, required, grants, convert_command(command)?, hosts)
                .map(ConfiguredSource::Ssh)
                .map_err(source_configuration_error)
        }
        SourceProfile::DownstreamMcp(source) => {
            let (id, required, grants, servers) = source.into_parts();
            let servers = servers
                .into_iter()
                .map(|server| {
                    let (id, schemes, transport) = server.into_parts();
                    let schemes = schemes
                        .into_iter()
                        .map(SchemeClaim::new)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(source_configuration_error)?;
                    Ok(DownstreamServer::new(
                        id,
                        schemes,
                        convert_transport(transport)?,
                    ))
                })
                .collect::<Result<Vec<_>, ProfileError>>()?;
            DownstreamMcpConfig::new(id, required, grants, servers)
                .map(ConfiguredSource::DownstreamMcp)
                .map_err(source_configuration_error)
        }
        SourceProfile::Documents(source) => {
            let (id, required, grants, converters) = source.into_parts();
            let converters = converters
                .into_iter()
                .map(|converter| {
                    let (extensions, input, command) = converter.into_parts();
                    let input = match input {
                        ConverterInputProfile::Stdin => ConverterInput::Stdin,
                        ConverterInputProfile::Path => ConverterInput::Path,
                    };
                    DocumentConverter::new(extensions, input, convert_command(command)?)
                        .map_err(source_configuration_error)
                })
                .collect::<Result<Vec<_>, ProfileError>>()?;
            DocumentsConfig::new(id, required, grants, converters)
                .map(ConfiguredSource::Documents)
                .map_err(source_configuration_error)
        }
        SourceProfile::Skills(source) => {
            let (id, required, grants, roots) = source.into_parts();
            SkillsConfig::new(id, required, grants, base, roots)
                .map(ConfiguredSource::Skills)
                .map_err(source_configuration_error)
        }
        SourceProfile::Rules(source) => {
            let (id, required, grants, manifests) = source.into_parts();
            RulesConfig::new(id, required, grants, base, manifests)
                .map(ConfiguredSource::Rules)
                .map_err(source_configuration_error)
        }
        SourceProfile::Memory(source) => {
            let (id, required, grants, roots) = source.into_parts();
            let roots = roots
                .into_iter()
                .map(|root| {
                    let (name, path) = root.into_parts();
                    MemoryRoot::new(name, path)
                })
                .collect();
            MemoryConfig::new(id, required, grants, base, roots)
                .map(ConfiguredSource::Memory)
                .map_err(source_configuration_error)
        }
        SourceProfile::Vault(source) => {
            let (id, required, grants, vaults) = source.into_parts();
            let vaults = vaults
                .into_iter()
                .map(|vault| {
                    let (name, path, grants) = vault.into_parts();
                    VaultRoot::new(name, path, grants)
                })
                .collect();
            VaultConfig::new(id, required, grants, base, vaults)
                .map(ConfiguredSource::Vault)
                .map_err(source_configuration_error)
        }
        SourceProfile::AgentExport(source) => {
            let (id, required, grants, manifests) = source.into_parts();
            AgentExportConfig::new(id, required, grants, base, manifests)
                .map(ConfiguredSource::AgentExport)
                .map_err(source_configuration_error)
        }
    }
}

fn convert_transport(
    transport: DownstreamTransportProfile,
) -> Result<DownstreamTransport, ProfileError> {
    match transport {
        DownstreamTransportProfile::Stdio { command } => {
            Ok(DownstreamTransport::stdio(convert_command(command)?))
        }
        DownstreamTransportProfile::Http {
            endpoint,
            allow_private_network,
            credential,
        } => Ok(DownstreamTransport::http(
            endpoint,
            allow_private_network,
            credential.map(convert_credential).transpose()?,
        )),
    }
}

fn convert_credential(
    credential: CredentialHeaderProfile,
) -> Result<CredentialHeader, ProfileError> {
    let (header, scheme, secret) = credential.into_parts();
    CredentialHeader::new(header, scheme, convert_secret(secret)?)
        .map_err(source_configuration_error)
}

fn convert_secret(reference: SecretReferenceProfile) -> Result<SecretReference, ProfileError> {
    match reference {
        SecretReferenceProfile::Environment { name } => {
            SecretReference::environment(name).map_err(source_configuration_error)
        }
        SecretReferenceProfile::Command { command } => {
            SecretReference::command(convert_command(command)?).map_err(source_configuration_error)
        }
    }
}

fn convert_command(command: CommandProfile) -> Result<CommandSpec, ProfileError> {
    let (argv, environment) = command.into_parts();
    let environment = environment
        .into_iter()
        .map(|(name, value)| Ok((name, convert_environment_value(value)?)))
        .collect::<Result<BTreeMap<_, _>, ProfileError>>()?;
    let environment = ChildEnvironment::new(environment).map_err(source_configuration_error)?;
    CommandSpec::new(argv, environment).map_err(source_configuration_error)
}

fn convert_environment_value(
    value: EnvironmentValueProfile,
) -> Result<EnvironmentValue, ProfileError> {
    match value {
        EnvironmentValueProfile::Literal { value } => {
            EnvironmentValue::literal(value).map_err(source_configuration_error)
        }
        EnvironmentValueProfile::Inherit { name } => {
            EnvironmentValue::inherit(name).map_err(source_configuration_error)
        }
        EnvironmentValueProfile::Secret { secret } => {
            Ok(EnvironmentValue::secret(convert_secret(secret)?))
        }
    }
}

fn source_configuration_error(error: resourcefs_sources::ConfigurationError) -> ProfileError {
    ProfileError::invalid(format!("source configuration: {error}"))
}
