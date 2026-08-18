use std::{
    fs,
    time::{Duration, Instant},
};

use resourcefs_core::{ErrorCategory, MAX_TEXT_BYTES, PathReference, RootName, SourceAdapter};
use resourcefs_sources::FilesystemSource;
use tempfile::TempDir;

fn root_name() -> RootName {
    RootName::new("workspace").expect("fixture root name should be valid")
}

fn reference(path: &str) -> PathReference {
    PathReference::parse(path, &root_name()).expect("fixture reference should be valid")
}

fn create_root() -> (TempDir, std::path::PathBuf) {
    let temporary = TempDir::new().expect("temporary directory");
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).expect("workspace root");
    (temporary, root)
}

#[tokio::test]
async fn reads_utf8_empty_and_unicode_resources() {
    let (_temporary, root) = create_root();
    fs::create_dir(root.join("notes")).expect("notes directory");
    fs::write(root.join("notes/ résumé final.txt "), "héllo\n").expect("unicode fixture");
    fs::write(root.join("empty.txt"), "").expect("empty fixture");
    let source = FilesystemSource::new(root_name(), &root)
        .await
        .expect("filesystem source");

    let unicode = source
        .read(&reference("notes/ résumé final.txt "))
        .await
        .expect("unicode read");
    assert_eq!(unicode.content(), "héllo\n");
    assert_eq!(
        unicode.canonical_reference(),
        "rfs://workspace/workspace/notes/ résumé final.txt "
    );

    let empty = source
        .read(&reference("empty.txt"))
        .await
        .expect("empty read");
    assert_eq!(empty.content(), "");
}

#[tokio::test]
async fn maps_missing_directory_and_binary_resources() {
    let (_temporary, root) = create_root();
    fs::create_dir(root.join("directory")).expect("directory fixture");
    fs::write(root.join("binary.bin"), [0xff, 0xfe]).expect("binary fixture");
    let source = FilesystemSource::new(root_name(), &root)
        .await
        .expect("filesystem source");

    let missing = source
        .read(&reference("missing.txt"))
        .await
        .expect_err("missing should fail");
    assert_eq!(missing.category(), ErrorCategory::NotFound);

    let directory = source
        .read(&reference("directory"))
        .await
        .expect_err("directory should fail");
    assert_eq!(directory.category(), ErrorCategory::UnsupportedProjection);

    let binary = source
        .read(&reference("binary.bin"))
        .await
        .expect_err("binary should fail");
    assert_eq!(binary.category(), ErrorCategory::UnsupportedProjection);
}

#[tokio::test]
async fn rejects_invalid_root_configuration() {
    let temporary = TempDir::new().expect("temporary directory");
    let missing = FilesystemSource::new(root_name(), temporary.path().join("missing"))
        .await
        .expect_err("missing root should fail");
    assert_eq!(missing.category(), ErrorCategory::SourceUnavailable);

    let file = temporary.path().join("file.txt");
    fs::write(&file, "not a directory").expect("file root fixture");
    let not_directory = FilesystemSource::new(root_name(), &file)
        .await
        .expect_err("file root should fail");
    assert_eq!(not_directory.category(), ErrorCategory::SourceUnavailable);
}

#[cfg(unix)]
#[tokio::test]
async fn permits_contained_symlink() {
    use std::os::unix::fs::symlink;

    let (_temporary, root) = create_root();
    fs::write(root.join("target.txt"), "inside").expect("inside fixture");
    symlink(root.join("target.txt"), root.join("link.txt")).expect("contained symlink");
    let source = FilesystemSource::new(root_name(), &root)
        .await
        .expect("filesystem source");

    let resource = source
        .read(&reference("link.txt"))
        .await
        .expect("contained link read");
    assert_eq!(resource.content(), "inside");
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let (temporary, root) = create_root();
    let outside = temporary.path().join("outside.txt");
    fs::write(&outside, "outside secret").expect("outside fixture");
    symlink(&outside, root.join("escape.txt")).expect("escaping symlink");
    let source = FilesystemSource::new(root_name(), &root)
        .await
        .expect("filesystem source");

    let error = source
        .read(&reference("escape.txt"))
        .await
        .expect_err("escaping link should fail");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    assert!(!error.message().contains("outside secret"));
}

fn exact_byte_limit_content() -> Vec<u8> {
    let mut content = Vec::with_capacity(MAX_TEXT_BYTES);
    for _ in 0..96 {
        content.extend(std::iter::repeat_n(b'x', 511));
        content.push(b'\n');
    }
    assert_eq!(content.len(), MAX_TEXT_BYTES);
    content
}

async fn assert_limit_exceeded(source: &FilesystemSource, path: &str) {
    let error = source
        .read(&reference(path))
        .await
        .expect_err("one-over fixture should fail");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
}

#[tokio::test]
async fn enforces_hard_read_limits() {
    let (_temporary, root) = create_root();
    let exact = exact_byte_limit_content();
    fs::write(root.join("exact.txt"), &exact).expect("exact-byte fixture");

    let mut bytes_over = exact.clone();
    bytes_over.push(b'x');
    fs::write(root.join("bytes-over.txt"), bytes_over).expect("byte-over fixture");
    fs::write(root.join("lines-over.txt"), "x\n".repeat(3_001)).expect("line-over fixture");
    fs::write(root.join("columns-over.txt"), "x".repeat(513)).expect("column-over fixture");

    let source = FilesystemSource::new(root_name(), &root)
        .await
        .expect("filesystem source");
    let exact_resource = source
        .read(&reference("exact.txt"))
        .await
        .expect("exact limit should pass");
    assert_eq!(exact_resource.content().as_bytes(), exact);

    assert_limit_exceeded(&source, "bytes-over.txt").await;
    assert_limit_exceeded(&source, "lines-over.txt").await;
    assert_limit_exceeded(&source, "columns-over.txt").await;
}

#[tokio::test]
async fn reads_maximum_sized_file_within_budget() {
    let (_temporary, root) = create_root();
    let exact = exact_byte_limit_content();
    fs::write(root.join("maximum.txt"), &exact).expect("maximum fixture");
    let source = FilesystemSource::new(root_name(), &root)
        .await
        .expect("filesystem source");

    let started = Instant::now();
    let resource = source
        .read(&reference("maximum.txt"))
        .await
        .expect("maximum read");
    let elapsed = started.elapsed();

    assert_eq!(resource.content().as_bytes(), exact);
    assert!(
        elapsed <= Duration::from_millis(100),
        "maximum-sized read took {elapsed:?}"
    );
}
