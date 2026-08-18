use std::time::{Duration, Instant};

use resourcefs_core::{ErrorCategory, PathReference, RootName};

fn root() -> RootName {
    RootName::new("workspace").expect("fixture root name should be valid")
}

#[test]
fn accepts_relative_and_canonical_workspace_references() {
    let relative = PathReference::parse("src/hello.txt", &root()).expect("relative reference");
    assert_eq!(relative.root().as_str(), "workspace");
    assert_eq!(relative.relative_path().to_string_lossy(), "src/hello.txt");
    assert_eq!(
        relative.canonical(),
        "rfs://workspace/workspace/src/hello.txt"
    );

    let canonical = PathReference::parse("rfs://workspace/workspace/src/hello.txt", &root())
        .expect("canonical reference");
    assert_eq!(canonical, relative);
}

#[test]
fn preserves_unicode_spaces_and_does_not_trim() {
    let reference = PathReference::parse("notes/ résumé final.txt ", &root())
        .expect("unicode reference should be valid");
    assert_eq!(
        reference.relative_path().to_string_lossy(),
        "notes/ résumé final.txt "
    );
}

#[test]
fn rejects_parent_traversal() {
    let error = PathReference::parse("../secret", &root()).expect_err("traversal must fail");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
}

#[test]
fn rejects_invalid_and_foreign_references() {
    for input in [
        "",
        ".",
        "rfs://workspace",
        "rfs://workspace/other/file.txt",
        "https://example.com/file.txt",
        "/etc/passwd",
        "C:\\Windows\\system.ini",
        "\\\\server\\share\\file.txt",
    ] {
        let error = PathReference::parse(input, &root()).expect_err("reference should be rejected");
        assert_eq!(
            error.category(),
            ErrorCategory::InvalidReference,
            "unexpected category for {input:?}"
        );
    }
}

#[test]
fn validates_root_names() {
    assert_eq!(
        RootName::new("workspace-1.0").expect("valid").as_str(),
        "workspace-1.0"
    );
    for invalid in ["", "a/b", "a:b", "a?b", "a#b", "a=b"] {
        let error = RootName::new(invalid).expect_err("root name should fail");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }
}

#[test]
fn parses_production_shaped_reference_within_budget() {
    let file_name = format!("{} file.txt", "é".repeat(2_000));
    let started = Instant::now();
    let reference = PathReference::parse(&file_name, &root()).expect("long unicode path");
    let elapsed = started.elapsed();

    assert_eq!(reference.relative_path().to_string_lossy(), file_name);
    assert!(
        elapsed <= Duration::from_millis(5),
        "4 KiB reference parsing took {elapsed:?}"
    );
}
