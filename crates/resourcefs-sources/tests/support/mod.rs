//! Shared fixture for the Session Scratch contracts.
#![allow(dead_code, reason = "each integration test binary uses a subset")]

use std::{fs, sync::Arc};

use resourcefs_core::{
    DiscoveryEngine, MutationEngine, OperationGuard, PathSession, ReadEngine, ServerLimits,
    WorkspaceRootId,
};
use resourcefs_sources::{
    ArtifactSource, BackingPathVisibility, CompiledSources, FilesystemSource, LaunchRoot,
    LaunchRootSource, LocalSource, MutationGrants, SESSION_CLEANUP_TTL, SessionStorageConfig,
    SessionStore, StoredSession,
};
use tempfile::TempDir;

pub struct ScratchFixture {
    pub compiled: Arc<CompiledSources>,
    pub engine: MutationEngine,
    pub reads: ReadEngine,
    pub local: LocalSource,
    pub filesystem: FilesystemSource,
    pub session: StoredSession,
    pub workspace_root: TempDir,
    pub granted_root: TempDir,
    _cache: TempDir,
}

impl ScratchFixture {
    pub fn path_session(&self) -> &PathSession {
        self.session.path_session()
    }
}

/// Two Workspace Roots with deliberately different authority:
///
/// - `workspace` has **no** grants, so a scratch mutation that succeeds proves
///   Path Session authority rather than a workspace grant, and a workspace
///   mutation there is denied.
/// - `granted` has full grants, so a rejected cross-source move there is
///   rejected *because it crosses sources*, not because a grant was missing.
///
/// The roots come from a Profile source: CLI roots have their grants stripped
/// by contract (rfs-73dz C18), which would make the cross-source test pass for
/// the wrong reason.
pub async fn scratch_fixture() -> ScratchFixture {
    let workspace = TempDir::new().expect("workspace");
    fs::write(workspace.path().join("tracked.txt"), "workspace bytes\n").expect("workspace file");
    let granted = TempDir::new().expect("granted workspace");
    fs::write(granted.path().join("tracked.txt"), "granted bytes\n").expect("granted file");
    let filesystem = FilesystemSource::new(
        LaunchRootSource::Profile(vec![
            LaunchRoot::read_only(
                WorkspaceRootId::new("workspace").expect("root ID"),
                workspace.path().to_owned(),
            ),
            LaunchRoot::new(
                WorkspaceRootId::new("granted").expect("root ID"),
                granted.path().to_owned(),
                MutationGrants::new(true, true, true),
            ),
        ]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");

    let cache = TempDir::new().expect("session cache");
    let store = SessionStore::open_with(
        SessionStorageConfig::new(cache.path(), SESSION_CLEANUP_TTL.as_secs() as i64)
            .expect("session storage config"),
    )
    .await
    .expect("session store");
    let session = store
        .create_session(ServerLimits::default())
        .await
        .expect("stored session");

    let local = LocalSource::new(session.path_session().clone());
    let compiled = Arc::new(
        CompiledSources::new(
            filesystem.clone(),
            ArtifactSource::new(session.path_session().clone()),
            local.clone(),
            None,
        )
        .await
        .expect("compiled sources"),
    );
    let reads = ReadEngine::new(
        Arc::clone(&compiled) as Arc<dyn resourcefs_core::SourceAdapter>,
        session.path_session().clone(),
        ServerLimits::default(),
    );
    let engine = MutationEngine::new(
        Arc::clone(&compiled) as Arc<dyn resourcefs_core::MutationAdapter>,
        session.path_session().clone(),
    );

    ScratchFixture {
        compiled,
        engine,
        reads,
        local,
        filesystem,
        session,
        workspace_root: workspace,
        granted_root: granted,
        _cache: cache,
    }
}

pub fn guard() -> OperationGuard {
    OperationGuard::new()
}

/// One Path Session's worth of scratch machinery, used by the isolation
/// contract where two sessions must be driven independently.
pub struct ScratchSession {
    pub compiled: Arc<CompiledSources>,
    pub engine: MutationEngine,
    pub reads: ReadEngine,
    pub discovery: DiscoveryEngine,
    pub local: LocalSource,
    pub session: StoredSession,
}

impl ScratchSession {
    pub fn path_session(&self) -> &PathSession {
        self.session.path_session()
    }

    /// The session's own directory under the shared sessions root.
    pub fn storage_dir(&self, sessions_root: &std::path::Path) -> std::path::PathBuf {
        sessions_root.join(self.path_session().token().as_str())
    }
}

/// Two concurrent Path Sessions backed by **one** `SessionStore` and one
/// sessions root.
///
/// Sharing the store is what makes the isolation claim falsifiable: if scratch
/// were keyed anywhere other than per-`SessionState`, these two sessions would
/// collide, because everything outside the session — the store, the sessions
/// root, and the Workspace Root — is deliberately common to both.
pub struct ScratchPair {
    pub a: ScratchSession,
    pub b: ScratchSession,
    pub store: SessionStore,
    pub retention_ttl: std::time::Duration,
    pub sessions_root: std::path::PathBuf,
    _workspace: TempDir,
    _cache: TempDir,
}

/// Retention window for the paired fixture: short enough that the TTL sweep is
/// exercised with an explicit clock rather than a real wait.
const PAIR_RETENTION_TTL_SECONDS: i64 = 60;

pub async fn scratch_fixture_pair() -> ScratchPair {
    let workspace = TempDir::new().expect("workspace");
    fs::write(workspace.path().join("tracked.txt"), "workspace bytes\n").expect("workspace file");
    let filesystem = FilesystemSource::new(
        LaunchRootSource::Profile(vec![LaunchRoot::read_only(
            WorkspaceRootId::new("workspace").expect("root ID"),
            workspace.path().to_owned(),
        )]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("filesystem source");

    let cache = TempDir::new().expect("session cache");
    let store = SessionStore::open_with(
        SessionStorageConfig::new(cache.path(), PAIR_RETENTION_TTL_SECONDS)
            .expect("session storage config"),
    )
    .await
    .expect("session store");
    let sessions_root = store.sessions_root_for_test().to_owned();

    let a = build_scratch_session(&store, &filesystem).await;
    let b = build_scratch_session(&store, &filesystem).await;
    assert_ne!(
        a.path_session().token().as_str(),
        b.path_session().token().as_str(),
        "the paired fixture must hold two distinct Path Sessions"
    );

    ScratchPair {
        a,
        b,
        store,
        retention_ttl: std::time::Duration::from_secs(
            u64::try_from(PAIR_RETENTION_TTL_SECONDS).expect("positive retention TTL"),
        ),
        sessions_root,
        _workspace: workspace,
        _cache: cache,
    }
}

async fn build_scratch_session(
    store: &SessionStore,
    filesystem: &FilesystemSource,
) -> ScratchSession {
    let session = store
        .create_session(ServerLimits::default())
        .await
        .expect("stored session");
    let local = LocalSource::new(session.path_session().clone());
    let compiled = Arc::new(
        CompiledSources::new(
            filesystem.clone(),
            ArtifactSource::new(session.path_session().clone()),
            local.clone(),
            None,
        )
        .await
        .expect("compiled sources"),
    );
    let reads = ReadEngine::new(
        Arc::clone(&compiled) as Arc<dyn resourcefs_core::SourceAdapter>,
        session.path_session().clone(),
        ServerLimits::default(),
    );
    let engine = MutationEngine::new(
        Arc::clone(&compiled) as Arc<dyn resourcefs_core::MutationAdapter>,
        session.path_session().clone(),
    );
    let discovery = DiscoveryEngine::new(
        Arc::clone(&compiled) as Arc<dyn resourcefs_core::DiscoveryAdapter>,
        session.path_session().clone(),
        ServerLimits::default(),
    );
    ScratchSession {
        compiled,
        engine,
        reads,
        discovery,
        local,
        session,
    }
}
