use std::time::{Duration, Instant};

use resourcefs_core::{
    BEHAVIOR_CONTRACT_VERSION, ErrorCategory, PathReference, ReadResource, VersionTag,
    WorkspacePath, WorkspaceRootId,
};

fn reference(path: &str) -> PathReference {
    PathReference::canonical(
        WorkspaceRootId::new("workspace").expect("fixture root id should be valid"),
        WorkspacePath::new(path).expect("fixture path should be valid"),
    )
}

#[test]
fn matches_published_sha256_vectors() {
    assert_eq!(
        VersionTag::from_content(b"").as_str(),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        VersionTag::from_content(b"abc").as_str(),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn parses_only_canonical_full_tags() {
    let canonical = VersionTag::from_content(b"abc").to_string();
    assert_eq!(
        VersionTag::parse(&canonical)
            .expect("canonical tag")
            .as_str(),
        canonical
    );
    for invalid in [
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "sha256:abc",
        "sha256:BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
    ] {
        let error = VersionTag::parse(invalid).expect_err("noncanonical Version Tag");
        assert_eq!(
            error.category(),
            ErrorCategory::InvalidReference,
            "{invalid}"
        );
    }
}

#[test]
fn ignores_path_and_metadata() {
    let left = ReadResource::text(reference("left.txt"), "same bytes".to_owned())
        .expect("canonical reference");
    let right = ReadResource::text(reference("nested/right.txt"), "same bytes".to_owned())
        .expect("canonical reference");

    assert_eq!(left.version_tag(), right.version_tag());
    assert_ne!(left.canonical_reference(), right.canonical_reference());
}

#[test]
fn selected_projection_preserves_authoritative_whole_resource_tag() {
    let whole_resource_tag = VersionTag::from_content(b"alpha\nbeta\n");
    let selected = ReadResource::text_projection(
        reference("selected.txt"),
        "beta\n".to_owned(),
        whole_resource_tag.clone(),
    )
    .expect("selected projection");

    assert_eq!(selected.content(), "beta\n");
    assert_eq!(selected.version_tag(), &whole_resource_tag);
    assert_ne!(
        selected.version_tag(),
        &VersionTag::from_content(selected.content().as_bytes())
    );
}

#[test]
fn changes_when_content_changes() {
    let original = VersionTag::from_content(b"same bytes");
    let changed = VersionTag::from_content(b"same byteS");
    assert_ne!(original, changed);
}

#[test]
fn exposes_stable_read_contract_fields() {
    let resource =
        ReadResource::text(reference("empty.txt"), String::new()).expect("canonical reference");

    assert_eq!(BEHAVIOR_CONTRACT_VERSION, "1.1.0");
    assert_eq!(
        resource.canonical_reference(),
        "rfs://workspace/workspace/empty.txt"
    );
    assert_eq!(resource.content_type(), "text/plain; charset=utf-8");
    assert_eq!(resource.content(), "");
    assert!(!resource.is_mutable());
    assert!(!resource.is_bounded());
}

#[test]
fn backing_file_uri_is_absent_unless_explicitly_attached() {
    let hidden =
        ReadResource::text(reference("visible.txt"), "content".to_owned()).expect("read resource");
    assert_eq!(hidden.backing_file_uri(), None);

    let backing_uri = resourcefs_core::test_support::file_uri("visible.txt");
    let visible = hidden
        .with_backing_file_uri(backing_uri.as_str())
        .expect("local backing URI");
    assert_eq!(visible.backing_file_uri(), Some(backing_uri.as_str()));

    let invalid = visible
        .with_backing_file_uri("https://example.com/visible.txt")
        .expect_err("non-file backing URI must fail");
    assert_eq!(invalid.category(), ErrorCategory::InvalidReference);
}

#[test]
fn hashes_maximum_sized_content_within_budget() {
    let content = vec![b'x'; 48 * 1024];
    let started = Instant::now();
    let tag = VersionTag::from_content(&content);
    let elapsed = started.elapsed();

    assert!(tag.as_str().starts_with("sha256:"));
    assert_eq!(tag.as_str().len(), "sha256:".len() + 64);
    assert!(
        elapsed <= Duration::from_millis(10),
        "48 KiB hashing took {elapsed:?}"
    );
}
