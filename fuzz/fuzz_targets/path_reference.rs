#![no_main]

use std::path::{Component, Path};

use libfuzzer_sys::fuzz_target;
use resourcefs_core::{
    ErrorCategory, PathReference, WorkspaceAddress, WorkspacePath, WorkspaceRootId,
};

fuzz_target!(|data: &[u8]| {
    assert_known_boundaries();
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };

    let first = PathReference::parse(input.to_owned());
    let second = PathReference::parse(input.to_owned());
    assert_eq!(first, second, "parsing must be deterministic");

    let encoded = input.to_ascii_lowercase();
    if encoded.contains("%2f") || encoded.contains("%5c") {
        assert!(first.is_err(), "encoded separators must be rejected");
    }

    let Ok(reference) = first else {
        return;
    };
    assert_address_is_contained(reference.literal());
    if let Some(selected) = reference.selector_candidate() {
        assert_address_is_contained(selected.base());
    }

    match reference.literal() {
        WorkspaceAddress::Relative(path) => {
            let canonical = PathReference::canonical(
                WorkspaceRootId::new("fuzz-root").expect("static root ID"),
                path.clone(),
            );
            assert_eq!(PathReference::parse(canonical.requested()), Ok(canonical));
        }
        WorkspaceAddress::Canonical { root, path } => {
            let canonical = PathReference::canonical(root.clone(), path.clone());
            assert_eq!(PathReference::parse(canonical.requested()), Ok(canonical));
        }
        WorkspaceAddress::Absolute(_) | WorkspaceAddress::FileUri(_) => {}
    }
});

fn assert_known_boundaries() {
    let single_pass = PathReference::parse("notes%253A2").expect("single-pass fixture");
    let WorkspaceAddress::Relative(path) = single_pass.literal() else {
        panic!("single-pass fixture must stay relative");
    };
    assert_eq!(path.as_path(), Path::new("notes%3A2"));

    let separator = PathReference::parse("notes%2Fsecret").expect_err("encoded separator");
    assert_eq!(separator.category(), ErrorCategory::InvalidReference);

    let canonical = PathReference::canonical(
        WorkspaceRootId::new("fuzz-root").expect("static root ID"),
        WorkspacePath::new("notes:2").expect("static workspace path"),
    );
    assert_eq!(canonical.requested(), "rfs://workspace/fuzz-root/notes%3A2");
    assert_eq!(PathReference::parse(canonical.requested()), Ok(canonical));
}

fn assert_address_is_contained(address: &WorkspaceAddress) {
    let path = match address {
        WorkspaceAddress::Relative(path) | WorkspaceAddress::Canonical { path, .. } => path,
        WorkspaceAddress::Absolute(_) | WorkspaceAddress::FileUri(_) => return,
    };
    assert!(
        path.as_path()
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    );
}
