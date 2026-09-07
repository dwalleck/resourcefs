//! Hermetic fixtures shared by test targets through the `test-support` feature.

use std::path::{Path, PathBuf};

use url::Url;

use crate::WorkspacePath;

fn fixture_path(relative_path: &str) -> PathBuf {
    let relative_path = WorkspacePath::new(relative_path).expect("valid relative fixture path");
    #[cfg(windows)]
    let root = Path::new(r"C:\resourcefs-fixtures");
    #[cfg(not(windows))]
    let root = Path::new("/resourcefs-fixtures");
    root.join(relative_path.as_path())
}

/// Returns a native-absolute file URI without accessing the environment or filesystem.
///
/// The path is synthetic: callers must not use it for filesystem I/O.
/// Panics if `relative_path` is not a valid nonempty Workspace Path.
pub fn file_uri(relative_path: &str) -> Url {
    Url::from_file_path(fixture_path(relative_path)).expect("absolute fixture file path")
}

/// Returns a native-absolute directory URI, including its trailing slash.
///
/// Like [`file_uri`], this uses a synthetic path and performs no filesystem I/O.
/// Panics if `relative_path` is not a valid nonempty Workspace Path.
pub fn directory_uri(relative_path: &str) -> Url {
    Url::from_directory_path(fixture_path(relative_path)).expect("absolute fixture directory path")
}
