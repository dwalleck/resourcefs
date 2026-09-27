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
/// object is `not_found` and stored bytes that are not UTF-8 are
/// `source_unavailable`, so a missing object and a corrupt one stay distinct.
/// `path_session_contract.rs` keeps its own richer fake (call log, injected
/// write failure) on purpose.
#[derive(Default)]
pub struct MemoryStorage {
    state: Mutex<MemoryState>,
}

/// Content and write log behind one lock, so a write's log entry and its
/// content become visible together.
#[derive(Default)]
struct MemoryState {
    content: HashMap<ArtifactId, Vec<u8>>,
    written: Vec<ArtifactId>,
}

impl MemoryStorage {
    /// How many `write_atomic` calls reached the store.
    pub async fn write_calls(&self) -> usize {
        self.state.lock().await.written.len()
    }

    /// Every id the store was handed, in write order.
    pub async fn written_ids(&self) -> Vec<ArtifactId> {
        self.state.lock().await.written.clone()
    }

    /// How many objects are stored now.
    pub async fn count(&self) -> usize {
        self.state.lock().await.content.len()
    }

    /// Every stored object keyed by its id, read from the store itself so it
    /// cannot agree with a session by construction.
    pub async fn inventory(&self) -> BTreeMap<ArtifactId, Vec<u8>> {
        self.state
            .lock()
            .await
            .content
            .iter()
            .map(|(id, bytes)| (*id, bytes.clone()))
            .collect()
    }
}

#[async_trait]
impl SessionStorage for MemoryStorage {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError> {
        Ok(self
            .state
            .lock()
            .await
            .content
            .get(&id)
            .is_some_and(|stored| stored.as_slice() == content))
    }

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError> {
        let mut state = self.state.lock().await;
        state.written.push(id);
        state.content.insert(id, content.to_vec());
        Ok(())
    }

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError> {
        let bytes = self
            .state
            .lock()
            .await
            .content
            .get(&id)
            .cloned()
            .ok_or_else(|| ResourceError::new(ErrorCategory::NotFound, "missing test artifact"))?;
        String::from_utf8(bytes).map_err(|_| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "invalid test artifact UTF-8",
            )
        })
    }

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError> {
        self.state.lock().await.content.remove(&id);
        Ok(())
    }

    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}
