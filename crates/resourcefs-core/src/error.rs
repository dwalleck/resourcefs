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
    details: Option<ResourceErrorDetails>,
}

impl ResourceError {
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
            details: None,
        }
    }

    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn with_details(mut self, details: ResourceErrorDetails) -> Self {
        self.details = Some(details);
        self
    }

    pub const fn details(&self) -> Option<&ResourceErrorDetails> {
        self.details.as_ref()
    }
}

impl fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl std::error::Error for ResourceError {}

/// Finite, source-neutral machine-readable vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorReason {
    InvalidAcquisitionLimit,
    AcquisitionControlsUnsupported,
    DeploymentIdentityUnavailable,
    RepositoryNotAuthorized,
    UpstreamNotFoundOrHidden,
    UpstreamDenied,
    UpstreamRateLimited,
    UpstreamUnavailable,
    UpstreamMalformed,
    UpstreamIdentityMismatch,
    TransportFailure,
    LimitExceeded,
    DeadlineExceeded,
    Cancelled,
}

impl ErrorReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidAcquisitionLimit => "invalid_acquisition_limit",
            Self::AcquisitionControlsUnsupported => "acquisition_controls_unsupported",
            Self::DeploymentIdentityUnavailable => "deployment_identity_unavailable",
            Self::RepositoryNotAuthorized => "repository_not_authorized",
            Self::UpstreamNotFoundOrHidden => "upstream_not_found_or_hidden",
            Self::UpstreamDenied => "upstream_denied",
            Self::UpstreamRateLimited => "upstream_rate_limited",
            Self::UpstreamUnavailable => "upstream_unavailable",
            Self::UpstreamMalformed => "upstream_malformed",
            Self::UpstreamIdentityMismatch => "upstream_identity_mismatch",
            Self::TransportFailure => "transport_failure",
            Self::LimitExceeded => "limit_exceeded",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Finite, source-neutral machine-readable vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionLimitKind {
    Attempts,
    ElapsedNanoseconds,
    ResponseBodyBytes,
    AcceptedBodyBytes,
    RepresentationBytes,
}

impl AcquisitionLimitKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Attempts => "attempts",
            Self::ElapsedNanoseconds => "elapsed_nanoseconds",
            Self::ResponseBodyBytes => "response_body_bytes",
            Self::AcceptedBodyBytes => "accepted_body_bytes",
            Self::RepresentationBytes => "representation_bytes",
        }
    }
}

/// A validated three-digit HTTP status, without retaining response text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpStatus(u16);
impl HttpStatus {
    pub fn new(value: u16) -> Result<Self, ResourceError> {
        if (100..=999).contains(&value) {
            Ok(Self(value))
        } else {
            // The status came from the far end, not from the caller's
            // reference; `invalid_reference` is what an agent reads as "the
            // path you wrote is wrong" and would stop it retrying a
            // recoverable upstream defect.
            Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "HTTP status must have three digits",
            )
            .with_details(ResourceErrorDetails::new(ErrorReason::UpstreamMalformed)))
        }
    }
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessAmbiguity {
    MissingOrAccessHidden,
}
impl AccessAmbiguity {
    pub const fn as_str(self) -> &'static str {
        "missing_or_access_hidden"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryGuidance {
    DelaySeconds(u64),
    AtUnixSeconds(u64),
}

/// A positive effective bound and optional measured/requested value in the kind's units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitDetail {
    kind: AcquisitionLimitKind,
    bound: std::num::NonZeroU64,
    observed: Option<u64>,
}
impl LimitDetail {
    pub fn new(
        kind: AcquisitionLimitKind,
        bound: u64,
        observed: Option<u64>,
    ) -> Result<Self, ResourceError> {
        let bound = std::num::NonZeroU64::new(bound).ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::LimitExceeded,
                "limit detail bound must be positive",
            )
            .with_details(ResourceErrorDetails::new(
                ErrorReason::InvalidAcquisitionLimit,
            ))
        })?;
        Ok(Self {
            kind,
            bound,
            observed,
        })
    }
    pub const fn kind(self) -> AcquisitionLimitKind {
        self.kind
    }
    pub const fn bound(self) -> u64 {
        self.bound.get()
    }
    pub const fn observed(self) -> Option<u64> {
        self.observed
    }
}

/// Bounded operational facts; no provider prose, URLs, headers or arbitrary metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceErrorDetails {
    reason: ErrorReason,
    http_status: Option<HttpStatus>,
    access_ambiguity: Option<AccessAmbiguity>,
    retry_guidance: Option<RetryGuidance>,
    limit: Option<LimitDetail>,
    rate_limit_reset: Option<u64>,
}
impl ResourceErrorDetails {
    pub const fn new(reason: ErrorReason) -> Self {
        Self {
            reason,
            http_status: None,
            access_ambiguity: None,
            retry_guidance: None,
            limit: None,
            rate_limit_reset: None,
        }
    }
    pub const fn with_http_status(mut self, value: HttpStatus) -> Self {
        self.http_status = Some(value);
        self
    }
    pub const fn with_access_ambiguity(mut self, value: AccessAmbiguity) -> Self {
        self.access_ambiguity = Some(value);
        self
    }
    pub const fn with_retry_guidance(mut self, value: RetryGuidance) -> Self {
        self.retry_guidance = Some(value);
        self
    }
    pub const fn with_rate_limit_reset(mut self, value: u64) -> Self {
        self.rate_limit_reset = Some(value);
        self
    }
    pub const fn with_limit(mut self, value: LimitDetail) -> Self {
        self.limit = Some(value);
        self
    }
    pub const fn reason(self) -> ErrorReason {
        self.reason
    }
    pub const fn http_status(self) -> Option<HttpStatus> {
        self.http_status
    }
    pub const fn access_ambiguity(self) -> Option<AccessAmbiguity> {
        self.access_ambiguity
    }
    pub const fn retry_guidance(self) -> Option<RetryGuidance> {
        self.retry_guidance
    }
    pub const fn rate_limit_reset(self) -> Option<u64> {
        self.rate_limit_reset
    }
    pub const fn limit(self) -> Option<LimitDetail> {
        self.limit
    }
}
