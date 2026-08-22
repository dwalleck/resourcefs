use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use directories::BaseDirs;
use resourcefs_core::{
    ArtifactId, ErrorCategory, PathSession, ResourceError, ServerLimits, SessionStorage,
    SessionToken,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use tokio::sync::{Mutex, RwLock, RwLockReadGuard};

const LEASE_FILE: &str = "session.lock";
const LIVE_MARKER: &str = "last-live";
const DISCONNECTED_MARKER: &str = "disconnected";
const OBJECTS_DIRECTORY: &str = "objects";
const CACHE_NAMESPACE: &str = "resourcefs";
pub const SESSION_CLEANUP_TTL: Duration = Duration::from_secs(86_400);

/// Validated retained-session cache authority and elapsed cleanup window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStorageConfig {
    cache_root: PathBuf,
    retention_ttl: Duration,
}

impl SessionStorageConfig {
    pub fn new(
        cache_root: impl Into<PathBuf>,
        retention_ttl_seconds: i64,
    ) -> Result<Self, ResourceError> {
        let cache_root = cache_root.into();
        if cache_root.as_os_str().is_empty() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "session.cacheDirectory must not be empty",
            ));
        }
        let retention_ttl_seconds =
            u64::try_from(retention_ttl_seconds).map_err(|_| retention_ttl_error())?;
        if retention_ttl_seconds > SESSION_CLEANUP_TTL.as_secs() {
            return Err(retention_ttl_error());
        }
        Ok(Self {
            cache_root,
            retention_ttl: Duration::from_secs(retention_ttl_seconds),
        })
    }

    pub fn for_current_user(retention_ttl_seconds: i64) -> Result<Self, ResourceError> {
        let base = BaseDirs::new().ok_or_else(|| storage_failure("locate account cache", None))?;
        Self::new(base.cache_dir(), retention_ttl_seconds)
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub const fn retention_ttl(&self) -> Duration {
        self.retention_ttl
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupReport {
    pub removed: usize,
    pub live: usize,
    pub fresh: usize,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    sessions_root: Arc<PathBuf>,
    retention_ttl: Duration,
}

impl SessionStore {
    pub async fn open_with(config: SessionStorageConfig) -> Result<Self, ResourceError> {
        let retention_ttl = config.retention_ttl;
        let cache_root = config.cache_root;
        let sessions_root = tokio::task::spawn_blocking(move || prepare_sessions_root(&cache_root))
            .await
            .map_err(join_error)??;
        let store = Self {
            sessions_root: Arc::new(sessions_root),
            retention_ttl,
        };
        store.cleanup_expired().await?;
        Ok(store)
    }

    pub async fn create_session(
        &self,
        limits: ServerLimits,
    ) -> Result<StoredSession, ResourceError> {
        let token = SessionToken::generate()?;
        let storage = Arc::new(
            DiskSessionStorage::create(self.sessions_root.as_ref().clone(), token.clone()).await?,
        );
        let trait_storage: Arc<dyn SessionStorage> = storage.clone();
        let session = PathSession::new(token, trait_storage, limits);
        Ok(StoredSession {
            session,
            storage,
            retention_ttl: self.retention_ttl,
        })
    }

    pub async fn cleanup_expired(&self) -> Result<CleanupReport, ResourceError> {
        self.cleanup_expired_at(SystemTime::now()).await
    }

    async fn cleanup_expired_at(&self, now: SystemTime) -> Result<CleanupReport, ResourceError> {
        let root = self.sessions_root.as_ref().clone();
        let retention_ttl = self.retention_ttl;
        tokio::task::spawn_blocking(move || cleanup_sessions(&root, now, retention_ttl))
            .await
            .map_err(join_error)?
    }

    #[cfg(feature = "test-support")]
    pub async fn cleanup_expired_at_for_test(
        &self,
        now: SystemTime,
    ) -> Result<CleanupReport, ResourceError> {
        self.cleanup_expired_at(now).await
    }

    #[cfg(feature = "test-support")]
    pub fn sessions_root_for_test(&self) -> &Path {
        self.sessions_root.as_path()
    }
}

#[derive(Clone)]
pub struct StoredSession {
    session: PathSession,
    storage: Arc<DiskSessionStorage>,
    retention_ttl: Duration,
}

impl StoredSession {
    pub fn path_session(&self) -> &PathSession {
        &self.session
    }

    pub async fn heartbeat(&self) -> Result<(), ResourceError> {
        self.storage.heartbeat().await
    }

    pub async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        self.session.mark_disconnected().await?;
        self.storage
            .finish_disconnect(self.retention_ttl.is_zero())
            .await
    }

    #[cfg(feature = "test-support")]
    pub fn storage_for_test(&self) -> &DiskSessionStorage {
        &self.storage
    }
}

pub struct DiskSessionStorage {
    session_dir: PathBuf,
    objects_dir: PathBuf,
    lease: Mutex<Option<SessionLease>>,
    activity: RwLock<()>,
    closed: AtomicBool,
    #[cfg(feature = "test-support")]
    failure: Mutex<Option<StorageFailurePoint>>,
}

impl DiskSessionStorage {
    async fn create(sessions_root: PathBuf, token: SessionToken) -> Result<Self, ResourceError> {
        tokio::task::spawn_blocking(move || {
            let session_dir = sessions_root.join(token.as_str());
            let cleanup_dir = session_dir.clone();
            fs::create_dir(&session_dir)
                .map_err(|error| storage_io_error("create session directory", error))?;
            let result = (|| {
                let objects_dir = session_dir.join(OBJECTS_DIRECTORY);
                fs::create_dir(&objects_dir)
                    .map_err(|error| storage_io_error("create object directory", error))?;
                let lease = SessionLease::acquire_new(&session_dir.join(LEASE_FILE))?;
                write_marker_sync(&session_dir.join(LIVE_MARKER), b"live\n")?;
                Ok(Self {
                    session_dir,
                    objects_dir,
                    lease: Mutex::new(Some(lease)),
                    activity: RwLock::new(()),
                    closed: AtomicBool::new(false),
                    #[cfg(feature = "test-support")]
                    failure: Mutex::new(None),
                })
            })();
            match result {
                Ok(storage) => Ok(storage),
                Err(error) => match fs::remove_dir_all(cleanup_dir) {
                    Ok(()) => Err(error),
                    Err(cleanup) if cleanup.kind() == io::ErrorKind::NotFound => Err(error),
                    Err(cleanup) => Err(storage_io_error(
                        "clean failed session construction",
                        cleanup,
                    )),
                },
            }
        })
        .await
        .map_err(join_error)?
    }

    async fn begin_io(&self) -> Result<RwLockReadGuard<'_, ()>, ResourceError> {
        let activity = self.activity.read().await;
        if self.closed.load(Ordering::Acquire) {
            return Err(storage_failure("access disconnected session storage", None));
        }
        Ok(activity)
    }

    pub async fn heartbeat(&self) -> Result<(), ResourceError> {
        let _activity = self.begin_io().await?;
        let marker = self.session_dir.join(LIVE_MARKER);
        tokio::task::spawn_blocking(move || write_marker_sync(&marker, b"live\n"))
            .await
            .map_err(join_error)?
    }

    async fn finish_disconnect(&self, remove: bool) -> Result<(), ResourceError> {
        let _activity = self.activity.write().await;
        self.closed.store(true, Ordering::Release);
        let lease = self.lease.lock().await.take();
        drop(lease);
        if !remove {
            return Ok(());
        }
        match tokio::fs::remove_dir_all(&self.session_dir).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(storage_io_error("remove disconnected session", error)),
        }
    }

    #[cfg(feature = "test-support")]
    pub async fn fail_next(&self, point: StorageFailurePoint) {
        *self.failure.lock().await = Some(point);
    }

    #[cfg(feature = "test-support")]
    pub fn session_dir_for_test(&self) -> &Path {
        &self.session_dir
    }

    #[cfg(feature = "test-support")]
    pub fn set_age_marker_for_test(
        &self,
        disconnected: bool,
        modified: SystemTime,
    ) -> Result<(), ResourceError> {
        let marker = self.session_dir.join(if disconnected {
            DISCONNECTED_MARKER
        } else {
            LIVE_MARKER
        });
        if disconnected && !marker.exists() {
            write_marker_sync(&marker, b"disconnected\n")?;
        }
        let file = OpenOptions::new()
            .write(true)
            .open(marker)
            .map_err(|error| storage_io_error("open age marker", error))?;
        file.set_times(fs::FileTimes::new().set_modified(modified))
            .map_err(|error| storage_io_error("set age marker", error))
    }

    #[cfg(feature = "test-support")]
    async fn inject(&self, point: StorageFailurePoint) -> Result<(), ResourceError> {
        let mut failure = self.failure.lock().await;
        if failure.as_ref() == Some(&point) {
            *failure = None;
            return Err(storage_failure(point.operation(), None));
        }
        Ok(())
    }

    #[cfg(not(feature = "test-support"))]
    async fn inject(&self, _point: StorageFailurePoint) -> Result<(), ResourceError> {
        Ok(())
    }
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFailurePoint {
    Open,
    Write,
    Sync,
    Persist,
    Remove,
    Disconnect,
}

#[cfg(not(feature = "test-support"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageFailurePoint {
    Open,
    Write,
    Sync,
    Persist,
    Remove,
    Disconnect,
}

#[cfg(feature = "test-support")]
impl StorageFailurePoint {
    const fn operation(self) -> &'static str {
        match self {
            Self::Open => "open object",
            Self::Write => "write object",
            Self::Sync => "sync object",
            Self::Persist => "persist object",
            Self::Remove => "remove object",
            Self::Disconnect => "mark session disconnected",
        }
    }
}

#[async_trait]
impl SessionStorage for DiskSessionStorage {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError> {
        let _activity = self.begin_io().await?;
        let mut file = tokio::fs::File::open(object_path(&self.objects_dir, id))
            .await
            .map_err(|error| storage_io_error("open object for comparison", error))?;
        let mut offset = 0_usize;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|error| storage_io_error("compare object", error))?;
            if read == 0 {
                return Ok(offset == content.len());
            }
            let Some(expected) = content.get(offset..offset.saturating_add(read)) else {
                return Ok(false);
            };
            if buffer[..read] != *expected {
                return Ok(false);
            }
            offset += read;
        }
    }

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError> {
        let _activity = self.begin_io().await?;
        self.inject(StorageFailurePoint::Open).await?;
        let temporary = temporary_object_path(&self.objects_dir, id);
        let final_path = object_path(&self.objects_dir, id);
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .await
            .map_err(|error| storage_io_error("open temporary object", error))?;

        if let Err(error) = self.inject(StorageFailurePoint::Write).await {
            drop(file);
            remove_temporary(&temporary).await?;
            return Err(error);
        }
        if let Err(error) = file.write_all(content).await {
            drop(file);
            remove_temporary(&temporary).await?;
            return Err(storage_io_error("write temporary object", error));
        }
        if let Err(error) = self.inject(StorageFailurePoint::Sync).await {
            drop(file);
            remove_temporary(&temporary).await?;
            return Err(error);
        }
        if let Err(error) = file.sync_all().await {
            drop(file);
            remove_temporary(&temporary).await?;
            return Err(storage_io_error("sync temporary object", error));
        }
        drop(file);
        if let Err(error) = self.inject(StorageFailurePoint::Persist).await {
            remove_temporary(&temporary).await?;
            return Err(error);
        }
        if let Err(error) = tokio::fs::rename(&temporary, &final_path).await {
            remove_temporary(&temporary).await?;
            return Err(storage_io_error("persist object", error));
        }
        if let Err(error) = sync_directory(self.objects_dir.clone()).await {
            let remove_result = tokio::fs::remove_file(&final_path).await;
            if let Err(remove_error) = remove_result {
                return Err(storage_io_error(
                    "remove object after directory sync failure",
                    remove_error,
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError> {
        let _activity = self.begin_io().await?;
        tokio::fs::read_to_string(object_path(&self.objects_dir, id))
            .await
            .map_err(|error| storage_io_error("read object", error))
    }

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError> {
        let _activity = self.begin_io().await?;
        self.inject(StorageFailurePoint::Remove).await?;
        tokio::fs::remove_file(object_path(&self.objects_dir, id))
            .await
            .map_err(|error| storage_io_error("remove object", error))?;
        sync_directory(self.objects_dir.clone()).await
    }
    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        let _activity = self.begin_io().await?;
        self.inject(StorageFailurePoint::Disconnect).await?;
        let marker = self.session_dir.join(DISCONNECTED_MARKER);
        tokio::task::spawn_blocking(move || write_marker_sync(&marker, b"disconnected\n"))
            .await
            .map_err(join_error)?
    }
}

async fn remove_temporary(path: &Path) -> Result<(), ResourceError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(storage_io_error("remove temporary object", error)),
    }
}

async fn sync_directory(path: PathBuf) -> Result<(), ResourceError> {
    #[cfg(unix)]
    {
        tokio::task::spawn_blocking(move || {
            File::open(path)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| storage_io_error("sync object directory", error))
        })
        .await
        .map_err(join_error)?
    }
    #[cfg(not(unix))]
    {
        let _path = path;
        Ok(())
    }
}

fn object_path(objects_dir: &Path, id: ArtifactId) -> PathBuf {
    objects_dir.join(format!("{}.txt", id.get()))
}

fn temporary_object_path(objects_dir: &Path, id: ArtifactId) -> PathBuf {
    objects_dir.join(format!(".{}.tmp", id.get()))
}

fn prepare_sessions_root(cache_root: &Path) -> Result<PathBuf, ResourceError> {
    fs::create_dir_all(cache_root).map_err(|error| storage_io_error("create cache root", error))?;
    let canonical_root = fs::canonicalize(cache_root)
        .map_err(|error| storage_io_error("canonicalize cache root", error))?;
    let sessions_root = canonical_root.join(CACHE_NAMESPACE);
    match fs::symlink_metadata(&sessions_root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(storage_failure(
                "session cache namespace is not a directory",
                None,
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&sessions_root)
                .map_err(|error| storage_io_error("create session cache namespace", error))?;
        }
        Err(error) => {
            return Err(storage_io_error("inspect session cache namespace", error));
        }
    }
    let sessions_root = fs::canonicalize(&sessions_root)
        .map_err(|error| storage_io_error("canonicalize session cache namespace", error))?;
    if sessions_root.parent() != Some(canonical_root.as_path()) {
        return Err(storage_failure(
            "session cache namespace escaped its configured cache root",
            None,
        ));
    }
    Ok(sessions_root)
}

fn cleanup_sessions(
    root: &Path,
    now: SystemTime,
    retention_ttl: Duration,
) -> Result<CleanupReport, ResourceError> {
    let mut report = CleanupReport {
        removed: 0,
        live: 0,
        fresh: 0,
    };
    let entries = fs::read_dir(root)
        .map_err(|error| storage_io_error("enumerate retained sessions", error))?;
    for entry in entries {
        let entry = entry.map_err(|error| storage_io_error("enumerate retained session", error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| storage_io_error("inspect retained session", error))?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if SessionToken::parse(name).is_err() {
            continue;
        }
        let session_dir = entry.path();
        let lease_path = session_dir.join(LEASE_FILE);
        if !regular_file_without_symlink(&lease_path)? {
            continue;
        }
        let Some(lease) = SessionLease::try_acquire(&lease_path)? else {
            report.live += 1;
            continue;
        };
        let marker = if regular_file_without_symlink(&session_dir.join(DISCONNECTED_MARKER))? {
            session_dir.join(DISCONNECTED_MARKER)
        } else if regular_file_without_symlink(&session_dir.join(LIVE_MARKER))? {
            session_dir.join(LIVE_MARKER)
        } else {
            report.fresh += 1;
            continue;
        };
        let modified = fs::metadata(marker)
            .and_then(|metadata| metadata.modified())
            .map_err(|error| storage_io_error("inspect retained session age", error))?;
        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        if age < retention_ttl {
            report.fresh += 1;
            continue;
        }
        drop(lease);
        match fs::remove_dir_all(&session_dir) {
            Ok(()) => report.removed += 1,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_io_error("remove expired session", error)),
        }
    }
    Ok(report)
}

fn regular_file_without_symlink(path: &Path) -> Result<bool, ResourceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_file() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(storage_io_error("inspect retained session marker", error)),
    }
}

fn write_marker_sync(path: &Path, content: &[u8]) -> Result<(), ResourceError> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|error| storage_io_error("open session marker", error))?;
    io::Write::write_all(&mut file, content)
        .map_err(|error| storage_io_error("write session marker", error))?;
    file.sync_all()
        .map_err(|error| storage_io_error("sync session marker", error))
}

struct SessionLease {
    _file: File,
}

impl SessionLease {
    fn acquire_new(path: &Path) -> Result<Self, ResourceError> {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| storage_io_error("create session lease", error))?;
        if !try_lock_exclusive(&file)
            .map_err(|error| storage_io_error("acquire session lease", error))?
        {
            return Err(storage_failure("acquire new session lease", None));
        }
        Ok(Self { _file: file })
    }

    fn try_acquire(path: &Path) -> Result<Option<Self>, ResourceError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| storage_io_error("open retained session lease", error))?;
        if try_lock_exclusive(&file)
            .map_err(|error| storage_io_error("inspect retained session lease", error))?
        {
            Ok(Some(Self { _file: file }))
        } else {
            Ok(None)
        }
    }
}

#[cfg(unix)]
fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    match rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(true),
        Err(error) if error == rustix::io::Errno::WOULDBLOCK => Ok(false),
        Err(error) => Err(io::Error::from_raw_os_error(error.raw_os_error())),
    }
}

#[cfg(windows)]
fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    use std::{mem::zeroed, os::windows::io::AsRawHandle};
    use windows_sys::Win32::{
        Foundation::ERROR_LOCK_VIOLATION,
        Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx},
        System::IO::OVERLAPPED,
    };

    // SAFETY: An all-zero OVERLAPPED selects offset zero; LockFileEx returns before this local is
    // dropped because FAIL_IMMEDIATELY is set and the file handle remains owned by SessionLease.
    let mut overlapped: OVERLAPPED = unsafe { zeroed() };
    let locked = unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if locked != 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        Ok(false)
    } else {
        Err(error)
    }
}

fn join_error(error: tokio::task::JoinError) -> ResourceError {
    storage_failure(&format!("session storage task failed: {error}"), None)
}

fn storage_io_error(operation: &str, error: io::Error) -> ResourceError {
    storage_failure(operation, Some(error.kind()))
}

fn retention_ttl_error() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        format!(
            "session.retentionTtlSeconds must be between 0 and {}",
            SESSION_CLEANUP_TTL.as_secs()
        ),
    )
}

fn storage_failure(operation: &str, kind: Option<io::ErrorKind>) -> ResourceError {
    let suffix = kind.map_or(String::new(), |kind| format!(" ({kind:?})"));
    ResourceError::new(
        resourcefs_core::ErrorCategory::SourceUnavailable,
        format!("{operation} failed{suffix}"),
    )
}
