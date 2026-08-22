use std::path::Path;

use resourcefs_core::OperationGuard;
use resourcefs_sources::ProbeRunner;

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
