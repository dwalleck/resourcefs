//! Explicit adapters used only by integration-test executables.

use std::{io, path::Path};

use crate::{BoxError, launch::LaunchPlan, server};

pub use resourcefs_sources::TestRootCertificate;

/// Runs the normal profile-launched server over stdio with one fixture CA.
///
/// The production CLI has no call to this interface. Integration tests invoke
/// it from their own re-executed test harness, so fixture trust is never
/// selectable through a shipped `resourcefs` process input.
pub async fn serve_profile_with_https_root(
    profile: &Path,
    root: TestRootCertificate,
) -> Result<(), BoxError> {
    let plan = LaunchPlan::from_profile_with_https_root(profile, root)
        .await
        .map_err(|error| Box::new(error) as BoxError)?;
    server::serve(plan).await.map_err(|failure| {
        let diagnostic = failure
            .diagnostic()
            .unwrap_or("ResourceFS server failed after reporting its diagnostic");
        Box::new(io::Error::other(diagnostic.to_owned())) as BoxError
    })
}
