use std::path::Path;
use std::sync::Arc;

use resourcefs_core::{
    AllowedOrigin, HttpCeilings, OperationGuard, OriginAllowlist, Redactor, ServerLimits,
};
#[cfg(feature = "test-support")]
use resourcefs_sources::TestRootCertificate;
use resourcefs_sources::{
    BackingPathVisibility, CommandExecutor, GithubConfig, GithubSourceMount, HttpSubstrate,
    HttpsConfig, HttpsSource, LaunchRootSource, MAX_LIVE_COMMAND_TREES, OriginCredential, ProbeRun,
    ProbeRunner, SessionStorageConfig, secret,
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
    pub(crate) github: Vec<GithubConfig>,
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

/// Loads a profile for serving and probes every configured source once.
///
/// The `ProbeRun` is returned rather than acted on here: whether an
/// unreachable source is fatal depends on its `required` flag, and mapping
/// that to an exit class is the launch path's job. Startup probes exactly once
/// — the same run decides both fatality and which sources mount degraded — so
/// no probe fires on the read path.
pub(crate) async fn load_for_serve(
    path: &Path,
    mountable: &[&str],
) -> Result<(CheckedLaunchProfile, ProbeRun), ProfileError> {
    // Mutable in every configuration: `probe_targets` records resolved secrets
    // on the profile as it builds them.
    let mut checked = check::check_profile(path, |name| std::env::var_os(name))?;
    #[cfg(feature = "test-support")]
    if let Some(value) = std::env::var_os("RESOURCEFS_TEST_REDACTION_SECRET") {
        let value = value
            .into_string()
            .map_err(|_| ProfileError::invalid("test credential injection was not Unicode"))?;
        checked.register_test_secret(value)?;
    }
    let operation = OperationGuard::new();
    let targets = checked.probe_targets(&operation, Some(mountable)).await?;
    let run = ProbeRunner::new().run(&targets, &operation).await;
    Ok((checked.into_launch_profile()?, run))
}

pub(crate) async fn check(path: &Path, probe: bool) -> Result<CheckOutput, ProfileError> {
    let mut checked = check::check_profile(path, |name| std::env::var_os(name))?;
    if !probe {
        let report = report::render_static(&checked)
            .map_err(|_| ProfileError::invalid("could not render the static check report"))?;
        return Ok(CheckOutput { report, ok: true });
    }

    let operation = OperationGuard::new();
    let targets = checked.probe_targets(&operation, None).await?;
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
    degraded_ids: &std::collections::HashSet<String>,
) -> Result<Option<HttpsSource>, ProfileError> {
    mount_https_with_trust(configs, base, degraded_ids, HttpsTrust::System).await
}

#[cfg(feature = "test-support")]
pub(crate) async fn mount_https_with_root(
    configs: Vec<HttpsConfig>,
    base: &std::path::Path,
    degraded_ids: &std::collections::HashSet<String>,
    root: TestRootCertificate,
) -> Result<Option<HttpsSource>, ProfileError> {
    mount_https_with_trust(configs, base, degraded_ids, HttpsTrust::Fixture(root)).await
}

enum HttpsTrust {
    System,
    #[cfg(feature = "test-support")]
    Fixture(TestRootCertificate),
}

async fn mount_https_with_trust(
    configs: Vec<HttpsConfig>,
    base: &std::path::Path,
    degraded_ids: &std::collections::HashSet<String>,
    trust: HttpsTrust,
) -> Result<Option<HttpsSource>, ProfileError> {
    if configs.is_empty() {
        return Ok(None);
    }
    let executor = CommandExecutor::new(MAX_LIVE_COMMAND_TREES, base)
        .map_err(|_| ProfileError::invalid("could not initialize the credential executor"))?;
    let operation = OperationGuard::new();

    let mut origins = Vec::new();
    let mut degraded = Vec::new();
    let mut credentials = Vec::new();
    for config in &configs {
        let is_degraded = degraded_ids.contains(config.id());
        for origin in config.origins() {
            let allowed = AllowedOrigin::new(origin.base_url(), origin.allow_private_network())
                .map_err(|error| ProfileError::invalid(format!("{error}")))?;
            if is_degraded {
                // Held apart from the allowlist so its references are refused
                // as unreachable rather than as undeclared, and so no
                // credential is resolved for a source that cannot be reached.
                degraded.push(allowed);
                continue;
            }
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
    let substrate = match trust {
        HttpsTrust::System => HttpSubstrate::new(allowlist, HttpCeilings::default(), credentials),
        #[cfg(feature = "test-support")]
        HttpsTrust::Fixture(root) => HttpSubstrate::with_system_lookup_and_root(
            allowlist,
            HttpCeilings::default(),
            root,
            credentials,
        ),
    }
    .map_err(|error| ProfileError::invalid(format!("{error}")))?
    .with_degraded_origins(degraded);
    Ok(Some(HttpsSource::new(Arc::new(substrate))))
}

pub(crate) async fn mount_github(
    configs: Vec<GithubConfig>,
    base: &std::path::Path,
    degraded_ids: &std::collections::HashSet<String>,
) -> Result<Option<GithubSourceMount>, ProfileError> {
    mount_github_with_trust(configs, base, degraded_ids, HttpsTrust::System).await
}

#[cfg(feature = "test-support")]
pub(crate) async fn mount_github_with_root(
    configs: Vec<GithubConfig>,
    base: &std::path::Path,
    degraded_ids: &std::collections::HashSet<String>,
    root: TestRootCertificate,
) -> Result<Option<GithubSourceMount>, ProfileError> {
    mount_github_with_trust(configs, base, degraded_ids, HttpsTrust::Fixture(root)).await
}

async fn mount_github_with_trust(
    mut configs: Vec<GithubConfig>,
    base: &std::path::Path,
    degraded_ids: &std::collections::HashSet<String>,
    trust: HttpsTrust,
) -> Result<Option<GithubSourceMount>, ProfileError> {
    if configs.is_empty() {
        return Ok(None);
    }
    if configs.len() != 1 {
        return Err(ProfileError::invalid(
            "exactly one GitHub source may own issue:// and pr://",
        ));
    }
    let config = configs.pop().expect("one checked GitHub config");
    let allowed = AllowedOrigin::new(config.api_base_url(), config.allow_private_network())
        .map_err(|error| ProfileError::invalid(format!("{error}")))?;
    let degraded_source = degraded_ids.contains(config.id());
    let mut origins = Vec::new();
    let mut degraded = Vec::new();
    let mut credentials = Vec::new();
    if degraded_source {
        degraded.push(allowed);
    } else {
        let executor = CommandExecutor::new(MAX_LIVE_COMMAND_TREES, base)
            .map_err(|_| ProfileError::invalid("could not initialize the credential executor"))?;
        let operation = OperationGuard::new();
        let resolved = secret::resolve(config.credential(), &executor, &operation, |name| {
            std::env::var_os(name)
        })
        .await
        .map_err(|_| {
            ProfileError::invalid(format!(
                "source '{}' could not resolve its configured credential",
                config.id()
            ))
        })?;
        credentials.push(
            OriginCredential::new(allowed.clone(), "Authorization", Some("Bearer"), &resolved)
                .map_err(|error| ProfileError::invalid(format!("{error}")))?,
        );
        origins.push(allowed);
    }
    let allowlist = OriginAllowlist::new(origins);
    let substrate = match trust {
        HttpsTrust::System => HttpSubstrate::new(allowlist, HttpCeilings::default(), credentials),
        #[cfg(feature = "test-support")]
        HttpsTrust::Fixture(root) => HttpSubstrate::with_system_lookup_and_root(
            allowlist,
            HttpCeilings::default(),
            root,
            credentials,
        ),
    }
    .map_err(|error| ProfileError::invalid(format!("{error}")))?
    .with_degraded_origins(degraded);
    Ok(Some(GithubSourceMount::new(config, Arc::new(substrate))))
}
