use async_trait::async_trait;
use resourcefs_core::{
    CatalogAddress, DiscoveryAdapter, ErrorCategory, GlobOptions, GlobSource, GlobTarget,
    MutationAccess, MutationAdapter, MutationState, MutationTarget, OperationGuard, PathReference,
    ResourceAddress, ResourceError, SearchOptions, SearchSourceResult, SearchTarget, SourceAdapter,
    SourceGlobResult, SourceMutation, SourceResource, catalog_discovery_unsupported,
};

use crate::{
    ArtifactSource, FilesystemSource, LocalSource,
    catalog::{NamespaceCatalog, SourceCatalogEntry, SourceCatalogMetadata},
    filesystem::mutation::FILESYSTEM_MUTATION_SOURCE_KEY,
    local::LOCAL_MUTATION_SOURCE_KEY,
};

/// Composite of the Source Adapters compiled into this ResourceFS build.
#[derive(Debug, Clone)]
pub struct CompiledSources {
    filesystem: FilesystemSource,
    artifacts: ArtifactSource,
    local: LocalSource,
}

impl CompiledSources {
    pub async fn new(
        filesystem: FilesystemSource,
        artifacts: ArtifactSource,
        local: LocalSource,
    ) -> Result<Self, ResourceError> {
        let compiled = Self {
            filesystem,
            artifacts,
            local,
        };
        let source_document = NamespaceCatalog::source_document(compiled.catalog_entries()?)?;
        let workspace_document = NamespaceCatalog::workspace_document(&compiled.filesystem).await?;
        debug_assert!(!source_document.content().is_empty());
        debug_assert!(!workspace_document.content().is_empty());
        drop(source_document.into_parts());
        drop(workspace_document.into_parts());
        Ok(compiled)
    }

    fn catalog_metadata(&self) -> [&dyn SourceCatalogMetadata; 3] {
        [&self.filesystem, &self.artifacts, &self.local]
    }

    pub(crate) fn catalog_entries(&self) -> Result<Vec<SourceCatalogEntry>, ResourceError> {
        let mut entries = Vec::new();
        for source in self.catalog_metadata() {
            let described = source.catalog_entries()?;
            if described.is_empty() {
                return Err(ResourceError::new(
                    resourcefs_core::ErrorCategory::InvalidReference,
                    "compiled Source Adapter must describe at least one scheme",
                ));
            }
            entries.extend(described);
        }
        Ok(entries)
    }
}

#[async_trait]
impl SourceAdapter for CompiledSources {
    async fn read(&self, reference: &PathReference) -> Result<SourceResource, ResourceError> {
        match reference.address() {
            ResourceAddress::Catalog(address) => {
                let document = match address {
                    CatalogAddress::Sources => {
                        NamespaceCatalog::source_document(self.catalog_entries()?)?
                    }
                    CatalogAddress::Workspace => {
                        NamespaceCatalog::workspace_document(&self.filesystem).await?
                    }
                };
                let (content, version_tag) = document.into_parts();
                SourceResource::text_projection(reference.clone(), content, version_tag)
            }
            ResourceAddress::Workspace(_) => self.filesystem.read(reference).await,
            ResourceAddress::Artifact(_) => self.artifacts.read(reference).await,
            ResourceAddress::Local(_) => self.local.read(reference).await,
            ResourceAddress::Https(_) => Err(https_not_mounted()),
        }
    }
}

/// The `https://` grammar is admitted before its Source Adapter exists.
///
/// Returning a stable `source_unavailable` keeps an HTTPS reference from being
/// misrouted to another adapter while increments B and C build the bounded HTTP
/// substrate and mount the adapter.
fn https_not_mounted() -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        "https source is not mounted",
    )
}

#[async_trait]
impl MutationAdapter for CompiledSources {
    async fn resolve(
        &self,
        reference: &PathReference,
        access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError> {
        match reference.address() {
            ResourceAddress::Workspace(_) => self.filesystem.resolve(reference, access).await,
            ResourceAddress::Catalog(_) | ResourceAddress::Artifact(_) => {
                Err(resourcefs_core::ResourceError::new(
                    resourcefs_core::ErrorCategory::PermissionDenied,
                    "catalog and Artifact Resources are immutable",
                ))
            }
            ResourceAddress::Local(_) => self.local.resolve(reference, access).await,
            ResourceAddress::Https(_) => Err(https_not_mounted()),
        }
    }

    /// Routes by the target's own source key.
    ///
    /// Hardcoding one adapter here would load or commit a `local://` mutation
    /// against the filesystem — the invariant this dispatch removes.
    async fn load(
        &self,
        target: &MutationTarget,
        access: MutationAccess,
        operation: &OperationGuard,
    ) -> Result<MutationState, ResourceError> {
        match self.mutation_adapter_for(target)? {
            MutationRoute::Filesystem => self.filesystem.load(target, access, operation).await,
            MutationRoute::Local => self.local.load(target, access, operation).await,
        }
    }

    async fn commit(
        &self,
        mutation: SourceMutation,
        operation: &OperationGuard,
    ) -> Result<(), ResourceError> {
        let target = match &mutation {
            SourceMutation::Create { target, .. }
            | SourceMutation::Replace { target, .. }
            | SourceMutation::Delete { target, .. } => target,
            SourceMutation::Move { source, .. } => source,
        };
        match self.mutation_adapter_for(target)? {
            MutationRoute::Filesystem => self.filesystem.commit(mutation, operation).await,
            MutationRoute::Local => self.local.commit(mutation, operation).await,
        }
    }
}

/// Which compiled adapter owns one mutation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationRoute {
    Filesystem,
    Local,
}

impl CompiledSources {
    fn mutation_adapter_for(
        &self,
        target: &MutationTarget,
    ) -> Result<MutationRoute, ResourceError> {
        match target.source_key().as_str() {
            FILESYSTEM_MUTATION_SOURCE_KEY => Ok(MutationRoute::Filesystem),
            LOCAL_MUTATION_SOURCE_KEY => Ok(MutationRoute::Local),
            other => Err(ResourceError::new(
                resourcefs_core::ErrorCategory::UnsupportedMutation,
                format!("no compiled Source Adapter owns mutation source '{other}'"),
            )),
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
            Some(ResourceAddress::Catalog(_)) => Err(catalog_discovery_unsupported(
                target
                    .reference()
                    .expect("matched a concrete catalog target")
                    .requested(),
            )),
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
            Some(ResourceAddress::Local(_)) => {
                self.local.search(target, pattern, options, operation).await
            }
            Some(ResourceAddress::Https(_)) => Err(https_not_mounted()),
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
            GlobSource::Local => self.local.glob(target, options, operation).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use resourcefs_core::{ServerLimits, WorkspaceRootId};
    use tempfile::TempDir;

    use super::CompiledSources;
    use crate::{
        ArtifactSource, BackingPathVisibility, FilesystemSource, LaunchRoot, LaunchRootSource,
        SESSION_CLEANUP_TTL, SessionStorageConfig, SessionStore, catalog::NamespaceCatalog,
    };

    #[tokio::test]
    async fn catalog_metadata_is_the_compiled_registry() {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path().join("workspace");
        fs::create_dir(&root).expect("workspace root");
        let filesystem = FilesystemSource::new(
            LaunchRootSource::Cli(vec![LaunchRoot::read_only(
                WorkspaceRootId::new("workspace").expect("root ID"),
                root,
            )]),
            None,
            BackingPathVisibility::Hidden,
        )
        .await
        .expect("filesystem source");
        let store = SessionStore::open_with(
            SessionStorageConfig::new(
                temporary.path().join("cache"),
                SESSION_CLEANUP_TTL.as_secs() as i64,
            )
            .expect("session storage config"),
        )
        .await
        .expect("session store");
        let session = store
            .create_session(ServerLimits::default())
            .await
            .expect("session");
        let compiled = CompiledSources::new(
            filesystem,
            ArtifactSource::new(session.path_session().clone()),
            crate::LocalSource::new(session.path_session().clone()),
        )
        .await
        .expect("compiled sources");
        let document =
            NamespaceCatalog::source_document(compiled.catalog_entries().expect("catalog entries"))
                .expect("catalog document");
        let source_lines = document
            .content()
            .lines()
            .filter(|line| line.contains(" — "))
            .collect::<Vec<_>>();
        assert_eq!(source_lines.len(), 3);
        assert!(source_lines[0].starts_with("artifact:// — "));
        assert!(source_lines[1].starts_with("local:// — "));
        assert!(source_lines[2].starts_with("rfs://workspace — "));
    }
}
