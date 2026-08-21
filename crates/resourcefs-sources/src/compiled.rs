use async_trait::async_trait;
use resourcefs_core::{
    DiscoveryAdapter, GlobOptions, GlobSource, GlobTarget, OperationGuard, PathReference,
    ResourceAddress, ResourceError, SearchOptions, SearchSourceResult, SearchTarget, SourceAdapter,
    SourceGlobResult, SourceResource,
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

#[async_trait]
impl DiscoveryAdapter for CompiledSources {
    async fn search(
        &self,
        target: &SearchTarget,
        pattern: &str,
        options: SearchOptions,
        operation: &OperationGuard,
    ) -> Result<SearchSourceResult, ResourceError> {
        match target.reference().map(PathReference::address) {
            None | Some(ResourceAddress::Workspace(_)) => {
                self.filesystem
                    .search(target, pattern, options, operation)
                    .await
            }
            Some(ResourceAddress::Artifact(_)) => {
                self.artifacts
                    .search(target, pattern, options, operation)
                    .await
            }
        }
    }

    async fn glob(
        &self,
        target: &GlobTarget,
        options: GlobOptions,
        operation: &OperationGuard,
    ) -> Result<SourceGlobResult, ResourceError> {
        match target.source() {
            GlobSource::Workspace => self.filesystem.glob(target, options, operation).await,
            GlobSource::Artifact => self.artifacts.glob(target, options, operation).await,
        }
    }
}
