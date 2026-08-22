use std::{ffi::OsString, fmt};

use resourcefs_core::{OperationGuard, Secret};

use crate::{
    CommandErrorKind, CommandExecutor, CommandInput, CommandRole, SecretReference,
    configuration::SecretReferenceKind,
};

/// Stable category for a bounded secret-resolution failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretResolutionErrorKind {
    Unavailable,
    InvalidValue,
    HelperFailed,
    LimitExceeded,
    Cancelled,
}

/// A bounded failure that never contains resolved credential or helper output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretResolutionError {
    kind: SecretResolutionErrorKind,
    message: &'static str,
}

impl SecretResolutionError {
    const fn new(kind: SecretResolutionErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    /// Returns the stable semantic category.
    #[must_use]
    pub const fn kind(&self) -> SecretResolutionErrorKind {
        self.kind
    }

    const fn unavailable() -> Self {
        Self::new(
            SecretResolutionErrorKind::Unavailable,
            "referenced secret is unavailable",
        )
    }

    const fn invalid() -> Self {
        Self::new(
            SecretResolutionErrorKind::InvalidValue,
            "resolved secret value is invalid",
        )
    }

    const fn helper_failed() -> Self {
        Self::new(
            SecretResolutionErrorKind::HelperFailed,
            "secret helper failed",
        )
    }
}

impl fmt::Display for SecretResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for SecretResolutionError {}

/// Resolves one environment or helper reference without retaining its source bytes.
pub async fn resolve<F>(
    reference: &SecretReference,
    executor: &CommandExecutor,
    operation: &OperationGuard,
    environment: F,
) -> Result<Secret, SecretResolutionError>
where
    F: FnOnce(&str) -> Option<OsString>,
{
    match reference.kind() {
        SecretReferenceKind::Environment(name) => {
            let value = environment(name).ok_or_else(SecretResolutionError::unavailable)?;
            let value = value
                .into_string()
                .map_err(|_| SecretResolutionError::invalid())?;
            validate_secret(value)
        }
        SecretReferenceKind::Command(command) => {
            let output = executor
                .run(
                    command,
                    CommandRole::SecretHelper,
                    CommandInput::None,
                    operation,
                )
                .await
                .map_err(map_command_error)?;
            if !output.success() {
                return Err(SecretResolutionError::helper_failed());
            }
            normalize_helper_stdout(output.into_stdout())
        }
    }
}

fn normalize_helper_stdout(stdout: Vec<u8>) -> Result<Secret, SecretResolutionError> {
    let mut value = String::from_utf8(stdout).map_err(|_| SecretResolutionError::invalid())?;
    if value.ends_with("\r\n") {
        value.truncate(value.len() - 2);
    } else if value.ends_with('\n') {
        value.truncate(value.len() - 1);
    }
    validate_secret(value)
}

fn validate_secret(value: String) -> Result<Secret, SecretResolutionError> {
    Secret::new(value).map_err(|_| SecretResolutionError::invalid())
}

fn map_command_error(error: crate::CommandError) -> SecretResolutionError {
    match error.kind() {
        CommandErrorKind::LimitExceeded => SecretResolutionError::new(
            SecretResolutionErrorKind::LimitExceeded,
            "secret helper exceeded a configured limit",
        ),
        CommandErrorKind::Cancelled => SecretResolutionError::new(
            SecretResolutionErrorKind::Cancelled,
            "secret helper was cancelled",
        ),
        CommandErrorKind::InvalidConfiguration
        | CommandErrorKind::InvalidEnvironment
        | CommandErrorKind::InvalidInput
        | CommandErrorKind::Spawn
        | CommandErrorKind::Io => SecretResolutionError::helper_failed(),
    }
}
