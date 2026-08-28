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
    ArtifactId, DisplayedLineRange, ErrorCategory, MAX_HASHLINE_PATCH_BYTES, MutationAccess,
    MutationAdapter, MutationCommitFailure, MutationCommitOutcome, MutationEngine,
    MutationOperation, MutationOperationOutcome, MutationOperationStart, MutationResourceKey,
    MutationSourceKey, MutationState, MutationTarget, MutationTargetMode, OperationGuard,
    OperationId, PathReference, PathSession, ResourceError, ServerLimits, ServerLimitsInput,
    SessionCacheEntry, SessionCacheKey, SessionStorage, SessionToken, SourceMutation,
    StorageLimitInput, VersionSelector, VersionTag, WriteRequest,
};
use tokio::sync::Mutex;

#[derive(Default)]
struct MemoryStorage {
    content: Mutex<HashMap<ArtifactId, Vec<u8>>>,
}

#[async_trait]
impl SessionStorage for MemoryStorage {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError> {
        Ok(self
            .content
            .lock()
            .await
            .get(&id)
            .is_some_and(|stored| stored == content))
    }

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError> {
        self.content.lock().await.insert(id, content.to_vec());
        Ok(())
    }

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError> {
        let bytes = self
            .content
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(|| ResourceError::new(ErrorCategory::NotFound, "missing"))?;
        String::from_utf8(bytes)
            .map_err(|_| ResourceError::new(ErrorCategory::SourceUnavailable, "invalid UTF-8"))
    }

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError> {
        self.content.lock().await.remove(&id);
        Ok(())
    }

    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}

#[derive(Debug)]
struct FakeAdapter {
    resolve_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl MutationAdapter for FakeAdapter {
    async fn resolve(
        &self,
        reference: &PathReference,
        _access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError> {
        self.resolve_calls.fetch_add(1, Ordering::Relaxed);
        MutationTarget::new(
            reference.clone(),
            MutationSourceKey::new("fake").expect("source key"),
            MutationTargetMode::AuthoredText,
        )
    }

    fn validate_write(
        &self,
        _target: &MutationTarget,
        _content: &str,
    ) -> Result<(), ResourceError> {
        Ok(())
    }

    async fn load(
        &self,
        _target: &MutationTarget,
        _access: MutationAccess,
        _operation: &OperationGuard,
    ) -> Result<MutationState, ResourceError> {
        Ok(MutationState::Missing)
    }

    async fn commit(
        &self,
        _mutation: SourceMutation,
        _operation: &OperationGuard,
    ) -> Result<MutationCommitOutcome, MutationCommitFailure> {
        Err(ResourceError::new(
            ErrorCategory::UnsupportedMutation,
            "fake adapter does not commit",
        )
        .into())
    }
}

struct StatefulAdapter {
    state: Mutex<MutationState>,
    fail_commit: AtomicBool,
}

impl StatefulAdapter {
    fn new(state: MutationState) -> Self {
        Self {
            state: Mutex::new(state),
            fail_commit: AtomicBool::new(false),
        }
    }

    fn fail_next_commit(&self) {
        self.fail_commit.store(true, Ordering::Release);
    }

    async fn text_content(&self) -> String {
        let state = self.state.lock().await;
        let MutationState::Text { content, .. } = &*state else {
            panic!("expected text state");
        };
        content.clone()
    }
}

#[async_trait]
impl MutationAdapter for StatefulAdapter {
    async fn resolve(
        &self,
        reference: &PathReference,
        _access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError> {
        MutationTarget::new(
            reference.clone(),
            MutationSourceKey::new("stateful").expect("source key"),
            MutationTargetMode::AuthoredText,
        )
    }

    fn validate_write(
        &self,
        _target: &MutationTarget,
        _content: &str,
    ) -> Result<(), ResourceError> {
        Ok(())
    }

    async fn load(
        &self,
        _target: &MutationTarget,
        _access: MutationAccess,
        _operation: &OperationGuard,
    ) -> Result<MutationState, ResourceError> {
        Ok(self.state.lock().await.clone())
    }

    async fn commit(
        &self,
        mutation: SourceMutation,
        _operation: &OperationGuard,
    ) -> Result<MutationCommitOutcome, MutationCommitFailure> {
        if self.fail_commit.swap(false, Ordering::AcqRel) {
            return Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "injected commit failure",
            )
            .into());
        }
        let mut state = self.state.lock().await;
        match mutation {
            SourceMutation::Create { content, .. } => {
                if !matches!(*state, MutationState::Missing) {
                    return Err(ResourceError::new(
                        ErrorCategory::VersionConflict,
                        "destination exists",
                    )
                    .into());
                }
                *state = MutationState::Text {
                    version_tag: VersionTag::from_content(content.as_bytes()),
                    content: content.to_string(),
                };
            }
            SourceMutation::Replace {
                expected, content, ..
            } => {
                let MutationState::Text { version_tag, .. } = &*state else {
                    return Err(ResourceError::new(
                        ErrorCategory::VersionConflict,
                        "destination is missing",
                    )
                    .into());
                };
                if version_tag != &expected {
                    return Err(ResourceError::new(
                        ErrorCategory::VersionConflict,
                        "stale destination",
                    )
                    .into());
                }
                *state = MutationState::Text {
                    version_tag: VersionTag::from_content(content.as_bytes()),
                    content: content.to_string(),
                };
            }
            SourceMutation::Delete { expected, .. } => {
                let MutationState::Text { version_tag, .. } = &*state else {
                    return Err(ResourceError::new(
                        ErrorCategory::VersionConflict,
                        "destination is missing",
                    )
                    .into());
                };
                if version_tag != &expected {
                    return Err(ResourceError::new(
                        ErrorCategory::VersionConflict,
                        "stale destination",
                    )
                    .into());
                }
                *state = MutationState::Missing;
            }
            SourceMutation::Move { .. } => {
                return Err(ResourceError::new(
                    ErrorCategory::UnsupportedMutation,
                    "unsupported fake mutation",
                )
                .into());
            }
        }
        Ok(MutationCommitOutcome::AuthoredText)
    }
}

fn lock_key(value: &str) -> MutationResourceKey {
    MutationResourceKey::new(value).expect("test mutation Resource key")
}
fn session(value: u8) -> PathSession {
    PathSession::new(
        SessionToken::parse(format!("{value:032x}")).expect("session token"),
        Arc::new(MemoryStorage::default()),
        ServerLimits::default(),
    )
}

fn engine() -> MutationEngine {
    MutationEngine::new(
        Arc::new(FakeAdapter {
            resolve_calls: Arc::new(AtomicUsize::new(0)),
        }),
        session(1),
    )
}

#[test]
fn operation_id_validation_matrix() {
    for valid in [
        "a",
        "A-Z_09.:",
        "550e8400-e29b-41d4-a716-446655440000",
        &"x".repeat(128),
    ] {
        assert_eq!(
            OperationId::parse(valid)
                .expect("[C3] valid operation ID")
                .as_str(),
            valid,
            "[C3] {valid}"
        );
    }
    for invalid in [
        "",
        "has space",
        "line\nbreak",
        "slash/value",
        "unicode-é",
        &"x".repeat(129),
    ] {
        assert_eq!(
            OperationId::parse(invalid)
                .expect_err("[C3] invalid operation ID")
                .category(),
            ErrorCategory::InvalidReference,
            "[C3] {invalid:?}"
        );
    }
}
#[tokio::test]
async fn operation_id_is_rejected_before_resolution() {
    let resolve_calls = Arc::new(AtomicUsize::new(0));
    let engine = MutationEngine::new(
        Arc::new(FakeAdapter {
            resolve_calls: Arc::clone(&resolve_calls),
        }),
        session(1),
    );
    let error = engine
        .write(
            WriteRequest::new(
                PathReference::local("not-created.md").expect("[C3] local reference"),
                "content".to_owned(),
                None,
                Some(OperationId::parse("local-op").expect("[C3] operation ID")),
            )
            .expect("[C3] write request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("[C3] non-Creation Target operation ID");
    assert_eq!(error.category(), ErrorCategory::InvalidReference, "[C3]");
    assert_eq!(
        resolve_calls.load(Ordering::Relaxed),
        0,
        "[C3] resolve calls"
    );
}

#[tokio::test]
async fn operation_journal_trace_matches_model() {
    let session = session(10);
    let id = OperationId::parse("create-1").expect("[C4] operation ID");
    let target = PathReference::parse("issue://owner/repo/new").expect("[C4] target");
    let content: Arc<str> = Arc::from("---\ntitle: One\n---\nbody");
    let created = PathReference::parse("issue://owner/repo/1").expect("[C4] created");

    let owner = match session
        .begin_mutation_operation(id.clone(), target.clone(), Arc::clone(&content))
        .await
        .expect("[C4] begin owner")
    {
        MutationOperationStart::Owner(owner) => owner,
        other => panic!("[C4] expected owner, got {other:?}"),
    };
    let waiter = match session
        .begin_mutation_operation(id.clone(), target.clone(), Arc::clone(&content))
        .await
        .expect("[C4] begin waiter")
    {
        MutationOperationStart::Wait(waiter) => waiter,
        other => panic!("[C4] expected waiter, got {other:?}"),
    };
    owner.finish(MutationOperationOutcome::Succeeded(Arc::new(
        created.clone(),
    )));
    assert_eq!(
        waiter.wait().await,
        MutationOperationOutcome::Succeeded(Arc::new(created.clone())),
        "[C4] waiter result"
    );
    assert!(matches!(
        session
            .begin_mutation_operation(id.clone(), target.clone(), Arc::clone(&content))
            .await
            .expect("[C4] replay"),
        MutationOperationStart::Replay(reference) if reference.as_ref() == &created
    ));

    let conflict = session
        .begin_mutation_operation(id, target.clone(), Arc::from("different"))
        .await
        .expect_err("[C4] changed content conflicts");
    assert_eq!(conflict.category(), ErrorCategory::VersionConflict);

    let retry_id = OperationId::parse("retry").expect("[C4] retry ID");
    let first = match session
        .begin_mutation_operation(retry_id.clone(), target.clone(), Arc::from("same"))
        .await
        .expect("[C4] retry owner")
    {
        MutationOperationStart::Owner(owner) => owner,
        other => panic!("[C4] expected retry owner, got {other:?}"),
    };
    let rejected_waiter = match session
        .begin_mutation_operation(retry_id.clone(), target.clone(), Arc::from("same"))
        .await
        .expect("[C4] rejected waiter")
    {
        MutationOperationStart::Wait(waiter) => waiter,
        other => panic!("[C4] expected rejected waiter, got {other:?}"),
    };
    let rejected = ResourceError::new(ErrorCategory::PermissionDenied, "denied");
    first.finish(MutationOperationOutcome::Conclusive(rejected.clone()));
    assert_eq!(
        rejected_waiter.wait().await,
        MutationOperationOutcome::Conclusive(rejected),
        "[C4] concurrent waiter shares conclusive result"
    );
    assert!(matches!(
        session
            .begin_mutation_operation(retry_id, target.clone(), Arc::from("same"))
            .await
            .expect("[C4] conclusive retry"),
        MutationOperationStart::Owner(_)
    ));

    let unknown_id = OperationId::parse("unknown").expect("[C4] unknown ID");
    let unknown_owner = match session
        .begin_mutation_operation(unknown_id.clone(), target.clone(), Arc::from("unknown"))
        .await
        .expect("[C4] unknown owner")
    {
        MutationOperationStart::Owner(owner) => owner,
        other => panic!("[C4] expected unknown owner, got {other:?}"),
    };
    let unknown = ResourceError::new(ErrorCategory::SourceUnavailable, "reconcile");
    unknown_owner.finish(MutationOperationOutcome::Unknown(unknown.clone()));
    assert!(matches!(
        session
            .begin_mutation_operation(unknown_id, target, Arc::from("unknown"))
            .await
            .expect("[C4] unknown replay"),
        MutationOperationStart::Unknown(error) if error == unknown
    ));
}

#[tokio::test]
async fn operation_journal_concurrent_repeat_coalesces() {
    let session = session(15);
    let id = OperationId::parse("concurrent").expect("[C4] ID");
    let target = PathReference::parse("issue://owner/repo/new").expect("[C4] target");
    let content: Arc<str> = Arc::from("same bytes");
    let created = PathReference::parse("issue://owner/repo/99").expect("[C4] created");
    let owner = match session
        .begin_mutation_operation(id.clone(), target.clone(), Arc::clone(&content))
        .await
        .expect("[C4] owner")
    {
        MutationOperationStart::Owner(owner) => owner,
        other => panic!("[C4] expected owner, got {other:?}"),
    };

    let mut tasks = Vec::new();
    for _ in 0..64 {
        let waiter = match session
            .begin_mutation_operation(id.clone(), target.clone(), Arc::clone(&content))
            .await
            .expect("[C4] waiter")
        {
            MutationOperationStart::Wait(waiter) => waiter,
            other => panic!("[C4] expected waiter, got {other:?}"),
        };
        tasks.push(tokio::spawn(waiter.wait()));
    }
    owner.finish(MutationOperationOutcome::Succeeded(Arc::new(
        created.clone(),
    )));
    for task in tasks {
        assert_eq!(
            task.await.expect("[C4] waiter task"),
            MutationOperationOutcome::Succeeded(Arc::new(created.clone())),
            "[C4] shared result"
        );
    }
}

#[tokio::test]
async fn operation_journal_forced_fingerprint_collision_conflicts() {
    let session = session(11);
    let target = PathReference::parse("issue://owner/repo/new").expect("[C4] target");
    let digest = [7_u8; 32];
    let owner = match session
        .begin_mutation_operation_with_fingerprint_for_test(
            OperationId::parse("collision").expect("[C4] ID"),
            target.clone(),
            Arc::from("alpha"),
            digest,
        )
        .await
        .expect("[C4] owner")
    {
        MutationOperationStart::Owner(owner) => owner,
        other => panic!("[C4] expected owner, got {other:?}"),
    };
    let error = session
        .begin_mutation_operation_with_fingerprint_for_test(
            OperationId::parse("collision").expect("[C4] ID"),
            target,
            Arc::from("bravo"),
            digest,
        )
        .await
        .expect_err("[C4] exact bytes conflict");
    assert_eq!(error.category(), ErrorCategory::VersionConflict);
    owner.finish(MutationOperationOutcome::Conclusive(ResourceError::new(
        ErrorCategory::InvalidReference,
        "fixture complete",
    )));
}

#[tokio::test]
async fn operation_journal_count_and_byte_ceilings() {
    let session = session(12);
    let target = PathReference::parse("issue://owner/repo/new").expect("[C4] target");
    for index in 0..10_000 {
        let owner = match session
            .begin_mutation_operation(
                OperationId::parse(format!("op-{index}")).expect("[C4] ID"),
                target.clone(),
                Arc::from("x"),
            )
            .await
            .expect("[C4] admitted operation")
        {
            MutationOperationStart::Owner(owner) => owner,
            other => panic!("[C4] expected owner, got {other:?}"),
        };
        owner.finish(MutationOperationOutcome::Conclusive(ResourceError::new(
            ErrorCategory::PermissionDenied,
            "fixture",
        )));
    }
    let count_error = session
        .begin_mutation_operation(
            OperationId::parse("overflow").expect("[C4] overflow ID"),
            target.clone(),
            Arc::from("x"),
        )
        .await
        .expect_err("[C4] count ceiling");
    assert_eq!(count_error.category(), ErrorCategory::LimitExceeded);

    let limits = ServerLimits::new(ServerLimitsInput {
        storage: StorageLimitInput {
            object_bytes: Some(1_024),
            session_bytes: Some(1_024),
        },
        ..ServerLimitsInput::default()
    })
    .expect("[C4] limits");
    let limited = PathSession::new(
        SessionToken::parse("00000000000000000000000000000013").expect("[C4] token"),
        Arc::new(MemoryStorage::default()),
        limits,
    );
    let id = OperationId::parse("quota").expect("[C4] quota ID");
    let retained_overhead = id.as_str().len() + target.requested().len();
    let exact: Arc<str> = Arc::from("x".repeat(1_024 - retained_overhead));
    assert!(matches!(
        limited
            .begin_mutation_operation(id, target.clone(), exact)
            .await
            .expect("[C4] exact byte ceiling"),
        MutationOperationStart::Owner(_)
    ));

    let over = PathSession::new(
        SessionToken::parse("00000000000000000000000000000014").expect("[C4] token"),
        Arc::new(MemoryStorage::default()),
        ServerLimits::new(ServerLimitsInput {
            storage: StorageLimitInput {
                object_bytes: Some(1_024),
                session_bytes: Some(1_024),
            },
            ..ServerLimitsInput::default()
        })
        .expect("[C4] limits"),
    );
    let error = over
        .begin_mutation_operation(
            OperationId::parse("quota").expect("[C4] quota ID"),
            target,
            Arc::from("x".repeat(1_025 - retained_overhead)),
        )
        .await
        .expect_err("[C4] one byte over");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
}

#[tokio::test]
#[ignore = "checkpointed-build production-scale budget"]
async fn operation_journal_budget() {
    let (count_budget, yield_budget, compare_budget) = if cfg!(debug_assertions) {
        (100, 100, 1_500)
    } else {
        (25, 25, 50)
    };
    let target = PathReference::parse("issue://owner/repo/new").expect("[C4] target");
    let count_session = session(16);
    let started = Instant::now();
    for index in 0..10_000 {
        let owner = match count_session
            .begin_mutation_operation(
                OperationId::parse(format!("budget-{index}")).expect("[C4] ID"),
                target.clone(),
                Arc::from("x"),
            )
            .await
            .expect("[C4] admitted")
        {
            MutationOperationStart::Owner(owner) => owner,
            other => panic!("[C4] expected owner, got {other:?}"),
        };
        owner.finish(MutationOperationOutcome::Conclusive(ResourceError::new(
            ErrorCategory::PermissionDenied,
            "fixture",
        )));
    }
    let count_elapsed = started.elapsed();
    assert!(
        count_elapsed <= Duration::from_millis(count_budget),
        "[C4] 10,000 journal transitions took {count_elapsed:?}"
    );

    let cache_limits = ServerLimits::new(ServerLimitsInput {
        storage: StorageLimitInput {
            object_bytes: Some(32 * 1024),
            session_bytes: Some(32 * 1024),
        },
        ..ServerLimitsInput::default()
    })
    .expect("[C4] cache-yield limits");
    let cache_session = PathSession::new(
        SessionToken::parse("00000000000000000000000000000018").expect("[C4] token"),
        Arc::new(MemoryStorage::default()),
        cache_limits,
    );
    let mut cache_keys = Vec::new();
    for index in 0..1_000 {
        let key = SessionCacheKey::new("budget", format!("key-{index}")).expect("[C4] cache key");
        cache_session
            .cache_put(
                key.clone(),
                SessionCacheEntry::new(Vec::new(), Vec::new()).expect("[C4] cache entry"),
            )
            .await
            .expect("[C4] cache put");
        cache_keys.push(key);
    }
    let started = Instant::now();
    assert!(matches!(
        cache_session
            .begin_mutation_operation(
                OperationId::parse("cache-yield").expect("[C4] ID"),
                target.clone(),
                Arc::from("x".repeat(25 * 1024)),
            )
            .await
            .expect("[C4] authoritative journal admission"),
        MutationOperationStart::Owner(_)
    ));
    let yield_elapsed = started.elapsed();
    assert!(
        yield_elapsed <= Duration::from_millis(yield_budget),
        "[C4] worst-case cache yield took {yield_elapsed:?}"
    );
    let mut evicted = 0;
    for key in &cache_keys {
        if cache_session
            .cache_get(key)
            .await
            .expect("[C4] cache get")
            .is_none()
        {
            evicted += 1;
        }
    }
    assert!(evicted > 0, "[C4] authoritative journal yields cache bytes");

    let compare_session = session(17);
    let content: Arc<str> = Arc::from("x".repeat(64 * 1024 * 1024));
    let id = OperationId::parse("maximum-content").expect("[C4] ID");
    let owner = match compare_session
        .begin_mutation_operation(id.clone(), target.clone(), Arc::clone(&content))
        .await
        .expect("[C4] owner")
    {
        MutationOperationStart::Owner(owner) => owner,
        other => panic!("[C4] expected owner, got {other:?}"),
    };
    owner.finish(MutationOperationOutcome::Conclusive(ResourceError::new(
        ErrorCategory::PermissionDenied,
        "fixture",
    )));
    let started = Instant::now();
    assert!(matches!(
        compare_session
            .begin_mutation_operation(id, target, content)
            .await
            .expect("[C4] compare"),
        MutationOperationStart::Owner(_)
    ));
    let compare_elapsed = started.elapsed();
    assert!(
        compare_elapsed <= Duration::from_millis(compare_budget),
        "[C4] 64 MiB fingerprint and exact comparison took {compare_elapsed:?}"
    );
}

#[derive(Debug)]
struct OutcomeAdapter {
    mode: MutationTargetMode,
    state: MutationState,
    outcome: Mutex<Option<Result<MutationCommitOutcome, MutationCommitFailure>>>,
}

#[async_trait]
impl MutationAdapter for OutcomeAdapter {
    async fn resolve(
        &self,
        reference: &PathReference,
        _access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError> {
        MutationTarget::new(
            reference.clone(),
            MutationSourceKey::new("outcome").expect("[C7] source"),
            self.mode,
        )
    }

    fn validate_write(
        &self,
        _target: &MutationTarget,
        _content: &str,
    ) -> Result<(), ResourceError> {
        Ok(())
    }

    async fn load(
        &self,
        _target: &MutationTarget,
        _access: MutationAccess,
        _operation: &OperationGuard,
    ) -> Result<MutationState, ResourceError> {
        Ok(self.state.clone())
    }

    async fn commit(
        &self,
        _mutation: SourceMutation,
        _operation: &OperationGuard,
    ) -> Result<MutationCommitOutcome, MutationCommitFailure> {
        self.outcome
            .lock()
            .await
            .take()
            .expect("[C7] one commit outcome")
    }
}

async fn illegal_mode_outcome(
    mode: MutationTargetMode,
    outcome: MutationCommitOutcome,
    session_id: u8,
) -> ResourceError {
    let old_tag = VersionTag::from_content(b"old");
    let (reference, state, if_version, operation_id) = match mode {
        MutationTargetMode::AuthoredText => (
            PathReference::parse("rfs://workspace/workspace/mismatch.txt")
                .expect("[C7] authored mismatch"),
            MutationState::Missing,
            None,
            None,
        ),
        MutationTargetMode::AuthoritativeText => (
            PathReference::parse("issue://owner/repo/1/title")
                .expect("[C7] authoritative mismatch"),
            MutationState::Text {
                content: "old".to_owned(),
                version_tag: old_tag.clone(),
            },
            Some(old_tag),
            None,
        ),
        MutationTargetMode::CreationTarget => (
            PathReference::parse("issue://owner/repo/new").expect("[C7] creation mismatch"),
            MutationState::Missing,
            None,
            Some(OperationId::parse(format!("mismatch-{session_id}")).expect("[C7] mismatch ID")),
        ),
    };
    MutationEngine::new(
        Arc::new(OutcomeAdapter {
            mode,
            state,
            outcome: Mutex::new(Some(Ok(outcome))),
        }),
        session(session_id),
    )
    .write(
        WriteRequest::new(reference, "content".to_owned(), if_version, operation_id)
            .expect("[C7] mismatch request"),
        &OperationGuard::new(),
    )
    .await
    .expect_err("[C7] illegal mode/outcome")
}

#[tokio::test]
async fn target_mode_outcome_matrix() {
    let authored_path =
        PathReference::parse("rfs://workspace/workspace/authored.txt").expect("[C7] authored");
    let authored = MutationEngine::new(
        Arc::new(OutcomeAdapter {
            mode: MutationTargetMode::AuthoredText,
            state: MutationState::Missing,
            outcome: Mutex::new(Some(Ok(MutationCommitOutcome::AuthoredText))),
        }),
        session(20),
    )
    .write(
        WriteRequest::new(authored_path, "authored\n".to_owned(), None, None)
            .expect("[C7] authored request"),
        &OperationGuard::new(),
    )
    .await
    .expect("[C7] authored outcome");
    assert!(authored.version_tag().is_some(), "[C7] authored tag");
    assert!(
        authored.displayed_ranges().is_some(),
        "[C7] authored coverage"
    );

    let old_tag = VersionTag::from_content(b"old");
    let response_tag = VersionTag::from_content(b"normalized");
    let authoritative_path =
        PathReference::parse("issue://owner/repo/1/title").expect("[C7] authoritative");
    let authoritative = MutationEngine::new(
        Arc::new(OutcomeAdapter {
            mode: MutationTargetMode::AuthoritativeText,
            state: MutationState::Text {
                content: "old".to_owned(),
                version_tag: old_tag.clone(),
            },
            outcome: Mutex::new(Some(Ok(MutationCommitOutcome::AuthoritativeText {
                version_tag: response_tag.clone(),
            }))),
        }),
        session(21),
    )
    .write(
        WriteRequest::new(
            authoritative_path,
            "submitted".to_owned(),
            Some(old_tag),
            None,
        )
        .expect("[C7] authoritative request"),
        &OperationGuard::new(),
    )
    .await
    .expect("[C7] authoritative outcome");
    assert_eq!(authoritative.version_tag(), Some(&response_tag));
    assert_eq!(
        authoritative.displayed_ranges(),
        None,
        "[C7] no seen coverage"
    );

    let creation_path =
        PathReference::parse("issue://owner/repo/new").expect("[C7] Creation Target");
    let created_path =
        PathReference::parse("issue://owner/repo/77").expect("[C7] created Resource");
    let creation = MutationEngine::new(
        Arc::new(OutcomeAdapter {
            mode: MutationTargetMode::CreationTarget,
            state: MutationState::Missing,
            outcome: Mutex::new(Some(Ok(MutationCommitOutcome::CreationTarget {
                canonical_reference: Box::new(created_path.clone()),
            }))),
        }),
        session(22),
    )
    .write(
        WriteRequest::new(
            creation_path,
            "creation document".to_owned(),
            None,
            Some(OperationId::parse("create-77").expect("[C7] ID")),
        )
        .expect("[C7] creation request"),
        &OperationGuard::new(),
    )
    .await
    .expect("[C7] creation outcome");
    assert_eq!(creation.canonical_reference(), &created_path);
    assert_eq!(creation.version_tag(), None);
    assert_eq!(creation.displayed_ranges(), None);

    let mismatch_reference =
        PathReference::parse("issue://owner/repo/88").expect("[C7] mismatch Resource");
    let mismatch_tag = VersionTag::from_content(b"mismatch");
    let mismatch_started = Instant::now();
    for (index, (mode, outcome)) in [
        (
            MutationTargetMode::AuthoredText,
            MutationCommitOutcome::AuthoritativeText {
                version_tag: mismatch_tag.clone(),
            },
        ),
        (
            MutationTargetMode::AuthoredText,
            MutationCommitOutcome::CreationTarget {
                canonical_reference: Box::new(mismatch_reference.clone()),
            },
        ),
        (
            MutationTargetMode::AuthoritativeText,
            MutationCommitOutcome::AuthoredText,
        ),
        (
            MutationTargetMode::AuthoritativeText,
            MutationCommitOutcome::CreationTarget {
                canonical_reference: Box::new(mismatch_reference.clone()),
            },
        ),
        (
            MutationTargetMode::CreationTarget,
            MutationCommitOutcome::AuthoredText,
        ),
        (
            MutationTargetMode::CreationTarget,
            MutationCommitOutcome::AuthoritativeText {
                version_tag: mismatch_tag.clone(),
            },
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let error = illegal_mode_outcome(mode, outcome, 30 + index as u8).await;
        assert_eq!(
            error.category(),
            ErrorCategory::SourceUnavailable,
            "[C7] mismatch {index}"
        );
    }
    let mismatch_average = mismatch_started.elapsed() / 6;
    assert!(
        mismatch_average <= Duration::from_millis(1),
        "[C7] target/outcome dispatch averaged {mismatch_average:?}"
    );
}

#[tokio::test]
async fn write_state_matrix_and_receipt_coverage() {
    let adapter = Arc::new(StatefulAdapter::new(MutationState::Missing));
    let path =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");
    let session = session(3);
    let engine = MutationEngine::new(adapter, session.clone());

    let created = engine
        .write(
            WriteRequest::new(path.clone(), "one\ntwo\n".to_owned(), None, None)
                .expect("create request"),
            &OperationGuard::new(),
        )
        .await
        .expect("create");
    assert_eq!(created.operation(), MutationOperation::Created);
    assert_eq!(created.canonical_reference(), &path);
    assert_eq!(created.source_reference(), None);
    let created_tag = created.version_tag().expect("created Version Tag").clone();
    assert_eq!(
        created.displayed_ranges().expect("created coverage"),
        &[DisplayedLineRange::new(1, 2).expect("created coverage")]
    );
    assert_eq!(created.displayed_eof(), Some(true));
    let snapshot = session
        .resolve_seen_for_test(
            path.requested(),
            &VersionSelector::full(created_tag.clone()),
        )
        .await
        .expect("created receipt snapshot");
    assert_eq!(
        snapshot.1,
        created.displayed_ranges().expect("created coverage")
    );
    assert!(snapshot.2);

    let duplicate = engine
        .write(
            WriteRequest::new(path.clone(), "duplicate".to_owned(), None, None)
                .expect("duplicate request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("create over existing");
    assert_eq!(duplicate.category(), ErrorCategory::VersionConflict);

    let stale = engine
        .write(
            WriteRequest::new(
                path.clone(),
                "replacement".to_owned(),
                Some(VersionTag::from_content(b"stale")),
                None,
            )
            .expect("stale request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("stale replacement");
    assert_eq!(stale.category(), ErrorCategory::VersionConflict);

    let replaced = engine
        .write(
            WriteRequest::new(
                path.clone(),
                "replacement".to_owned(),
                Some(created_tag),
                None,
            )
            .expect("replace request"),
            &OperationGuard::new(),
        )
        .await
        .expect("replace");
    assert_eq!(replaced.operation(), MutationOperation::Replaced);
    assert_eq!(
        replaced.displayed_ranges().expect("replacement coverage"),
        &[DisplayedLineRange::new(1, 1).expect("replacement coverage")]
    );
}
#[test]
fn mutation_operation_spellings_are_stable() {
    assert_eq!(
        [
            MutationOperation::Created.as_str(),
            MutationOperation::Replaced.as_str(),
            MutationOperation::Edited.as_str(),
            MutationOperation::Deleted.as_str(),
            MutationOperation::Moved.as_str(),
        ],
        ["created", "replaced", "edited", "deleted", "moved"]
    );
}

#[tokio::test]
async fn put_cut_use_original_coordinates_and_never_promote_unseen_content() {
    let original = "one\r\ntwo\r\nthree\nfour";
    let original_tag = VersionTag::from_content(original.as_bytes());
    let adapter = Arc::new(StatefulAdapter::new(MutationState::Text {
        content: original.to_owned(),
        version_tag: original_tag.clone(),
    }));
    let path =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");
    let session = session(5);
    session
        .record_seen_for_test(
            path.requested(),
            &original_tag,
            &[
                DisplayedLineRange::new(1, 2).expect("first seen range"),
                DisplayedLineRange::new(4, 4).expect("last seen line"),
            ],
            true,
        )
        .await
        .expect("original snapshot");
    let engine = MutationEngine::new(adapter.clone(), session.clone());
    let patch = format!(
        "[{}#{}]\nPUT 2.=2:\n+TWO\nPUT <4:\n+inserted\nCUT 1.=1",
        path.requested(),
        original_tag
    );

    let receipt = engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect("snapshot-bound edit");
    assert_eq!(receipt.operation(), MutationOperation::Edited);
    assert_eq!(adapter.text_content().await, "TWO\r\nthree\ninserted\nfour");
    assert_eq!(
        receipt.displayed_ranges().expect("edit coverage"),
        &[
            DisplayedLineRange::new(1, 1).expect("authored first line"),
            DisplayedLineRange::new(3, 4).expect("authored plus remapped tail"),
        ]
    );
    assert_eq!(receipt.displayed_eof(), Some(true));
    let edited_tag = receipt.version_tag().expect("edited tag").clone();

    let unseen = format!(
        "[{}#{}]\nPUT 2.=2:\n+forbidden",
        path.requested(),
        edited_tag
    );
    let before = adapter.text_content().await;
    let error = engine
        .edit(&unseen, &OperationGuard::new())
        .await
        .expect_err("unseen preserved line");
    assert_eq!(error.category(), ErrorCategory::InvalidPatch);
    assert_eq!(adapter.text_content().await, before);

    let consecutive = format!("[{}#{}]\nPUT 3.=3:\n+again", path.requested(), edited_tag);
    engine
        .edit(&consecutive, &OperationGuard::new())
        .await
        .expect("receipt-authored line remains editable");
    assert_eq!(adapter.text_content().await, "TWO\r\nthree\nagain\nfour");
}

#[tokio::test]
async fn tail_insert_requires_seen_eof() {
    let original = "one\n";
    let tag = VersionTag::from_content(original.as_bytes());
    let adapter = Arc::new(StatefulAdapter::new(MutationState::Text {
        content: original.to_owned(),
        version_tag: tag.clone(),
    }));
    let path =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");
    let session = session(6);
    session
        .record_seen_for_test(
            path.requested(),
            &tag,
            &[DisplayedLineRange::new(1, 1).expect("seen line")],
            false,
        )
        .await
        .expect("bounded snapshot");
    let engine = MutationEngine::new(adapter, session);
    let patch = format!("[{}#{}]\nPUT >$:\n+tail", path.requested(), tag);

    let error = engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect_err("unseen EOF gap");
    assert_eq!(error.category(), ErrorCategory::InvalidPatch);
}

#[tokio::test]
async fn empty_put_body_writes_a_blank_line_instead_of_deleting() {
    let content = "keep\n";
    let tag = VersionTag::from_content(content.as_bytes());
    let adapter = Arc::new(StatefulAdapter::new(MutationState::Text {
        content: content.to_owned(),
        version_tag: tag.clone(),
    }));
    let path =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");
    let session = session(9);
    session
        .record_seen_for_test(
            path.requested(),
            &tag,
            &[DisplayedLineRange::new(1, 1).expect("seen line")],
            true,
        )
        .await
        .expect("snapshot");
    let engine = MutationEngine::new(adapter.clone(), session);
    let patch = format!("[{}#{}]\nPUT 1.=1:\n+", path.requested(), tag);

    engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect("blank-line PUT");
    assert_eq!(adapter.text_content().await, "\n");
}

#[tokio::test]
async fn rem_is_only_delete_form_and_returns_no_version_or_coverage() {
    let content = "delete me\n";
    let tag = VersionTag::from_content(content.as_bytes());
    let adapter = Arc::new(StatefulAdapter::new(MutationState::Text {
        content: content.to_owned(),
        version_tag: tag.clone(),
    }));
    let path =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");
    let session = session(8);
    session
        .record_seen_for_test(
            path.requested(),
            &tag,
            &[DisplayedLineRange::new(1, 1).expect("seen line")],
            true,
        )
        .await
        .expect("delete snapshot");
    let engine = MutationEngine::new(adapter.clone(), session);
    let patch = format!("[{}#{}]\nREM", path.requested(), tag);

    let receipt = engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect("REM");
    assert_eq!(receipt.operation(), MutationOperation::Deleted);
    assert_eq!(receipt.version_tag(), None);
    assert_eq!(receipt.displayed_ranges(), None);
    assert_eq!(receipt.displayed_eof(), None);
    assert!(matches!(
        *adapter.state.lock().await,
        MutationState::Missing
    ));

    let second = engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect_err("REM of missing Resource");
    assert_eq!(second.category(), ErrorCategory::VersionConflict);
}

#[tokio::test]
async fn exact_limit_edit_budget() {
    let original_tag = VersionTag::from_content(b"");
    let adapter = Arc::new(StatefulAdapter::new(MutationState::Text {
        content: String::new(),
        version_tag: original_tag.clone(),
    }));
    let path =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");
    let session = session(7);
    session
        .record_seen_for_test(path.requested(), &original_tag, &[], true)
        .await
        .expect("empty EOF snapshot");
    let engine = MutationEngine::new(adapter, session);
    let prefix = format!("[{}#{}]\nPUT >$:\n+", path.requested(), original_tag);
    let mut patch = String::with_capacity(MAX_HASHLINE_PATCH_BYTES);
    patch.push_str(&prefix);
    patch.extend(std::iter::repeat_n(
        'x',
        MAX_HASHLINE_PATCH_BYTES - prefix.len(),
    ));
    let started = Instant::now();
    engine
        .edit(&patch, &OperationGuard::new())
        .await
        .expect("exact-limit edit");
    let elapsed = started.elapsed();
    assert!(
        elapsed <= Duration::from_secs(5),
        "64 MiB edit took {elapsed:?}"
    );
}

#[tokio::test]
async fn failed_commit_releases_seen_reservation() {
    let adapter = Arc::new(StatefulAdapter::new(MutationState::Missing));
    adapter.fail_next_commit();
    let session = session(4);
    let engine = MutationEngine::new(adapter, session.clone());
    let path =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");

    let error = engine
        .write(
            WriteRequest::new(path, "content".to_owned(), None, None).expect("write request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("injected failure");

    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert_eq!(session.used_bytes().await, 0);
}

#[tokio::test]
async fn same_resource_serializes_while_distinct_resources_progress() {
    let engine = engine();
    let first = engine
        .lock_resources_for_test(vec![lock_key("same")])
        .await
        .expect("first lock");

    let blocked_engine = engine.clone();
    let acquired = Arc::new(AtomicBool::new(false));
    let blocked = {
        let acquired = Arc::clone(&acquired);
        tokio::spawn(async move {
            let _guard = blocked_engine
                .lock_resources_for_test(vec![lock_key("same")])
                .await
                .expect("second lock");
            acquired.store(true, Ordering::Release);
        })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !acquired.load(Ordering::Acquire),
        "same canonical key must remain blocked"
    );

    let distinct = tokio::time::timeout(
        Duration::from_millis(100),
        engine.lock_resources_for_test(vec![lock_key("different")]),
    )
    .await
    .expect("distinct key must progress")
    .expect("distinct lock");
    drop(distinct);
    drop(first);
    tokio::time::timeout(Duration::from_millis(100), blocked)
        .await
        .expect("blocked lock release")
        .expect("blocked task");
    assert!(acquired.load(Ordering::Acquire));
}

#[tokio::test]
async fn reverse_moves_do_not_deadlock() {
    let first_engine = engine();
    let second_engine = first_engine.clone();
    let first = tokio::spawn(async move {
        let _guard = first_engine
            .lock_resources_for_test(vec![lock_key("b"), lock_key("a")])
            .await
            .expect("first pair");
        tokio::task::yield_now().await;
    });
    let second = tokio::spawn(async move {
        let _guard = second_engine
            .lock_resources_for_test(vec![lock_key("a"), lock_key("b")])
            .await
            .expect("reverse pair");
        tokio::task::yield_now().await;
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        first.await.expect("first task");
        second.await.expect("second task");
    })
    .await
    .expect("sorted paired locks must terminate");
}

#[tokio::test]
async fn seen_regions_never_cross_tags_and_prefixes_must_be_unique() {
    let session = session(2);
    let canonical = "rfs://workspace/workspace/fixture.txt";
    let first = VersionTag::parse(format!("sha256:{}", "a".repeat(64))).expect("first tag");
    let second = VersionTag::parse(format!("sha256:{}{}", "a".repeat(12), "b".repeat(52)))
        .expect("second tag");
    let range = DisplayedLineRange::new(1, 2).expect("seen range");
    session
        .record_seen_for_test(canonical, &first, &[range], false)
        .await
        .expect("first snapshot");
    session
        .record_seen_for_test(canonical, &second, &[range], true)
        .await
        .expect("second snapshot");

    let ambiguous = session
        .resolve_seen_for_test(
            canonical,
            &VersionSelector::prefix("a".repeat(12)).expect("valid prefix"),
        )
        .await
        .expect_err("shared prefix");
    assert_eq!(ambiguous.category(), ErrorCategory::InvalidPatch);

    let unique = session
        .resolve_seen_for_test(
            canonical,
            &VersionSelector::prefix("a".repeat(13)).expect("valid prefix"),
        )
        .await
        .expect("unique first prefix");
    assert_eq!(unique.0, first);
    assert!(!unique.2);

    let missing = session
        .resolve_seen_for_test(
            canonical,
            &VersionSelector::prefix("c".repeat(12)).expect("valid prefix"),
        )
        .await
        .expect_err("unknown prefix");
    assert_eq!(missing.category(), ErrorCategory::VersionConflict);
}

#[tokio::test]
async fn commit_masks_cancellation_after_transition() {
    let before = OperationGuard::new();
    assert!(before.cancel());
    let error = before
        .begin_commit()
        .expect_err("cancel wins before commit");
    assert_eq!(error.category(), ErrorCategory::Cancelled);

    let committing = OperationGuard::new();
    committing.begin_commit().expect("begin commit");
    assert!(committing.is_committing());
    assert!(!committing.cancel(), "commit masks later cancellation");
    assert!(committing.is_active());
    assert!(committing.finish_commit());
    assert!(!committing.is_active());
}

#[test]
fn mutation_identities_reject_empty_sources_and_relative_targets() {
    let empty = MutationSourceKey::new("").expect_err("empty source key");
    assert_eq!(empty.category(), ErrorCategory::InvalidReference);
    let empty_resource = MutationResourceKey::new("").expect_err("empty mutation Resource key");
    assert_eq!(empty_resource.category(), ErrorCategory::InvalidReference);

    let source = MutationSourceKey::new("workspace").expect("source key");
    assert_eq!(source.as_str(), "workspace");
    let relative = PathReference::parse("fixture.txt").expect("relative reference");
    let error = MutationTarget::new(relative, source.clone(), MutationTargetMode::AuthoredText)
        .expect_err("relative mutation target");
    assert_eq!(error.category(), ErrorCategory::InvalidReference);

    let canonical =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");
    let target = MutationTarget::new(canonical.clone(), source, MutationTargetMode::AuthoredText)
        .expect("mutation target");
    assert_eq!(target.canonical_reference(), &canonical);
    assert_eq!(target.source_key().as_str(), "workspace");
}

#[test]
fn engine_exposes_the_same_strict_parser_interface() {
    let engine = engine();
    let document = format!(
        "[rfs://workspace/workspace/fixture.txt#{}]\nCUT 1.=1",
        VersionTag::from_content(b"fixture")
    );
    let direct = resourcefs_core::HashlinePatch::parse(&document).expect("direct parser");
    let through_engine = engine.parse_edit(&document).expect("engine parser");
    assert_eq!(through_engine, direct);
    assert_eq!(
        engine.session().token().as_str(),
        "00000000000000000000000000000001"
    );
    let _adapter = engine.adapter();
}

/// C2 — the engine drives a **non-workspace** family with no workspace-specific
/// branch. Every other case here uses a workspace reference, so a workspace-only
/// guard in `MutationEngine::write` would slip past them; this case is what makes
/// that mutation observable.
#[tokio::test]
async fn engine_drives_non_workspace_families() {
    let session = session(9);
    let adapter = Arc::new(StatefulAdapter::new(MutationState::Missing));
    let engine = MutationEngine::new(adapter.clone(), session.clone());
    let reference = PathReference::local("plan.md").expect("scratch reference");

    let created = engine
        .write(
            WriteRequest::new(reference.clone(), "scratch bytes\n".to_owned(), None, None)
                .expect("create request"),
            &OperationGuard::new(),
        )
        .await
        .expect("the engine must drive a local:// target with no workspace branch");
    assert_eq!(created.operation(), MutationOperation::Created);
    assert_eq!(
        created.canonical_reference().requested(),
        reference.requested()
    );
}
