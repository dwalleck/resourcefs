use std::{
    io,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use resourcefs_core::{
    ErrorCategory, MAX_TEXT_BYTES, MAX_TEXT_COLUMNS, MAX_TEXT_LINES, PathReference, ReadResource,
    ResourceError, RootName, SourceAdapter,
};
use tokio::io::AsyncReadExt;

/// Read-only Source Adapter for one explicitly configured Workspace Root.
#[derive(Debug)]
pub struct FilesystemSource {
    root_name: RootName,
    canonical_root: PathBuf,
}

impl FilesystemSource {
    pub async fn new(root_name: RootName, root: impl AsRef<Path>) -> Result<Self, ResourceError> {
        let canonical_root = tokio::fs::canonicalize(root.as_ref())
            .await
            .map_err(|error| root_error("canonicalize Workspace Root", error))?;
        let metadata = tokio::fs::metadata(&canonical_root)
            .await
            .map_err(|error| root_error("inspect Workspace Root", error))?;
        if !metadata.is_dir() {
            return Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "configured Workspace Root is not a directory",
            ));
        }
        Ok(Self {
            root_name,
            canonical_root,
        })
    }

    async fn read_contained(
        &self,
        reference: &PathReference,
    ) -> Result<ReadResource, ResourceError> {
        if reference.root() != &self.root_name {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "Path Reference names an unconfigured Workspace Root",
            ));
        }

        let lexical_target = self.canonical_root.join(reference.relative_path());
        let canonical_target = tokio::fs::canonicalize(&lexical_target)
            .await
            .map_err(|error| resource_io_error(reference, "resolve", error))?;
        if !canonical_target.starts_with(&self.canonical_root) {
            return Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                format!(
                    "Resource '{}' resolves outside the Primary Workspace Root",
                    reference.canonical()
                ),
            ));
        }

        let metadata = tokio::fs::metadata(&canonical_target)
            .await
            .map_err(|error| resource_io_error(reference, "inspect", error))?;
        if !metadata.is_file() {
            return Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                format!("Resource '{}' is not a text file", reference.canonical()),
            ));
        }
        let file = tokio::fs::File::open(&canonical_target)
            .await
            .map_err(|error| resource_io_error(reference, "open", error))?;

        let initial_capacity = metadata.len().min(MAX_TEXT_BYTES as u64) as usize;
        let mut bytes = Vec::with_capacity(initial_capacity);
        file.take(MAX_TEXT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| resource_io_error(reference, "read", error))?;
        if bytes.len() > MAX_TEXT_BYTES {
            return Err(limit_error(reference, "bytes", MAX_TEXT_BYTES));
        }

        let content = String::from_utf8(bytes).map_err(|_| {
            ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                format!(
                    "Resource '{}' is not valid UTF-8 text",
                    reference.canonical()
                ),
            )
        })?;
        let mut lines = 0_usize;
        let mut maximum_columns = 0_usize;
        for line in content.lines() {
            lines += 1;
            maximum_columns = maximum_columns.max(line.chars().count());
        }
        if lines > MAX_TEXT_LINES {
            return Err(limit_error(reference, "lines", MAX_TEXT_LINES));
        }
        if maximum_columns > MAX_TEXT_COLUMNS {
            return Err(limit_error(reference, "columns", MAX_TEXT_COLUMNS));
        }

        Ok(ReadResource::text(reference.clone(), content))
    }
}

#[async_trait]
impl SourceAdapter for FilesystemSource {
    async fn read(&self, reference: &PathReference) -> Result<ReadResource, ResourceError> {
        self.read_contained(reference).await
    }
}

fn root_error(operation: &str, error: io::Error) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!("failed to {operation}: {error}"),
    )
}

fn resource_io_error(
    reference: &PathReference,
    operation: &str,
    error: io::Error,
) -> ResourceError {
    let category = match error.kind() {
        io::ErrorKind::NotFound => ErrorCategory::NotFound,
        io::ErrorKind::PermissionDenied => ErrorCategory::PermissionDenied,
        _ => ErrorCategory::SourceUnavailable,
    };
    ResourceError::new(
        category,
        format!(
            "failed to {operation} Resource '{}': {error}",
            reference.canonical()
        ),
    )
}

fn limit_error(reference: &PathReference, dimension: &str, limit: usize) -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        format!(
            "Resource '{}' exceeds the hard {dimension} limit of {limit}",
            reference.canonical()
        ),
    )
}
