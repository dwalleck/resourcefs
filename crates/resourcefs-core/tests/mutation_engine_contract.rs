use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use resourcefs_core::{
    ArtifactId, DisplayedLineRange, ErrorCategory, MutationAccess, MutationAdapter, MutationEngine,
    MutationOperation, MutationSourceKey, MutationState, MutationTarget, OperationGuard,
    PathReference, PathSession, ResourceError, ServerLimits, SessionStorage, SessionToken,
    SourceMutation, VersionSelector, VersionTag, WriteRequest,
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
struct FakeAdapter;

#[async_trait]
impl MutationAdapter for FakeAdapter {
    async fn resolve(
        &self,
        reference: &PathReference,
        _access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError> {
        MutationTarget::new(
            reference.clone(),
            MutationSourceKey::new("fake").expect("source key"),
        )
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
    ) -> Result<(), ResourceError> {
        Err(ResourceError::new(
            ErrorCategory::UnsupportedMutation,
            "fake adapter does not commit",
        ))
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
        )
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
    ) -> Result<(), ResourceError> {
        if self.fail_commit.swap(false, Ordering::AcqRel) {
            return Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "injected commit failure",
            ));
        }
        let mut state = self.state.lock().await;
        match mutation {
            SourceMutation::Create { content, .. } => {
                if !matches!(*state, MutationState::Missing) {
                    return Err(ResourceError::new(
                        ErrorCategory::VersionConflict,
                        "destination exists",
                    ));
                }
                *state = MutationState::Text {
                    version_tag: VersionTag::from_content(content.as_bytes()),
                    content,
                };
            }
            SourceMutation::Replace {
                expected, content, ..
            } => {
                let MutationState::Text { version_tag, .. } = &*state else {
                    return Err(ResourceError::new(
                        ErrorCategory::VersionConflict,
                        "destination is missing",
                    ));
                };
                if version_tag != &expected {
                    return Err(ResourceError::new(
                        ErrorCategory::VersionConflict,
                        "stale destination",
                    ));
                }
                *state = MutationState::Text {
                    version_tag: VersionTag::from_content(content.as_bytes()),
                    content,
                };
            }
            SourceMutation::Delete { .. } | SourceMutation::Move { .. } => {
                return Err(ResourceError::new(
                    ErrorCategory::UnsupportedMutation,
                    "unsupported fake mutation",
                ));
            }
        }
        Ok(())
    }
}

fn session(value: u8) -> PathSession {
    PathSession::new(
        SessionToken::parse(format!("{value:032x}")).expect("session token"),
        Arc::new(MemoryStorage::default()),
        ServerLimits::default(),
    )
}

fn engine() -> MutationEngine {
    MutationEngine::new(Arc::new(FakeAdapter), session(1))
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
            WriteRequest::new(path.clone(), "one\ntwo\n".to_owned(), None).expect("create request"),
            &OperationGuard::new(),
        )
        .await
        .expect("create");
    assert_eq!(created.operation(), MutationOperation::Created);
    assert_eq!(created.canonical_reference(), &path);
    assert_eq!(created.source_reference(), None);
    let created_tag = created.version_tag().expect("created Version Tag").clone();
    assert_eq!(
        created.displayed_ranges(),
        &[DisplayedLineRange::new(1, 2).expect("created coverage")]
    );
    assert!(created.displayed_eof());
    let snapshot = session
        .resolve_seen_for_test(
            path.requested(),
            &VersionSelector::full(created_tag.clone()),
        )
        .await
        .expect("created receipt snapshot");
    assert_eq!(snapshot.1, created.displayed_ranges());
    assert!(snapshot.2);

    let duplicate = engine
        .write(
            WriteRequest::new(path.clone(), "duplicate".to_owned(), None)
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
            )
            .expect("stale request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("stale replacement");
    assert_eq!(stale.category(), ErrorCategory::VersionConflict);

    let replaced = engine
        .write(
            WriteRequest::new(path.clone(), "replacement".to_owned(), Some(created_tag))
                .expect("replace request"),
            &OperationGuard::new(),
        )
        .await
        .expect("replace");
    assert_eq!(replaced.operation(), MutationOperation::Replaced);
    assert_eq!(
        replaced.displayed_ranges(),
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
async fn failed_commit_releases_seen_reservation() {
    let adapter = Arc::new(StatefulAdapter::new(MutationState::Missing));
    adapter.fail_next_commit();
    let session = session(4);
    let engine = MutationEngine::new(adapter, session.clone());
    let path =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");

    let error = engine
        .write(
            WriteRequest::new(path, "content".to_owned(), None).expect("write request"),
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
        .lock_resources_for_test(vec!["same".to_owned()])
        .await
        .expect("first lock");

    let blocked_engine = engine.clone();
    let acquired = Arc::new(AtomicBool::new(false));
    let blocked = {
        let acquired = Arc::clone(&acquired);
        tokio::spawn(async move {
            let _guard = blocked_engine
                .lock_resources_for_test(vec!["same".to_owned()])
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
        engine.lock_resources_for_test(vec!["different".to_owned()]),
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
            .lock_resources_for_test(vec!["b".to_owned(), "a".to_owned()])
            .await
            .expect("first pair");
        tokio::task::yield_now().await;
    });
    let second = tokio::spawn(async move {
        let _guard = second_engine
            .lock_resources_for_test(vec!["a".to_owned(), "b".to_owned()])
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

    let source = MutationSourceKey::new("workspace").expect("source key");
    assert_eq!(source.as_str(), "workspace");
    let relative = PathReference::parse("fixture.txt").expect("relative reference");
    let error =
        MutationTarget::new(relative, source.clone()).expect_err("relative mutation target");
    assert_eq!(error.category(), ErrorCategory::InvalidReference);

    let canonical =
        PathReference::parse("rfs://workspace/workspace/fixture.txt").expect("canonical reference");
    let target = MutationTarget::new(canonical.clone(), source).expect("mutation target");
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
