use async_trait::async_trait;
use resourcefs_core::{
    PathReference, ResourceAddress, ResourceError, SourceAdapter, SourceResource,
};

use crate::{ArtifactSource, FilesystemSource};

/// Composite of the Source Adapters compiled into this ResourceFS build.
#[derive(Debug, Clone)]
pub struct CompiledSources {
    filesystem: FilesystemSource,
    artifacts: ArtifactSource,
}

impl CompiledSources {
    pub fn new(filesystem: FilesystemSource, artifacts: ArtifactSource) -> Self {
        Self {
            filesystem,
            artifacts,
        }
    }
}

#[async_trait]
impl SourceAdapter for CompiledSources {
    async fn read(&self, reference: &PathReference) -> Result<SourceResource, ResourceError> {
        match reference.address() {
            ResourceAddress::Workspace(_) => self.filesystem.read(reference).await,
            ResourceAddress::Artifact(_) => self.artifacts.read(reference).await,
        }
    }
}
