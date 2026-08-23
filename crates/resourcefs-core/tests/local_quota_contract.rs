//! C6 — Session Scratch shares the Path Session quota, fails atomically, and
//! never evicts a live Resource.
//!
//! The oracle is a ledger the test maintains itself plus a raw inventory of the
//! backing store; neither consults the session's own accounting.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use async_trait::async_trait;
use resourcefs_core::{
    ArtifactId, ErrorCategory, LocalName, MAX_SESSION_ARTIFACTS, OperationGuard, PathSession,
    ResourceError, ServerLimits, ServerLimitsInput, SessionStorage, SessionToken,
    StorageLimitInput,
};
use tokio::sync::Mutex;

#[derive(Default)]
struct FakeStorage {
    content: Mutex<HashMap<ArtifactId, Vec<u8>>>,
}

impl FakeStorage {
    /// Raw inventory of every stored object, keyed by its numeric id. Built from
    /// the store itself so it cannot agree with the session by construction.
    async fn inventory(&self) -> BTreeMap<u64, Vec<u8>> {
        self.content
            .lock()
            .await
            .iter()
            .map(|(id, bytes)| (id.get(), bytes.clone()))
            .collect()
    }
}

#[async_trait]
impl SessionStorage for FakeStorage {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError> {
        Ok(self
            .content
            .lock()
            .await
            .get(&id)
            .is_some_and(|stored| stored.as_slice() == content))
    }

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError> {
        self.content.lock().await.insert(id, content.to_vec());
        Ok(())
    }

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError> {
        let content = self.content.lock().await;
        let bytes = content
            .get(&id)
            .ok_or_else(|| ResourceError::new(ErrorCategory::NotFound, "absent fake object"))?;
        String::from_utf8(bytes.clone())
            .map_err(|_| ResourceError::new(ErrorCategory::SourceUnavailable, "invalid fake UTF-8"))
    }

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError> {
        self.content.lock().await.remove(&id);
        Ok(())
    }

    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}

fn ceilings(object_bytes: usize, session_bytes: usize) -> ServerLimits {
    ServerLimits::new(ServerLimitsInput {
        storage: StorageLimitInput {
            object_bytes: Some(object_bytes),
            session_bytes: Some(session_bytes),
        },
        ..Default::default()
    })
    .expect("lower-only storage ceilings")
}

fn new_session(value: u8, storage: Arc<FakeStorage>, limits: ServerLimits) -> PathSession {
    let trait_storage: Arc<dyn SessionStorage> = storage;
    PathSession::new(
        SessionToken::parse(format!("{value:032x}")).expect("fixture token"),
        trait_storage,
        limits,
    )
}

fn name(value: &str) -> LocalName {
    LocalName::new(value).expect("fixture scratch name")
}

#[tokio::test]
async fn scratch_shares_and_never_evicts() {
    let storage = Arc::new(FakeStorage::default());
    let session = new_session(1, Arc::clone(&storage), ceilings(1024, 4096));
    let guard = OperationGuard::new();

    // Ledger maintained by the test, never by the session's accounting.
    let mut ledger_bytes = 0_usize;
    let mut ledger_objects = 0_usize;

    // Mixed fill: one artifact plus scratch, to exactly the session ceiling.
    session
        .retain(&"a".repeat(1024), &guard)
        .await
        .expect("artifact at the object ceiling");
    ledger_bytes += 1024;
    ledger_objects += 1;

    for scratch in ["one.md", "two.md", "three.md"] {
        session
            .scratch_put(&name(scratch), &"s".repeat(1024), &guard)
            .await
            .expect("scratch within the shared ceiling");
        ledger_bytes += 1024;
        ledger_objects += 1;
    }

    assert_eq!(
        ledger_bytes, 4096,
        "fixture fills the session ceiling exactly"
    );
    assert_eq!(
        session.used_bytes().await,
        ledger_bytes,
        "scratch bytes must be charged to the shared session ledger"
    );

    let before = storage.inventory().await;
    assert_eq!(before.len(), ledger_objects);

    // One byte past the shared ceiling.
    let error = session
        .scratch_put(&name("over.md"), "x", &guard)
        .await
        .expect_err("one byte over the session ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);

    // Atomic: nothing added, nothing evicted, ledger unmoved.
    assert_eq!(
        storage.inventory().await,
        before,
        "a quota failure added or evicted a stored object"
    );
    assert_eq!(session.used_bytes().await, ledger_bytes);
    assert!(
        session
            .scratch_load(&name("over.md"))
            .await
            .expect("load a rejected name")
            .is_none()
    );

    // Every pre-existing scratch Resource still reads back byte-for-byte.
    for scratch in ["one.md", "two.md", "three.md"] {
        let content = session
            .scratch_load(&name(scratch))
            .await
            .expect("load")
            .expect("scratch survived the quota failure")
            .content;
        assert_eq!(content, "s".repeat(1024));
    }

    // The object ceiling is shared with artifacts and enforced before commit.
    let counted = Arc::new(FakeStorage::default());
    let counting = new_session(2, Arc::clone(&counted), ceilings(1024, 8 * 1024 * 1024));
    for index in 0..MAX_SESSION_ARTIFACTS {
        counting
            .scratch_put(&name(&format!("n{index}.md")), "x", &guard)
            .await
            .expect("under the object ceiling");
    }
    let before_objects = counted.inventory().await;
    assert_eq!(before_objects.len(), MAX_SESSION_ARTIFACTS);
    let error = counting
        .scratch_put(&name("one-too-many.md"), "x", &guard)
        .await
        .expect_err("one object over the ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(counted.inventory().await, before_objects);

    // The per-object ceiling: exact is accepted, one byte over is refused.
    let object = Arc::new(FakeStorage::default());
    let object_session = new_session(3, Arc::clone(&object), ceilings(1024, 8 * 1024 * 1024));
    object_session
        .scratch_put(&name("exact.md"), &"e".repeat(1024), &guard)
        .await
        .expect("exact object ceiling");
    let inventory = object.inventory().await;
    let error = object_session
        .scratch_put(&name("over.md"), &"e".repeat(1025), &guard)
        .await
        .expect_err("one byte over the object ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(object.inventory().await, inventory);
}

#[tokio::test]
async fn replacing_scratch_releases_its_previous_bytes() {
    let storage = Arc::new(FakeStorage::default());
    let session = new_session(4, Arc::clone(&storage), ceilings(1024, 2048));
    let guard = OperationGuard::new();

    session
        .scratch_put(&name("plan.md"), &"a".repeat(1024), &guard)
        .await
        .expect("initial scratch");
    assert_eq!(session.used_bytes().await, 1024);

    // Replacing must charge the delta, not the sum: a second 1 KiB write to the
    // same name keeps the session at one object's worth of bytes.
    session
        .scratch_put(&name("plan.md"), &"b".repeat(1024), &guard)
        .await
        .expect("replacement reuses the released bytes");
    assert_eq!(session.used_bytes().await, 1024);

    let content = session
        .scratch_load(&name("plan.md"))
        .await
        .expect("load")
        .expect("replaced scratch")
        .content;
    assert_eq!(content, "b".repeat(1024));

    // Removal returns the bytes to the session.
    session
        .scratch_remove(&name("plan.md"))
        .await
        .expect("remove");
    assert_eq!(session.used_bytes().await, 0);
    assert!(
        session
            .scratch_load(&name("plan.md"))
            .await
            .expect("load")
            .is_none()
    );
}

#[tokio::test]
async fn scratch_name_enumeration_fits_its_budget() {
    // Plan budget: enumeration of the 1,000-name production ceiling < 1 ms.
    let storage = Arc::new(FakeStorage::default());
    let session = new_session(5, Arc::clone(&storage), ceilings(1024, 8 * 1024 * 1024));
    let guard = OperationGuard::new();
    for index in 0..MAX_SESSION_ARTIFACTS {
        session
            .scratch_put(&name(&format!("n{index:04}.md")), "x", &guard)
            .await
            .expect("scratch put");
    }

    let started = std::time::Instant::now();
    let names = session.scratch_names().await.expect("names");
    let elapsed = started.elapsed();

    assert_eq!(names.len(), MAX_SESSION_ARTIFACTS);
    assert!(
        names.windows(2).all(|pair| pair[0] < pair[1]),
        "enumeration must be sorted"
    );
    eprintln!("scratch_names at {MAX_SESSION_ARTIFACTS} names: {elapsed:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(1),
        "scratch name enumeration exceeded its 1 ms budget at the production ceiling: {elapsed:?}"
    );
}
