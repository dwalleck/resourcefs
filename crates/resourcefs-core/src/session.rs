use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::{ArtifactAddress, ErrorCategory, MAX_ARTIFACT_BYTES, ResourceError};

pub const MAX_SESSION_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_SESSION_ARTIFACTS: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn generate() -> Result<Self, ResourceError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|error| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("could not generate a Path Session token: {error}"),
            )
        })?;
        let mut token = String::with_capacity(32);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in random {
            token.push(HEX[(byte >> 4) as usize] as char);
            token.push(HEX[(byte & 0x0f) as usize] as char);
        }
        Ok(Self(token))
    }

    pub fn parse(token: impl Into<String>) -> Result<Self, ResourceError> {
        let token = token.into();
        ArtifactAddress::new(token.clone(), 1)?;
        Ok(Self(token))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArtifactId(u64);

impl ArtifactId {
    pub fn new(value: u64) -> Result<Self, ResourceError> {
        if value == 0 {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "artifact object ID must be positive",
            ));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[async_trait]
pub trait SessionStorage: Send + Sync {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError>;

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError>;

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError>;

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError>;

    async fn mark_disconnected(&self) -> Result<(), ResourceError>;
}

#[derive(Debug, Clone)]
pub struct OperationGuard {
    active: Arc<AtomicBool>,
}

impl OperationGuard {
    pub fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn cancel(&self) {
        self.active.store(false, Ordering::Release);
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
}

impl Default for OperationGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct PathSession {
    inner: Arc<PathSessionInner>,
}

struct PathSessionInner {
    token: SessionToken,
    active: AtomicBool,
    admission: Mutex<SessionState>,
    storage: Arc<dyn SessionStorage>,
}

#[derive(Debug)]
struct SessionState {
    next_object_id: u64,
    used_bytes: usize,
    records: HashSet<ArtifactId>,
    digest_index: HashMap<[u8; 32], Vec<ArtifactId>>,
}

impl PathSession {
    pub fn new(token: SessionToken, storage: Arc<dyn SessionStorage>) -> Self {
        Self {
            inner: Arc::new(PathSessionInner {
                token,
                active: AtomicBool::new(true),
                admission: Mutex::new(SessionState {
                    next_object_id: 1,
                    used_bytes: 0,
                    records: HashSet::new(),
                    digest_index: HashMap::new(),
                }),
                storage,
            }),
        }
    }

    pub fn token(&self) -> &SessionToken {
        &self.inner.token
    }

    pub fn is_active(&self) -> bool {
        self.inner.active.load(Ordering::Acquire)
    }

    pub fn invalidate(&self) {
        self.inner.active.store(false, Ordering::Release);
    }

    pub async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        self.invalidate();
        self.inner.storage.mark_disconnected().await
    }

    pub async fn retain(
        &self,
        content: &str,
        operation: &OperationGuard,
    ) -> Result<ArtifactAddress, ResourceError> {
        let digest: [u8; 32] = Sha256::digest(content.as_bytes()).into();
        self.retain_with_digest(content, digest, operation).await
    }

    #[cfg(feature = "test-support")]
    pub async fn retain_with_digest_for_test(
        &self,
        content: &str,
        digest: [u8; 32],
        operation: &OperationGuard,
    ) -> Result<ArtifactAddress, ResourceError> {
        self.retain_with_digest(content, digest, operation).await
    }

    async fn retain_with_digest(
        &self,
        content: &str,
        digest: [u8; 32],
        operation: &OperationGuard,
    ) -> Result<ArtifactAddress, ResourceError> {
        let content_bytes = content.as_bytes();
        if content_bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "selected projection exceeds the {MAX_ARTIFACT_BYTES}-byte artifact ceiling; narrow the selector"
                ),
            ));
        }
        ensure_commit_live(self, operation)?;

        let mut state = self.inner.admission.lock().await;
        ensure_commit_live(self, operation)?;
        if let Some(candidates) = state.digest_index.get(&digest).cloned() {
            for id in candidates {
                if self.inner.storage.content_equals(id, content_bytes).await? {
                    return self.address(id);
                }
            }
        }

        if state.records.len() >= MAX_SESSION_ARTIFACTS {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "Path Session exceeds the {MAX_SESSION_ARTIFACTS}-artifact ceiling; reuse an existing Recovery Reference"
                ),
            ));
        }
        let resulting_bytes = state
            .used_bytes
            .checked_add(content_bytes.len())
            .ok_or_else(session_quota_error)?;
        if resulting_bytes > MAX_SESSION_BYTES {
            return Err(session_quota_error());
        }

        let id = ArtifactId::new(state.next_object_id)?;
        state.next_object_id = state.next_object_id.checked_add(1).ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::LimitExceeded,
                "artifact object ID space exhausted",
            )
        })?;
        self.inner.storage.write_atomic(id, content_bytes).await?;
        if let Err(error) = ensure_commit_live(self, operation) {
            self.inner.storage.remove(id).await?;
            return Err(error);
        }

        state.records.insert(id);
        state.digest_index.entry(digest).or_default().push(id);
        state.used_bytes = resulting_bytes;
        self.address(id)
    }

    pub async fn read_artifact(&self, address: &ArtifactAddress) -> Result<String, ResourceError> {
        if !self.is_active() || address.session_token() != self.inner.token.as_str() {
            return Err(artifact_not_found());
        }
        let id = ArtifactId::new(address.object_id())?;
        let known = self.inner.admission.lock().await.records.contains(&id);
        if !known {
            return Err(artifact_not_found());
        }
        let content = self.inner.storage.read(id).await;
        if !self.is_active() {
            return Err(artifact_not_found());
        }
        content
    }

    pub async fn used_bytes(&self) -> usize {
        self.inner.admission.lock().await.used_bytes
    }

    pub async fn artifact_count(&self) -> usize {
        self.inner.admission.lock().await.records.len()
    }

    pub async fn artifact_catalog(&self) -> Result<Vec<ArtifactAddress>, ResourceError> {
        if !self.is_active() {
            return Err(inactive_catalog_error());
        }

        let state = self.inner.admission.lock().await;
        if !self.is_active() {
            return Err(inactive_catalog_error());
        }
        let mut ids = state.records.iter().copied().collect::<Vec<_>>();
        ids.sort_unstable_by_key(|id| id.get());
        let catalog = ids
            .into_iter()
            .map(|id| self.address(id))
            .collect::<Result<Vec<_>, _>>()?;
        if !self.is_active() {
            return Err(inactive_catalog_error());
        }
        Ok(catalog)
    }

    fn address(&self, id: ArtifactId) -> Result<ArtifactAddress, ResourceError> {
        ArtifactAddress::new(self.inner.token.as_str(), id.get())
    }
}

fn ensure_commit_live(
    session: &PathSession,
    operation: &OperationGuard,
) -> Result<(), ResourceError> {
    if !session.is_active() {
        return Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "Path Session disconnected before artifact publication",
        ));
    }
    if !operation.is_active() {
        return Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "Resource operation was cancelled before artifact publication",
        ));
    }
    Ok(())
}

fn inactive_catalog_error() -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        "Path Session disconnected before Artifact catalog enumeration",
    )
}

fn session_quota_error() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        format!(
            "Path Session exceeds the {MAX_SESSION_BYTES}-byte artifact quota; narrow the selector"
        ),
    )
}

fn artifact_not_found() -> ResourceError {
    ResourceError::new(
        ErrorCategory::NotFound,
        "artifact is not available in this Path Session",
    )
}
