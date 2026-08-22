use std::{sync::Arc, time::Duration};

use resourcefs_core::{OperationGuard, ProbeOutcome, ProbeState, SourceProbe};
use tokio::{net::TcpStream, time::timeout};

use crate::{ConfigurationError, MAX_CONFIGURATION_ENTRIES};

const NETWORK_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Probe for a local source whose paths were opened during profile validation.
#[derive(Debug, Default, Clone, Copy)]
pub struct ValidatedLocalProbe;

#[async_trait::async_trait]
impl SourceProbe for ValidatedLocalProbe {
    async fn probe(&self, _operation: &OperationGuard) -> ProbeOutcome {
        ProbeOutcome::new(ProbeState::Available, None)
    }
}

/// Bounded TCP-connectivity probe for one network-backed source.
#[derive(Debug, Clone)]
pub struct NetworkProbe {
    endpoints: Vec<(String, u16)>,
    required: bool,
}

impl NetworkProbe {
    pub fn new(endpoints: Vec<(String, u16)>, required: bool) -> Result<Self, ConfigurationError> {
        if endpoints.is_empty() || endpoints.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "network probe endpoints must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }
        if endpoints
            .iter()
            .any(|(host, port)| host.is_empty() || *port == 0)
        {
            return Err(ConfigurationError::new(
                "network probe endpoints require a non-empty host and nonzero port",
            ));
        }
        Ok(Self {
            endpoints,
            required,
        })
    }
}

#[async_trait::async_trait]
impl SourceProbe for NetworkProbe {
    async fn probe(&self, operation: &OperationGuard) -> ProbeOutcome {
        let connected = tokio::select! {
            biased;
            () = operation.cancelled() => false,
            result = timeout(NETWORK_PROBE_TIMEOUT, async {
                for (host, port) in &self.endpoints {
                    if TcpStream::connect((host.as_str(), *port)).await.is_err() {
                        return false;
                    }
                }
                true
            }) => matches!(result, Ok(true)),
        };
        ProbeOutcome::new(
            if connected {
                ProbeState::Available
            } else if self.required {
                ProbeState::Failed
            } else {
                ProbeState::Degraded
            },
            None,
        )
    }
}

/// One configured source and its optional compiled probe adapter.
#[derive(Clone)]
pub struct ProbeTarget {
    id: String,
    kind: String,
    required: bool,
    probe: Option<Arc<dyn SourceProbe>>,
}

impl ProbeTarget {
    /// Registers a compiled probe adapter for one configured source.
    pub fn compiled<P>(
        id: impl Into<String>,
        kind: impl Into<String>,
        required: bool,
        probe: P,
    ) -> Self
    where
        P: SourceProbe + 'static,
    {
        Self {
            id: id.into(),
            kind: kind.into(),
            required,
            probe: Some(Arc::new(probe)),
        }
    }

    /// Registers a configured source with no adapter in this binary.
    pub fn unsupported(id: impl Into<String>, kind: impl Into<String>, required: bool) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            required,
            probe: None,
        }
    }
}

/// One ordered source row produced by a probe run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRecord {
    id: String,
    kind: String,
    required: bool,
    outcome: ProbeOutcome,
}

impl ProbeRecord {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }

    #[must_use]
    pub const fn outcome(&self) -> &ProbeOutcome {
        &self.outcome
    }
}

/// Complete ordered result from probing one profile source catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRun {
    ok: bool,
    records: Vec<ProbeRecord>,
}

impl ProbeRun {
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.ok
    }

    #[must_use]
    pub fn records(&self) -> &[ProbeRecord] {
        &self.records
    }
}

/// Executes each configured probe adapter exactly once in profile order.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProbeRunner;

impl ProbeRunner {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub async fn run(&self, targets: &[ProbeTarget], operation: &OperationGuard) -> ProbeRun {
        let mut ok = true;
        let mut records = Vec::with_capacity(targets.len());
        for target in targets {
            let outcome = match &target.probe {
                Some(probe) => probe.probe(operation).await,
                None => ProbeOutcome::new(ProbeState::Unsupported, None),
            };
            ok &= outcome_is_acceptable(outcome.state(), target.required);
            records.push(ProbeRecord {
                id: target.id.clone(),
                kind: target.kind.clone(),
                required: target.required,
                outcome,
            });
        }
        ProbeRun { ok, records }
    }
}

const fn outcome_is_acceptable(state: ProbeState, required: bool) -> bool {
    match state {
        ProbeState::Available => true,
        ProbeState::Degraded => !required,
        ProbeState::Failed | ProbeState::Unsupported => false,
    }
}
