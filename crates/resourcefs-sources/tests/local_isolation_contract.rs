//! C9 — Session Scratch is per-Path-Session, and disconnect ends it.
//!
//! Two sessions share one `SessionStore`, one sessions root, and one Workspace
//! Root, so the only thing separating their scratch is the per-`SessionState`
//! keying this contract fences. Both sessions deliberately use the *same*
//! scratch name: if scratch were keyed anywhere else, the collision is what
//! makes that visible.

mod support;

use std::collections::BTreeMap;

use resourcefs_core::{
    GlobLimits, GlobOptions, GlobRequest, GlobTarget, MutationOperation, OperationGuard,
    PathReference, ReadRequest, SearchLimits, SearchOptions, SearchRequest, SearchTarget,
    SourceAdapter, TextLimits, WriteRequest,
};

use support::{ScratchSession, scratch_fixture_pair};

/// The name both sessions use. A collision is the point.
const SHARED: &str = "shared.md";

/// Sentinels are distinct per session and never derived from the store, so the
/// expectation is independent of how scratch happens to be keyed.
const A_SENTINEL: &str = "alpha-only-content\n";
const B_SENTINEL: &str = "bravo-only-content\n";

async fn create(session: &ScratchSession, name: &str, content: &str) {
    session
        .engine
        .write(
            WriteRequest::new(
                PathReference::local(name).expect("scratch reference"),
                content.to_owned(),
                None,
            )
            .expect("create request"),
            &OperationGuard::new(),
        )
        .await
        .unwrap_or_else(|error| panic!("create {name}: {error}"));
}

async fn read_scratch(session: &ScratchSession, name: &str) -> String {
    session
        .local
        .read(
            &PathReference::local(name).expect("scratch reference"),
            &OperationGuard::new(),
        )
        .await
        .unwrap_or_else(|error| panic!("read {name}: {error}"))
        .content()
        .to_owned()
}

async fn search_all_scratch(session: &ScratchSession, pattern: &str) -> String {
    session
        .discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(PathReference::local_root()),
                pattern,
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("search request"),
            &OperationGuard::new(),
        )
        .await
        .expect("scratch search")
        .text()
        .to_owned()
}

async fn glob_all_scratch(session: &ScratchSession) -> String {
    session
        .discovery
        .glob(
            GlobRequest::new(
                GlobTarget::new("local://*").expect("scratch glob"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("scratch glob")
        .text()
        .to_owned()
}

#[tokio::test]
async fn sessions_never_cross() {
    let pair = scratch_fixture_pair().await;

    // Same name in both sessions, plus one name unique to each.
    create(&pair.a, SHARED, A_SENTINEL).await;
    create(&pair.b, SHARED, B_SENTINEL).await;
    create(&pair.a, "only-alpha.md", A_SENTINEL).await;
    create(&pair.b, "only-bravo.md", B_SENTINEL).await;

    // Oracle: what each session must see, built by the test from what it wrote.
    let expected: BTreeMap<&str, (&str, &str, &str)> = BTreeMap::from([
        ("a", (A_SENTINEL, "only-alpha.md", "only-bravo.md")),
        ("b", (B_SENTINEL, "only-bravo.md", "only-alpha.md")),
    ]);

    for (label, session) in [("a", &pair.a), ("b", &pair.b)] {
        let (own_content, own_name, foreign_name) = expected[label];
        let foreign_content = if label == "a" { B_SENTINEL } else { A_SENTINEL };

        // read
        assert_eq!(
            read_scratch(session, SHARED).await,
            own_content,
            "session {label} must read its own {SHARED}"
        );

        // search — the other session's sentinel must never match
        let own_hits = search_all_scratch(session, own_content.trim()).await;
        assert!(
            own_hits.contains(SHARED),
            "session {label} must find its own content: {own_hits}"
        );
        let foreign_hits = search_all_scratch(session, foreign_content.trim()).await;
        assert!(
            !foreign_hits.contains(SHARED) && !foreign_hits.contains(foreign_name),
            "session {label} must not match the other session's content: {foreign_hits}"
        );

        // glob — only this session's names appear
        let names = glob_all_scratch(session).await;
        assert!(
            names.contains(own_name),
            "session {label} must glob its own name: {names}"
        );
        assert!(
            !names.contains(foreign_name),
            "session {label} must not glob the other session's name: {names}"
        );

        // root listing — same boundary through the family root
        let listing = session
            .local
            .read(&PathReference::local_root(), &OperationGuard::new())
            .await
            .expect("root listing")
            .content()
            .to_owned();
        assert!(
            listing.contains(own_name) && !listing.contains(foreign_name),
            "session {label} listing must name only its own scratch: {listing}"
        );
    }

    // mutation — A replaces the shared name; B's Resource is untouched.
    let a_tag = pair
        .a
        .local
        .read(
            &PathReference::local(SHARED).expect("scratch reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("A reads its own shared scratch")
        .version_tag()
        .clone();
    let replaced = pair
        .a
        .engine
        .write(
            WriteRequest::new(
                PathReference::local(SHARED).expect("scratch reference"),
                "alpha-replaced\n".to_owned(),
                Some(a_tag),
            )
            .expect("replace request"),
            &OperationGuard::new(),
        )
        .await
        .expect("A replaces its own scratch");
    assert_eq!(replaced.operation(), MutationOperation::Replaced);

    assert_eq!(
        read_scratch(&pair.a, SHARED).await,
        "alpha-replaced\n",
        "A must observe its own replacement"
    );
    assert_eq!(
        read_scratch(&pair.b, SHARED).await,
        B_SENTINEL,
        "B's identically named scratch must be untouched by A's mutation"
    );
}

#[tokio::test]
async fn disconnect_invalidates() {
    let pair = scratch_fixture_pair().await;
    create(&pair.a, SHARED, A_SENTINEL).await;
    create(&pair.b, SHARED, B_SENTINEL).await;

    let a_dir = pair.a.storage_dir(&pair.sessions_root);
    let b_dir = pair.b.storage_dir(&pair.sessions_root);
    assert!(a_dir.is_dir() && b_dir.is_dir(), "both sessions are live");

    pair.a
        .session
        .mark_disconnected()
        .await
        .expect("A disconnects");

    // Every scratch surface fails for A, immediately.
    let read = pair
        .a
        .local
        .read(
            &PathReference::local(SHARED).expect("scratch reference"),
            &OperationGuard::new(),
        )
        .await;
    assert!(read.is_err(), "a disconnected session cannot read scratch");

    let listing = pair
        .a
        .local
        .read(&PathReference::local_root(), &OperationGuard::new())
        .await;
    assert!(
        listing.is_err(),
        "a disconnected session cannot list scratch"
    );

    let engine_read = pair
        .a
        .reads
        .read(
            ReadRequest {
                reference: PathReference::local(SHARED).expect("scratch reference"),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await;
    assert!(
        engine_read.is_err(),
        "a disconnected session cannot read through the engine"
    );

    let mutation = pair
        .a
        .engine
        .write(
            WriteRequest::new(
                PathReference::local("post-disconnect.md").expect("scratch reference"),
                "should not land\n".to_owned(),
                None,
            )
            .expect("create request"),
            &OperationGuard::new(),
        )
        .await;
    assert!(
        mutation.is_err(),
        "a disconnected session cannot mutate scratch"
    );

    // B is entirely unaffected.
    assert_eq!(
        read_scratch(&pair.b, SHARED).await,
        B_SENTINEL,
        "one session's disconnect must not disturb another"
    );
    create(&pair.b, "after-a-left.md", B_SENTINEL).await;

    // A's backing state is TTL-eligible; B's is not.
    let past_ttl = std::time::SystemTime::now() + pair.retention_ttl + pair.retention_ttl;
    pair.store
        .cleanup_expired_at_for_test(past_ttl)
        .await
        .expect("TTL sweep");
    assert!(
        !a_dir.exists(),
        "a disconnected session's backing state must be TTL-eligible"
    );
    assert!(
        b_dir.is_dir(),
        "a live session's backing state must survive the sweep"
    );
}
