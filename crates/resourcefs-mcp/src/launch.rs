use std::{fmt, path::Path};

use resourcefs_core::{Redactor, Secret, ServerLimits};
use resourcefs_sources::{
    BackingPathVisibility, FilesystemSource, LaunchRoot, LaunchRootSource, SESSION_CLEANUP_TTL,
    SessionStorageConfig,
};

use crate::{
    logging::LogConfig,
    profile::{self, CheckedLaunchProfile},
};

const COMPILED_PROFILE_SOURCE_KINDS: &[&str] = &[];

pub(crate) struct LaunchPlan {
    source: FilesystemSource,
    limits: ServerLimits,
    session_storage: SessionStorageConfig,
    logging: LogConfig,
    redactor: Redactor,
}

impl LaunchPlan {
    pub(crate) async fn from_profile(path: &Path) -> Result<Self, LaunchError> {
        let checked = profile::load_for_serve(path).map_err(LaunchError::configuration)?;
        Self::from_checked_profile(checked).await
    }

    async fn from_checked_profile(checked: CheckedLaunchProfile) -> Result<Self, LaunchError> {
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
        let source = FilesystemSource::new(
            checked.root_source,
            checked.primary_selector,
            checked.visibility,
        )
        .await
        .map_err(LaunchError::configuration)?;
        Ok(Self {
            source,
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
        ServerLimits,
        SessionStorageConfig,
        LogConfig,
        Redactor,
    ) {
        (
            self.source,
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
