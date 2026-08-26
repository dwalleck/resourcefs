use std::{collections::HashSet, fmt, path::Path};

use resourcefs_core::{ProbeState, Redactor, Secret, ServerLimits};
use resourcefs_sources::{
    BackingPathVisibility, FilesystemSource, GithubSourceMount, HttpsSource, LaunchRoot,
    LaunchRootSource, ProbeRun, SESSION_CLEANUP_TTL, SessionStorageConfig,
};

use crate::{
    logging::LogConfig,
    profile::{self, CheckedLaunchProfile},
};

/// Source kinds this binary can actually mount from a profile.
///
/// A configured kind absent here fails startup rather than being ignored, so a
/// profile never silently serves less than it declares. HTTPS and GitHub are
/// the currently mounted network-backed profile sources.
const COMPILED_PROFILE_SOURCE_KINDS: &[&str] = &["https", "github"];

pub(crate) struct LaunchPlan {
    source: FilesystemSource,
    https: Option<HttpsSource>,
    github: Option<GithubSourceMount>,
    limits: ServerLimits,
    session_storage: SessionStorageConfig,
    logging: LogConfig,
    redactor: Redactor,
}

impl LaunchPlan {
    pub(crate) async fn from_profile(path: &Path) -> Result<Self, LaunchError> {
        let (checked, run) = profile::load_for_serve(path, COMPILED_PROFILE_SOURCE_KINDS)
            .await
            .map_err(LaunchError::configuration)?;
        Self::from_checked_profile(checked, &run).await
    }

    /// Classifies one startup probe run into the launch outcome.
    ///
    /// A required source that is unreachable or has no compiled adapter stops
    /// startup; an optional one degrades. The two are distinguished by the
    /// source's own `required` flag rather than by the failure's shape, so a
    /// host that is down and a grant that is withheld degrade identically —
    /// which is the intended contract, and why the diagnostic names the source
    /// id rather than guessing at a cause. Giving probe failures a real
    /// diagnostic is tracked at rfs-0p7j.
    fn degraded_ids(run: &ProbeRun) -> Result<HashSet<String>, LaunchError> {
        let mut degraded = HashSet::new();
        for record in run.records() {
            match record.outcome().state() {
                ProbeState::Available => {}
                ProbeState::Degraded if !record.required() => {
                    degraded.insert(record.id().to_owned());
                }
                state => {
                    return Err(LaunchError::required_unavailable(format!(
                        "required source '{}' of kind '{}' is unavailable at startup ({state:?})",
                        record.id(),
                        record.kind()
                    )));
                }
            }
        }
        Ok(degraded)
    }

    async fn from_checked_profile(
        checked: CheckedLaunchProfile,
        run: &ProbeRun,
    ) -> Result<Self, LaunchError> {
        // Ordered before the probe verdict on purpose. A kind this binary
        // cannot mount is not a reachability question, and answering it as one
        // would replace an exact diagnostic with a vaguer "unavailable".
        if let Some(source) = checked
            .sources
            .iter()
            .find(|source| !is_profile_source_compiled(source.kind))
        {
            return Err(LaunchError::required_unavailable(format!(
                "configured source '{}' has kind '{}' which is not supported by this ResourceFS binary",
                source.id, source.kind
            )));
        }
        let degraded = Self::degraded_ids(run)?;
        let source = FilesystemSource::new(
            checked.root_source,
            checked.primary_selector,
            checked.visibility,
        )
        .await
        .map_err(LaunchError::configuration)?;
        let https = profile::mount_https(checked.https, &checked.configuration_base, &degraded)
            .await
            .map_err(LaunchError::configuration)?;
        let github = profile::mount_github(checked.github, &checked.configuration_base, &degraded)
            .await
            .map_err(LaunchError::configuration)?;
        Ok(Self {
            source,
            https,
            github,
            limits: checked.limits,
            session_storage: checked.session_storage,
            logging: checked.logging,
            redactor: checked.redactor,
        })
    }

    pub(crate) async fn from_cli(
        roots: Vec<LaunchRoot>,
        primary_selector: Option<String>,
    ) -> Result<Self, LaunchError> {
        let source = FilesystemSource::new(
            LaunchRootSource::Cli(roots),
            primary_selector,
            BackingPathVisibility::Hidden,
        )
        .await
        .map_err(LaunchError::configuration)?;
        let session_storage =
            SessionStorageConfig::for_current_user(SESSION_CLEANUP_TTL.as_secs() as i64)
                .map_err(LaunchError::internal)?;
        let redactor = Redactor::new(std::iter::empty::<&Secret>())
            .map_err(|_| LaunchError::internal("could not construct the credential redactor"))?;
        Ok(Self {
            source,
            // CLI launches declare no profile, so no HTTPS origin exists.
            https: None,
            github: None,
            limits: ServerLimits::default(),
            session_storage,
            logging: LogConfig::default(),
            redactor,
        })
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        FilesystemSource,
        Option<HttpsSource>,
        Option<GithubSourceMount>,
        ServerLimits,
        SessionStorageConfig,
        LogConfig,
        Redactor,
    ) {
        (
            self.source,
            self.https,
            self.github,
            self.limits,
            self.session_storage,
            self.logging,
            self.redactor,
        )
    }
}

fn is_profile_source_compiled(kind: &str) -> bool {
    COMPILED_PROFILE_SOURCE_KINDS.contains(&kind)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchErrorKind {
    Configuration,
    RequiredUnavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchError {
    kind: LaunchErrorKind,
    message: String,
}

impl LaunchError {
    fn configuration(error: impl fmt::Display) -> Self {
        Self::new(LaunchErrorKind::Configuration, error.to_string())
    }

    fn required_unavailable(message: impl Into<String>) -> Self {
        Self::new(LaunchErrorKind::RequiredUnavailable, message)
    }

    fn internal(error: impl fmt::Display) -> Self {
        Self::new(LaunchErrorKind::Internal, error.to_string())
    }

    fn new(kind: LaunchErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) const fn kind(&self) -> LaunchErrorKind {
        self.kind
    }
}

impl fmt::Display for LaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LaunchError {}
