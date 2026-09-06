use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use resourcefs_core::{
    ArtifactAddress, ArtifactId, DisplayedLineRange, ErrorCategory, MAX_ARTIFACT_BYTES,
    MAX_SESSION_ARTIFACTS, MAX_SESSION_BYTES, OperationGuard, PathReference, PathSession,
    ResourceError, ServerLimits, ServerLimitsInput, SessionCacheEntry, SessionCacheKey,
    SessionStorage, SessionToken, StorageLimitInput, VersionTag,
};
use sha2::{Digest, Sha256};
use tokio::sync::{Barrier, Mutex, Notify};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Compare(ArtifactId, usize),
    Write(ArtifactId, usize),
    Read(ArtifactId),
    Remove(ArtifactId),
    Disconnect,
}

#[derive(Clone)]
struct StoredContent {
    bytes: Option<Vec<u8>>,
    digest: [u8; 32],
    len: usize,
}

#[derive(Default)]
struct WriteGate {
    entered: Notify,
    release: Notify,
}

#[derive(Default)]
struct FakeStorage {
    content: Mutex<HashMap<ArtifactId, StoredContent>>,
    calls: Mutex<Vec<Call>>,
    fail_write: AtomicBool,
    inflight_writes: AtomicUsize,
    maximum_inflight_writes: AtomicUsize,
    write_gate: Mutex<Option<Arc<WriteGate>>>,
    fast_writes: AtomicBool,
}

impl FakeStorage {
    async fn calls(&self) -> Vec<Call> {
        self.calls.lock().await.clone()
    }

    async fn arm_write_gate(&self) -> Arc<WriteGate> {
        let gate = Arc::new(WriteGate::default());
        *self.write_gate.lock().await = Some(Arc::clone(&gate));
        gate
    }

    fn fail_next_write(&self) {
        self.fail_write.store(true, Ordering::Release);
    }

    fn skip_write_delays(&self) {
        self.fast_writes.store(true, Ordering::Release);
    }

    fn observe_write_entry(&self) -> WriteEntry<'_> {
        let current = self.inflight_writes.fetch_add(1, Ordering::AcqRel) + 1;
        self.maximum_inflight_writes
            .fetch_max(current, Ordering::AcqRel);
        WriteEntry { storage: self }
    }
}

struct WriteEntry<'a> {
    storage: &'a FakeStorage,
}

impl Drop for WriteEntry<'_> {
    fn drop(&mut self) {
        self.storage.inflight_writes.fetch_sub(1, Ordering::AcqRel);
    }
}

#[async_trait]
impl SessionStorage for FakeStorage {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError> {
        self.calls
            .lock()
            .await
            .push(Call::Compare(id, content.len()));
        let stored = self
            .content
            .lock()
            .await
            .get(&id)
            .cloned()
            .expect("published fake object");
        let digest: [u8; 32] = Sha256::digest(content).into();
        Ok(stored.len == content.len()
            && stored.digest == digest
            && stored
                .bytes
                .as_ref()
                .is_none_or(|bytes| bytes.as_slice() == content))
    }

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError> {
        let _entry = self.observe_write_entry();
        self.calls.lock().await.push(Call::Write(id, content.len()));
        if self.fail_write.swap(false, Ordering::AcqRel) {
            return Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "injected write failure",
            ));
        }
        if let Some(gate) = self.write_gate.lock().await.take() {
            gate.entered.notify_one();
            gate.release.notified().await;
        } else if !self.fast_writes.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        self.content.lock().await.insert(
            id,
            StoredContent {
                bytes: (content.len() <= 1024 * 1024).then(|| content.to_vec()),
                digest: Sha256::digest(content).into(),
                len: content.len(),
            },
        );
        Ok(())
    }

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError> {
        self.calls.lock().await.push(Call::Read(id));
        let content = self.content.lock().await;
        let stored = content.get(&id).expect("published fake object");
        let bytes = stored
            .bytes
            .as_ref()
            .expect("large stress objects are not read");
        String::from_utf8(bytes.clone())
            .map_err(|_| ResourceError::new(ErrorCategory::SourceUnavailable, "invalid fake UTF-8"))
    }

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError> {
        self.calls.lock().await.push(Call::Remove(id));
        self.content.lock().await.remove(&id);
        Ok(())
    }

    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        self.calls.lock().await.push(Call::Disconnect);
        Ok(())
    }
}

fn token(value: u8) -> SessionToken {
    SessionToken::parse(format!("{value:032x}")).expect("fixture token")
}

fn session(value: u8, storage: Arc<FakeStorage>) -> PathSession {
    session_with_limits(value, storage, ServerLimits::default())
}

fn session_with_limits(value: u8, storage: Arc<FakeStorage>, limits: ServerLimits) -> PathSession {
    let trait_storage: Arc<dyn SessionStorage> = storage;
    PathSession::new(token(value), trait_storage, limits)
}

#[tokio::test]
async fn seen_regions_union_only_within_one_tag_and_session() {
    let first = session(30, Arc::new(FakeStorage::default()));
    let second = session(31, Arc::new(FakeStorage::default()));
    let original = VersionTag::from_content(b"one\ntwo\nthree\n");
    let changed = VersionTag::from_content(b"one\nTWO\nthree\n");
    let first_two = DisplayedLineRange::new(1, 2).expect("first range");
    let last_two = DisplayedLineRange::new(2, 3).expect("second range");

    first
        .record_seen_for_test(
            "rfs://workspace/workspace/fixture.txt",
            &original,
            &[first_two],
            false,
        )
        .await
        .expect("record first region");
    first
        .record_seen_for_test(
            "rfs://workspace/workspace/fixture.txt",
            &original,
            &[last_two],
            true,
        )
        .await
        .expect("union second region");
    first
        .record_seen_for_test(
            "rfs://workspace/workspace/fixture.txt",
            &changed,
            &[last_two],
            false,
        )
        .await
        .expect("record changed tag");

    assert_eq!(
        first
            .seen_snapshot_for_test("rfs://workspace/workspace/fixture.txt", &original)
            .await
            .expect("canonical snapshot key"),
        Some((
            vec![DisplayedLineRange::new(1, 3).expect("merged range")],
            true
        ))
    );
    assert_eq!(
        first
            .seen_snapshot_for_test("rfs://workspace/workspace/fixture.txt", &changed)
            .await
            .expect("canonical snapshot key"),
        Some((vec![last_two], false))
    );
    assert_eq!(
        second
            .seen_snapshot_for_test("rfs://workspace/workspace/fixture.txt", &original)
            .await
            .expect("canonical snapshot key"),
        None
    );
    let invalid = first
        .record_seen_for_test("relative.txt", &original, &[first_two], false)
        .await
        .expect_err("relative snapshot identity");
    assert_eq!(invalid.category(), ErrorCategory::InvalidReference);
}

#[tokio::test]
async fn configured_object_and_session_quotas_are_enforced_atomically() {
    let limits = ServerLimits::new(ServerLimitsInput {
        storage: StorageLimitInput {
            object_bytes: Some(4),
            session_bytes: Some(6),
        },
        ..ServerLimitsInput::default()
    })
    .expect("lower storage quotas");
    let storage = Arc::new(FakeStorage::default());
    storage.skip_write_delays();
    let session = session_with_limits(19, Arc::clone(&storage), limits);
    let operation = OperationGuard::new();

    session
        .retain("1234", &operation)
        .await
        .expect("exact object quota");
    let object_error = session
        .retain("12345", &operation)
        .await
        .expect_err("one over object quota");
    assert_eq!(object_error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(session.used_bytes().await, 4);

    session
        .retain("xy", &operation)
        .await
        .expect("exact session quota");
    let session_error = session
        .retain("z", &operation)
        .await
        .expect_err("one over session quota");
    assert_eq!(session_error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(session.used_bytes().await, 6);
    assert_eq!(session.artifact_count().await, 2);
    assert_eq!(
        storage
            .calls()
            .await
            .into_iter()
            .filter(|call| matches!(call, Call::Write(_, _)))
            .count(),
        2,
        "rejected quota rows must not write"
    );
}

#[test]
fn session_tokens_and_artifact_ids_are_strict() {
    let first = SessionToken::generate().expect("random token");
    let second = SessionToken::generate().expect("random token");
    assert_eq!(first.as_str().len(), 32);
    assert!(first.as_str().bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(first, second);
    assert_eq!(
        SessionToken::parse("ABCDEF0123456789abcdef0123456789")
            .expect_err("uppercase token is noncanonical")
            .category(),
        ErrorCategory::InvalidReference
    );
    assert_eq!(
        ArtifactId::new(0).expect_err("zero object ID").category(),
        ErrorCategory::InvalidReference
    );
    assert_eq!(ArtifactId::new(7).expect("object ID").get(), 7);
}

#[tokio::test]
async fn byte_identical_content_reuses_identity_and_collision_checks_bytes() {
    let storage = Arc::new(FakeStorage::default());
    let session = session(1, Arc::clone(&storage));
    let operation = OperationGuard::new();
    let digest = [7_u8; 32];

    let alpha = session
        .retain_with_digest_for_test("alpha", digest, &operation)
        .await
        .expect("first artifact");
    let beta = session
        .retain_with_digest_for_test("beta", digest, &operation)
        .await
        .expect("digest collision remains distinct");
    let alpha_retry = session
        .retain_with_digest_for_test("alpha", digest, &operation)
        .await
        .expect("identical retry");

    assert_ne!(alpha, beta);
    assert_eq!(alpha, alpha_retry);
    assert_eq!(session.used_bytes().await, "alpha".len() + "beta".len());
    assert_eq!(session.artifact_count().await, 2);
    assert_eq!(
        storage
            .calls()
            .await
            .iter()
            .filter(|call| matches!(call, Call::Write(_, _)))
            .count(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_admission_is_linearizable_at_exact_session_quota() {
    let storage = Arc::new(FakeStorage::default());
    let session = session(2, Arc::clone(&storage));
    let barrier = Arc::new(Barrier::new(5));
    let mut tasks = Vec::new();
    for marker in *b"abcd" {
        let mut bytes = vec![marker; MAX_ARTIFACT_BYTES];
        bytes[MAX_ARTIFACT_BYTES - 1] = b'\n';
        let content: Arc<str> = String::from_utf8(bytes)
            .expect("UTF-8 stress fixture")
            .into();
        let task_session = session.clone();
        let task_barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            task_barrier.wait().await;
            task_session.retain(&content, &OperationGuard::new()).await
        }));
    }
    barrier.wait().await;

    let mut object_ids = Vec::new();
    for task in tasks {
        object_ids.push(
            task.await
                .expect("admission task")
                .expect("exact quota")
                .object_id(),
        );
    }
    object_ids.sort_unstable();
    assert_eq!(object_ids, vec![1, 2, 3, 4]);
    assert_eq!(session.used_bytes().await, MAX_SESSION_BYTES);
    assert_eq!(session.artifact_count().await, 4);
    assert_eq!(
        storage.maximum_inflight_writes.load(Ordering::Acquire),
        1,
        "the admission mutex must span durable writes"
    );

    let calls_before = storage.calls().await.len();
    let error = session
        .retain("x", &OperationGuard::new())
        .await
        .expect_err("one byte over session quota");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(storage.calls().await.len(), calls_before);
}

#[tokio::test]
async fn object_and_record_ceilings_fail_before_storage_io() {
    let storage = Arc::new(FakeStorage::default());
    let session = session(3, Arc::clone(&storage));
    let too_large = "x".repeat(MAX_ARTIFACT_BYTES + 1);
    let error = session
        .retain(&too_large, &OperationGuard::new())
        .await
        .expect_err("one byte over object ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert!(storage.calls().await.is_empty());

    for index in 0..MAX_SESSION_ARTIFACTS {
        session
            .retain(&format!("artifact-{index}"), &OperationGuard::new())
            .await
            .expect("record within ceiling");
    }
    let calls_before = storage.calls().await.len();
    let error = session
        .retain("artifact-over", &OperationGuard::new())
        .await
        .expect_err("one record over ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(storage.calls().await.len(), calls_before);
}

#[tokio::test]
async fn storage_failure_publishes_nothing() {
    let storage = Arc::new(FakeStorage::default());
    let session = session(4, Arc::clone(&storage));
    storage.fail_next_write();
    let error = session
        .retain("failed", &OperationGuard::new())
        .await
        .expect_err("write failure");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert_eq!(session.used_bytes().await, 0);
    assert_eq!(session.artifact_count().await, 0);
    assert!(
        !storage
            .calls()
            .await
            .contains(&Call::Remove(ArtifactId::new(1).expect("ID"))),
        "an unpublished object must not be cleaned up"
    );
}

#[tokio::test]
async fn operation_cancellation_wait_is_lossless() {
    let cancelled_first = OperationGuard::new();
    cancelled_first.cancel();
    tokio::time::timeout(Duration::from_millis(100), cancelled_first.cancelled())
        .await
        .expect("a prior cancellation must remain observable");

    let cancelled_later = OperationGuard::new();
    let waiter_guard = cancelled_later.clone();
    let waiter = tokio::spawn(async move {
        waiter_guard.cancelled().await;
    });
    tokio::task::yield_now().await;
    cancelled_later.cancel();
    tokio::time::timeout(Duration::from_millis(100), waiter)
        .await
        .expect("a waiting operation must be notified")
        .expect("cancellation waiter");
}

#[tokio::test]
async fn request_cancel_prevents_commit() {
    let storage = Arc::new(FakeStorage::default());
    let session = session(5, Arc::clone(&storage));
    let gate = storage.arm_write_gate().await;
    let operation = OperationGuard::new();
    let pending_session = session.clone();
    let pending_operation = operation.clone();
    let pending = tokio::spawn(async move {
        pending_session
            .retain("cancelled", &pending_operation)
            .await
    });

    // Write: the commit reaches durable storage and parks on the gate.
    gate.entered.notified().await;
    let id = ArtifactId::new(1).expect("first artifact ID");
    assert_eq!(
        storage.calls().await,
        vec![Call::Write(id, "cancelled".len())],
        "the commit must write exactly once before the gate"
    );

    // Cancel: the operation is invalidated before the gated write is released.
    operation.cancel();
    assert!(!operation.is_active());

    // Release: the storage write completes, then the commit fence removes the object.
    gate.release.notify_one();
    let error = pending
        .await
        .expect("retain task")
        .expect_err("cancelled commit");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert_eq!(
        storage.calls().await,
        vec![Call::Write(id, "cancelled".len()), Call::Remove(id)],
        "the event order must be Write before Remove with no other storage calls"
    );
    assert_eq!(session.used_bytes().await, 0, "no quota may be charged");
    assert_eq!(
        session.artifact_count().await,
        0,
        "no record may be published"
    );
}

#[tokio::test]
async fn foreign_unknown_and_inactive_artifacts_are_indistinguishable() {
    let first_storage = Arc::new(FakeStorage::default());
    let second_storage = Arc::new(FakeStorage::default());
    let first = session(5, first_storage);
    let second = session(6, Arc::clone(&second_storage));
    let address = first
        .retain("secret", &OperationGuard::new())
        .await
        .expect("first artifact");
    let second_address = second
        .retain("other", &OperationGuard::new())
        .await
        .expect("second artifact");
    assert_eq!(address.object_id(), second_address.object_id());

    let foreign = second
        .read_artifact(&address)
        .await
        .expect_err("foreign artifact");
    let unknown_address = ArtifactAddress::new(second.token().as_str(), 999).expect("unknown ID");
    let unknown = second
        .read_artifact(&unknown_address)
        .await
        .expect_err("unknown artifact");
    second.invalidate();
    let inactive = second
        .read_artifact(&second_address)
        .await
        .expect_err("inactive artifact");

    for error in [&foreign, &unknown, &inactive] {
        assert_eq!(error.category(), ErrorCategory::NotFound);
        assert_eq!(error.message(), foreign.message());
    }
    second
        .mark_disconnected()
        .await
        .expect("disconnect marker remains idempotent after invalidation");
    assert!(second_storage.calls().await.contains(&Call::Disconnect));
}

#[tokio::test]
async fn artifact_catalog_is_ordered_and_live() {
    let storage = Arc::new(FakeStorage::default());
    storage.skip_write_delays();
    let path_session = session(7, Arc::clone(&storage));
    let operation = OperationGuard::new();

    let mut first_content = String::new();
    for index in 1..MAX_SESSION_ARTIFACTS {
        let content = format!("artifact-{:04}", MAX_SESSION_ARTIFACTS - index);
        if index == 1 {
            first_content.clone_from(&content);
        }
        path_session
            .retain(&content, &operation)
            .await
            .expect("C15 fixture artifact");
    }
    let saved = path_session
        .artifact_catalog()
        .await
        .expect("C15 live catalog");
    assert_eq!(saved.len(), MAX_SESSION_ARTIFACTS - 1, "C15 saved count");

    let duplicate = path_session
        .retain(&first_content, &operation)
        .await
        .expect("C15 duplicate artifact");
    assert_eq!(duplicate.object_id(), 1, "C15 duplicate identity");
    let last = path_session
        .retain("artifact-final", &operation)
        .await
        .expect("C15 final artifact");
    assert_eq!(
        saved.last().map(ArtifactAddress::object_id),
        Some(last.object_id() - 1),
        "C15 saved snapshot must exclude post-snapshot retention"
    );

    let started = Instant::now();
    let catalog = path_session
        .artifact_catalog()
        .await
        .expect("C15 ceiling catalog");
    let elapsed = started.elapsed();
    assert_eq!(catalog.len(), MAX_SESSION_ARTIFACTS, "C15 ceiling count");
    assert!(
        catalog
            .windows(2)
            .all(|pair| pair[0].object_id() < pair[1].object_id()),
        "C15 catalog must be object-ID ordered"
    );
    assert!(
        catalog
            .iter()
            .all(|address| address.session_token() == path_session.token().as_str()),
        "C15 catalog must contain only the active session"
    );
    let canonical_payload_bytes = catalog
        .iter()
        .map(|address| {
            PathReference::artifact(address.clone(), None)
                .expect("C15 canonical address")
                .requested()
                .len()
        })
        .sum::<usize>();
    assert!(
        canonical_payload_bytes <= 256 * 1024,
        "C15 canonical address payload exceeds 256 KiB"
    );
    assert!(
        elapsed <= Duration::from_millis(250),
        "C15 catalog exceeded 250 ms: {elapsed:?}"
    );

    let foreign_storage = Arc::new(FakeStorage::default());
    foreign_storage.skip_write_delays();
    let foreign = session(8, foreign_storage);
    foreign
        .retain("foreign", &OperationGuard::new())
        .await
        .expect("C15 foreign fixture artifact");
    assert!(
        catalog
            .iter()
            .all(|address| address.session_token() != foreign.token().as_str()),
        "C15 catalog leaked another Path Session"
    );

    path_session.invalidate();
    let error = path_session
        .artifact_catalog()
        .await
        .expect_err("C15 inactive catalog");
    assert_eq!(
        error.category(),
        ErrorCategory::SourceUnavailable,
        "C15 inactive category"
    );
}

/// C7 (rfs-60g1) — the seen-region snapshot key admits canonical
/// `local://` Session Scratch references alongside canonical workspace
/// references, and still refuses every non-canonical spelling and every other
/// address family.
///
/// The expectation table below is authored from the approved spec, never
/// computed from `SnapshotResourceKey::parse`: each row states the reference
/// and whether a snapshot key may be formed from it, so the oracle fails
/// independently of the parser's branch structure.
#[tokio::test]
async fn snapshot_key_accepts_local_rejects_foreign() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum KeyExpectation {
        Accepted,
        Rejected,
    }

    let artifact_reference = {
        let session = session(70, Arc::new(FakeStorage::default()));
        let guard = OperationGuard::new();
        let address = session
            .retain("artifact fixture\n", &guard)
            .await
            .expect("fixture artifact");
        PathReference::artifact(address, None)
            .expect("fixture artifact reference")
            .requested()
            .to_owned()
    };

    // Reference -> may a snapshot key be formed? Authored from the spec.
    let expected: Vec<(String, KeyExpectation)> = vec![
        // Canonical workspace identity: the pre-existing accepted family.
        ("rfs://workspace/r/p".to_owned(), KeyExpectation::Accepted),
        // Canonical Session Scratch identity: admitted by C7.
        ("local://plan.md".to_owned(), KeyExpectation::Accepted),
        // A scratch name whose canonical spelling is itself: still accepted.
        (
            "local://review notes.md".to_owned(),
            KeyExpectation::Accepted,
        ),
        // Non-canonical workspace spelling: a relative path is not an identity.
        ("plan.md".to_owned(), KeyExpectation::Rejected),
        // Immutable recovery storage is not a snapshot identity.
        (artifact_reference, KeyExpectation::Rejected),
        // Catalog roots are synthetic and hold no seen regions.
        ("rfs://".to_owned(), KeyExpectation::Rejected),
        ("rfs://workspace".to_owned(), KeyExpectation::Rejected),
        // A scratch name carrying a decoded separator never parses at all.
        ("local://a%2Fb".to_owned(), KeyExpectation::Rejected),
        // A non-canonical scratch spelling: the colon re-encodes, so the
        // requested form differs from its canonical rendering.
        ("local://plan.md:1-5".to_owned(), KeyExpectation::Rejected),
    ];

    let session = session(71, Arc::new(FakeStorage::default()));
    let version_tag = VersionTag::from_content(b"one\ntwo\n");
    let ranges = [DisplayedLineRange::new(1, 2).expect("fixture range")];

    for (reference, expectation) in expected {
        let outcome = session
            .record_seen_for_test(&reference, &version_tag, &ranges, true)
            .await;

        match expectation {
            KeyExpectation::Accepted => {
                outcome.unwrap_or_else(|error| {
                    panic!("C7 expected {reference} to form a snapshot key, got {error}")
                });
                let stored = session
                    .seen_snapshot_for_test(&reference, &version_tag)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("C7 expected {reference} to resolve its snapshot, got {error}")
                    });
                assert!(
                    stored.is_some(),
                    "C7 expected {reference} to retain its recorded seen region"
                );
            }
            KeyExpectation::Rejected => {
                let error = outcome.expect_err(&format!(
                    "C7 expected {reference} to be refused as a snapshot identity"
                ));
                assert_eq!(
                    error.category(),
                    ErrorCategory::InvalidReference,
                    "C7 rejection category for {reference}"
                );
            }
        }
    }
}

#[tokio::test]
async fn opaque_cache_replacement_is_quota_charged_and_zero_copy() {
    let storage = Arc::new(FakeStorage::default());
    let session = session(91, storage);
    let key = SessionCacheKey::new("github", "issue://owner/repo/42").expect("cache key");
    let first =
        SessionCacheEntry::new(b"etag-1".to_vec(), b"first body".to_vec()).expect("cache entry");
    let expected_first_bytes = key.retained_bytes() + first.retained_bytes();
    session
        .cache_put(key.clone(), first.clone())
        .await
        .expect("cache put");
    assert_eq!(session.used_bytes().await, expected_first_bytes);
    let loaded = session
        .cache_get(&key)
        .await
        .expect("cache get")
        .expect("cache hit");
    assert!(Arc::ptr_eq(loaded.content_arc(), first.content_arc()));
    assert_eq!(loaded.metadata(), b"etag-1");
    assert_eq!(loaded.content(), b"first body");

    let second =
        SessionCacheEntry::new(b"etag-2".to_vec(), b"x".to_vec()).expect("replacement entry");
    let expected_second_bytes = key.retained_bytes() + second.retained_bytes();
    session
        .cache_put(key.clone(), second.clone())
        .await
        .expect("cache replacement");
    assert_eq!(session.used_bytes().await, expected_second_bytes);
    assert_eq!(
        session
            .cache_get(&key)
            .await
            .expect("cache get")
            .expect("cache hit")
            .metadata(),
        b"etag-2"
    );

    assert!(session.cache_remove(&key).await.expect("cache remove"));
    assert_eq!(session.used_bytes().await, 0);
    assert!(!session.cache_remove(&key).await.expect("missing remove"));
}

/// Cache entries are the only reconstructible object class in a session, so
/// they yield — least recently used first — before an artifact or a newer
/// cache entry is refused for room the cache holds. A session that has cached
/// a thousand upstream responses must never wedge Recovery References.
#[tokio::test]
async fn opaque_cache_yields_to_authoritative_objects_least_recently_used_first() {
    let storage = Arc::new(FakeStorage::default());
    let session = session(94, storage);
    let entry = || SessionCacheEntry::new(Vec::new(), b"c".to_vec()).expect("cache entry");
    let keys = (0..MAX_SESSION_ARTIFACTS)
        .map(|index| SessionCacheKey::new("github", format!("k{index}")).expect("cache key"))
        .collect::<Vec<_>>();
    for key in &keys {
        session
            .cache_put(key.clone(), entry())
            .await
            .expect("cache put under the object ceiling");
    }
    // Touching k0 makes k1 the least recently used entry.
    session
        .cache_get(&keys[0])
        .await
        .expect("cache get")
        .expect("k0 cached");

    let extra = SessionCacheKey::new("github", "extra").expect("extra key");
    session
        .cache_put(extra.clone(), entry())
        .await
        .expect("a new cache entry evicts the least recently used one");
    assert!(
        session.cache_get(&keys[1]).await.expect("get").is_none(),
        "k1 was least recently used and yielded"
    );
    assert!(
        session.cache_get(&keys[0]).await.expect("get").is_some(),
        "a recently used entry survives"
    );
    assert!(session.cache_get(&extra).await.expect("get").is_some());

    session
        .retain("recovery bytes", &OperationGuard::new())
        .await
        .expect("an artifact evicts a cache entry rather than failing");
    assert!(
        session.cache_get(&keys[2]).await.expect("get").is_none(),
        "the next least recently used entry yielded to the artifact"
    );

    // Bytes yield the same way: a small session whose quota is held by cache
    // entries admits an artifact by releasing them.
    let limits = ServerLimits::new(ServerLimitsInput {
        storage: StorageLimitInput {
            object_bytes: Some(48),
            session_bytes: Some(64),
        },
        ..ServerLimitsInput::default()
    })
    .expect("limits");
    let small = session_with_limits(95, Arc::new(FakeStorage::default()), limits);
    let cached = SessionCacheKey::new("github", "a").expect("cache key");
    small
        .cache_put(
            cached.clone(),
            SessionCacheEntry::new(vec![b'm'; 8], vec![b'c'; 24]).expect("entry"),
        )
        .await
        .expect("cache within quota");
    small
        .retain(&"x".repeat(40), &OperationGuard::new())
        .await
        .expect("the artifact fits once the cache yields its bytes");
    assert!(small.cache_get(&cached).await.expect("get").is_none());
    assert_eq!(small.used_bytes().await, 40);
    // With only authoritative bytes left, a cache put that cannot fit is
    // refused rather than evicting something it must not.
    let refused = small
        .cache_put(
            cached,
            SessionCacheEntry::new(Vec::new(), vec![b'c'; 40]).expect("entry"),
        )
        .await
        .expect_err("authoritative bytes hold the quota");
    assert_eq!(refused.category(), ErrorCategory::LimitExceeded);
    assert_eq!(small.used_bytes().await, 40);
}

#[tokio::test]
async fn opaque_cache_obeys_session_limits_and_disconnect_isolation() {
    let limits = ServerLimits::new(ServerLimitsInput {
        storage: StorageLimitInput {
            object_bytes: Some(32),
            session_bytes: Some(48),
        },
        ..ServerLimitsInput::default()
    })
    .expect("limits");
    let first_session = session_with_limits(92, Arc::new(FakeStorage::default()), limits);
    let second_session = session_with_limits(93, Arc::new(FakeStorage::default()), limits);
    let key = SessionCacheKey::new("github", "k").expect("cache key");
    let entry = SessionCacheEntry::new(vec![b'm'; 8], vec![b'c'; 8]).expect("entry");
    first_session
        .cache_put(key.clone(), entry)
        .await
        .expect("within cache quota");
    assert!(
        second_session
            .cache_get(&key)
            .await
            .expect("second session")
            .is_none()
    );
    let over_object = SessionCacheEntry::new(Vec::new(), vec![b'x'; 33]).expect("large entry");
    assert_eq!(
        first_session
            .cache_put(
                SessionCacheKey::new("github", "large").expect("large key"),
                over_object,
            )
            .await
            .expect_err("object ceiling")
            .category(),
        ErrorCategory::LimitExceeded
    );
    first_session.invalidate();
    assert_eq!(
        first_session
            .cache_get(&key)
            .await
            .expect_err("inactive cache")
            .category(),
        ErrorCategory::SourceUnavailable
    );
}
