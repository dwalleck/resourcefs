use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use resourcefs_core::{
    MAX_PROBE_DIAGNOSTIC_BYTES, OperationGuard, ProbeDiagnostic, ProbeDiagnosticError,
    ProbeOutcome, ProbeState, SourceProbe,
};
use resourcefs_sources::{MAX_CONFIGURATION_ENTRIES, NetworkProbe, ProbeRunner, ProbeTarget};

struct CountingProbe {
    attempts: Arc<AtomicUsize>,
    outcome: ProbeOutcome,
}

#[async_trait]
impl SourceProbe for CountingProbe {
    async fn probe(&self, _operation: &OperationGuard) -> ProbeOutcome {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        self.outcome.clone()
    }
}

#[tokio::test]
async fn probe_state_and_exit_matrix() {
    let diagnostic = ProbeDiagnostic::new("bounded diagnostic".to_owned())
        .unwrap_or_else(|error| panic!("diagnostic failed: {error}"));
    let rows = [
        Row::compiled(
            "available-optional",
            false,
            ProbeState::Available,
            None,
            true,
        ),
        Row::compiled(
            "available-required",
            true,
            ProbeState::Available,
            None,
            true,
        ),
        Row::compiled("degraded-optional", false, ProbeState::Degraded, None, true),
        Row::compiled(
            "degraded-required",
            true,
            ProbeState::Degraded,
            Some(diagnostic),
            false,
        ),
        Row::compiled("failed-optional", false, ProbeState::Failed, None, false),
        Row::compiled("failed-required", true, ProbeState::Failed, None, false),
        Row::unsupported("unsupported-optional", false, false),
        Row::unsupported("unsupported-required", true, false),
    ];

    let empty = ProbeRunner::new().run(&[], &OperationGuard::new()).await;
    assert!(empty.ok());
    assert!(empty.records().is_empty());

    for row in rows {
        let result = ProbeRunner::new()
            .run(std::slice::from_ref(&row.target), &OperationGuard::new())
            .await;
        assert_eq!(result.ok(), row.expected_ok, "row {}", row.id);
        assert_eq!(result.records().len(), 1, "row {}", row.id);
        let record = &result.records()[0];
        assert_eq!(record.id(), row.id);
        assert_eq!(record.kind(), "fixture");
        assert_eq!(record.required(), row.required);
        assert_eq!(record.outcome().state(), row.expected_state);
        assert_eq!(
            record.outcome().diagnostic().map(ProbeDiagnostic::as_str),
            row.expected_diagnostic
        );
        assert_eq!(row.attempts.load(Ordering::SeqCst), row.expected_attempts);
    }
}

#[test]
fn probe_diagnostic_boundaries_are_exact() {
    assert_eq!(
        ProbeDiagnostic::new(String::new()),
        Err(ProbeDiagnosticError::Empty)
    );
    let exact = ProbeDiagnostic::new("x".repeat(MAX_PROBE_DIAGNOSTIC_BYTES))
        .unwrap_or_else(|error| panic!("exact diagnostic failed: {error}"));
    assert_eq!(exact.as_str().len(), MAX_PROBE_DIAGNOSTIC_BYTES);
    assert_eq!(
        ProbeDiagnostic::new("x".repeat(MAX_PROBE_DIAGNOSTIC_BYTES + 1)),
        Err(ProbeDiagnosticError::LimitExceeded)
    );
}

#[test]
fn network_probe_requires_bounded_valid_endpoints() {
    assert!(NetworkProbe::new(Vec::new(), false).is_err());
    assert!(NetworkProbe::new(vec![(String::new(), 443)], false).is_err());
    assert!(NetworkProbe::new(vec![("example.test".to_owned(), 0)], false).is_err());
    assert!(
        NetworkProbe::new(
            vec![("example.test".to_owned(), 443); MAX_CONFIGURATION_ENTRIES],
            false
        )
        .is_ok()
    );
    assert!(
        NetworkProbe::new(
            vec![("example.test".to_owned(), 443); MAX_CONFIGURATION_ENTRIES + 1],
            false
        )
        .is_err()
    );
}

#[tokio::test]
async fn network_probe_observes_connectivity_and_cancellation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("probe listener failed: {error}"));
    let endpoint = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("probe listener address failed: {error}"));
    let probe = NetworkProbe::new(vec![(endpoint.ip().to_string(), endpoint.port())], true)
        .unwrap_or_else(|error| panic!("network probe construction failed: {error}"));
    let operation = OperationGuard::new();
    let (outcome, accepted) = tokio::join!(
        probe.probe(&operation),
        tokio::time::timeout(std::time::Duration::from_secs(1), listener.accept())
    );
    assert_eq!(outcome.state(), ProbeState::Available);
    assert!(matches!(accepted, Ok(Ok(_))));

    operation.cancel();
    let cancelled = probe.probe(&operation).await;
    assert_eq!(cancelled.state(), ProbeState::Failed);
}

struct Row {
    id: &'static str,
    required: bool,
    target: ProbeTarget,
    attempts: Arc<AtomicUsize>,
    expected_attempts: usize,
    expected_state: ProbeState,
    expected_diagnostic: Option<&'static str>,
    expected_ok: bool,
}

impl Row {
    fn compiled(
        id: &'static str,
        required: bool,
        state: ProbeState,
        diagnostic: Option<ProbeDiagnostic>,
        expected_ok: bool,
    ) -> Self {
        let expected_diagnostic = diagnostic.as_ref().map(|_| "bounded diagnostic");
        let attempts = Arc::new(AtomicUsize::new(0));
        let probe = CountingProbe {
            attempts: Arc::clone(&attempts),
            outcome: ProbeOutcome::new(state, diagnostic),
        };
        Self {
            id,
            required,
            target: ProbeTarget::compiled(id, "fixture", required, probe),
            attempts,
            expected_attempts: 1,
            expected_state: state,
            expected_diagnostic,
            expected_ok,
        }
    }

    fn unsupported(id: &'static str, required: bool, expected_ok: bool) -> Self {
        Self {
            id,
            required,
            target: ProbeTarget::unsupported(id, "fixture", required),
            attempts: Arc::new(AtomicUsize::new(0)),
            expected_attempts: 0,
            expected_state: ProbeState::Unsupported,
            expected_diagnostic: None,
            expected_ok,
        }
    }
}
