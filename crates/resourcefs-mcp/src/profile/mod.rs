use std::path::Path;

use resourcefs_core::{OperationGuard, Redactor, ServerLimits};
use resourcefs_sources::{
    BackingPathVisibility, LaunchRootSource, ProbeRunner, SessionStorageConfig,
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
