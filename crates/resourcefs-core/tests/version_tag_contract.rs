use std::time::{Duration, Instant};

use resourcefs_core::{
    BEHAVIOR_CONTRACT_VERSION, PathReference, ReadResource, RootName, VersionTag,
};

fn reference(path: &str) -> PathReference {
    PathReference::parse(
        path,
        &RootName::new("workspace").expect("fixture root name should be valid"),
    )
    .expect("fixture path should be valid")
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
fn ignores_path_and_metadata() {
    let left = ReadResource::text(reference("left.txt"), "same bytes".to_owned());
    let right = ReadResource::text(reference("nested/right.txt"), "same bytes".to_owned());

    assert_eq!(left.version_tag(), right.version_tag());
    assert_ne!(left.canonical_reference(), right.canonical_reference());
}

#[test]
fn changes_when_content_changes() {
    let original = VersionTag::from_content(b"same bytes");
    let changed = VersionTag::from_content(b"same byteS");
    assert_ne!(original, changed);
}

#[test]
fn exposes_stable_read_contract_fields() {
    let resource = ReadResource::text(reference("empty.txt"), String::new());

    assert_eq!(BEHAVIOR_CONTRACT_VERSION, "1.0.0");
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
