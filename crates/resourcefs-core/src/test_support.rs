//! Hermetic fixtures shared by test targets through the `test-support` feature.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use tokio::sync::Mutex;
use url::Url;

use crate::{ArtifactId, ErrorCategory, ResourceError, SessionStorage, WorkspacePath};

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

/// An in-memory [`SessionStorage`] that records what it was handed.
///
/// Six contract targets across two crates each wrote their own copy of this
/// fake, differing only in field names, error wording, and which one
/// observation they exposed (rfs-exoi). This one exposes all four. A missing
/// object is `not_found` and non-UTF-8 content is `source_unavailable`; no test
/// asserts on either message. `path_session_contract.rs` keeps its own richer
/// fake (call log, injected write failure) on purpose.
#[derive(Default)]
pub struct MemoryStorage {
    content: Mutex<HashMap<ArtifactId, Vec<u8>>>,
    written: Mutex<Vec<ArtifactId>>,
}

impl MemoryStorage {
    /// How many `write_atomic` calls reached the store.
    pub async fn write_calls(&self) -> usize {
        self.written.lock().await.len()
    }

    /// Every id the store was handed, in write order.
    pub async fn written_ids(&self) -> Vec<u64> {
        self.written
            .lock()
            .await
            .iter()
            .map(|id| id.get())
            .collect()
    }

    /// How many objects are stored now.
    pub async fn count(&self) -> usize {
        self.content.lock().await.len()
    }

    /// Every stored object keyed by its numeric id, read from the store itself
    /// so it cannot agree with a session by construction.
    pub async fn inventory(&self) -> BTreeMap<u64, Vec<u8>> {
        self.content
            .lock()
            .await
            .iter()
            .map(|(id, bytes)| (id.get(), bytes.clone()))
            .collect()
    }
}

#[async_trait]
impl SessionStorage for MemoryStorage {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError> {
        Ok(self
            .content
            .lock()
            .await
            .get(&id)
            .is_some_and(|stored| stored.as_slice() == content))
    }

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError> {
        self.written.lock().await.push(id);
        self.content.lock().await.insert(id, content.to_vec());
        Ok(())
    }

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError> {
        let bytes =
            self.content.lock().await.get(&id).cloned().ok_or_else(|| {
                ResourceError::new(ErrorCategory::NotFound, "missing test artifact")
            })?;
        String::from_utf8(bytes).map_err(|_| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "invalid test artifact UTF-8",
            )
        })
    }

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError> {
        self.content.lock().await.remove(&id);
        Ok(())
    }

    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}
