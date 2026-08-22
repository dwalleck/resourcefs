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
    MutationOutcome, MutationSourceKey, MutationState, MutationTarget, OperationGuard,
    PathReference, PathSession, ResourceError, ServerLimits, SessionStorage, SessionToken,
    SourceMutation, VersionSelector, VersionTag,
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
    ) -> Result<MutationOutcome, ResourceError> {
        Err(ResourceError::new(
            ErrorCategory::UnsupportedMutation,
            "fake adapter does not commit",
        ))
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
