//! C10 — concurrent mutations of one scratch Resource serialize.
//!
//! Two writers race the same name from the same read tag. Exactly one may
//! commit; the loser must see `version_conflict`, and the surviving content must
//! be exactly one writer's bytes — never a blend.

mod support;

use resourcefs_core::{ErrorCategory, LocalName, OperationGuard, PathReference, WriteRequest};

use support::scratch_fixture;

#[tokio::test]
async fn same_name_serializes() {
    let fixture = scratch_fixture().await;
    let engine = fixture.engine.clone();

    let created = engine
        .write(
            WriteRequest::new(
                PathReference::local("contended.md").expect("scratch reference"),
                "original\n".to_owned(),
                None,
            )
            .expect("create request"),
            &OperationGuard::new(),
        )
        .await
        .expect("scratch create");
    let stale = created.version_tag().expect("created tag").clone();

    // Both writers present the SAME tag: whichever commits second is stale.
    const FIRST: &str = "first writer\n";
    const SECOND: &str = "second writer\n";
    let one = {
        let engine = engine.clone();
        let tag = stale.clone();
        tokio::spawn(async move {
            engine
                .write(
                    WriteRequest::new(
                        PathReference::local("contended.md").expect("scratch reference"),
                        FIRST.to_owned(),
                        Some(tag),
                    )
                    .expect("replace request"),
                    &OperationGuard::new(),
                )
                .await
        })
    };
    let two = {
        let engine = engine.clone();
        let tag = stale.clone();
        tokio::spawn(async move {
            engine
                .write(
                    WriteRequest::new(
                        PathReference::local("contended.md").expect("scratch reference"),
                        SECOND.to_owned(),
                        Some(tag),
                    )
                    .expect("replace request"),
                    &OperationGuard::new(),
                )
                .await
        })
    };

    let results = [one.await.expect("join"), two.await.expect("join")];
    let winners = results.iter().filter(|result| result.is_ok()).count();
    let conflicts = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .filter(|error| error.category() == ErrorCategory::VersionConflict)
        .count();

    // Oracle: independent counting of outcomes, not the lock registry.
    assert_eq!(winners, 1, "exactly one stale-tag writer may commit");
    assert_eq!(conflicts, 1, "the losing writer must see version_conflict");

    // No torn content: the survivor is exactly one writer's bytes.
    let survivor = fixture
        .session
        .path_session()
        .scratch_load(&LocalName::new("contended.md").expect("name"))
        .await
        .expect("scratch load")
        .expect("scratch exists");
    assert!(
        survivor.content == FIRST || survivor.content == SECOND,
        "content must be exactly one writer's bytes, found {:?}",
        survivor.content
    );
}
