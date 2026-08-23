use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use resourcefs_core::{
    AddressPolicy, OperationGuard, ProbeDiagnostic, ProbeOutcome, ProbeState, Redactor, Secret,
    SourceProbe,
};
use resourcefs_sources::{
    ChildEnvironment, CommandExecutor, CommandSpec, EnvironmentValue, MAX_LIVE_COMMAND_TREES,
    NetworkProbe, ProbeTarget, SecretReference, ValidatedLocalProbe, secret,
};
use url::Url;

use super::{
    CheckedLaunchProfile, CheckedSourceClaim, ProfileDocument, ProfileError,
    model::{
        StaticCommand, StaticEnvironmentValue, StaticProbe, StaticRequirement, StaticSecret,
        StaticSecretKind, StaticSource,
    },
};

pub(super) struct CheckedProfile {
    schema_version: u32,
    sources: Vec<CheckedSource>,
    configuration_base: PathBuf,
    secrets: Vec<Secret>,
    profile: ProfileDocument,
}

impl CheckedProfile {
    pub(super) const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub(super) fn sources(&self) -> &[CheckedSource] {
        &self.sources
    }

    pub(super) fn redactor(&self) -> Result<Redactor, ProfileError> {
        Redactor::new(&self.secrets).map_err(|_| {
            ProfileError::invalid("could not construct the profile credential redactor")
        })
    }

    #[cfg(feature = "test-support")]
    pub(super) fn register_test_secret(&mut self, value: String) -> Result<(), ProfileError> {
        let secret = Secret::new(value)
            .map_err(|_| ProfileError::invalid("test credential injection was invalid"))?;
        self.secrets.push(secret);
        Ok(())
    }

    pub(super) fn into_launch_profile(self) -> Result<CheckedLaunchProfile, ProfileError> {
        let redactor = Redactor::new(&self.secrets).map_err(|_| {
            ProfileError::invalid("could not construct the profile credential redactor")
        })?;
        let sources = self
            .sources
            .into_iter()
            .map(|source| CheckedSourceClaim {
                id: source.id,
                kind: source.kind,
            })
            .collect();
        let components = self.profile.into_launch_components()?;
        Ok(CheckedLaunchProfile {
            root_source: components.root_source,
            primary_selector: components.primary_selector,
            visibility: components.visibility,
            limits: components.limits,
            session_storage: components.session_storage,
            logging: components.logging,
            redactor,
            sources,
            https: components.https,
            configuration_base: self.configuration_base.clone(),
        })
    }

    /// Builds one probe target per configured source.
    ///
    /// `mountable` restricts which kinds are actually probed. `None` probes
    /// everything, which is what an explicit `check` wants — the operator asked
    /// about the profile as written. The launch path passes the kinds this
    /// binary can mount, because a source it is about to refuse must not have
    /// its origin dialled or, worse, its credential helper executed on the way
    /// to that refusal.
    pub(super) async fn probe_targets(
        &mut self,
        operation: &OperationGuard,
        mountable: Option<&[&str]>,
    ) -> Result<Vec<ProbeTarget>, ProfileError> {
        let executor = CommandExecutor::new(MAX_LIVE_COMMAND_TREES, &self.configuration_base)
            .map_err(|_| {
                ProfileError::invalid("could not initialize the probe command executor")
            })?;
        let mut targets = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            if mountable.is_some_and(|kinds| !kinds.contains(&source.kind)) {
                targets.push(ProbeTarget::unsupported(
                    &source.id,
                    source.kind,
                    source.required,
                ));
                continue;
            }
            let mut dependency_available = true;
            if !matches!(source.probe, StaticProbe::Unsupported) {
                for deferred in &source.deferred_secrets {
                    match secret::resolve(&deferred.reference, &executor, operation, |name| {
                        std::env::var_os(name)
                    })
                    .await
                    {
                        Ok(resolved) => self.secrets.push(resolved),
                        Err(_) => {
                            dependency_available = false;
                            break;
                        }
                    }
                }
            }
            if dependency_available {
                targets.push(probe_target(source)?);
            } else {
                let diagnostic =
                    ProbeDiagnostic::new("source credential could not be resolved".to_owned())
                        .map_err(|_| {
                            ProfileError::invalid(
                                "could not construct the probe failure diagnostic",
                            )
                        })?;
                targets.push(ProbeTarget::compiled(
                    &source.id,
                    source.kind,
                    source.required,
                    DependencyFailureProbe {
                        required: source.required,
                        diagnostic,
                    },
                ));
            }
        }
        Ok(targets)
    }
}

#[derive(Debug, Clone)]
struct DeferredSecret {
    reference: SecretReference,
}

#[derive(Debug, Clone)]
pub(super) struct CheckedSource {
    id: String,
    kind: &'static str,
    required: bool,
    probe: StaticProbe,
    deferred_secrets: Vec<DeferredSecret>,
}

impl CheckedSource {
    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) const fn kind(&self) -> &'static str {
        self.kind
    }

    pub(super) const fn required(&self) -> bool {
        self.required
    }
}

#[derive(Debug, Clone)]
struct DependencyFailureProbe {
    required: bool,
    diagnostic: ProbeDiagnostic,
}

#[async_trait]
impl SourceProbe for DependencyFailureProbe {
    async fn probe(&self, _operation: &OperationGuard) -> ProbeOutcome {
        ProbeOutcome::new(
            if self.required {
                ProbeState::Failed
            } else {
                ProbeState::Degraded
            },
            Some(self.diagnostic.clone()),
        )
    }
}

fn probe_target(source: &CheckedSource) -> Result<ProbeTarget, ProfileError> {
    Ok(match &source.probe {
        StaticProbe::ValidatedLocal => ProbeTarget::compiled(
            &source.id,
            source.kind,
            source.required,
            ValidatedLocalProbe,
        ),
        StaticProbe::Network(urls) => {
            let endpoints = urls
                .iter()
                .map(|(value, allow_private_network)| {
                    let url = Url::parse(value).map_err(|_| {
                        ProfileError::invalid("validated network probe URL became invalid")
                    })?;
                    let host = url.host_str().ok_or_else(|| {
                        ProfileError::invalid("validated network probe URL has no host")
                    })?;
                    let port = url.port_or_known_default().ok_or_else(|| {
                        ProfileError::invalid("validated network probe URL has no port")
                    })?;
                    // The probe carries the same grant the request path uses,
                    // so a withheld private-network grant also withholds the
                    // probe's egress.
                    Ok((
                        host.to_owned(),
                        port,
                        AddressPolicy::new(*allow_private_network),
                    ))
                })
                .collect::<Result<Vec<_>, ProfileError>>()?;
            let probe = NetworkProbe::new(endpoints, source.required).map_err(|_| {
                ProfileError::invalid("validated network probe endpoints became invalid")
            })?;
            ProbeTarget::compiled(&source.id, source.kind, source.required, probe)
        }
        StaticProbe::Unsupported => {
            ProbeTarget::unsupported(&source.id, source.kind, source.required)
        }
    })
}

pub(super) fn check_profile<F>(path: &Path, environment: F) -> Result<CheckedProfile, ProfileError>
where
    F: FnMut(&str) -> Option<OsString>,
{
    let profile = ProfileDocument::load(path)?;
    let mut checker = StaticChecker::new(profile.configuration_base(), environment);
    let mut sources = Vec::with_capacity(profile.static_sources().len());
    for source in profile.static_sources() {
        checker.check_source(source)?;
        sources.push(CheckedSource {
            id: source.id.clone(),
            kind: source.kind,
            required: source.required,
            probe: source.probe.clone(),
            deferred_secrets: std::mem::take(&mut checker.deferred_secrets),
        });
    }
    Ok(CheckedProfile {
        schema_version: profile.schema_version(),
        sources,
        configuration_base: profile.configuration_base().to_owned(),
        secrets: checker.secrets,
        profile,
    })
}

struct StaticChecker<F> {
    base: PathBuf,
    environment: F,
    environment_cache: HashMap<String, Option<OsString>>,
    executable_cache: HashSet<(String, Option<OsString>)>,
    secrets: Vec<Secret>,
    deferred_secrets: Vec<DeferredSecret>,
}

impl<F> StaticChecker<F>
where
    F: FnMut(&str) -> Option<OsString>,
{
    fn new(base: &Path, environment: F) -> Self {
        Self {
            base: base.to_owned(),
            environment,
            environment_cache: HashMap::new(),
            executable_cache: HashSet::new(),
            secrets: Vec::new(),
            deferred_secrets: Vec::new(),
        }
    }

    fn check_source(&mut self, source: &StaticSource) -> Result<(), ProfileError> {
        for requirement in &source.requirements {
            match requirement {
                StaticRequirement::Secret(secret) => {
                    self.check_secret(secret)?;
                }
                StaticRequirement::Command(command) => self.check_command(command)?,
            }
        }
        Ok(())
    }

    fn check_secret(&mut self, secret: &StaticSecret) -> Result<Option<OsString>, ProfileError> {
        match &secret.kind {
            StaticSecretKind::Environment(name) => {
                let value = self.environment_value(name).ok_or_else(|| {
                    ProfileError::invalid(format!(
                        "{} references an absent environment variable",
                        secret.field
                    ))
                })?;
                let value = value.into_string().map_err(|_| {
                    ProfileError::invalid(format!(
                        "{} references a non-Unicode environment value",
                        secret.field
                    ))
                })?;
                let value = Secret::new(value).map_err(|error| {
                    ProfileError::invalid(format!("{} is invalid: {error}", secret.field))
                })?;
                let resolved = OsString::from(value.expose());
                self.secrets.push(value);
                Ok(Some(resolved))
            }
            StaticSecretKind::Command(command) => {
                self.check_command(command)?;
                let reference = command_secret_reference(command)?;
                self.deferred_secrets.push(DeferredSecret { reference });
                Ok(None)
            }
        }
    }

    fn check_command(&mut self, command: &StaticCommand) -> Result<(), ProfileError> {
        let mut path = None;
        for entry in &command.environment {
            let resolved = match &entry.value {
                StaticEnvironmentValue::Literal(value) => Some(OsString::from(value)),
                StaticEnvironmentValue::Inherit(name) => {
                    Some(self.environment_value(name).ok_or_else(|| {
                        ProfileError::invalid(format!(
                            "{} references an absent inherited environment variable",
                            entry.field
                        ))
                    })?)
                }
                StaticEnvironmentValue::Secret(secret) => self.check_secret(secret)?,
            };
            if entry.destination.eq_ignore_ascii_case("PATH") {
                let resolved = resolved.ok_or_else(|| {
                    ProfileError::invalid(format!(
                        "{} cannot resolve command PATH without executing a secret helper",
                        entry.field
                    ))
                })?;
                path = Some(resolved);
            }
        }
        if path.is_none() {
            path = self.environment_value("PATH");
        }
        self.check_executable(command, path)
    }

    fn check_executable(
        &mut self,
        command: &StaticCommand,
        path: Option<OsString>,
    ) -> Result<(), ProfileError> {
        let program_value = command.argv.first().ok_or_else(|| {
            ProfileError::invalid(format!("{}.argv must not be empty", command.field))
        })?;
        let cache_key = (program_value.clone(), path.clone());
        if self.executable_cache.contains(&cache_key) {
            return Ok(());
        }

        let program = Path::new(program_value);
        let resolved = if program.is_absolute() || contains_path_separator(program_value) {
            let candidate = if program.is_absolute() {
                program.to_owned()
            } else {
                self.base.join(program)
            };
            is_executable(&candidate)
        } else {
            let path = path.ok_or_else(|| {
                ProfileError::invalid(format!(
                    "{}.argv[0] cannot resolve a bare executable without PATH",
                    command.field
                ))
            })?;
            std::env::split_paths(&path).any(|directory| {
                if directory.is_absolute() {
                    is_executable_in(&directory, program_value)
                } else {
                    is_executable_in(&self.base.join(directory), program_value)
                }
            })
        };

        if !resolved {
            return Err(ProfileError::invalid(format!(
                "{}.argv[0] does not resolve to an executable file",
                command.field
            )));
        }
        self.executable_cache.insert(cache_key);
        Ok(())
    }

    fn environment_value(&mut self, name: &str) -> Option<OsString> {
        if let Some(value) = self.environment_cache.get(name) {
            return value.clone();
        }
        let value = (self.environment)(name);
        self.environment_cache
            .insert(name.to_owned(), value.clone());
        value
    }
}

fn command_secret_reference(command: &StaticCommand) -> Result<SecretReference, ProfileError> {
    let environment = command
        .environment
        .iter()
        .map(|entry| {
            let value = match &entry.value {
                StaticEnvironmentValue::Literal(value) => EnvironmentValue::literal(value.clone()),
                StaticEnvironmentValue::Inherit(name) => EnvironmentValue::inherit(name.clone()),
                StaticEnvironmentValue::Secret(_) => {
                    return Err(ProfileError::invalid(format!(
                        "{} contains a recursive secret reference",
                        entry.field
                    )));
                }
            }
            .map_err(|error| {
                ProfileError::invalid(format!("{} is invalid: {error}", entry.field))
            })?;
            Ok((entry.destination.clone(), value))
        })
        .collect::<Result<BTreeMap<_, _>, ProfileError>>()?;
    let environment = ChildEnvironment::new(environment).map_err(|error| {
        ProfileError::invalid(format!("{} environment is invalid: {error}", command.field))
    })?;
    let command_spec = CommandSpec::new(command.argv.clone(), environment)
        .map_err(|error| ProfileError::invalid(format!("{} is invalid: {error}", command.field)))?;
    SecretReference::command(command_spec)
        .map_err(|error| ProfileError::invalid(format!("{} is invalid: {error}", command.field)))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(unix)]
fn contains_path_separator(value: &str) -> bool {
    value.contains('/')
}

#[cfg(windows)]
fn contains_path_separator(value: &str) -> bool {
    value.contains(['/', '\\'])
}

#[cfg(unix)]
fn is_executable_in(directory: &Path, program: &str) -> bool {
    is_executable(&directory.join(program))
}
#[cfg(windows)]
fn is_executable_in(directory: &Path, program: &str) -> bool {
    let program = Path::new(program);
    if program.extension().is_some() {
        return is_executable(&directory.join(program));
    }
    is_executable(&directory.join(program).with_extension("exe"))
        || is_executable(&directory.join(program).with_extension("com"))
}
