use std::{
    collections::HashMap,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use cap_std::{
    ambient_authority,
    fs::{Dir, File},
};
use resourcefs_core::{
    ErrorCategory, MAX_TEXT_BYTES, MAX_TEXT_COLUMNS, MAX_TEXT_LINES, MAX_WORKSPACE_ROOTS,
    PathReference, ProjectionSelector, ResourceError, SourceAdapter, SourceResource,
    WorkspaceAddress, WorkspacePath, WorkspaceRoot, WorkspaceRootId, WorkspaceRootSet, select_utf8,
};
use sha2::{Digest, Sha256};
#[cfg(feature = "test-support")]
use tokio::sync::Notify;
use tokio::{
    sync::{Mutex, RwLock},
    time::Instant,
};
use url::Url;

const ROOT_CONSTRUCTION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct LaunchRoot {
    pub id: WorkspaceRootId,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum LaunchRootSource {
    Cli(Vec<LaunchRoot>),
    Profile(Vec<LaunchRoot>),
}

impl LaunchRootSource {
    fn into_roots(self) -> Vec<LaunchRoot> {
        match self {
            Self::Cli(roots) | Self::Profile(roots) => roots,
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
}

#[derive(Debug)]
struct WorkspaceView {
    roots: WorkspaceRootSet,
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
    primary_selector: Option<String>,
    visibility: BackingPathVisibility,
    authority: RwLock<AuthorityState>,
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
        let roots = root_source.into_roots();
        if roots.len() > MAX_WORKSPACE_ROOTS {
            return Err(limit_error("Workspace Root count exceeds 256"));
        }
        let selector = primary_selector.clone();
        let view = construct_launch_view(roots, selector).await?;
        let launch_view = Arc::new(view);
        let identity_history = launch_view
            .roots
            .roots()
            .iter()
            .map(|root| (root.id().clone(), root.canonical_uri().to_owned()))
            .collect();
        Ok(Self {
            inner: Arc::new(FilesystemSourceInner {
                launch_view: Arc::clone(&launch_view),
                primary_selector,
                visibility,
                authority: RwLock::new(AuthorityState::Active {
                    epoch: 0,
                    generation: 1,
                    view: launch_view,
                }),
                identity_history: Mutex::new(identity_history),
                #[cfg(feature = "test-support")]
                delivery_gate: RwLock::new(None),
            }),
        })
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

    async fn read_contained(
        &self,
        reference: &PathReference,
    ) -> Result<SourceResource, ResourceError> {
        let (generation, view) = {
            let authority = self.inner.authority.read().await;
            match &*authority {
                AuthorityState::Active {
                    generation, view, ..
                } => (*generation, Arc::clone(view)),
                AuthorityState::Refreshing { .. } => {
                    return Err(authority_unavailable(
                        "Workspace Root authority is refreshing",
                    ));
                }
                AuthorityState::Disabled { .. } => {
                    return Err(authority_unavailable(
                        "Workspace Root authority is disabled",
                    ));
                }
            }
        };
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
            if armed
                .as_ref()
                .is_some_and(|candidate| candidate.identity == identity)
            {
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
        AuthorityState::Active { .. } => Err(authority_unavailable(
            "Workspace Root authority changed while reading",
        )),
        AuthorityState::Refreshing { .. } | AuthorityState::Disabled { .. } => Err(
            authority_unavailable("Workspace Root authority changed while reading"),
        ),
    }
}

#[async_trait]
impl SourceAdapter for FilesystemSource {
    async fn read(&self, reference: &PathReference) -> Result<SourceResource, ResourceError> {
        self.read_contained(reference).await
    }
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
    timeout: Duration,
) -> Result<WorkspaceView, ResourceError> {
    if roots.len() > MAX_WORKSPACE_ROOTS {
        return Err(limit_error("Workspace Root count exceeds 256"));
    }
    construct_view(timeout, move || {
        build_client_workspace_view(roots, primary_selector.as_deref())
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
        }));
    }
    workspace_view(opened, primary_selector)
}

fn build_client_workspace_view(
    roots: Vec<ClientRoot>,
    primary_selector: Option<&str>,
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
    let root_set = WorkspaceRootSet::new(
        opened.iter().map(|root| root.metadata.clone()).collect(),
        primary_selector,
    )?;
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
    let canonical_reference =
        PathReference::canonical(resolved.root.metadata.id().clone(), relative_path);
    let mut resource = if let Some(projection) = projection {
        let selected = select_utf8(&mut file, Some(projection))?;
        let (content, version_tag, _) = selected.into_parts();
        SourceResource::text_projection(canonical_reference, content, version_tag)?
    } else {
        let initial_capacity = metadata.len().min(MAX_TEXT_BYTES as u64) as usize;
        let mut bytes = Vec::with_capacity(initial_capacity);
        (&mut file)
            .take(MAX_TEXT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| resource_io_error(identity, "read", error))?;
        if bytes.len() > MAX_TEXT_BYTES {
            return Err(resource_limit_error(identity, "bytes", MAX_TEXT_BYTES));
        }
        let content = validate_text_content(identity, bytes)?;
        SourceResource::text(canonical_reference, content)?
    };
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

#[cfg(not(windows))]
fn strip_beneath(target: &Path, root: &Path) -> Option<PathBuf> {
    target.strip_prefix(root).ok().map(Path::to_owned)
}

#[cfg(windows)]
fn strip_beneath(target: &Path, root: &Path) -> Option<PathBuf> {
    let target_components = target.components().collect::<Vec<_>>();
    let root_components = root.components().collect::<Vec<_>>();
    if target_components.len() < root_components.len()
        || !target_components
            .iter()
            .zip(&root_components)
            .all(|(left, right)| {
                left.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
            })
    {
        return None;
    }
    let mut relative = PathBuf::new();
    for component in &target_components[root_components.len()..] {
        relative.push(component.as_os_str());
    }
    Some(relative)
}

fn validate_text_content(identity: &str, bytes: Vec<u8>) -> Result<String, ResourceError> {
    let content = String::from_utf8(bytes).map_err(|_| {
        ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            format!("Resource '{identity}' is not valid UTF-8 text"),
        )
    })?;
    let mut lines = 0_usize;
    let mut maximum_columns = 0_usize;
    for line in content.lines() {
        lines += 1;
        maximum_columns = maximum_columns.max(line.chars().count());
    }
    if lines > MAX_TEXT_LINES {
        return Err(resource_limit_error(identity, "lines", MAX_TEXT_LINES));
    }
    if maximum_columns > MAX_TEXT_COLUMNS {
        return Err(resource_limit_error(identity, "columns", MAX_TEXT_COLUMNS));
    }
    Ok(content)
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

#[cfg(not(windows))]
fn normalize_platform_path(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
fn normalize_platform_path(path: PathBuf) -> PathBuf {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };

    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let verbatim_unc = "\\\\?\\UNC\\".encode_utf16().collect::<Vec<_>>();
    let verbatim = "\\\\?\\".encode_utf16().collect::<Vec<_>>();
    let normalized = if wide.starts_with(&verbatim_unc) {
        let mut value = "\\\\".encode_utf16().collect::<Vec<_>>();
        value.extend_from_slice(&wide[verbatim_unc.len()..]);
        value
    } else if wide.starts_with(&verbatim) {
        wide[verbatim.len()..].to_vec()
    } else {
        wide
    };
    PathBuf::from(OsString::from_wide(&normalized))
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

fn limit_error(message: &str) -> ResourceError {
    ResourceError::new(ErrorCategory::LimitExceeded, message)
}

fn resource_limit_error(identity: &str, dimension: &str, limit: usize) -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        format!("Resource '{identity}' exceeds the hard {dimension} limit of {limit}"),
    )
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
            LaunchRootSource::Cli(vec![LaunchRoot {
                id: WorkspaceRootId::new("launch").expect("launch root ID"),
                path: launch,
            }]),
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
}
