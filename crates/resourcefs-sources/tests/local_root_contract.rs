//! C13 — the bare `local://` reference is the Session Scratch family root.
//!
//! The listing must name exactly this session's scratch, sorted, with every
//! entry a reference that re-parses; it is read-only, and a scratch-free session
//! still returns non-empty content.

mod support;

use std::collections::BTreeSet;

use resourcefs_core::{
    ErrorCategory, MutationAccess, MutationAdapter, OperationGuard, PathReference, SourceAdapter,
};

use support::scratch_fixture;

/// Names the fixture wrote, collected independently of the adapter's listing.
fn expected_entries(created: &BTreeSet<String>) -> Vec<String> {
    created
        .iter()
        .map(|name| {
            PathReference::local(name.as_str())
                .expect("fixture names are valid scratch names")
                .requested()
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn root_lists_all_scratch_sorted() {
    // Bind the whole fixture: destructuring would drop its TempDir guards and
    // delete the session cache out from under the store.
    let fixture = scratch_fixture().await;
    let local = &fixture.local;
    let session = &fixture.session;
    let guard = OperationGuard::new();

    // Zero scratch: the listing is still non-empty and says so explicitly.
    let empty = local
        .read(&PathReference::local_root(), &OperationGuard::new(), None)
        .await
        .expect("root listing with no scratch");
    assert!(
        !empty.content().trim().is_empty(),
        "an empty scratch session must still render a status line"
    );
    assert!(
        empty.content().contains("No Session Scratch Resources"),
        "empty listing must name the empty state: {}",
        empty.content()
    );
    // Entries are the lines that *begin* a reference; the header's usage hint
    // legitimately mentions `local://<name>`.
    assert!(
        empty
            .content()
            .lines()
            .all(|line| !line.starts_with("local://")),
        "an empty listing must not name any scratch entry"
    );

    // Deliberately created out of sorted order, with a space and a colon.
    let mut created = BTreeSet::new();
    for name in ["zebra.md", "alpha.md", "review notes.md", "plan.md:1-5"] {
        session
            .path_session()
            .scratch_put(
                &resourcefs_core::LocalName::new(name).expect("scratch name"),
                &format!("content of {name}\n"),
                &guard,
            )
            .await
            .expect("scratch put");
        created.insert(name.to_owned());
    }

    let listing = local
        .read(&PathReference::local_root(), &OperationGuard::new(), None)
        .await
        .expect("root listing");

    let rendered = listing
        .content()
        .lines()
        .filter(|line| line.starts_with("local://"))
        .map(str::to_owned)
        .collect::<Vec<_>>();

    // Oracle: the BTreeSet is sorted by construction and never consults the
    // adapter's rendering order.
    assert_eq!(
        rendered,
        expected_entries(&created),
        "the listing must be exactly this session's scratch, sorted by name"
    );

    // Every rendered entry must re-parse and address the Resource it names.
    for entry in &rendered {
        let reference = PathReference::parse(entry.clone())
            .unwrap_or_else(|error| panic!("rendered entry {entry} must re-parse: {error}"));
        local
            .read(&reference, &OperationGuard::new(), None)
            .await
            .unwrap_or_else(|error| panic!("rendered entry {entry} must resolve: {error}"));
    }
}

#[tokio::test]
async fn root_is_immutable() {
    let fixture = scratch_fixture().await;
    let local = &fixture.local;

    // The listing itself reports read-only, unlike a named scratch Resource.
    let listing = local
        .read(&PathReference::local_root(), &OperationGuard::new(), None)
        .await
        .expect("root listing");
    assert!(
        !listing.is_mutable(),
        "the scratch listing is a synthetic projection and must report mutable:false"
    );

    for access in [
        MutationAccess::Create,
        MutationAccess::Update,
        MutationAccess::Delete,
    ] {
        let error = local
            .resolve(&PathReference::local_root(), access)
            .await
            .expect_err("the family root is not a mutation target");
        assert_eq!(
            error.category(),
            ErrorCategory::PermissionDenied,
            "mutating the listing must be denied, not merely unsupported"
        );
    }
}
