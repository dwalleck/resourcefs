#[path = "filesystem_mutation.rs"]
mod mutation;

use std::{
    collections::{HashMap, HashSet},
    io::{self, BufRead, BufReader},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use cap_std::{
    ambient_authority,
    fs::{Dir, File},
};
use ignore::{
    Match,
    gitignore::{Gitignore, GitignoreBuilder},
};
use resourcefs_core::{
    DiscoveryAdapter, DiscoveryDiagnostic, ErrorCategory, GlobEntry, GlobKind, GlobOptions,
    GlobSource, GlobTarget, LineSelector, MAX_ARTIFACT_BYTES, MAX_WORKSPACE_ROOTS, OperationGuard,
    PathReference, ProjectionSelector, ResourceError, SearchEngine, SearchOptions, SearchRecord,
    SearchSourceResult, SearchTarget, SourceAdapter, SourceGlobResult, SourceResource,
    WorkspaceAddress, WorkspacePath, WorkspaceRoot, WorkspaceRootId, WorkspaceRootSet, select_utf8,
};
use sha2::{Digest, Sha256};
#[cfg(feature = "test-support")]
use tokio::sync::Notify;
use tokio::{
    sync::{Mutex, OwnedRwLockReadGuard, RwLock},
    time::Instant,
};
use url::Url;

use crate::{
    catalog::{SourceCatalogEntry, SourceCatalogMetadata},
    configuration::{
        MutationGrants,
        paths::{normalize_platform_path, strip_beneath},
    },
    pattern::{GlobMatcher, SearchMatcher},
};

const ROOT_CONSTRUCTION_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_WORKSPACE_DISCOVERY_ENTRIES: usize = 100_000;
const MAX_WORKSPACE_DISCOVERY_RECORDS: usize = 100_000;
const MAX_WORKSPACE_DISCOVERY_SCAN_BYTES: usize = 256 * 1024 * 1024;
const MAX_WORKSPACE_PCRE2_SUBJECTS: usize = 10_000;
const MAX_WORKSPACE_DISCOVERY_STATE_BYTES: usize = 32 * 1024 * 1024;
const RETAINED_COLLECTION_ENTRY_BYTES: usize = 32;

#[derive(Default)]
struct WorkspaceDiscoveryBudget {
    entries: usize,
    diagnostics: usize,
    exhausted: bool,
    records: usize,
    result_bytes: usize,
    scanned_bytes: usize,
    state_bytes: usize,
    pcre2_subjects: usize,
}

impl WorkspaceDiscoveryBudget {
    fn charge_entry(&mut self) -> Result<(), ResourceError> {
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| limit_error("Workspace discovery entry count is not representable"))?;
        if self.entries > MAX_WORKSPACE_DISCOVERY_ENTRIES {
            return self.exhaust("Workspace discovery exceeds the 100,000-entry traversal ceiling");
        }
        Ok(())
    }

    fn charge_scan(&mut self, bytes: usize) -> Result<(), ResourceError> {
        self.scanned_bytes = self.scanned_bytes.checked_add(bytes).ok_or_else(|| {
            limit_error("Workspace discovery scanned byte count is not representable")
        })?;
        if self.scanned_bytes > MAX_WORKSPACE_DISCOVERY_SCAN_BYTES {
            return self.exhaust("Workspace discovery exceeds the 256 MiB cumulative scan ceiling");
        }
        Ok(())
    }

    fn charge_match(&mut self, engine: SearchEngine) -> Result<(), ResourceError> {
        if engine != SearchEngine::Pcre2 {
            return Ok(());
        }
        self.pcre2_subjects = self
            .pcre2_subjects
            .checked_add(1)
            .ok_or_else(|| limit_error("Workspace PCRE2 subject count is not representable"))?;
        if self.pcre2_subjects > MAX_WORKSPACE_PCRE2_SUBJECTS {
            return self.exhaust(
                "Workspace search exceeds the 10,000-subject cumulative PCRE2 work ceiling",
            );
        }
        Ok(())
    }

    fn charge_state(&mut self, bytes: usize) -> Result<(), ResourceError> {
        self.state_bytes = self.state_bytes.checked_add(bytes).ok_or_else(|| {
            limit_error("Workspace discovery state byte count is not representable")
        })?;
        if self.state_bytes > MAX_WORKSPACE_DISCOVERY_STATE_BYTES {
            return self.exhaust(
                "Workspace discovery exceeds the 32 MiB retained traversal-state ceiling",
            );
        }
        Ok(())
    }

    fn release_state(&mut self, bytes: usize) {
        self.state_bytes = self
            .state_bytes
            .checked_sub(bytes)
            .expect("Workspace discovery state charges are balanced");
    }

    fn charge_search_record(
        &mut self,
        reference: &PathReference,
        line: u64,
        text: &str,
    ) -> Result<(), ResourceError> {
        let escaped = text
            .bytes()
            .filter(|byte| matches!(byte, b'\\' | b'\t' | b'\r' | b'\n'))
            .count();
        let line_digits = if line == 0 {
            1
        } else {
            line.ilog10() as usize + 1
        };
        let additional = reference
            .requested()
            .len()
            .checked_add(line_digits)
            .and_then(|bytes| bytes.checked_add(text.len()))
            .and_then(|bytes| bytes.checked_add(escaped))
            .and_then(|bytes| bytes.checked_add(3))
            .ok_or_else(|| {
                limit_error("Workspace search result byte count is not representable")
            })?;
        self.charge_result(additional, "Workspace search")
    }

    fn charge_glob_entry(
        &mut self,
        reference: &PathReference,
        kind: GlobKind,
    ) -> Result<(), ResourceError> {
        let suffix = usize::from(kind == GlobKind::Directory);
        let additional = reference
            .requested()
            .len()
            .checked_add(suffix + 1)
            .ok_or_else(|| limit_error("Workspace glob result byte count is not representable"))?;
        self.charge_result(additional, "Workspace glob")
    }

    fn charge_diagnostic(&mut self, diagnostic: &DiscoveryDiagnostic) -> Result<(), ResourceError> {
        self.diagnostics = self.diagnostics.checked_add(1).ok_or_else(|| {
            limit_error("Workspace discovery diagnostic count is not representable")
        })?;
        if self.diagnostics > MAX_WORKSPACE_DISCOVERY_RECORDS {
            return self
                .exhaust("Workspace discovery exceeds the 100,000-diagnostic result ceiling");
        }
        let message = diagnostic.message();
        let escaped = message
            .bytes()
            .filter(|byte| matches!(byte, b'\\' | b'\t' | b'\r' | b'\n'))
            .count();
        let additional = diagnostic
            .reference()
            .map_or(1, str::len)
            .checked_add(diagnostic.category().as_str().len())
            .and_then(|bytes| bytes.checked_add(message.len()))
            .and_then(|bytes| bytes.checked_add(escaped))
            .and_then(|bytes| bytes.checked_add(4))
            .ok_or_else(|| limit_error("Workspace diagnostic byte count is not representable"))?;
        self.charge_result_bytes(additional, "Workspace discovery")
    }

    fn charge_result(&mut self, bytes: usize, operation: &str) -> Result<(), ResourceError> {
        self.records = self
            .records
            .checked_add(1)
            .ok_or_else(|| limit_error(format!("{operation} result count is not representable")))?;
        if self.records > MAX_WORKSPACE_DISCOVERY_RECORDS {
            return self.exhaust(format!(
                "{operation} exceeds the 100,000-record result ceiling"
            ));
        }
        self.charge_result_bytes(bytes, operation)
    }

    fn charge_result_bytes(&mut self, bytes: usize, operation: &str) -> Result<(), ResourceError> {
        self.result_bytes = self.result_bytes.checked_add(bytes).ok_or_else(|| {
            limit_error(format!(
                "{operation} result byte count is not representable"
            ))
        })?;

        if self.result_bytes > MAX_ARTIFACT_BYTES {
            return self.exhaust(format!(
                "{operation} complete result exceeds the 64 MiB Resource ceiling"
            ));
        }
        Ok(())
    }
    const fn is_exhausted(&self) -> bool {
        self.exhausted
    }

    fn exhaust(&mut self, message: impl Into<String>) -> Result<(), ResourceError> {
        self.exhausted = true;
        Err(limit_error(message))
    }
}

fn retained_path_state_bytes(path: &Path, value_bytes: usize) -> Result<usize, ResourceError> {
    path.as_os_str()
        .as_encoded_bytes()
        .len()
        .checked_add(size_of::<PathBuf>())
        .and_then(|bytes| bytes.checked_add(value_bytes))
        .and_then(|bytes| bytes.checked_add(RETAINED_COLLECTION_ENTRY_BYTES))
        .ok_or_else(|| limit_error("Workspace retained path size is not representable"))
}

fn retained_string_state_bytes(value: &str) -> Result<usize, ResourceError> {
    value
        .len()
        .checked_add(size_of::<String>())
        .and_then(|bytes| bytes.checked_add(RETAINED_COLLECTION_ENTRY_BYTES))
        .ok_or_else(|| limit_error("Workspace retained string size is not representable"))
}

fn retained_names_state_bytes(
    names: &[std::ffi::OsString],
    capacity: usize,
) -> Result<usize, ResourceError> {
    names.iter().try_fold(
        capacity
            .checked_mul(size_of::<std::ffi::OsString>())
            .ok_or_else(|| {
                limit_error("Workspace directory name state size is not representable")
            })?,
        |total, name| {
            total
                .checked_add(name.as_encoded_bytes().len())
                .ok_or_else(|| {
                    limit_error("Workspace directory name state size is not representable")
                })
        },
    )
}

#[derive(Debug, Clone)]
pub struct LaunchRoot {
    id: WorkspaceRootId,
    path: PathBuf,
    grants: MutationGrants,
}

impl LaunchRoot {
    pub fn new(id: WorkspaceRootId, path: PathBuf, grants: MutationGrants) -> Self {
        Self { id, path, grants }
    }

    pub fn read_only(id: WorkspaceRootId, path: PathBuf) -> Self {
        Self::new(id, path, MutationGrants::default())
    }

    pub const fn id(&self) -> &WorkspaceRootId {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn grants(&self) -> MutationGrants {
        self.grants
    }
}

#[derive(Debug, Clone)]
pub enum LaunchRootSource {
    Cli(Vec<LaunchRoot>),
    Profile(Vec<LaunchRoot>),
}

impl LaunchRootSource {
    fn into_roots(self) -> (Vec<LaunchRoot>, bool) {
        match self {
            Self::Cli(mut roots) => {
                for root in &mut roots {
                    root.grants = MutationGrants::default();
                }
                (roots, false)
            }
            Self::Profile(roots) => (roots, true),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClientRoot {
    pub uri: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BackingPathVisibility {
    #[default]
    Hidden,
    Visible,
}

#[derive(Debug)]
struct FilesystemRoot {
    metadata: WorkspaceRoot,
    canonical_path: PathBuf,
    directory: Dir,
    grants: MutationGrants,
}

#[derive(Debug)]
struct WorkspaceView {
    roots: Arc<WorkspaceRootSet>,
    filesystems: HashMap<WorkspaceRootId, Arc<FilesystemRoot>>,
}

#[derive(Debug)]
enum AuthorityState {
    Active {
        epoch: u64,
        generation: u64,
        view: Arc<WorkspaceView>,
    },
    Refreshing {
        epoch: u64,
        generation: u64,
        baseline: Option<Arc<WorkspaceView>>,
    },
    Disabled {
        epoch: u64,
        generation: u64,
    },
}

impl AuthorityState {
    fn epoch(&self) -> u64 {
        match self {
            Self::Active { epoch, .. }
            | Self::Refreshing { epoch, .. }
            | Self::Disabled { epoch, .. } => *epoch,
        }
    }

    fn generation(&self) -> u64 {
        match self {
            Self::Active { generation, .. }
            | Self::Refreshing { generation, .. }
            | Self::Disabled { generation, .. } => *generation,
        }
    }
}

pub(super) struct MutationAuthorityGuard {
    _guard: OwnedRwLockReadGuard<AuthorityState>,
}

#[cfg(feature = "test-support")]
pub struct TestAuthorityGuard {
    _guard: MutationAuthorityGuard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootRefresh {
    epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootAcquisition {
    refresh: RootRefresh,
    deadline: Instant,
}

impl RootAcquisition {
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootRefreshOutcome {
    Applied {
        generation: u64,
        removed: Vec<WorkspaceRootId>,
        active: Vec<WorkspaceRootId>,
    },
    Unchanged {
        generation: u64,
    },
    Superseded,
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone)]
pub struct TestDeliveryGate {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[cfg(feature = "test-support")]
impl TestDeliveryGate {
    pub async fn wait_until_entered(&self) {
        self.entered.notified().await;
    }

    pub fn release(&self) {
        self.release.notify_one();
    }
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone)]
struct ArmedDeliveryGate {
    identity: String,
    control: TestDeliveryGate,
}

#[derive(Debug)]
struct FilesystemSourceInner {
    launch_view: Arc<WorkspaceView>,
    profile_grants: Arc<HashMap<PathBuf, MutationGrants>>,
    primary_selector: Option<String>,
    visibility: BackingPathVisibility,
    authority: Arc<RwLock<AuthorityState>>,
    identity_history: Mutex<HashMap<WorkspaceRootId, String>>,
    #[cfg(feature = "test-support")]
    delivery_gate: RwLock<Option<ArmedDeliveryGate>>,
}

/// Read-only Source Adapter for one connection-owned, atomically replaceable root set.
#[derive(Debug, Clone)]
pub struct FilesystemSource {
    inner: Arc<FilesystemSourceInner>,
}

impl FilesystemSource {
    pub async fn new(
        root_source: LaunchRootSource,
        primary_selector: Option<String>,
        visibility: BackingPathVisibility,
    ) -> Result<Self, ResourceError> {
        let (roots, inherit_profile_grants) = root_source.into_roots();
        if roots.len() > MAX_WORKSPACE_ROOTS {
            return Err(limit_error("Workspace Root count exceeds 256"));
        }
        let selector = primary_selector.clone();
        let view = construct_launch_view(roots, selector).await?;
        let launch_view = Arc::new(view);
        let profile_grants = if inherit_profile_grants {
            launch_view
                .filesystems
                .values()
                .map(|root| (root.canonical_path.clone(), root.grants))
                .collect()
        } else {
            HashMap::new()
        };
        let identity_history = launch_view
            .roots
            .roots()
            .iter()
            .map(|root| (root.id().clone(), root.canonical_uri().to_owned()))
            .collect();
        Ok(Self {
            inner: Arc::new(FilesystemSourceInner {
                launch_view: Arc::clone(&launch_view),
                profile_grants: Arc::new(profile_grants),
                primary_selector,
                visibility,
                authority: Arc::new(RwLock::new(AuthorityState::Active {
                    epoch: 0,
                    generation: 1,
                    view: launch_view,
                })),
                identity_history: Mutex::new(identity_history),
                #[cfg(feature = "test-support")]
                delivery_gate: RwLock::new(None),
            }),
        })
    }

    pub(super) async fn mutation_authority_guard(
        &self,
    ) -> Result<MutationAuthorityGuard, ResourceError> {
        let guard = Arc::clone(&self.inner.authority).read_owned().await;
        if !matches!(&*guard, AuthorityState::Active { .. }) {
            return Err(authority_unavailable(
                "Workspace authority is not active for mutation",
            ));
        }
        Ok(MutationAuthorityGuard { _guard: guard })
    }

    #[cfg(feature = "test-support")]
    pub async fn hold_test_authority(&self) -> Result<TestAuthorityGuard, ResourceError> {
        self.mutation_authority_guard()
            .await
            .map(|guard| TestAuthorityGuard { _guard: guard })
    }

    #[cfg(feature = "test-support")]
    pub async fn arm_test_delivery_gate(&self, identity: impl Into<String>) -> TestDeliveryGate {
        let control = TestDeliveryGate {
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        };
        *self.inner.delivery_gate.write().await = Some(ArmedDeliveryGate {
            identity: identity.into(),
            control: control.clone(),
        });
        control
    }

    pub async fn begin_client_root_refresh(&self) -> RootRefresh {
        let mut authority = self.inner.authority.write().await;
        let epoch = authority.epoch().saturating_add(1);
        let generation = authority.generation();
        let baseline = match &*authority {
            AuthorityState::Active { view, .. } => Some(Arc::clone(view)),
            AuthorityState::Refreshing { baseline, .. } => baseline.clone(),
            AuthorityState::Disabled { .. } => None,
        };
        *authority = AuthorityState::Refreshing {
            epoch,
            generation,
            baseline,
        };
        RootRefresh { epoch }
    }

    pub fn start_client_root_acquisition(&self, refresh: RootRefresh) -> RootAcquisition {
        let acquisition = RootAcquisition {
            refresh,
            deadline: Instant::now() + ROOT_CONSTRUCTION_TIMEOUT,
        };
        let source = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(acquisition.deadline).await;
            source.disable_refresh(acquisition.refresh).await;
        });
        acquisition
    }

    pub async fn complete_client_root_refresh(
        &self,
        acquisition: RootAcquisition,
        roots: Vec<ClientRoot>,
    ) -> Result<RootRefreshOutcome, ResourceError> {
        let refresh = acquisition.refresh;
        if acquisition.deadline <= Instant::now() {
            self.disable_refresh(refresh).await;
            return Err(authority_unavailable(
                "Workspace Root acquisition exceeded 5 seconds",
            ));
        }
        let candidate = if roots.is_empty() {
            Ok(Arc::clone(&self.inner.launch_view))
        } else {
            construct_client_view(
                roots,
                self.inner.primary_selector.clone(),
                Arc::clone(&self.inner.profile_grants),
                acquisition.remaining(),
            )
            .await
            .map(Arc::new)
        };
        if acquisition.deadline <= Instant::now() {
            self.disable_refresh(refresh).await;
            return Err(authority_unavailable(
                "Workspace Root acquisition exceeded 5 seconds",
            ));
        }
        let candidate = match candidate {
            Ok(candidate) => candidate,
            Err(error) => {
                self.disable_refresh(refresh).await;
                return Err(error);
            }
        };

        let mut authority = self.inner.authority.write().await;
        let AuthorityState::Refreshing {
            epoch,
            generation,
            baseline,
        } = &*authority
        else {
            return Ok(RootRefreshOutcome::Superseded);
        };
        if *epoch != refresh.epoch {
            return Ok(RootRefreshOutcome::Superseded);
        }
        if acquisition.deadline <= Instant::now() {
            let generation = *generation;
            *authority = AuthorityState::Disabled {
                epoch: refresh.epoch,
                generation,
            };
            return Err(authority_unavailable(
                "Workspace Root acquisition exceeded 5 seconds",
            ));
        }
        let generation = *generation;
        let mut identity_history = self.inner.identity_history.lock().await;
        if candidate.roots.roots().iter().any(|root| {
            identity_history
                .get(root.id())
                .is_some_and(|known_uri| known_uri != root.canonical_uri())
        }) {
            *authority = AuthorityState::Disabled {
                epoch: refresh.epoch,
                generation,
            };
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "Workspace Root identifier maps to a different canonical URI",
            ));
        }
        identity_history.extend(
            candidate
                .roots
                .roots()
                .iter()
                .map(|root| (root.id().clone(), root.canonical_uri().to_owned())),
        );
        drop(identity_history);
        if baseline
            .as_ref()
            .is_some_and(|view| view.roots.equivalent_to(&candidate.roots))
        {
            let view = Arc::clone(baseline.as_ref().expect("checked baseline"));
            *authority = AuthorityState::Active {
                epoch: refresh.epoch,
                generation,
                view,
            };
            return Ok(RootRefreshOutcome::Unchanged { generation });
        }
        let removed = baseline
            .as_ref()
            .map(|previous| {
                previous
                    .roots
                    .roots()
                    .iter()
                    .filter(|prior| {
                        candidate
                            .roots
                            .get(prior.id())
                            .is_none_or(|current| current.canonical_uri() != prior.canonical_uri())
                    })
                    .map(|prior| prior.id().clone())
                    .collect()
            })
            .unwrap_or_default();
        let active = candidate
            .roots
            .roots()
            .iter()
            .map(|root| root.id().clone())
            .collect();
        let next_generation = generation.saturating_add(1);
        *authority = AuthorityState::Active {
            epoch: refresh.epoch,
            generation: next_generation,
            view: candidate,
        };
        Ok(RootRefreshOutcome::Applied {
            generation: next_generation,
            removed,
            active,
        })
    }

    pub async fn fail_client_root_refresh(&self, acquisition: RootAcquisition) {
        self.disable_refresh(acquisition.refresh).await;
    }

    async fn disable_refresh(&self, refresh: RootRefresh) {
        let mut authority = self.inner.authority.write().await;
        if let AuthorityState::Refreshing {
            epoch, generation, ..
        } = &*authority
            && *epoch == refresh.epoch
        {
            *authority = AuthorityState::Disabled {
                epoch: refresh.epoch,
                generation: *generation,
            };
        }
    }

    async fn active_view(&self) -> Result<(u64, Arc<WorkspaceView>), ResourceError> {
        let authority = self.inner.authority.read().await;
        match &*authority {
            AuthorityState::Active {
                generation, view, ..
            } => Ok((*generation, Arc::clone(view))),
            AuthorityState::Refreshing { .. } => Err(authority_unavailable(
                "Workspace Root authority is refreshing",
            )),
            AuthorityState::Disabled { .. } => Err(authority_unavailable(
                "Workspace Root authority is disabled",
            )),
        }
    }

    pub(crate) async fn workspace_root_set(&self) -> Result<Arc<WorkspaceRootSet>, ResourceError> {
        let (_, view) = self.active_view().await?;
        Ok(Arc::clone(&view.roots))
    }

    async fn read_contained(
        &self,
        reference: &PathReference,
    ) -> Result<SourceResource, ResourceError> {
        let (generation, view) = self.active_view().await?;
        let reference = reference.clone();
        let visibility = self.inner.visibility;
        let completed =
            tokio::task::spawn_blocking(move || read_from_view(&view, &reference, visibility))
                .await
                .map_err(|error| {
                    ResourceError::new(
                        ErrorCategory::SourceUnavailable,
                        format!("filesystem read task failed: {error}"),
                    )
                })??;
        self.wait_for_test_delivery_release(completed.resource.canonical_reference())
            .await?;

        let authority = self.inner.authority.read().await;
        validate_read_delivery(&authority, generation, &completed.root)?;
        Ok(completed.resource)
    }

    #[cfg(feature = "test-support")]
    async fn wait_for_test_delivery_release(&self, identity: &str) -> Result<(), ResourceError> {
        let gate = {
            let mut armed = self.inner.delivery_gate.write().await;
            if armed.as_ref().is_some_and(|candidate| {
                candidate.identity == "*" || candidate.identity == identity
            }) {
                armed.take()
            } else {
                None
            }
        };
        let Some(gate) = gate else {
            return Ok(());
        };
        gate.control.entered.notify_one();
        tokio::time::timeout(Duration::from_secs(10), gate.control.release.notified())
            .await
            .map_err(|_| authority_unavailable("test delivery gate exceeded 10 seconds"))?;
        Ok(())
    }

    #[cfg(not(feature = "test-support"))]
    async fn wait_for_test_delivery_release(&self, _identity: &str) -> Result<(), ResourceError> {
        Ok(())
    }
}

fn validate_read_delivery(
    authority: &AuthorityState,
    generation: u64,
    root: &FilesystemRoot,
) -> Result<(), ResourceError> {
    match authority {
        AuthorityState::Active {
            generation: current_generation,
            view,
            ..
        } if *current_generation == generation || view_retains_root(view, root) => Ok(()),
        AuthorityState::Active { .. } => Err(ResourceError::new(
            ErrorCategory::InvalidReference,
            "Workspace Root authority changed while reading",
        )),
        AuthorityState::Refreshing { .. } | AuthorityState::Disabled { .. } => Err(
            authority_unavailable("Workspace Root authority changed while reading"),
        ),
    }
}
impl SourceCatalogMetadata for FilesystemSource {
    fn catalog_entries(&self) -> Result<Vec<SourceCatalogEntry>, ResourceError> {
        Ok(vec![SourceCatalogEntry::new(
            "rfs://workspace",
            "<relative-path> | rfs://workspace/<root>/<path>[:selector] | file://<absolute-path> (relative paths use the Primary Workspace Root)",
            "rfs://workspace/workspace/src/lib.rs",
            None,
        )?])
    }
}

#[async_trait]
impl SourceAdapter for FilesystemSource {
    async fn read(&self, reference: &PathReference) -> Result<SourceResource, ResourceError> {
        self.read_contained(reference).await
    }
}

#[async_trait]
impl DiscoveryAdapter for FilesystemSource {
    async fn search(
        &self,
        target: &SearchTarget,
        pattern: &str,
        options: SearchOptions,
        operation: &OperationGuard,
    ) -> Result<SearchSourceResult, ResourceError> {
        ensure_workspace_discovery_live(operation)?;
        let (generation, view) = self.active_view().await?;
        let target = target.clone();
        let pattern = pattern.to_owned();
        let worker_operation = operation.clone();
        let completed = tokio::task::spawn_blocking(move || {
            search_workspace(&view, &target, &pattern, options, &worker_operation)
        })
        .await
        .map_err(workspace_discovery_worker_error)??;
        self.wait_for_test_delivery_release(&completed.delivery_identity)
            .await?;
        ensure_workspace_discovery_live(operation)?;
        let authority = self.inner.authority.read().await;
        validate_read_delivery(&authority, generation, &completed.root)?;
        Ok(completed.result)
    }

    async fn glob(
        &self,
        target: &GlobTarget,
        options: GlobOptions,
        operation: &OperationGuard,
    ) -> Result<SourceGlobResult, ResourceError> {
        if target.source() != GlobSource::Workspace {
            return Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "filesystem Source Adapter requires a Workspace glob",
            ));
        }
        ensure_workspace_discovery_live(operation)?;
        let (generation, view) = self.active_view().await?;
        let target = target.clone();
        let worker_operation = operation.clone();
        let completed = tokio::task::spawn_blocking(move || {
            glob_workspace(&view, &target, options, &worker_operation)
        })
        .await
        .map_err(workspace_discovery_worker_error)??;
        self.wait_for_test_delivery_release(&completed.delivery_identity)
            .await?;
        ensure_workspace_discovery_live(operation)?;
        let authority = self.inner.authority.read().await;
        validate_read_delivery(&authority, generation, &completed.root)?;
        Ok(completed.result)
    }
}

struct CompletedWorkspaceDiscovery<T> {
    result: T,
    root: Arc<FilesystemRoot>,
    delivery_identity: String,
}

struct WorkspaceScope {
    root: Arc<FilesystemRoot>,
    entry: OpenedWorkspaceEntry,
    projection: Option<ProjectionSelector>,
}

enum OpenedWorkspaceEntry {
    File {
        file: File,
        path: PathBuf,
        reference: PathReference,
    },
    Directory {
        directory: Dir,
        path: PathBuf,
        reference: PathReference,
    },
}

impl OpenedWorkspaceEntry {
    fn path(&self) -> &Path {
        match self {
            Self::File { path, .. } | Self::Directory { path, .. } => path,
        }
    }

    fn reference(&self) -> &PathReference {
        match self {
            Self::File { reference, .. } | Self::Directory { reference, .. } => reference,
        }
    }

    const fn kind(&self) -> GlobKind {
        match self {
            Self::File { .. } => GlobKind::File,
            Self::Directory { .. } => GlobKind::Directory,
        }
    }
}

#[derive(Clone)]
struct IgnoreStack {
    matcher: Gitignore,
    parent: Option<Arc<Self>>,
}

#[derive(Clone, Copy)]
struct DiscoveryControls {
    gitignore: bool,
    hidden: bool,
}

struct IgnoreContext<'a> {
    controls: DiscoveryControls,
    diagnostics: &'a mut Vec<DiscoveryDiagnostic>,
    operation: &'a OperationGuard,
    budget: &'a mut WorkspaceDiscoveryBudget,
}

#[derive(Clone)]
enum InitialIgnoreStack {
    Filtered,
    Ready(Option<Arc<IgnoreStack>>),
}

struct WalkFrame {
    directory: Option<Dir>,
    path: PathBuf,
    ignores: Option<Arc<IgnoreStack>>,
    pending_state_bytes: usize,
}

fn search_workspace(
    view: &WorkspaceView,
    target: &SearchTarget,
    pattern: &str,
    options: SearchOptions,
    operation: &OperationGuard,
) -> Result<CompletedWorkspaceDiscovery<SearchSourceResult>, ResourceError> {
    ensure_workspace_discovery_live(operation)?;
    let mut matcher = SearchMatcher::compile(pattern, options.case_sensitive())?;
    let engine = matcher.engine();
    let WorkspaceScope {
        root,
        entry,
        projection,
    } = open_search_scope(view, target)?;
    let delivery_identity = entry.reference().requested().to_owned();

    let mut records = Vec::new();
    let mut diagnostics = Vec::new();
    let mut budget = WorkspaceDiscoveryBudget::default();
    match entry {
        OpenedWorkspaceEntry::File {
            mut file,
            reference,
            ..
        } => {
            records.extend(search_open_file(
                &mut file,
                &reference,
                projection.as_ref(),
                &mut matcher,
                operation,
                &mut budget,
            )?);
        }
        OpenedWorkspaceEntry::Directory {
            directory, path, ..
        } => {
            if projection.is_some() {
                return Err(ResourceError::new(
                    ErrorCategory::UnsupportedProjection,
                    "filesystem search selectors require one text file",
                ));
            }
            let controls = DiscoveryControls {
                gitignore: options.gitignore(),
                hidden: options.hidden(),
            };
            let ignores = {
                let mut context = IgnoreContext {
                    controls,
                    diagnostics: &mut diagnostics,
                    operation,
                    budget: &mut budget,
                };
                let InitialIgnoreStack::Ready(ignores) =
                    initial_ignore_stack(&root, &path, GlobKind::Directory, true, &mut context)?
                else {
                    unreachable!("an exact directory search bypasses filters");
                };
                ignores
            };
            walk_workspace(
                &root,
                WalkFrame {
                    directory: Some(directory),
                    path,
                    ignores,
                    pending_state_bytes: 0,
                },
                controls,
                operation,
                &mut diagnostics,
                &mut budget,
                |reference, kind, file, diagnostics, budget| {
                    if kind != GlobKind::File {
                        return Ok(());
                    }
                    let file = file.expect("file kind carries an open file");
                    match search_open_file(file, reference, None, &mut matcher, operation, budget) {
                        Ok(found) => records.extend(found),
                        Err(error) if error.category() == ErrorCategory::Cancelled => {
                            return Err(error);
                        }
                        Err(error) if budget.is_exhausted() => return Err(error),
                        Err(error) => {
                            push_workspace_diagnostic(
                                diagnostics,
                                diagnostic(reference, &error),
                                budget,
                            )?;
                        }
                    }
                    Ok(())
                },
            )?;
        }
    }
    ensure_workspace_discovery_live(operation)?;
    Ok(CompletedWorkspaceDiscovery {
        result: SearchSourceResult::new(engine, records, diagnostics),
        root,
        delivery_identity,
    })
}

fn glob_workspace(
    view: &WorkspaceView,
    target: &GlobTarget,
    options: GlobOptions,
    operation: &OperationGuard,
) -> Result<CompletedWorkspaceDiscovery<SourceGlobResult>, ResourceError> {
    ensure_workspace_discovery_live(operation)?;
    let WorkspaceGlobScope {
        root,
        path,
        match_pattern,
    } = workspace_glob_scope(view, target.pattern(), options.case_sensitive())?;
    let matcher = GlobMatcher::compile(&match_pattern, options.case_sensitive())?;
    let entry = match open_workspace_entry(&root, &path) {
        Ok(entry) => entry,
        Err(error)
            if !path.as_os_str().is_empty() && error.category() == ErrorCategory::NotFound =>
        {
            return Ok(CompletedWorkspaceDiscovery {
                result: SourceGlobResult::new(Vec::new(), Vec::new()),
                delivery_identity: workspace_root_identity(&root),
                root,
            });
        }
        Err(error) => return Err(error),
    };
    let delivery_identity = entry.reference().requested().to_owned();
    let controls = DiscoveryControls {
        gitignore: options.gitignore(),
        hidden: options.hidden(),
    };
    let mut diagnostics = Vec::new();
    let mut budget = WorkspaceDiscoveryBudget::default();
    let entry_kind = entry.kind();
    let ignores = {
        let mut context = IgnoreContext {
            controls,
            diagnostics: &mut diagnostics,
            operation,
            budget: &mut budget,
        };
        initial_ignore_stack(&root, entry.path(), entry_kind, false, &mut context)?
    };
    let InitialIgnoreStack::Ready(ignores) = ignores else {
        return Ok(CompletedWorkspaceDiscovery {
            result: SourceGlobResult::new(Vec::new(), diagnostics),
            root,
            delivery_identity,
        });
    };

    let mut entries = Vec::new();
    let mut collect = |reference: &PathReference,
                       kind: GlobKind,
                       _file: Option<&mut File>,
                       _diagnostics: &mut Vec<DiscoveryDiagnostic>,
                       budget: &mut WorkspaceDiscoveryBudget|
     -> Result<(), ResourceError> {
        if matcher.is_match(workspace_match_path(reference, &root)?) {
            budget.charge_glob_entry(reference, kind)?;
            entries.push(GlobEntry::new(reference.clone(), kind)?);
        }
        Ok(())
    };
    match entry {
        OpenedWorkspaceEntry::File {
            mut file,
            reference,
            ..
        } => collect(
            &reference,
            GlobKind::File,
            Some(&mut file),
            &mut diagnostics,
            &mut budget,
        )?,
        OpenedWorkspaceEntry::Directory {
            directory,
            path,
            reference,
        } => {
            if !path.as_os_str().is_empty() {
                collect(
                    &reference,
                    GlobKind::Directory,
                    None,
                    &mut diagnostics,
                    &mut budget,
                )?;
            }
            walk_workspace(
                &root,
                WalkFrame {
                    directory: Some(directory),
                    path,
                    ignores,
                    pending_state_bytes: 0,
                },
                controls,
                operation,
                &mut diagnostics,
                &mut budget,
                collect,
            )?;
        }
    }
    ensure_workspace_discovery_live(operation)?;
    Ok(CompletedWorkspaceDiscovery {
        result: SourceGlobResult::new(entries, diagnostics),
        root,
        delivery_identity,
    })
}

struct WorkspaceGlobScope {
    root: Arc<FilesystemRoot>,
    path: PathBuf,
    match_pattern: String,
}

fn workspace_glob_scope(
    view: &WorkspaceView,
    pattern: &str,
    case_sensitive: bool,
) -> Result<WorkspaceGlobScope, ResourceError> {
    const PREFIX: &str = "rfs://workspace/";
    if let Some(canonical) = pattern.strip_prefix(PREFIX) {
        let (root, path_pattern) = canonical.split_once('/').ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::InvalidPattern,
                "canonical Workspace glob must include a path pattern",
            )
        })?;
        if path_pattern.is_empty() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidPattern,
                "canonical Workspace glob must include a path pattern",
            ));
        }
        let root_id = WorkspaceRootId::new(root.to_owned())?;
        let root = filesystem_root(view, &root_id)?;
        let fixed = if case_sensitive {
            fixed_glob_prefix(path_pattern)
        } else {
            ""
        };
        let path = if fixed.is_empty() {
            PathBuf::new()
        } else {
            let reference = PathReference::parse(format!("{PREFIX}{root_id}/{fixed}"))?;
            let resolved = resolve_address(
                view,
                reference
                    .workspace_address()
                    .ok_or_else(workspace_glob_required)?,
            )?;
            if resolved.root.metadata.id() != root.metadata.id() {
                return Err(workspace_glob_required());
            }
            resolved.path.as_path().to_owned()
        };
        return Ok(WorkspaceGlobScope {
            root,
            path,
            match_pattern: path_pattern.to_owned(),
        });
    }

    if pattern.contains("://") || Path::new(pattern).is_absolute() {
        return Err(workspace_glob_required());
    }
    let primary = view.roots.primary().ok_or_else(|| {
        ResourceError::new(
            ErrorCategory::AmbiguousReference,
            "relative Workspace glob requires one unique Primary Workspace Root",
        )
    })?;
    let root = filesystem_root(view, primary)?;
    let fixed = if case_sensitive {
        fixed_glob_prefix(pattern)
    } else {
        ""
    };
    let path = if fixed.is_empty() {
        PathBuf::new()
    } else {
        let reference = PathReference::parse(fixed)?;
        let WorkspaceAddress::Relative(path) = reference
            .workspace_address()
            .ok_or_else(workspace_glob_required)?
        else {
            return Err(workspace_glob_required());
        };
        path.as_path().to_owned()
    };
    Ok(WorkspaceGlobScope {
        root,
        path,
        match_pattern: pattern.to_owned(),
    })
}

fn fixed_glob_prefix(pattern: &str) -> &str {
    let mut start = 0_usize;
    for component in pattern.split('/') {
        if component_requires_glob_walk(component) {
            return pattern[..start].trim_end_matches('/');
        }
        start = start.saturating_add(component.len() + 1);
    }
    pattern
}

fn component_requires_glob_walk(component: &str) -> bool {
    let bytes = component.as_bytes();
    let mut index = 0_usize;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1..=index + 2].is_ascii()
        {
            index += 3;
            continue;
        }
        if bytes[index] == b'\\' || matches!(bytes[index], b'*' | b'?' | b'[' | b'{') {
            return true;
        }
        index += 1;
    }
    false
}

fn workspace_glob_required() -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        "filesystem Source Adapter requires a relative or canonical Workspace glob",
    )
}
fn open_search_scope(
    view: &WorkspaceView,
    target: &SearchTarget,
) -> Result<WorkspaceScope, ResourceError> {
    let Some(reference) = target.reference() else {
        let primary = view.roots.primary().ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::AmbiguousReference,
                "Workspace search requires one unique Primary Workspace Root",
            )
        })?;
        let root = filesystem_root(view, primary)?;
        let entry = open_workspace_entry(&root, Path::new(""))?;
        return Ok(WorkspaceScope {
            root,
            entry,
            projection: None,
        });
    };
    let address = reference.workspace_address().ok_or_else(|| {
        ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            "filesystem Source Adapter requires a Workspace search target",
        )
    })?;
    match open_workspace_address(view, address) {
        Ok((root, entry)) => Ok(WorkspaceScope {
            root,
            entry,
            projection: None,
        }),
        Err(error) if error.category() == ErrorCategory::NotFound => {
            if let Some(selector_error) = reference.selector_error() {
                return Err(selector_error.clone());
            }
            let Some(candidate) = reference.selector_candidate() else {
                return Err(error);
            };
            let (root, entry) = open_workspace_address(view, candidate.base())?;
            Ok(WorkspaceScope {
                root,
                entry,
                projection: Some(candidate.selector().clone()),
            })
        }
        Err(error) => Err(error),
    }
}

fn open_workspace_address(
    view: &WorkspaceView,
    address: &WorkspaceAddress,
) -> Result<(Arc<FilesystemRoot>, OpenedWorkspaceEntry), ResourceError> {
    let resolved = resolve_address(view, address)?;
    let entry = open_workspace_entry(&resolved.root, resolved.path.as_path())?;
    Ok((resolved.root, entry))
}

fn open_workspace_entry(
    root: &Arc<FilesystemRoot>,
    path: &Path,
) -> Result<OpenedWorkspaceEntry, ResourceError> {
    if path.as_os_str().is_empty() {
        let directory = root
            .directory
            .try_clone()
            .map_err(|error| root_error(root.metadata.id(), "clone directory", error))?;
        let final_path = normalize_handle_path(
            final_directory_path(&directory)
                .map_err(|error| root_error(root.metadata.id(), "resolve directory", error))?,
        )?;
        if !strip_beneath(&final_path, &root.canonical_path)
            .is_some_and(|path| path.as_os_str().is_empty())
        {
            return Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "Workspace Root handle no longer resolves to its configured directory",
            ));
        }
        return Ok(OpenedWorkspaceEntry::Directory {
            directory,
            path: PathBuf::new(),
            reference: PathReference::canonical(root.metadata.id().clone(), WorkspacePath::root()),
        });
    }

    let workspace_path = WorkspacePath::new(path)?;
    let provisional = PathReference::canonical(root.metadata.id().clone(), workspace_path.clone());
    let identity = provisional.requested();
    let metadata = capability_metadata(root, &workspace_path, identity)?;
    if metadata.is_dir() {
        let directory = open_capability_directory(root, &workspace_path, identity)?;
        let final_path = normalize_handle_path(
            final_directory_path(&directory)
                .map_err(|error| resource_io_error(identity, "resolve", error))?,
        )?;
        let final_workspace_path = contained_workspace_path(&final_path, &root.canonical_path)?;
        let reference =
            PathReference::canonical(root.metadata.id().clone(), final_workspace_path.clone());
        return Ok(OpenedWorkspaceEntry::Directory {
            directory,
            path: final_workspace_path.as_path().to_owned(),
            reference,
        });
    }
    if metadata.is_file() {
        let file = open_capability_file(root, &workspace_path, identity)?;
        let actual = file
            .metadata()
            .map_err(|error| resource_io_error(identity, "inspect", error))?;
        if !actual.is_file() {
            return Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("Resource '{identity}' changed type while opening"),
            ));
        }
        let final_path = normalize_handle_path(
            final_file_path(&file)
                .map_err(|error| resource_io_error(identity, "resolve", error))?,
        )?;
        let final_workspace_path = contained_workspace_path(&final_path, &root.canonical_path)?;
        let reference =
            PathReference::canonical(root.metadata.id().clone(), final_workspace_path.clone());
        return Ok(OpenedWorkspaceEntry::File {
            file,
            path: final_workspace_path.as_path().to_owned(),
            reference,
        });
    }
    Err(ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        format!("Resource '{identity}' is not a text file or directory"),
    ))
}

fn capability_metadata(
    root: &FilesystemRoot,
    path: &WorkspacePath,
    identity: &str,
) -> Result<cap_std::fs::Metadata, ResourceError> {
    let mut candidate = path.clone();
    for _ in 0..40 {
        match root.directory.metadata(candidate.as_path()) {
            Ok(metadata) => return Ok(metadata),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                let Some(resolved) = rewrite_absolute_symlink(root, &candidate)? else {
                    return Err(resource_io_error(identity, "inspect", error));
                };
                candidate = resolved;
            }
            Err(error) => return Err(resource_io_error(identity, "inspect", error)),
        }
    }
    Err(ResourceError::new(
        ErrorCategory::PermissionDenied,
        format!("Resource '{identity}' exceeds the symlink resolution limit"),
    ))
}

fn open_capability_directory(
    root: &FilesystemRoot,
    path: &WorkspacePath,
    identity: &str,
) -> Result<Dir, ResourceError> {
    let mut candidate = path.clone();
    for _ in 0..40 {
        match root.directory.open_dir(candidate.as_path()) {
            Ok(directory) => return Ok(directory),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                let Some(resolved) = rewrite_absolute_symlink(root, &candidate)? else {
                    return Err(resource_io_error(identity, "open directory", error));
                };
                candidate = resolved;
            }
            Err(error) => return Err(resource_io_error(identity, "open directory", error)),
        }
    }
    Err(ResourceError::new(
        ErrorCategory::PermissionDenied,
        format!("Resource '{identity}' exceeds the symlink resolution limit"),
    ))
}

fn search_open_file(
    file: &mut File,
    reference: &PathReference,
    projection: Option<&ProjectionSelector>,
    matcher: &mut SearchMatcher,
    operation: &OperationGuard,
    budget: &mut WorkspaceDiscoveryBudget,
) -> Result<Vec<SearchRecord>, ResourceError> {
    if projection.is_some_and(|selector| selector.page_offset().is_some()) {
        return Err(ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            "page selectors cannot target Workspace files",
        ));
    }
    let selection = projection.and_then(ProjectionSelector::line_selection);
    let mut reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut line = Vec::new();
    let mut line_number = 0_u64;
    let mut source_bytes = 0_usize;
    loop {
        let remaining = MAX_ARTIFACT_BYTES.saturating_sub(source_bytes);
        let read = read_bounded_line(
            &mut reader,
            &mut line,
            remaining,
            operation,
            reference.requested(),
            "search",
            "Workspace search source exceeds the 64 MiB Resource ceiling",
        )?;
        if read == 0 {
            break;
        }
        source_bytes = source_bytes.checked_add(read).ok_or_else(|| {
            limit_error("Workspace search source byte count is not representable")
        })?;
        budget.charge_scan(read)?;
        line_number = line_number
            .checked_add(1)
            .ok_or_else(|| limit_error("Workspace search line number is not representable"))?;
        let mut content_end = line.len();
        if line.get(content_end.wrapping_sub(1)) == Some(&b'\n') {
            content_end -= 1;
            if line.get(content_end.wrapping_sub(1)) == Some(&b'\r') {
                content_end -= 1;
            }
        }
        let text = std::str::from_utf8(&line[..content_end]).map_err(|_| {
            ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                format!(
                    "Resource '{}' is not valid UTF-8 text",
                    reference.requested()
                ),
            )
        })?;
        if !line_is_selected(line_number, selection) {
            continue;
        }
        budget.charge_match(matcher.engine())?;
        if !matcher.is_match(text)? {
            continue;
        }
        budget.charge_search_record(reference, line_number, text)?;
        let text = if line.len() > 1024 * 1024 {
            line.truncate(content_end);
            String::from_utf8(std::mem::take(&mut line))
                .expect("the Workspace search line was validated as UTF-8")
        } else {
            text.to_owned()
        };
        records.push(SearchRecord::new(reference.clone(), line_number, text)?);
    }
    ensure_workspace_discovery_live(operation)?;
    Ok(records)
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
    remaining: usize,
    operation: &OperationGuard,
    identity: &str,
    io_operation: &str,
    limit_message: &str,
) -> Result<usize, ResourceError> {
    line.clear();
    loop {
        ensure_workspace_discovery_live(operation)?;
        let (consumed, terminated) = {
            let available = reader
                .fill_buf()
                .map_err(|error| resource_io_error(identity, io_operation, error))?;
            if available.is_empty() {
                return Ok(line.len());
            }
            let consumed = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |position| position + 1);
            let total = line
                .len()
                .checked_add(consumed)
                .ok_or_else(|| limit_error(limit_message))?;
            if total > remaining {
                return Err(limit_error(limit_message));
            }
            line.extend_from_slice(&available[..consumed]);
            (
                consumed,
                available.get(consumed.wrapping_sub(1)) == Some(&b'\n'),
            )
        };
        reader.consume(consumed);
        if terminated {
            return Ok(line.len());
        }
    }
}

fn line_is_selected(line: u64, selection: Option<&LineSelector>) -> bool {
    selection.is_none_or(|selection| {
        selection.ranges().iter().any(|range| {
            range.start() <= line
                && range
                    .inclusive_end()
                    .is_none_or(|inclusive_end| line <= inclusive_end)
        })
    })
}

fn initial_ignore_stack(
    root: &Arc<FilesystemRoot>,
    path: &Path,
    kind: GlobKind,
    bypass_filters: bool,
    context: &mut IgnoreContext<'_>,
) -> Result<InitialIgnoreStack, ResourceError> {
    let mut stack = None;
    if context.controls.gitignore {
        stack = load_gitignore(root, Path::new(""), stack, context)?;
    }
    let components = path.components().collect::<Vec<_>>();
    let mut current = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        ensure_workspace_discovery_live(context.operation)?;
        let Component::Normal(name) = component else {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "Workspace discovery path contains a non-relative component",
            ));
        };
        current.push(name);
        let last = index + 1 == components.len();
        let is_directory = !last || kind == GlobKind::Directory;
        if !bypass_filters
            && ((!context.controls.hidden && is_hidden_name(name))
                || (context.controls.gitignore
                    && ignored_by_stack(stack.as_ref(), &current, is_directory)))
        {
            return Ok(InitialIgnoreStack::Filtered);
        }
        if context.controls.gitignore && is_directory {
            stack = load_gitignore(root, &current, stack, context)?;
        }
    }
    Ok(InitialIgnoreStack::Ready(stack))
}

fn final_entry_ignore_stack(
    root: &Arc<FilesystemRoot>,
    path: &Path,
    kind: GlobKind,
    cache: &mut HashMap<PathBuf, InitialIgnoreStack>,
    context: &mut IgnoreContext<'_>,
) -> Result<InitialIgnoreStack, ResourceError> {
    let parent = if kind == GlobKind::Directory {
        path
    } else {
        path.parent().unwrap_or_else(|| Path::new(""))
    };
    let mut current = PathBuf::new();
    let mut state = cache
        .get(&current)
        .cloned()
        .expect("final-filter cache contains the Workspace Root");
    for component in parent.components() {
        ensure_workspace_discovery_live(context.operation)?;
        let Component::Normal(name) = component else {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "Workspace discovery path contains a non-relative component",
            ));
        };
        current.push(name);
        if let Some(cached) = cache.get(&current) {
            state = cached.clone();
            continue;
        }
        state = match state {
            InitialIgnoreStack::Filtered => InitialIgnoreStack::Filtered,
            InitialIgnoreStack::Ready(parent) => {
                if (!context.controls.hidden && is_hidden_name(name))
                    || (context.controls.gitignore
                        && ignored_by_stack(parent.as_ref(), &current, true))
                {
                    InitialIgnoreStack::Filtered
                } else {
                    let ignores = if context.controls.gitignore {
                        load_gitignore(root, &current, parent, context)?
                    } else {
                        None
                    };
                    InitialIgnoreStack::Ready(ignores)
                }
            }
        };
        context.budget.charge_state(retained_path_state_bytes(
            &current,
            size_of::<InitialIgnoreStack>(),
        )?)?;
        cache.insert(current.clone(), state.clone());
    }
    let InitialIgnoreStack::Ready(parent) = state else {
        return Ok(InitialIgnoreStack::Filtered);
    };
    if kind == GlobKind::File {
        let name = path.file_name().ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::InvalidReference,
                "Workspace file has no final path component",
            )
        })?;
        if (!context.controls.hidden && is_hidden_name(name))
            || (context.controls.gitignore && ignored_by_stack(parent.as_ref(), path, false))
        {
            return Ok(InitialIgnoreStack::Filtered);
        }
    }
    Ok(InitialIgnoreStack::Ready(parent))
}

fn load_gitignore(
    root: &Arc<FilesystemRoot>,
    directory_path: &Path,
    parent: Option<Arc<IgnoreStack>>,
    context: &mut IgnoreContext<'_>,
) -> Result<Option<Arc<IgnoreStack>>, ResourceError> {
    let operation = context.operation;
    let diagnostics = &mut *context.diagnostics;
    let budget = &mut *context.budget;
    ensure_workspace_discovery_live(operation)?;
    let path = directory_path.join(".gitignore");
    let workspace_path = WorkspacePath::new(&path)?;
    let provisional = PathReference::canonical(root.metadata.id().clone(), workspace_path);
    let entry = match open_workspace_entry(root, &path) {
        Ok(entry) => entry,
        Err(error) if error.category() == ErrorCategory::NotFound => return Ok(parent),
        Err(error) => {
            push_workspace_diagnostic(diagnostics, diagnostic(&provisional, &error), budget)?;
            return Ok(parent);
        }
    };
    let OpenedWorkspaceEntry::File {
        file, reference, ..
    } = entry
    else {
        push_workspace_diagnostic(
            diagnostics,
            DiscoveryDiagnostic::new(
                Some(provisional),
                ErrorCategory::UnsupportedProjection,
                "contained .gitignore Resource is not a text file",
            ),
            budget,
        )?;
        return Ok(parent);
    };
    let mut reader = BufReader::new(file);
    let mut builder = GitignoreBuilder::new(directory_path);
    let mut line = Vec::new();
    let mut total = 0_usize;
    let mut line_number = 0_u64;
    let retained_rule_overhead = path
        .as_os_str()
        .as_encoded_bytes()
        .len()
        .checked_add(size_of::<PathBuf>())
        .and_then(|bytes| bytes.checked_add(RETAINED_COLLECTION_ENTRY_BYTES))
        .ok_or_else(|| limit_error("contained .gitignore state size is not representable"))?;
    loop {
        let remaining = MAX_ARTIFACT_BYTES.saturating_sub(total);
        let read = match read_bounded_line(
            &mut reader,
            &mut line,
            remaining,
            operation,
            reference.requested(),
            "read",
            "contained .gitignore exceeds the 64 MiB Resource ceiling",
        ) {
            Ok(read) => read,
            Err(error) if error.category() == ErrorCategory::Cancelled => return Err(error),
            Err(error) => {
                push_workspace_diagnostic(
                    diagnostics,
                    DiscoveryDiagnostic::new(
                        Some(reference.clone()),
                        error.category(),
                        if error.category() == ErrorCategory::LimitExceeded {
                            "contained .gitignore exceeds the 64 MiB Resource ceiling"
                        } else {
                            "contained .gitignore could not be read"
                        },
                    ),
                    budget,
                )?;
                return Ok(parent);
            }
        };
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read)
            .ok_or_else(|| limit_error("contained .gitignore byte count is not representable"))?;
        budget.charge_scan(read)?;
        line_number = line_number.saturating_add(1);
        let rule = match std::str::from_utf8(&line) {
            Ok(rule) => rule,
            Err(_) => {
                push_workspace_diagnostic(
                    diagnostics,
                    DiscoveryDiagnostic::new(
                        Some(reference.clone()),
                        ErrorCategory::UnsupportedProjection,
                        "contained .gitignore could not be read as UTF-8 text",
                    ),
                    budget,
                )?;
                return Ok(parent);
            }
        };
        let rule = rule.strip_suffix('\n').unwrap_or(rule);
        let rule = rule.strip_suffix('\r').unwrap_or(rule);
        let rule = if line_number == 1 {
            rule.trim_start_matches('\u{feff}')
        } else {
            rule
        };
        let retained_rule_bytes = retained_rule_overhead
            .checked_add(rule.len())
            .ok_or_else(|| limit_error("contained .gitignore state size is not representable"))?;
        budget.charge_state(retained_rule_bytes)?;
        if let Err(error) = builder.add_line(Some(path.clone()), rule) {
            push_workspace_diagnostic(
                diagnostics,
                DiscoveryDiagnostic::new(
                    Some(reference.clone()),
                    ErrorCategory::InvalidPattern,
                    format!("invalid contained .gitignore line {line_number}: {error}"),
                ),
                budget,
            )?;
        }
    }
    let matcher = match builder.build() {
        Ok(matcher) => matcher,
        Err(error) => {
            push_workspace_diagnostic(
                diagnostics,
                DiscoveryDiagnostic::new(
                    Some(reference),
                    ErrorCategory::InvalidPattern,
                    format!("contained .gitignore could not be compiled: {error}"),
                ),
                budget,
            )?;
            return Ok(parent);
        }
    };
    if matcher.is_empty() {
        return Ok(parent);
    }
    Ok(Some(Arc::new(IgnoreStack { matcher, parent })))
}

fn ignored_by_stack(stack: Option<&Arc<IgnoreStack>>, path: &Path, is_directory: bool) -> bool {
    let mut current = stack.map(Arc::as_ref);
    while let Some(ignores) = current {
        match ignores.matcher.matched(path, is_directory) {
            Match::Ignore(_) => return true,
            Match::Whitelist(_) => return false,
            Match::None => current = ignores.parent.as_deref(),
        }
    }
    false
}

fn walk_workspace<F>(
    root: &Arc<FilesystemRoot>,
    initial: WalkFrame,
    controls: DiscoveryControls,
    operation: &OperationGuard,
    diagnostics: &mut Vec<DiscoveryDiagnostic>,
    budget: &mut WorkspaceDiscoveryBudget,
    mut visit: F,
) -> Result<(), ResourceError>
where
    F: FnMut(
        &PathReference,
        GlobKind,
        Option<&mut File>,
        &mut Vec<DiscoveryDiagnostic>,
        &mut WorkspaceDiscoveryBudget,
    ) -> Result<(), ResourceError>,
{
    let mut seen_directories = HashSet::new();
    budget.charge_state(retained_path_state_bytes(&initial.path, 0)?)?;
    seen_directories.insert(initial.path.clone());
    let mut seen_files = HashSet::new();
    let root_ignores = if initial.path.as_os_str().is_empty() {
        initial.ignores.clone()
    } else if controls.gitignore {
        let mut context = IgnoreContext {
            controls,
            diagnostics,
            operation,
            budget,
        };
        load_gitignore(root, Path::new(""), None, &mut context)?
    } else {
        None
    };
    budget.charge_state(retained_path_state_bytes(
        Path::new(""),
        size_of::<InitialIgnoreStack>(),
    )?)?;
    let mut final_filter_cache =
        HashMap::from([(PathBuf::new(), InitialIgnoreStack::Ready(root_ignores))]);
    if !initial.path.as_os_str().is_empty() {
        budget.charge_state(retained_path_state_bytes(
            &initial.path,
            size_of::<InitialIgnoreStack>(),
        )?)?;
        final_filter_cache.insert(
            initial.path.clone(),
            InitialIgnoreStack::Ready(initial.ignores.clone()),
        );
    }
    let mut pending = vec![initial];
    while let Some(mut frame) = pending.pop() {
        budget.release_state(frame.pending_state_bytes);
        ensure_workspace_discovery_live(operation)?;
        let directory = match frame.directory.take() {
            Some(directory) => directory,
            None => match open_workspace_entry(root, &frame.path) {
                Ok(OpenedWorkspaceEntry::Directory { directory, .. }) => directory,
                Ok(OpenedWorkspaceEntry::File { reference, .. }) => {
                    push_workspace_diagnostic(
                        diagnostics,
                        DiscoveryDiagnostic::new(
                            Some(reference),
                            ErrorCategory::SourceUnavailable,
                            "Workspace directory changed to a file during discovery",
                        ),
                        budget,
                    )?;
                    continue;
                }
                Err(error) => {
                    let reference = WorkspacePath::new(&frame.path)
                        .ok()
                        .map(|path| PathReference::canonical(root.metadata.id().clone(), path));
                    push_workspace_diagnostic(
                        diagnostics,
                        DiscoveryDiagnostic::new(
                            reference,
                            error.category(),
                            error.message().to_owned(),
                        ),
                        budget,
                    )?;
                    continue;
                }
            },
        };
        let mut names = Vec::new();
        let entries = match directory.entries() {
            Ok(entries) => entries,
            Err(error) => {
                push_workspace_diagnostic(
                    diagnostics,
                    directory_diagnostic(root, &frame.path, "enumerate", error),
                    budget,
                )?;
                continue;
            }
        };
        for entry in entries {
            ensure_workspace_discovery_live(operation)?;
            budget.charge_entry()?;
            match entry {
                Ok(entry) => names.push(entry.file_name()),
                Err(error) => {
                    push_workspace_diagnostic(
                        diagnostics,
                        directory_diagnostic(root, &frame.path, "enumerate entry", error),
                        budget,
                    )?;
                }
            }
        }
        drop(directory);
        names.sort_unstable();
        let names_state_bytes = retained_names_state_bytes(&names, names.capacity())?;
        budget.charge_state(names_state_bytes)?;
        for name in names {
            ensure_workspace_discovery_live(operation)?;
            if !controls.hidden && is_hidden_name(&name) {
                continue;
            }
            let candidate_path = frame.path.join(&name);
            let candidate = match WorkspacePath::new(&candidate_path) {
                Ok(path) => path,
                Err(error) => {
                    push_workspace_diagnostic(
                        diagnostics,
                        DiscoveryDiagnostic::new(
                            None,
                            error.category(),
                            "Workspace entry name is not a valid UTF-8 Path Reference",
                        ),
                        budget,
                    )?;
                    continue;
                }
            };
            let provisional =
                PathReference::canonical(root.metadata.id().clone(), candidate.clone());
            if controls.gitignore
                && ignored_by_stack(frame.ignores.as_ref(), &candidate_path, false)
            {
                continue;
            }
            let mut entry = match open_workspace_entry(root, &candidate_path) {
                Ok(entry) => entry,
                Err(error) => {
                    push_workspace_diagnostic(
                        diagnostics,
                        diagnostic(&provisional, &error),
                        budget,
                    )?;
                    continue;
                }
            };
            let is_directory = entry.kind() == GlobKind::Directory;
            if controls.gitignore
                && ignored_by_stack(frame.ignores.as_ref(), &candidate_path, is_directory)
            {
                continue;
            }
            let final_ignores = {
                let mut context = IgnoreContext {
                    controls,
                    diagnostics,
                    operation,
                    budget,
                };
                final_entry_ignore_stack(
                    root,
                    entry.path(),
                    entry.kind(),
                    &mut final_filter_cache,
                    &mut context,
                )?
            };
            let final_ignores = match final_ignores {
                InitialIgnoreStack::Filtered => continue,
                InitialIgnoreStack::Ready(ignores) => ignores,
            };
            match &mut entry {
                OpenedWorkspaceEntry::File {
                    file, reference, ..
                } => {
                    if seen_files.contains(reference.requested()) {
                        continue;
                    }
                    budget.charge_state(retained_string_state_bytes(reference.requested())?)?;
                    seen_files.insert(reference.requested().to_owned());
                    visit(reference, GlobKind::File, Some(file), diagnostics, budget)?;
                }
                OpenedWorkspaceEntry::Directory {
                    directory: _,
                    path,
                    reference,
                } => {
                    if seen_directories.contains(path) {
                        continue;
                    }
                    budget.charge_state(retained_path_state_bytes(path, 0)?)?;
                    seen_directories.insert(path.clone());
                    visit(reference, GlobKind::Directory, None, diagnostics, budget)?;
                    let child_ignores = final_ignores;
                    let pending_state_bytes =
                        retained_path_state_bytes(path, size_of::<WalkFrame>())?;
                    budget.charge_state(pending_state_bytes)?;
                    pending.push(WalkFrame {
                        directory: None,
                        path: path.clone(),
                        ignores: child_ignores,
                        pending_state_bytes,
                    });
                }
            }
        }
        budget.release_state(names_state_bytes);
    }
    Ok(())
}

fn is_hidden_name(name: &std::ffi::OsStr) -> bool {
    name.as_encoded_bytes().first().copied() == Some(b'.')
}

fn workspace_match_path<'a>(
    reference: &'a PathReference,
    root: &FilesystemRoot,
) -> Result<&'a str, ResourceError> {
    let prefix = workspace_root_identity(root);
    reference.requested().strip_prefix(&prefix).ok_or_else(|| {
        ResourceError::new(
            ErrorCategory::InvalidReference,
            "Workspace discovery produced an identity under the wrong root",
        )
    })
}

fn workspace_root_identity(root: &FilesystemRoot) -> String {
    format!("rfs://workspace/{}/", root.metadata.id())
}

fn diagnostic(reference: &PathReference, error: &ResourceError) -> DiscoveryDiagnostic {
    DiscoveryDiagnostic::new(Some(reference.clone()), error.category(), error.message())
}

fn push_workspace_diagnostic(
    diagnostics: &mut Vec<DiscoveryDiagnostic>,
    diagnostic: DiscoveryDiagnostic,
    budget: &mut WorkspaceDiscoveryBudget,
) -> Result<(), ResourceError> {
    budget.charge_diagnostic(&diagnostic)?;
    diagnostics.push(diagnostic);
    Ok(())
}

fn directory_diagnostic(
    root: &FilesystemRoot,
    path: &Path,
    operation: &str,
    error: io::Error,
) -> DiscoveryDiagnostic {
    let reference = WorkspacePath::new(path)
        .ok()
        .map(|path| PathReference::canonical(root.metadata.id().clone(), path));
    let identity = reference.as_ref().map_or_else(
        || workspace_root_identity(root),
        |reference| reference.requested().to_owned(),
    );
    let error = resource_io_error(&identity, operation, error);
    DiscoveryDiagnostic::new(reference, error.category(), error.message())
}

fn ensure_workspace_discovery_live(operation: &OperationGuard) -> Result<(), ResourceError> {
    if operation.is_active() {
        Ok(())
    } else {
        Err(ResourceError::new(
            ErrorCategory::Cancelled,
            "Workspace discovery operation was cancelled",
        ))
    }
}

fn workspace_discovery_worker_error(error: tokio::task::JoinError) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!("filesystem discovery worker failed: {error}"),
    )
}
async fn construct_launch_view(
    roots: Vec<LaunchRoot>,
    primary_selector: Option<String>,
) -> Result<WorkspaceView, ResourceError> {
    construct_view(ROOT_CONSTRUCTION_TIMEOUT, move || {
        build_launch_workspace_view(roots, primary_selector.as_deref())
    })
    .await
}

async fn construct_client_view(
    roots: Vec<ClientRoot>,
    primary_selector: Option<String>,
    profile_grants: Arc<HashMap<PathBuf, MutationGrants>>,
    timeout: Duration,
) -> Result<WorkspaceView, ResourceError> {
    if roots.len() > MAX_WORKSPACE_ROOTS {
        return Err(limit_error("Workspace Root count exceeds 256"));
    }
    construct_view(timeout, move || {
        build_client_workspace_view(roots, primary_selector.as_deref(), profile_grants.as_ref())
    })
    .await
}

async fn construct_view(
    timeout: Duration,
    build: impl FnOnce() -> Result<WorkspaceView, ResourceError> + Send + 'static,
) -> Result<WorkspaceView, ResourceError> {
    let construction = tokio::task::spawn_blocking(build);
    tokio::time::timeout(timeout, construction)
        .await
        .map_err(|_| authority_unavailable("Workspace Root acquisition exceeded 5 seconds"))?
        .map_err(|error| {
            authority_unavailable(&format!("Workspace Root acquisition task failed: {error}"))
        })?
}

fn build_launch_workspace_view(
    roots: Vec<LaunchRoot>,
    primary_selector: Option<&str>,
) -> Result<WorkspaceView, ResourceError> {
    let mut opened = Vec::with_capacity(roots.len());
    for root in roots {
        let directory = Dir::open_ambient_dir(&root.path, ambient_authority())
            .map_err(|error| root_error(&root.id, "open", error))?;
        let metadata = directory
            .dir_metadata()
            .map_err(|error| root_error(&root.id, "inspect", error))?;
        if !metadata.is_dir() {
            return Err(authority_unavailable(&format!(
                "configured Workspace Root '{}' is not a directory",
                root.id
            )));
        }
        let canonical_path = normalize_handle_path(
            final_directory_path(&directory)
                .map_err(|error| root_error(&root.id, "resolve final path for", error))?,
        )?;
        let canonical_uri = directory_uri(&canonical_path, Some(&root.id))?;
        let metadata = WorkspaceRoot::new(
            root.id.clone(),
            canonical_uri,
            Some(root.id.as_str().to_owned()),
        )?;
        opened.push(Arc::new(FilesystemRoot {
            metadata,
            canonical_path,
            directory,
            grants: root.grants,
        }));
    }
    workspace_view(opened, primary_selector)
}

fn build_client_workspace_view(
    roots: Vec<ClientRoot>,
    primary_selector: Option<&str>,
    profile_grants: &HashMap<PathBuf, MutationGrants>,
) -> Result<WorkspaceView, ResourceError> {
    let mut opened = Vec::with_capacity(roots.len());
    for root in roots {
        let uri = Url::parse(&root.uri).map_err(|_| {
            ResourceError::new(
                ErrorCategory::InvalidReference,
                "client Workspace Root URI is invalid",
            )
        })?;
        if uri.scheme() != "file" || uri.query().is_some() || uri.fragment().is_some() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "client Workspace Root must be a local file URI",
            ));
        }
        let path = uri.to_file_path().map_err(|()| {
            ResourceError::new(
                ErrorCategory::InvalidReference,
                "client Workspace Root must be a local file URI",
            )
        })?;
        let directory = Dir::open_ambient_dir(path, ambient_authority())
            .map_err(|error| client_root_error("open", error))?;
        let metadata = directory
            .dir_metadata()
            .map_err(|error| client_root_error("inspect", error))?;
        if !metadata.is_dir() {
            return Err(authority_unavailable(
                "configured client Workspace Root is not a directory",
            ));
        }
        let canonical_path = normalize_handle_path(
            final_directory_path(&directory)
                .map_err(|error| client_root_error("resolve final path for", error))?,
        )?;
        let grants = profile_grants
            .get(&canonical_path)
            .copied()
            .unwrap_or_default();
        let canonical_uri = directory_uri(&canonical_path, None)?;
        let id = client_root_id(&canonical_uri)?;
        let metadata = WorkspaceRoot::new(id, canonical_uri, root.name).map_err(|_| {
            authority_unavailable(
                "client Workspace Root display name must use Workspace Root ID grammar",
            )
        })?;
        opened.push(Arc::new(FilesystemRoot {
            metadata,
            canonical_path,
            directory,
            grants,
        }));
    }
    workspace_view(opened, primary_selector)
}

fn directory_uri(
    canonical_path: &Path,
    root_id: Option<&WorkspaceRootId>,
) -> Result<String, ResourceError> {
    Url::from_directory_path(canonical_path)
        .map_err(|()| match root_id {
            Some(root_id) => authority_unavailable(&format!(
                "configured Workspace Root '{root_id}' has no local file URI"
            )),
            None => authority_unavailable("configured client Workspace Root has no local file URI"),
        })
        .map(|uri| uri.to_string())
}

fn client_root_id(canonical_uri: &str) -> Result<WorkspaceRootId, ResourceError> {
    let digest = Sha256::digest(canonical_uri.as_bytes());
    WorkspaceRootId::new(format!("client-{digest:x}"))
}

fn workspace_view(
    opened: Vec<Arc<FilesystemRoot>>,
    primary_selector: Option<&str>,
) -> Result<WorkspaceView, ResourceError> {
    let root_set = Arc::new(WorkspaceRootSet::new(
        opened.iter().map(|root| root.metadata.clone()).collect(),
        primary_selector,
    )?);
    let filesystems = opened
        .into_iter()
        .map(|root| (root.metadata.id().clone(), root))
        .collect();
    Ok(WorkspaceView {
        roots: root_set,
        filesystems,
    })
}

fn view_retains_root(view: &WorkspaceView, prior: &FilesystemRoot) -> bool {
    view.roots
        .get(prior.metadata.id())
        .is_some_and(|current| current.canonical_uri() == prior.metadata.canonical_uri())
}

struct CompletedRead {
    resource: SourceResource,
    root: Arc<FilesystemRoot>,
}

fn read_from_view(
    view: &WorkspaceView,
    reference: &PathReference,
    visibility: BackingPathVisibility,
) -> Result<CompletedRead, ResourceError> {
    let address = reference.workspace_address().ok_or_else(|| {
        ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            "filesystem Source Adapter cannot read non-workspace Resources",
        )
    })?;
    let result = read_address(view, address, None, visibility);
    if !result
        .as_ref()
        .is_err_and(|error| error.category() == ErrorCategory::NotFound)
    {
        return result;
    }
    if let Some(error) = reference.selector_error() {
        return Err(error.clone());
    }
    if let Some(candidate) = reference.selector_candidate() {
        return read_address(
            view,
            candidate.base(),
            Some(candidate.selector()),
            visibility,
        );
    }
    result
}

struct ResolvedAddress {
    root: Arc<FilesystemRoot>,
    path: WorkspacePath,
    absolute_input: bool,
}

struct MatchingRoots {
    resources: Vec<(Arc<FilesystemRoot>, WorkspacePath)>,
    root_directory: bool,
}

fn read_address(
    view: &WorkspaceView,
    address: &WorkspaceAddress,
    projection: Option<&ProjectionSelector>,
    visibility: BackingPathVisibility,
) -> Result<CompletedRead, ResourceError> {
    let resolved = resolve_address(view, address)?;
    let provisional =
        PathReference::canonical(resolved.root.metadata.id().clone(), resolved.path.clone());
    let identity = provisional.requested();
    if resolved.path.is_root() {
        return Err(ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            format!(
                "Resource '{identity}' is a directory; enumerate it with rfs_glob or read a file below it; bounded directory listings are tracked by rfs-hwlm"
            ),
        ));
    }
    let mut file = open_capability_file(&resolved.root, &resolved.path, identity)?;
    let metadata = file
        .metadata()
        .map_err(|error| resource_io_error(identity, "inspect", error))?;
    if !metadata.is_file() {
        return Err(ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            format!("Resource '{identity}' is not a text file"),
        ));
    }

    let final_path = normalize_handle_path(
        final_file_path(&file).map_err(|error| resource_io_error(identity, "resolve", error))?,
    )?;
    let relative_path = contained_workspace_path(&final_path, &resolved.root.canonical_path)?;
    if resolved.absolute_input {
        let final_matches = matching_roots(view, &final_path)?;
        match (
            final_matches.resources.as_slice(),
            final_matches.root_directory,
        ) {
            ([(root, _)], false) if root.metadata.id() == resolved.root.metadata.id() => {}
            ([], false) => {
                return Err(ResourceError::new(
                    ErrorCategory::PermissionDenied,
                    "absolute Resource resolves outside every configured Workspace Root",
                ));
            }
            _ => {
                return Err(ResourceError::new(
                    ErrorCategory::AmbiguousReference,
                    "absolute Resource resolves beneath multiple Workspace Roots",
                ));
            }
        }
    }
    let mutable = resolved.root.grants.update() && relative_path == resolved.path;
    let canonical_reference =
        PathReference::canonical(resolved.root.metadata.id().clone(), relative_path);
    let selected = select_utf8(&mut file, projection)?;
    let mut resource =
        SourceResource::selected_text(canonical_reference, selected)?.with_mutability(mutable);
    if visibility == BackingPathVisibility::Visible {
        let backing_uri = Url::from_file_path(&final_path)
            .map_err(|()| {
                ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    format!("Resource '{identity}' has no local backing URI"),
                )
            })?
            .to_string();
        resource = resource.with_backing_file_uri(backing_uri)?;
    }
    Ok(CompletedRead {
        resource,
        root: resolved.root,
    })
}

fn open_capability_file(
    root: &FilesystemRoot,
    path: &WorkspacePath,
    identity: &str,
) -> Result<File, ResourceError> {
    let mut candidate = path.clone();
    for _ in 0..40 {
        match root.directory.open(candidate.as_path()) {
            Ok(file) => return Ok(file),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                let Some(resolved) = rewrite_absolute_symlink(root, &candidate)? else {
                    return Err(resource_io_error(identity, "open", error));
                };
                candidate = resolved;
            }
            Err(error) => return Err(resource_io_error(identity, "open", error)),
        }
    }
    Err(ResourceError::new(
        ErrorCategory::PermissionDenied,
        format!("Resource '{identity}' exceeds the symlink resolution limit"),
    ))
}

fn rewrite_absolute_symlink(
    root: &FilesystemRoot,
    path: &WorkspacePath,
) -> Result<Option<WorkspacePath>, ResourceError> {
    let components = path.as_path().components().collect::<Vec<_>>();
    let mut prefix = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(value) = component else {
            unreachable!("WorkspacePath contains only normal components");
        };
        prefix.push(value);
        let metadata = match root.directory.symlink_metadata(&prefix) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(None),
        };
        if !metadata.file_type().is_symlink() {
            continue;
        }
        let target = root
            .directory
            .read_link_contents(&prefix)
            .map_err(|error| resource_io_error("symlink", "resolve", error))?;
        if !target.is_absolute() {
            continue;
        }
        let target = normalize_platform_path(target);
        let Some(relative_target) = strip_beneath(&target, &root.canonical_path) else {
            return Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "Resource symlink resolves outside the selected Workspace Root",
            ));
        };
        let mut rewritten = normalize_relative_symlink_target(&relative_target)?;
        for remainder in &components[index + 1..] {
            rewritten.push(remainder.as_os_str());
        }
        return WorkspacePath::new(rewritten).map(Some);
    }
    Ok(None)
}

fn normalize_relative_symlink_target(path: &Path) -> Result<PathBuf, ResourceError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ResourceError::new(
                    ErrorCategory::PermissionDenied,
                    "Resource symlink resolves outside the selected Workspace Root",
                ));
            }
        }
    }
    Ok(normalized)
}

fn resolve_address(
    view: &WorkspaceView,
    address: &WorkspaceAddress,
) -> Result<ResolvedAddress, ResourceError> {
    match address {
        WorkspaceAddress::Relative(path) => {
            let primary = view.roots.primary().ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::AmbiguousReference,
                    "relative Path Reference requires one unique Primary Workspace Root",
                )
            })?;
            Ok(ResolvedAddress {
                root: filesystem_root(view, primary)?,
                path: path.clone(),
                absolute_input: false,
            })
        }
        WorkspaceAddress::Canonical { root, path } => Ok(ResolvedAddress {
            root: filesystem_root(view, root)?,
            path: path.clone(),
            absolute_input: false,
        }),
        WorkspaceAddress::Absolute(path) => resolve_absolute(view, path),
        WorkspaceAddress::FileUri(uri) => {
            let path = uri.to_file_path().map_err(|()| {
                ResourceError::new(
                    ErrorCategory::InvalidReference,
                    "file URI does not identify a local filesystem path",
                )
            })?;
            resolve_absolute(view, &path)
        }
    }
}

fn filesystem_root(
    view: &WorkspaceView,
    id: &WorkspaceRootId,
) -> Result<Arc<FilesystemRoot>, ResourceError> {
    view.filesystems.get(id).cloned().ok_or_else(|| {
        ResourceError::new(
            ErrorCategory::InvalidReference,
            "Path Reference names an unknown or removed Workspace Root",
        )
    })
}

fn resolve_absolute(view: &WorkspaceView, input: &Path) -> Result<ResolvedAddress, ResourceError> {
    if !input.is_absolute() {
        return Err(ResourceError::new(
            ErrorCategory::InvalidReference,
            "absolute Resource spelling is not native on this platform",
        ));
    }
    let lexical_target = normalize_platform_path(input.to_owned());
    let target = match std::fs::canonicalize(&lexical_target) {
        Ok(target) => normalize_platform_path(target),
        Err(error) if error.kind() == io::ErrorKind::NotFound => lexical_target,
        Err(_) => {
            return Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "absolute Resource could not be safely resolved",
            ));
        }
    };
    let matches = matching_roots(view, &target)?;
    let match_count = matches.resources.len() + usize::from(matches.root_directory);
    if match_count > 1 {
        return Err(ResourceError::new(
            ErrorCategory::AmbiguousReference,
            "absolute Resource resolves beneath multiple Workspace Roots",
        ));
    }
    if matches.root_directory {
        return Err(ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            "Workspace Root directory is not a text file",
        ));
    }
    match matches.resources.as_slice() {
        [] => Err(ResourceError::new(
            ErrorCategory::PermissionDenied,
            "absolute Resource resolves outside every configured Workspace Root",
        )),
        [(root, path)] => Ok(ResolvedAddress {
            root: Arc::clone(root),
            path: path.clone(),
            absolute_input: true,
        }),
        _ => unreachable!("multiple root matches were rejected"),
    }
}

fn matching_roots(view: &WorkspaceView, target: &Path) -> Result<MatchingRoots, ResourceError> {
    let mut matches = Vec::new();
    let mut matched_root_directory = false;
    for root in view.filesystems.values() {
        if let Some(relative) = strip_beneath(target, &root.canonical_path) {
            if relative.as_os_str().is_empty() {
                matched_root_directory = true;
            } else {
                matches.push((Arc::clone(root), WorkspacePath::new(relative)?));
            }
        }
    }
    Ok(MatchingRoots {
        resources: matches,
        root_directory: matched_root_directory,
    })
}

fn contained_workspace_path(target: &Path, root: &Path) -> Result<WorkspacePath, ResourceError> {
    let relative = strip_beneath(target, root).ok_or_else(|| {
        ResourceError::new(
            ErrorCategory::PermissionDenied,
            "Resource handle resolves outside the selected Workspace Root",
        )
    })?;
    WorkspacePath::new(relative)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn final_directory_path(directory: &Dir) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;

    final_proc_path(directory.as_raw_fd())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn final_file_path(file: &File) -> io::Result<PathBuf> {
    use std::os::fd::AsRawFd;

    final_proc_path(file.as_raw_fd())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn final_proc_path(fd: std::os::fd::RawFd) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::fs::read_link(format!("/proc/self/fd/{fd}"))?;
    if path.as_os_str().as_bytes().ends_with(b" (deleted)") {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "open filesystem handle has been deleted",
        ));
    }
    Ok(path)
}

#[cfg(target_os = "macos")]
fn final_directory_path(directory: &Dir) -> io::Result<PathBuf> {
    final_macos_path(directory)
}

#[cfg(target_os = "macos")]
fn final_file_path(file: &File) -> io::Result<PathBuf> {
    final_macos_path(file)
}

#[cfg(target_os = "macos")]
fn final_macos_path(handle: &impl std::os::fd::AsFd) -> io::Result<PathBuf> {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    let path = rustix::fs::getpath(handle)?;
    Ok(PathBuf::from(OsStr::from_bytes(path.to_bytes())))
}

#[cfg(windows)]
fn final_directory_path(directory: &Dir) -> io::Result<PathBuf> {
    final_windows_path(directory)
}

#[cfg(windows)]
fn final_file_path(file: &File) -> io::Result<PathBuf> {
    final_windows_path(file)
}

#[cfg(windows)]
fn final_windows_path(handle: &impl std::os::windows::io::AsRawHandle) -> io::Result<PathBuf> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };

    let mut buffer = vec![0_u16; 32_768];
    loop {
        let length = unsafe {
            GetFinalPathNameByHandleW(
                handle.as_raw_handle(),
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).expect("Windows path buffer fits u32"),
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        let length = length as usize;
        if length < buffer.len() {
            buffer.truncate(length);
            return Ok(PathBuf::from(OsString::from_wide(&buffer)));
        }
        buffer.resize(length + 1, 0);
    }
}

fn normalize_handle_path(path: PathBuf) -> Result<PathBuf, ResourceError> {
    let normalized = normalize_platform_path(path);
    if !normalized.is_absolute() {
        return Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "filesystem handle did not resolve to an absolute path",
        ));
    }
    Ok(normalized)
}

fn authority_unavailable(message: &str) -> ResourceError {
    ResourceError::new(ErrorCategory::SourceUnavailable, message)
}

fn client_root_error(operation: &str, error: io::Error) -> ResourceError {
    authority_unavailable(&format!(
        "failed to {operation} client Workspace Root ({:?})",
        error.kind()
    ))
}

fn root_error(id: &WorkspaceRootId, operation: &str, error: io::Error) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!(
            "failed to {operation} Workspace Root '{id}' ({:?})",
            error.kind()
        ),
    )
}

fn resource_io_error(identity: &str, operation: &str, error: io::Error) -> ResourceError {
    let category = match error.kind() {
        io::ErrorKind::NotFound => ErrorCategory::NotFound,
        io::ErrorKind::PermissionDenied => ErrorCategory::PermissionDenied,
        _ => ErrorCategory::SourceUnavailable,
    };
    ResourceError::new(
        category,
        format!(
            "failed to {operation} Resource '{identity}' ({:?})",
            error.kind()
        ),
    )
}

fn limit_error(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::LimitExceeded, message)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn delivery_validation_rejects_removed_generation_only() {
        let temporary = TempDir::new().expect("temporary directory");
        let launch = temporary.path().join("launch");
        let removed = temporary.path().join("removed");
        let retained = temporary.path().join("retained");
        for root in [&launch, &removed, &retained] {
            fs::create_dir(root).expect("root fixture");
        }
        let source = FilesystemSource::new(
            LaunchRootSource::Cli(vec![LaunchRoot::read_only(
                WorkspaceRootId::new("launch").expect("launch root ID"),
                launch,
            )]),
            None,
            BackingPathVisibility::Hidden,
        )
        .await
        .expect("filesystem source");
        let root = |path: &Path| ClientRoot {
            uri: Url::from_directory_path(path)
                .expect("client root URI")
                .to_string(),
            name: None,
        };

        let refresh = source.begin_client_root_refresh().await;
        source
            .complete_client_root_refresh(
                source.start_client_root_acquisition(refresh),
                vec![root(&removed), root(&retained)],
            )
            .await
            .expect("initial client roots");
        let (generation, removed_root, retained_root) = {
            let authority = source.inner.authority.read().await;
            let AuthorityState::Active {
                generation, view, ..
            } = &*authority
            else {
                panic!("client roots should be active");
            };
            let find = |path: &Path| {
                view.filesystems
                    .values()
                    .find(|root| root.canonical_path == path)
                    .cloned()
                    .expect("active client root")
            };
            (
                *generation,
                find(&fs::canonicalize(&removed).expect("removed root path")),
                find(&fs::canonicalize(&retained).expect("retained root path")),
            )
        };

        let refresh = source.begin_client_root_refresh().await;
        source
            .complete_client_root_refresh(
                source.start_client_root_acquisition(refresh),
                vec![root(&retained)],
            )
            .await
            .expect("remove one client root");
        let authority = source.inner.authority.read().await;
        assert_eq!(
            validate_read_delivery(&authority, generation, &removed_root)
                .expect_err("removed-root result must fail")
                .category(),
            ErrorCategory::InvalidReference
        );
        validate_read_delivery(&authority, generation, &retained_root)
            .expect("retained-root result remains valid");
    }

    #[test]
    fn retained_traversal_state_has_an_exact_ceiling() {
        let mut budget = WorkspaceDiscoveryBudget::default();
        budget
            .charge_state(MAX_WORKSPACE_DISCOVERY_STATE_BYTES)
            .expect("exact traversal-state ceiling");
        let error = budget
            .charge_state(1)
            .expect_err("one byte over traversal-state ceiling");
        assert_eq!(error.category(), ErrorCategory::LimitExceeded);
        assert!(budget.is_exhausted());
    }
}
