use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use resourcefs_core::{
    ArtifactId, DisplayedLineRange, ErrorCategory, MAX_HASHLINE_PATCH_BYTES, MutationAccess,
    MutationAdapter, MutationEngine, MutationOperation, MutationResourceKey, MutationSourceKey,
    MutationState, MutationTarget, OperationGuard, PathReference, PathSession, ResourceError,
    ServerLimits, SessionStorage, SessionToken, SourceMutation, VersionSelector, VersionTag,
    WriteRequest,
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
            SourceMutation::Delete { expected, .. } => {
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
                *state = MutationState::Missing;
            }
            SourceMutation::Move { .. } => {
                return Err(ResourceError::new(
                    ErrorCategory::UnsupportedMutation,
                    "unsupported fake mutation",
                ));
            }
        }
        Ok(())
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
