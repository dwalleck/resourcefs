//! C4 — mutation dispatch routes by the target's own source, both directions.
//! C5 — `artifact://` and catalog references stay immutable.
//!
//! Oracle: observable per-source state (which store actually changed) plus a
//! literal reference→category table — never the dispatch match itself.

mod support;

use resourcefs_core::{
    ErrorCategory, LocalName, MutationAccess, MutationAdapter, OperationGuard, PathReference,
    SourceAdapter, WriteRequest,
};

use support::scratch_fixture;

#[tokio::test]
async fn local_never_touches_filesystem() {
    let fixture = scratch_fixture().await;

    // A workspace file of the same base name exists in the granted root; if a
    // `local://` mutation were routed to the filesystem adapter it would load or
    // overwrite that file instead of scratch.
    let workspace_before =
        std::fs::read_to_string(fixture.granted_root.path().join("tracked.txt")).expect("read");

    fixture
        .engine
        .write(
            WriteRequest::new(
                PathReference::local("tracked.txt").expect("scratch reference"),
                "scratch content, not workspace content\n".to_owned(),
                None,
            )
            .expect("create request"),
            &OperationGuard::new(),
        )
        .await
        .expect("scratch create");

    // The scratch store changed...
    let scratch = fixture
        .session
        .path_session()
        .scratch_load(&LocalName::new("tracked.txt").expect("name"))
        .await
        .expect("scratch load")
        .expect("scratch exists");
    assert_eq!(scratch.content, "scratch content, not workspace content\n");

    // ...and the filesystem did not.
    let workspace_after =
        std::fs::read_to_string(fixture.granted_root.path().join("tracked.txt")).expect("read");
    assert_eq!(
        workspace_after, workspace_before,
        "a local:// mutation must never reach the filesystem adapter"
    );
}

#[tokio::test]
async fn workspace_never_touches_local() {
    let fixture = scratch_fixture().await;

    // Same name on both sides again, this time mutating the workspace.
    fixture
        .session
        .path_session()
        .scratch_put(
            &LocalName::new("tracked.txt").expect("name"),
            "scratch sentinel\n",
            &OperationGuard::new(),
        )
        .await
        .expect("scratch seed");

    fixture
        .engine
        .write(
            WriteRequest::new(
                PathReference::parse("rfs://workspace/granted/tracked.txt".to_owned())
                    .expect("workspace reference"),
                "workspace content, not scratch content\n".to_owned(),
                Some(resourcefs_core::VersionTag::from_content(
                    b"granted bytes\n",
                )),
            )
            .expect("replace request"),
            &OperationGuard::new(),
        )
        .await
        .expect("workspace replace under a granted root");

    // The workspace changed...
    let workspace =
        std::fs::read_to_string(fixture.granted_root.path().join("tracked.txt")).expect("read");
    assert_eq!(workspace, "workspace content, not scratch content\n");

    // ...and scratch did not.
    let scratch = fixture
        .session
        .path_session()
        .scratch_load(&LocalName::new("tracked.txt").expect("name"))
        .await
        .expect("scratch load")
        .expect("scratch exists");
    assert_eq!(
        scratch.content, "scratch sentinel\n",
        "a workspace mutation must never reach the Local adapter"
    );
}

#[tokio::test]
async fn artifact_and_catalog_immutable() {
    let fixture = scratch_fixture().await;

    let artifact = fixture
        .session
        .path_session()
        .retain("recoverable bytes\n", &OperationGuard::new())
        .await
        .expect("artifact");
    let artifact_reference = PathReference::artifact(artifact, None).expect("artifact reference");

    // Literal reference → expected category, authored from the spec.
    let immutable: [(&PathReference, ErrorCategory); 3] = [
        (&artifact_reference, ErrorCategory::PermissionDenied),
        (
            &PathReference::parse("rfs://".to_owned()).expect("source catalog"),
            ErrorCategory::PermissionDenied,
        ),
        (
            &PathReference::parse("rfs://workspace".to_owned()).expect("workspace catalog"),
            ErrorCategory::PermissionDenied,
        ),
    ];

    // Every access on every immutable reference is denied at resolve...
    for (reference, expected) in immutable {
        for access in [
            MutationAccess::Create,
            MutationAccess::Update,
            MutationAccess::Delete,
        ] {
            let error = fixture
                .compiled
                .resolve(reference, access)
                .await
                .expect_err("an immutable Resource must not resolve as a mutation target");
            assert_eq!(
                error.category(),
                expected,
                "{} must stay immutable",
                reference.requested()
            );
        }
    }

    // ...and through the public engine, with no state change.
    for (reference, expected) in immutable {
        let error = fixture
            .engine
            .write(
                WriteRequest::new(reference.clone(), "forbidden\n".to_owned(), None)
                    .expect("write request"),
                &OperationGuard::new(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.category(),
            expected,
            "{} must stay immutable",
            reference.requested()
        );
    }

    // The artifact still reads its original bytes.
    let survived = fixture
        .compiled
        .read(&artifact_reference, &OperationGuard::new())
        .await
        .expect("artifact still readable");
    assert_eq!(survived.content(), "recoverable bytes\n");
}
