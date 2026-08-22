use async_trait::async_trait;
use std::fmt;

use crate::OperationGuard;

/// Maximum UTF-8 bytes carried by one already-redacted probe diagnostic.
pub const MAX_PROBE_DIAGNOSTIC_BYTES: usize = 4_096;

/// Source availability observed by one bounded, non-mutating probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeState {
    Available,
    Degraded,
    Failed,
    Unsupported,
}

/// Bounded diagnostic text that callers must redact before construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeDiagnostic(String);

impl ProbeDiagnostic {
    /// Accepts one non-empty diagnostic within the published byte ceiling.
    pub fn new(message: String) -> Result<Self, ProbeDiagnosticError> {
        if message.is_empty() {
            return Err(ProbeDiagnosticError::Empty);
        }
        if message.len() > MAX_PROBE_DIAGNOSTIC_BYTES {
            return Err(ProbeDiagnosticError::LimitExceeded);
        }
        Ok(Self(message))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable validation failure for probe diagnostic text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeDiagnosticError {
    Empty,
    LimitExceeded,
}

impl fmt::Display for ProbeDiagnosticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "probe diagnostic must not be empty",
            Self::LimitExceeded => "probe diagnostic exceeded its byte ceiling",
        })
    }
}

impl std::error::Error for ProbeDiagnosticError {}

/// Complete outcome from one probe attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    state: ProbeState,
    diagnostic: Option<ProbeDiagnostic>,
}

impl ProbeOutcome {
    #[must_use]
    pub const fn new(state: ProbeState, diagnostic: Option<ProbeDiagnostic>) -> Self {
        Self { state, diagnostic }
    }

    #[must_use]
    pub const fn state(&self) -> ProbeState {
        self.state
    }

    #[must_use]
    pub const fn diagnostic(&self) -> Option<&ProbeDiagnostic> {
        self.diagnostic.as_ref()
    }
}

/// One compiled source adapter's bounded, non-mutating availability check.
#[async_trait]
pub trait SourceProbe: Send + Sync {
    async fn probe(&self, operation: &OperationGuard) -> ProbeOutcome;
}
