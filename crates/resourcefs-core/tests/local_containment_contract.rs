//! C12 — no valid Session Scratch name reaches the backing store as a path.
//!
//! Containment is structural: the grammar refuses every separator, and the store
//! is keyed by an id the session allocates, never by anything derived from the
//! name. The oracle is the allocation sequence the test computes for itself plus
//! the raw ids the store was handed.

use std::{collections::BTreeSet, sync::Arc};

use resourcefs_core::{
    ArtifactId, LocalName, OperationGuard, PathSession, ServerLimits, SessionStorage, SessionToken,
    test_support::MemoryStorage,
};

fn new_session(value: u8, storage: Arc<MemoryStorage>) -> PathSession {
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

    let storage = Arc::new(MemoryStorage::default());
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
            .scratch_put(&name(raw), &format!("content-{index}"), &guard)
            .await
            .expect("valid scratch name is storable");
    }

    // Oracle: the ids handed to storage are exactly the sequence the session
    // allocates, computed here without consulting the session. The write log is
    // every id the store was handed, in order: the only channel through which a
    // name could reach storage, so a name-derived key would show up here.
    let observed = storage.written_ids().await;
    let expected = (1..=names.len() as u64)
        .map(|value| ArtifactId::new(value).expect("allocated artifact id"))
        .collect::<Vec<_>>();
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
        let content = session
            .scratch_load(&name(raw))
            .await
            .expect("load")
            .expect("retained scratch")
            .content;
        assert_eq!(content, format!("content-{index}"));
    }

    // Renaming moves the name, never the backing object identity.
    let ids_before_rename = storage.written_ids().await;
    session
        .scratch_rename(&name("设计.md"), &name("renamed.md"))
        .await
        .expect("same-source rename");
    assert_eq!(
        storage.written_ids().await,
        ids_before_rename,
        "rename must not rewrite the backing object"
    );
    let content = session
        .scratch_load(&name("renamed.md"))
        .await
        .expect("load")
        .expect("renamed scratch")
        .content;
    assert_eq!(content, "content-0");
    assert!(
        session
            .scratch_load(&name("设计.md"))
            .await
            .expect("load")
            .is_none(),
        "the old name must not survive a rename"
    );
}

#[tokio::test]
async fn scratch_names_are_sorted_and_session_scoped() {
    let first_storage = Arc::new(MemoryStorage::default());
    let first = new_session(10, Arc::clone(&first_storage));
    let second = new_session(11, Arc::new(MemoryStorage::default()));
    let guard = OperationGuard::new();

    for raw in ["zebra.md", "alpha.md", "middle.md"] {
        first
            .scratch_put(&name(raw), "x", &guard)
            .await
            .expect("scratch put");
    }
    second
        .scratch_put(&name("alpha.md"), "other", &guard)
        .await
        .expect("scratch put in a second session");

    let names = first.scratch_names().await.expect("names");
    let rendered = names.iter().map(LocalName::as_str).collect::<Vec<_>>();
    assert_eq!(
        rendered,
        vec!["alpha.md", "middle.md", "zebra.md"],
        "scratch names must enumerate in sorted order"
    );

    // The colliding name in the other session is invisible here and holds its
    // own content there.
    let content = second
        .scratch_load(&name("alpha.md"))
        .await
        .expect("load")
        .expect("second session scratch")
        .content;
    assert_eq!(content, "other");
    let content = first
        .scratch_load(&name("alpha.md"))
        .await
        .expect("load")
        .expect("first session scratch")
        .content;
    assert_eq!(content, "x");
}
