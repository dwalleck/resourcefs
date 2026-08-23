//! C14 — Session Scratch selectors resolve literal-first.
//!
//! A colon is a legal scratch-name character, so an existing Resource whose
//! literal name carries the trailing text wins; only when no such Resource
//! exists is the trailing text a projection selector.

mod support;

use resourcefs_core::{LocalName, OperationGuard, PathReference, SourceAdapter};

use support::scratch_fixture;

fn name(value: &str) -> LocalName {
    LocalName::new(value).expect("valid scratch name")
}

#[tokio::test]
async fn literal_scratch_name_wins() {
    let fixture = scratch_fixture().await;
    let local = &fixture.local;
    let session = fixture.session.path_session();
    let guard = OperationGuard::new();

    // `plan.md` holds five lines; the selector reading of `plan.md:1-2` would
    // return its first two.
    session
        .scratch_put(&name("plan.md"), "one\ntwo\nthree\nfour\nfive\n", &guard)
        .await
        .expect("base scratch");

    // Expectations are held by the test, not computed from the adapter.
    const SELECTOR_READING: &str = "one\ntwo\n";
    const LITERAL_READING: &str = "a Resource literally named with a colon\n";

    // With no literal `plan.md:1-2`, the trailing text is a selector.
    let selector_reference = PathReference::parse("local://plan.md:1-2".to_owned())
        .expect("a colon-bearing scratch reference parses");
    let projected = local
        .read(&selector_reference)
        .await
        .expect("selector reading when no literal Resource exists");
    assert_eq!(
        projected.content(),
        SELECTOR_READING,
        "absent a literal Resource, the trailing text must select lines"
    );
    // The identity is the RESOLVED Resource, so a later edit finds its snapshot.
    assert_eq!(
        projected.canonical_reference(),
        PathReference::local("plan.md")
            .expect("base reference")
            .requested(),
        "a selector-bearing read must be identified by the Resource it resolved to"
    );

    // Now create the literal name. It must win.
    session
        .scratch_put(&name("plan.md:1-2"), LITERAL_READING, &guard)
        .await
        .expect("literal colon-bearing scratch");

    let literal = local
        .read(&selector_reference)
        .await
        .expect("literal reading once the Resource exists");
    assert_eq!(
        literal.content(),
        LITERAL_READING,
        "an existing literal scratch name must beat the selector interpretation"
    );
    assert_eq!(
        literal.canonical_reference(),
        PathReference::local("plan.md:1-2")
            .expect("literal reference")
            .requested(),
        "the literal reading is identified by the literal Resource"
    );
}
