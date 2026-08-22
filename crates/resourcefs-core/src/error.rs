use std::fmt;

use serde::Serialize;

/// Stable semantic category for an operational ResourceFS failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    InvalidReference,
    NotFound,
    PermissionDenied,
    VersionConflict,
    InvalidPatch,
    LimitExceeded,
    SourceUnavailable,
    UnsupportedProjection,
    UnsupportedMutation,
    AmbiguousReference,
    InvalidPattern,
    Cancelled,
}

impl ErrorCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidReference => "invalid_reference",
            Self::NotFound => "not_found",
            Self::PermissionDenied => "permission_denied",
            Self::VersionConflict => "version_conflict",
            Self::InvalidPatch => "invalid_patch",
            Self::LimitExceeded => "limit_exceeded",
            Self::SourceUnavailable => "source_unavailable",
            Self::UnsupportedProjection => "unsupported_projection",
            Self::UnsupportedMutation => "unsupported_mutation",
            Self::AmbiguousReference => "ambiguous_reference",
            Self::InvalidPattern => "invalid_pattern",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Caller-visible ResourceFS operational error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceError {
    category: ErrorCategory,
    message: String,
}

impl ResourceError {
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl std::error::Error for ResourceError {}
