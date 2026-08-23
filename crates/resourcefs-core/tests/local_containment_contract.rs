//! C12 — no valid Session Scratch name reaches the backing store as a path.
//!
//! Containment is structural: the grammar refuses every separator, and the store
//! is keyed by an id the session allocates, never by anything derived from the
//! name. The oracle is the allocation sequence the test computes for itself plus
//! the raw ids the store was handed.

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use async_trait::async_trait;
use resourcefs_core::{
    ArtifactId, ErrorCategory, LocalName, OperationGuard, PathSession, ResourceError, ServerLimits,
    SessionStorage, SessionToken,
};
use tokio::sync::Mutex;

#[derive(Default)]
struct FakeStorage {
    content: Mutex<HashMap<ArtifactId, Vec<u8>>>,
    written_ids: Mutex<Vec<u64>>,
}

impl FakeStorage {
    /// Every id the store was handed, in write order — the only channel through
    /// which a name could reach storage.
    async fn written_ids(&self) -> Vec<u64> {
        self.written_ids.lock().await.clone()
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
        self.written_ids.lock().await.push(id.get());
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

fn new_session(value: u8, storage: Arc<FakeStorage>) -> PathSession {
    let trait_storage: Arc<dyn SessionStorage> = storage;
    PathSession::new(
        SessionToken::parse(format!("{value:032x}")).expect("fixture token"),
        trait_storage,
        ServerLimits::default(),
    )
}

fn name(value: &str) -> LocalName {
    LocalName::new(value).expect("fixture scratch name")
}

#[tokio::test]
async fn names_cannot_escape() {
    // A name that could denote a path never becomes a LocalName at all, so it
    // can never reach the store.
    for escaping in [
        "../secret",
        "a/b",
        "a\\b",
        "/etc/passwd",
        ".",
        "..",
        "",
        "sub/dir/file.md",
    ] {
        assert!(
            LocalName::new(escaping).is_err(),
            "escaping scratch name must not construct: {escaping:?}"
        );
    }

    let storage = Arc::new(FakeStorage::default());
    let session = new_session(9, Arc::clone(&storage));
    let guard = OperationGuard::new();

    // Adversarial but valid names: Unicode, spaces, leading dots, an embedded
    // colon, a percent sign, and the maximum length.
    let maximum = "n".repeat(255);
    let names = [
        "设计.md",
        "review notes.md",
        "...md",
        "plan.md:1-5",
        "100%.md",
        maximum.as_str(),
    ];

    for (index, raw) in names.iter().enumerate() {
        session
            .scratch_put_for_test(&name(raw), &format!("content-{index}"), &guard)
            .await
            .expect("valid scratch name is storable");
    }

    // Oracle: the ids handed to storage are exactly the sequence the session
    // allocates, computed here without consulting the session.
    let observed = storage.written_ids().await;
    let expected = (1..=names.len() as u64).collect::<Vec<_>>();
    assert_eq!(
        observed, expected,
        "scratch object ids must be the allocated sequence, never derived from the name"
    );

    // Distinct names never collapse onto one object.
    assert_eq!(
        observed.iter().copied().collect::<BTreeSet<_>>().len(),
        names.len(),
        "two scratch names shared one backing object"
    );

    // Every name reads back exactly its own content — no cross-talk.
    for (index, raw) in names.iter().enumerate() {
        let (content, _) = session
            .scratch_load_for_test(&name(raw))
            .await
            .expect("load")
            .expect("retained scratch");
        assert_eq!(content, format!("content-{index}"));
    }

    // Renaming moves the name, never the backing object identity.
    let ids_before_rename = storage.written_ids().await;
    session
        .scratch_rename_for_test(&name("设计.md"), &name("renamed.md"))
        .await
        .expect("same-source rename");
    assert_eq!(
        storage.written_ids().await,
        ids_before_rename,
        "rename must not rewrite the backing object"
    );
    let (content, _) = session
        .scratch_load_for_test(&name("renamed.md"))
        .await
        .expect("load")
        .expect("renamed scratch");
    assert_eq!(content, "content-0");
    assert!(
        session
            .scratch_load_for_test(&name("设计.md"))
            .await
            .expect("load")
            .is_none(),
        "the old name must not survive a rename"
    );
}

#[tokio::test]
async fn scratch_names_are_sorted_and_session_scoped() {
    let first_storage = Arc::new(FakeStorage::default());
    let first = new_session(10, Arc::clone(&first_storage));
    let second = new_session(11, Arc::new(FakeStorage::default()));
    let guard = OperationGuard::new();

    for raw in ["zebra.md", "alpha.md", "middle.md"] {
        first
            .scratch_put_for_test(&name(raw), "x", &guard)
            .await
            .expect("scratch put");
    }
    second
        .scratch_put_for_test(&name("alpha.md"), "other", &guard)
        .await
        .expect("scratch put in a second session");

    let names = first.scratch_names_for_test().await.expect("names");
    let rendered = names.iter().map(LocalName::as_str).collect::<Vec<_>>();
    assert_eq!(
        rendered,
        vec!["alpha.md", "middle.md", "zebra.md"],
        "scratch names must enumerate in sorted order"
    );

    // The colliding name in the other session is invisible here and holds its
    // own content there.
    let (content, _) = second
        .scratch_load_for_test(&name("alpha.md"))
        .await
        .expect("load")
        .expect("second session scratch");
    assert_eq!(content, "other");
    let (content, _) = first
        .scratch_load_for_test(&name("alpha.md"))
        .await
        .expect("load")
        .expect("first session scratch");
    assert_eq!(content, "x");
}
