//! C3 — Session Scratch mutation is authorized by Path Session ownership.
//! C8 — a cross-source `MV` is rejected with both sides unchanged.
//!
//! The fixture's Workspace Root carries **no** mutation grants, so every scratch
//! operation that succeeds here proves session authority rather than a grant.

mod support;

use resourcefs_core::{
    ErrorCategory, MutationOperation, OperationGuard, PathReference, SourceAdapter, WriteRequest,
};

use support::scratch_fixture;

/// Independent truth table: which operations must succeed on scratch with zero
/// grants, and which must be denied on the workspace under the same session.
const SCRATCH_MUST_SUCCEED: &[&str] = &["create", "replace", "edit", "delete", "move"];

#[tokio::test]
async fn scratch_needs_no_grant() {
    let fixture = scratch_fixture().await;
    let engine = &fixture.engine;
    let mut proven = Vec::new();

    // create
    let created = engine
        .write(
            WriteRequest::new(
                PathReference::local("plan.md").expect("scratch reference"),
                "one\ntwo\nthree\n".to_owned(),
                None,
                None,
            )
            .expect("create request"),
            &OperationGuard::new(),
        )
        .await
        .expect("scratch create needs no Workspace Mutation grant");
    assert_eq!(created.operation(), MutationOperation::Created);
    proven.push("create");

    // read back through the adapter to confirm the bytes landed
    let read = fixture
        .local
        .read(
            &PathReference::local("plan.md").expect("scratch reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("scratch read");
    assert_eq!(read.content(), "one\ntwo\nthree\n");
    assert!(read.is_mutable(), "named scratch must report mutable:true");

    // replace (ifVersion)
    let replaced = engine
        .write(
            WriteRequest::new(
                PathReference::local("plan.md").expect("scratch reference"),
                "ONE\ntwo\nthree\n".to_owned(),
                Some(created.version_tag().expect("created tag").clone()),
                None,
            )
            .expect("replace request"),
            &OperationGuard::new(),
        )
        .await
        .expect("scratch replace needs no grant");
    assert_eq!(replaced.operation(), MutationOperation::Replaced);
    proven.push("replace");

    // edit (hashline PUT against the seen region recorded by the read above)
    let document = format!(
        "[{}#{}]\nPUT 2.=2:\n+TWO\n",
        PathReference::local("plan.md")
            .expect("scratch reference")
            .requested(),
        replaced.version_tag().expect("replaced tag").as_str()
    );
    // The edit must be preceded by a real read: ReadEngine is what records the
    // seen region, keyed by the resolved Resource's canonical reference.
    fixture
        .reads
        .read(
            resourcefs_core::ReadRequest {
                reference: PathReference::local("plan.md").expect("scratch reference"),
                limits: resourcefs_core::TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("pre-edit read records the seen region");
    let edited = engine
        .edit(&document, &OperationGuard::new())
        .await
        .expect("scratch edit needs no grant");
    assert_eq!(edited.operation(), MutationOperation::Edited);
    proven.push("edit");

    // move (same-source, no-clobber)
    let move_document = format!(
        "[{}#{}]\nMV {}\n",
        PathReference::local("plan.md")
            .expect("scratch reference")
            .requested(),
        edited.version_tag().expect("edited tag").as_str(),
        PathReference::local("final.md")
            .expect("destination")
            .requested()
    );
    let moved = engine
        .edit(&move_document, &OperationGuard::new())
        .await
        .expect("scratch move needs no grant");
    assert_eq!(moved.operation(), MutationOperation::Moved);
    proven.push("move");

    // delete — REM validates against a seen snapshot, so the moved Resource
    // must be read at its current tag first.
    fixture
        .reads
        .read(
            resourcefs_core::ReadRequest {
                reference: PathReference::local("final.md").expect("destination"),
                limits: resourcefs_core::TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("pre-delete read records the seen region");
    let delete_document = format!(
        "[{}#{}]\nREM\n",
        PathReference::local("final.md")
            .expect("destination")
            .requested(),
        moved.version_tag().expect("moved tag").as_str()
    );
    let deleted = engine
        .edit(&delete_document, &OperationGuard::new())
        .await
        .expect("scratch delete needs no grant");
    assert_eq!(deleted.operation(), MutationOperation::Deleted);
    proven.push("delete");

    proven.sort_unstable();
    let mut expected = SCRATCH_MUST_SUCCEED.to_vec();
    expected.sort_unstable();
    assert_eq!(
        proven, expected,
        "every scratch operation must succeed under session authority alone"
    );

    // The same session cannot mutate the workspace: no grant was configured.
    let denied = engine
        .write(
            WriteRequest::new(
                PathReference::parse("rfs://workspace/workspace/tracked.txt".to_owned())
                    .expect("workspace reference"),
                "forbidden\n".to_owned(),
                None,
                None,
            )
            .expect("workspace write request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("workspace mutation without a grant must be denied");
    assert_eq!(
        denied.category(),
        ErrorCategory::PermissionDenied,
        "scratch authority must not leak into the workspace"
    );
}

#[tokio::test]
async fn cross_source_mv_rejected() {
    let fixture = scratch_fixture().await;
    let engine = &fixture.engine;
    let created = engine
        .write(
            WriteRequest::new(
                PathReference::local("source.md").expect("scratch reference"),
                "scratch bytes\n".to_owned(),
                None,
                None,
            )
            .expect("create request"),
            &OperationGuard::new(),
        )
        .await
        .expect("scratch create");

    // scratch -> workspace
    let document = format!(
        "[{}#{}]\nMV rfs://workspace/granted/moved.txt\n",
        PathReference::local("source.md")
            .expect("scratch reference")
            .requested(),
        created.version_tag().expect("created tag").as_str()
    );
    let error = engine
        .edit(&document, &OperationGuard::new())
        .await
        .expect_err("a cross-source move must be rejected");
    assert_eq!(error.category(), ErrorCategory::UnsupportedMutation);

    // Both sides unchanged: the scratch Resource still reads its original bytes
    // and the workspace destination was never created.
    let survived = fixture
        .local
        .read(
            &PathReference::local("source.md").expect("scratch reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("source scratch survives a rejected move");
    assert_eq!(survived.content(), "scratch bytes\n");
    assert!(
        !fixture.granted_root.path().join("moved.txt").exists(),
        "a rejected cross-source move must not create the destination"
    );
}
