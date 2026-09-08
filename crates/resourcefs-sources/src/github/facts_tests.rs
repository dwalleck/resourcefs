//! Tests the real final serialization/publication boundary without network or public hooks.
use super::*;
use crate::http::HttpSubstrate;
use resourcefs_core::{
    AcquisitionLimitKind, ErrorCategory, ErrorReason, HttpCeilings, OperationGuard,
    OriginAllowlist, PathReference, ReadAcquisitionLimits,
};
use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::time::Duration;

// Hand-authored independently of serde's serializer. The byte cap includes the
// envelope, indentation, multibyte UTF-8, and JSON expansion of control bytes.
const EXPECTED: &str = r#"{
  "schemaVersion": {
    "major": 1,
    "minor": 0
  },
  "kind": "github.pull_request",
  "text": "雪 λ \"quote\" \\ slash\nline\t\u0001"
}"#;

#[derive(Serialize)]
struct LiteralVersion {
    major: u8,
    minor: u8,
}

#[derive(Serialize)]
struct LiteralFacts {
    #[serde(rename = "schemaVersion")]
    schema_version: LiteralVersion,
    kind: &'static str,
    text: &'static str,
}

fn literal_facts() -> LiteralFacts {
    LiteralFacts {
        schema_version: LiteralVersion { major: 1, minor: 0 },
        kind: "github.pull_request",
        text: "雪 λ \"quote\" \\ slash\nline\t\u{0001}",
    }
}

fn no_network_substrate() -> HttpSubstrate {
    HttpSubstrate::new(
        OriginAllowlist::new(Vec::new()),
        HttpCeilings::default(),
        Vec::new(),
    )
    .expect("empty-authority substrate needs no network")
}

fn facts_reference() -> PathReference {
    PathReference::parse("pr://owner/repo/7/facts").expect("facts reference")
}

// The hook runs after the complete inner value has serialized, but while the
// actual finish_facts call is still executing. Moving acceptance before serde
// or dropping it entirely makes the cancelled/expired cases return success.
struct AfterSerialize<'a, F> {
    value: &'a LiteralFacts,
    after: F,
}

impl<F: Fn()> Serialize for AfterSerialize<'_, F> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let result = self.value.serialize(serializer)?;
        (self.after)();
        Ok(result)
    }
}

#[tokio::test]
async fn final_facts_exact_pretty_bytes_and_hash_fit_cap_but_one_less_refuses() {
    let substrate = no_network_substrate();
    let operation = OperationGuard::new();
    let value = literal_facts();
    let read = substrate.begin_read(&operation).expect("logical deadline");
    let exact = finish_facts(&value, EXPECTED.len(), facts_reference(), read)
        .expect("exact complete representation fits");
    assert_eq!(exact.content().as_bytes(), EXPECTED.as_bytes());
    let independent_hash = format!("sha256:{:x}", Sha256::digest(EXPECTED.as_bytes()));
    assert_eq!(exact.version_tag().as_str(), independent_hash);
    assert!(exact.continuation().is_none());

    let read = substrate
        .begin_read(&operation)
        .expect("new logical deadline");
    let error = finish_facts(&value, EXPECTED.len() - 1, facts_reference(), read)
        .expect_err("one byte less cannot publish truncated JSON");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    let details = error.details().expect("bounded representation details");
    assert_eq!(details.reason(), ErrorReason::LimitExceeded);
    let limit = details.limit().expect("representation dimension");
    assert_eq!(limit.kind(), AcquisitionLimitKind::RepresentationBytes);
    assert_eq!(limit.bound(), (EXPECTED.len() - 1) as u64);
}

#[tokio::test]
async fn final_facts_cancellation_after_serialization_prevents_publication() {
    let substrate = no_network_substrate();
    let operation = OperationGuard::new();
    let value = literal_facts();
    let control = AfterSerialize {
        value: &value,
        after: || {},
    };
    let read = substrate.begin_read(&operation).expect("control deadline");
    let completed = finish_facts(&control, EXPECTED.len(), facts_reference(), read)
        .expect("same bytes without cancellation succeed");
    assert_eq!(completed.content(), EXPECTED);

    let cancelled = AfterSerialize {
        value: &value,
        after: || {
            assert!(operation.cancel(), "hook cancels the active operation");
        },
    };
    let read = substrate
        .begin_read(&operation)
        .expect("cancelled-case deadline");
    let error = finish_facts(&cancelled, EXPECTED.len(), facts_reference(), read)
        .expect_err("cancellation after serde cannot publish a late resource");
    assert_eq!(error.category(), ErrorCategory::Cancelled);
    assert_eq!(
        error.details().expect("cancellation reason").reason(),
        ErrorReason::Cancelled
    );
    assert_eq!(
        completed.content(),
        EXPECTED,
        "earlier independent result stays usable"
    );
}

#[tokio::test(start_paused = true)]
async fn final_facts_deadline_advanced_after_serialization_prevents_publication() {
    let substrate = no_network_substrate();
    let operation = OperationGuard::new();
    let controls = ReadAcquisitionLimits::new(None, Some(Duration::from_secs(1)), None, None, None)
        .expect("one-second logical deadline");
    let value = literal_facts();
    let (read, _) = substrate
        .begin_read_with_limits(&operation, &controls)
        .expect("control deadline");
    let completed = finish_facts(&value, EXPECTED.len(), facts_reference(), read)
        .expect("same bytes before the deadline succeed");
    assert_eq!(completed.content(), EXPECTED);

    let runtime = tokio::runtime::Handle::current();
    let expired = AfterSerialize {
        value: &value,
        after: || {
            // A scoped thread drives the existing runtime's paused time future;
            // block_on is never nested on its currently executing runtime thread.
            // No sleep, secondary runtime, replacement clock, or spin loop.
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        runtime.block_on(async {
                            tokio::time::advance(Duration::from_secs(2)).await;
                        })
                    })
                    .join()
                    .expect("paused-clock worker");
            });
        },
    };
    let (read, _) = substrate
        .begin_read_with_limits(&operation, &controls)
        .expect("expiry deadline");
    let before = tokio::time::Instant::now();
    let error = finish_facts(&expired, EXPECTED.len(), facts_reference(), read)
        .expect_err("deadline crossed after serde cannot publish a late resource");
    assert_eq!(tokio::time::Instant::now() - before, Duration::from_secs(2));
    assert!(
        operation.is_active(),
        "deadline refusal is not cancellation"
    );
    assert_eq!(
        error.details().expect("deadline reason").reason(),
        ErrorReason::DeadlineExceeded
    );
    assert_eq!(
        completed.content(),
        EXPECTED,
        "previous result is independent"
    );
}
