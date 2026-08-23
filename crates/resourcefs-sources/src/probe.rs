use std::{sync::Arc, time::Duration};

use resourcefs_core::{AddressPolicy, OperationGuard, ProbeOutcome, ProbeState, SourceProbe};
use tokio::{
    net::{TcpStream, lookup_host},
    time::timeout,
};

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

/// One probe endpoint and the address policy its origin grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeEndpoint<'a> {
    pub host: &'a str,
    pub port: u16,
    pub allow_private_network: bool,
}

/// Bounded TCP-connectivity probe for one network-backed source.
///
/// Reachability is still a connect, but it is a *policed* connect: the host is
/// resolved, every resolved address is authorized through the same
/// [`AddressPolicy`] the request path uses, and only an authorized address is
/// dialled. Without that, an origin declaring `allowPrivateNetwork: false`
/// would still have its startup probe reach private space — an egress that
/// bypasses the very grant the operator withheld.
#[derive(Debug, Clone)]
pub struct NetworkProbe {
    endpoints: Vec<(String, u16, AddressPolicy)>,
    required: bool,
}

impl NetworkProbe {
    pub fn new(
        endpoints: Vec<(String, u16, AddressPolicy)>,
        required: bool,
    ) -> Result<Self, ConfigurationError> {
        if endpoints.is_empty() || endpoints.len() > MAX_CONFIGURATION_ENTRIES {
            return Err(ConfigurationError::new(format!(
                "network probe endpoints must contain 1–{MAX_CONFIGURATION_ENTRIES} entries"
            )));
        }
        if endpoints
            .iter()
            .any(|(host, port, _)| host.is_empty() || *port == 0)
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

    /// Connects to one endpoint only after its resolved addresses are authorized.
    ///
    /// Every resolved address must pass: a host resolving partly into
    /// restricted space is refused outright rather than probed through
    /// whichever address happened to be acceptable.
    async fn connect_policed(host: &str, port: u16, policy: AddressPolicy) -> bool {
        let Ok(addresses) = lookup_host((host, port)).await else {
            return false;
        };
        let addresses: Vec<_> = addresses.collect();
        if addresses.is_empty() {
            return false;
        }
        if addresses
            .iter()
            .any(|address| policy.authorize(address.ip()).is_err())
        {
            return false;
        }
        for address in addresses {
            if TcpStream::connect(address).await.is_ok() {
                return true;
            }
        }
        false
    }
}

#[async_trait::async_trait]
impl SourceProbe for NetworkProbe {
    async fn probe(&self, operation: &OperationGuard) -> ProbeOutcome {
        let connected = tokio::select! {
            biased;
            () = operation.cancelled() => false,
            result = timeout(NETWORK_PROBE_TIMEOUT, async {
                for (host, port, policy) in &self.endpoints {
                    if !Self::connect_policed(host.as_str(), *port, *policy).await {
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
