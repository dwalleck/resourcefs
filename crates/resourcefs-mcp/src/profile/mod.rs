use std::path::Path;
use std::sync::Arc;

use resourcefs_core::{
    AllowedOrigin, HttpCeilings, OperationGuard, OriginAllowlist, Redactor, ServerLimits,
};
use resourcefs_sources::{
    BackingPathVisibility, CommandExecutor, HttpSubstrate, HttpsConfig, HttpsSource,
    LaunchRootSource, MAX_LIVE_COMMAND_TREES, OriginCredential, ProbeRunner, SessionStorageConfig,
    secret,
};

use crate::logging::LogConfig;

mod check;
mod convert;
mod model;
mod report;
mod schema;
mod validate;

pub use model::{
    MAX_ALLOWLIST_ENTRIES, MAX_PROFILE_BYTES, ProfileDocument, ProfileError, ProfileErrorKind,
};
pub use schema::profile_schema_json;

pub(crate) struct CheckedLaunchProfile {
    pub(crate) root_source: LaunchRootSource,
    pub(crate) primary_selector: Option<String>,
    pub(crate) visibility: BackingPathVisibility,
    pub(crate) limits: ServerLimits,
    pub(crate) session_storage: SessionStorageConfig,
    pub(crate) logging: LogConfig,
    pub(crate) redactor: Redactor,
    pub(crate) sources: Vec<CheckedSourceClaim>,
    /// Validated HTTPS sources, consumed by the launch path to mount a live
    /// `HttpsSource`.
    pub(crate) https: Vec<HttpsConfig>,
    /// Base directory for resolving credential-command paths at launch.
    pub(crate) configuration_base: std::path::PathBuf,
}

pub(crate) struct CheckedSourceClaim {
    pub(crate) id: String,
    pub(crate) kind: &'static str,
}

pub(crate) struct CheckOutput {
    report: String,
    ok: bool,
}

impl CheckOutput {
    pub(crate) fn report(&self) -> &str {
        &self.report
    }

    pub(crate) const fn ok(&self) -> bool {
        self.ok
    }
}

pub(crate) fn load_for_serve(path: &Path) -> Result<CheckedLaunchProfile, ProfileError> {
    let checked = check::check_profile(path, |name| std::env::var_os(name))?;
    #[cfg(feature = "test-support")]
    let mut checked = checked;
    #[cfg(feature = "test-support")]
    if let Some(value) = std::env::var_os("RESOURCEFS_TEST_REDACTION_SECRET") {
        let value = value
            .into_string()
            .map_err(|_| ProfileError::invalid("test credential injection was not Unicode"))?;
        checked.register_test_secret(value)?;
    }
    checked.into_launch_profile()
}

pub(crate) async fn check(path: &Path, probe: bool) -> Result<CheckOutput, ProfileError> {
    let mut checked = check::check_profile(path, |name| std::env::var_os(name))?;
    if !probe {
        let report = report::render_static(&checked)
            .map_err(|_| ProfileError::invalid("could not render the static check report"))?;
        return Ok(CheckOutput { report, ok: true });
    }

    let operation = OperationGuard::new();
    let targets = checked.probe_targets(&operation).await?;
    let run = ProbeRunner::new().run(&targets, &operation).await;
    let redactor = checked.redactor()?;
    let report = report::render_probe(checked.schema_version(), &run, &redactor)
        .map_err(|_| ProfileError::invalid("could not render the probe check report"))?;
    Ok(CheckOutput {
        report,
        ok: run.ok(),
    })
}

/// Mounts a live `HttpsSource` from the profile's declared HTTPS sources.
///
/// Returns `None` when no origin is declared, so no HTTP client is built for a
/// profile that never reaches the network. Credentials are resolved here — the
/// one place a `Secret` is produced on the launch path — and handed to the
/// substrate, which exposes them only when attaching a header to a request
/// belonging to the owning origin.
pub(crate) async fn mount_https(
    configs: Vec<HttpsConfig>,
    base: &std::path::Path,
) -> Result<Option<HttpsSource>, ProfileError> {
    if configs.is_empty() {
        return Ok(None);
    }
    let executor = CommandExecutor::new(MAX_LIVE_COMMAND_TREES, base)
        .map_err(|_| ProfileError::invalid("could not initialize the credential executor"))?;
    let operation = OperationGuard::new();

    let mut origins = Vec::new();
    let mut credentials = Vec::new();
    for config in &configs {
        for origin in config.origins() {
            let allowed = AllowedOrigin::new(origin.base_url(), origin.allow_private_network())
                .map_err(|error| ProfileError::invalid(format!("{error}")))?;
            if let Some(credential) = origin.credential() {
                let secret = secret::resolve(credential.secret(), &executor, &operation, |name| {
                    std::env::var_os(name)
                })
                .await
                .map_err(|_| {
                    // The reference is named, never the value: a credential
                    // that failed to resolve must not have its contents — or
                    // the resolver's raw output — reach a diagnostic.
                    ProfileError::invalid(format!(
                        "source '{}' could not resolve its configured credential",
                        config.id()
                    ))
                })?;
                credentials.push(
                    OriginCredential::new(
                        allowed.clone(),
                        credential.header(),
                        credential.scheme(),
                        &secret,
                    )
                    .map_err(|error| ProfileError::invalid(format!("{error}")))?,
                );
            }
            origins.push(allowed);
        }
    }
    let allowlist = OriginAllowlist::new(origins);
    // The signed ceilings: 8 MiB fetch, 5 redirect hops, 30-second timeout.
    let substrate = HttpSubstrate::new(allowlist, HttpCeilings::default(), credentials)
        .map_err(|error| ProfileError::invalid(format!("{error}")))?;
    Ok(Some(HttpsSource::new(Arc::new(substrate))))
}
