use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};

use crate::{
    ArtifactAddress, DisplayedLineRange, ErrorCategory, LocalAddress, LocalName,
    MAX_PATH_REFERENCE_BYTES, PathReference, ResourceAddress, ResourceError, ServerLimits,
    VersionSelector, VersionTag, WorkspaceAddress, mutation::OperationId,
};

/// Bytes retained by artifacts, scratch, cache, snapshots, and exact mutation
/// operation content in one live Path Session.
pub const MAX_SESSION_BYTES: usize = 256 * 1024 * 1024;

/// Artifact, Session Scratch, and cache objects share this count ceiling.
/// Mutation journal entries have the independent [`MAX_MUTATION_OPERATIONS`]
/// ceiling because their exact content is already charged to the byte ceiling.
pub const MAX_SESSION_ARTIFACTS: usize = 1_000;
/// Distinct operation identities retained by one live Path Session.
pub const MAX_MUTATION_OPERATIONS: usize = 10_000;

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

/// Validated source namespace plus opaque representation key for one session cache entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionCacheKey {
    namespace: String,
    key: String,
}

impl SessionCacheKey {
    pub fn new(
        namespace: impl Into<String>,
        key: impl Into<String>,
    ) -> Result<Self, ResourceError> {
        let namespace = namespace.into();
        let key = key.into();
        validate_cache_namespace(&namespace)?;
        if key.is_empty() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "session cache key must not be empty",
            ));
        }
        let retained_bytes = namespace
            .len()
            .checked_add(key.len())
            .ok_or_else(|| session_quota_error(MAX_SESSION_BYTES))?;
        if retained_bytes > MAX_PATH_REFERENCE_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!("session cache key exceeds the {MAX_PATH_REFERENCE_BYTES}-byte ceiling"),
            ));
        }
        Ok(Self { namespace, key })
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub const fn retained_bytes(&self) -> usize {
        self.namespace.len() + self.key.len()
    }
}

fn validate_cache_namespace(namespace: &str) -> Result<(), ResourceError> {
    if namespace.is_empty()
        || !namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ResourceError::new(
            ErrorCategory::InvalidReference,
            "session cache namespace must be non-empty canonical ASCII",
        ));
    }
    Ok(())
}

/// Opaque metadata and content bytes retained without copying on cache hits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCacheEntry {
    metadata: Arc<[u8]>,
    content: Arc<[u8]>,
}

impl SessionCacheEntry {
    pub fn new(metadata: Vec<u8>, content: Vec<u8>) -> Result<Self, ResourceError> {
        metadata
            .len()
            .checked_add(content.len())
            .ok_or_else(|| session_quota_error(MAX_SESSION_BYTES))?;
        Ok(Self {
            metadata: Arc::from(metadata),
            content: Arc::from(content),
        })
    }

    pub fn metadata(&self) -> &[u8] {
        &self.metadata
    }

    pub fn content(&self) -> &[u8] {
        &self.content
    }

    pub const fn metadata_arc(&self) -> &Arc<[u8]> {
        &self.metadata
    }

    pub const fn content_arc(&self) -> &Arc<[u8]> {
        &self.content
    }

    pub fn retained_bytes(&self) -> usize {
        self.metadata.len() + self.content.len()
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

const OPERATION_ACTIVE: u8 = 0;
const OPERATION_CANCELLED: u8 = 1;
const OPERATION_COMMITTING: u8 = 2;
const OPERATION_COMPLETE: u8 = 3;

#[derive(Debug, Clone)]
pub struct OperationGuard {
    state: Arc<AtomicU8>,
    cancelled: Arc<Notify>,
}

impl OperationGuard {
    pub fn new() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(OPERATION_ACTIVE)),
            cancelled: Arc::new(Notify::new()),
        }
    }

    pub fn cancel(&self) -> bool {
        if self
            .state
            .compare_exchange(
                OPERATION_ACTIVE,
                OPERATION_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.cancelled.notify_waiters();
            true
        } else {
            false
        }
    }

    pub fn begin_commit(&self) -> Result<(), ResourceError> {
        self.state
            .compare_exchange(
                OPERATION_ACTIVE,
                OPERATION_COMMITTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|state| {
                let message = if state == OPERATION_CANCELLED {
                    "Resource operation was cancelled before commit"
                } else {
                    "Resource operation commit transition is no longer available"
                };
                ResourceError::new(ErrorCategory::Cancelled, message)
            })
    }

    pub fn finish_commit(&self) -> bool {
        self.state
            .compare_exchange(
                OPERATION_COMMITTING,
                OPERATION_COMPLETE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.state.load(Ordering::Acquire),
            OPERATION_ACTIVE | OPERATION_COMMITTING
        )
    }

    pub fn is_committing(&self) -> bool {
        self.state.load(Ordering::Acquire) == OPERATION_COMMITTING
    }

    /// Waits until this operation is cancelled without polling.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.cancelled.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.state.load(Ordering::Acquire) == OPERATION_CANCELLED {
                return;
            }
            notified.await;
        }
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
    limits: ServerLimits,
}

#[derive(Debug)]
struct SessionState {
    next_object_id: u64,
    used_bytes: usize,
    records: HashSet<ArtifactId>,
    digest_index: HashMap<[u8; 32], Vec<ArtifactId>>,
    scratch: HashMap<LocalName, ScratchEntry>,
    cache: HashMap<SessionCacheKey, CachedObject>,
    cache_generations: HashMap<String, u64>,
    /// Monotonic recency stamp handed to each cache access, so eviction can
    /// pick the least recently used entry without a linked list.
    cache_clock: u64,
    snapshots: HashMap<SnapshotKey, SeenSnapshot>,
    snapshot_bytes: usize,
    reserved_snapshot_bytes: usize,
    mutation_operations: HashMap<OperationId, MutationJournalEntry>,
}

#[derive(Debug)]
struct MutationJournalEntry {
    target: String,
    content: Arc<str>,
    fingerprint: [u8; 32],
    attempt: Arc<MutationAttempt>,
}

#[derive(Debug)]
struct MutationAttempt {
    outcome: OnceLock<MutationOperationOutcome>,
    notify: Notify,
}

impl MutationAttempt {
    fn new() -> Self {
        Self {
            outcome: OnceLock::new(),
            notify: Notify::new(),
        }
    }

    fn complete(&self, outcome: MutationOperationOutcome) {
        let inserted = self.outcome.set(outcome).is_ok();
        debug_assert!(inserted, "mutation operation attempt completes once");
        if inserted {
            self.notify.notify_waiters();
        }
    }

    async fn wait(&self) -> MutationOperationOutcome {
        loop {
            if let Some(outcome) = self.outcome.get() {
                return outcome.clone();
            }
            let notified = self.notify.notified();
            if let Some(outcome) = self.outcome.get() {
                return outcome.clone();
            }
            notified.await;
        }
    }
}

/// Terminal observation for one Creation Target attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationOperationOutcome {
    /// Upstream creation succeeded and returned this canonical Resource.
    Succeeded(Arc<PathReference>),
    /// Upstream conclusively rejected without creating; identical retry may own.
    Conclusive(ResourceError),
    /// Upstream may have created; all repeats are blocked for this session.
    Unknown(ResourceError),
}

/// Result of binding an operation ID in one Path Session.
#[derive(Debug)]
pub enum MutationOperationStart {
    /// This caller alone may execute and complete the attempt.
    Owner(MutationOperationLease),
    /// An identical attempt is in flight; wait for its exact outcome.
    Wait(MutationOperationWaiter),
    /// Creation already succeeded; return the recorded canonical Resource.
    Replay(Arc<PathReference>),
    /// The last attempt may have created; return reconciliation guidance.
    Unknown(ResourceError),
}

/// Exclusive right to complete one journal attempt.
#[derive(Debug)]
pub struct MutationOperationLease {
    attempt: Arc<MutationAttempt>,
    completed: bool,
}

impl MutationOperationLease {
    /// Publishes the attempt outcome to waiters and later repeats.
    pub fn finish(mut self, outcome: MutationOperationOutcome) {
        self.attempt.complete(outcome);
        self.completed = true;
    }
}

impl Drop for MutationOperationLease {
    fn drop(&mut self) {
        if !self.completed {
            self.attempt
                .complete(MutationOperationOutcome::Unknown(ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "Creation Target outcome is unknown because its owner ended before completion; reconcile upstream state",
                )));
        }
    }
}

/// Wait handle for an identical in-flight operation.
#[derive(Debug)]
pub struct MutationOperationWaiter {
    attempt: Arc<MutationAttempt>,
}

impl MutationOperationWaiter {
    /// Waits without polling and returns the owner's published outcome.
    pub async fn wait(self) -> MutationOperationOutcome {
        self.attempt.wait().await
    }
}

/// One cached upstream representation plus when it was last used.
///
/// Cache entries are the only reconstructible object class in a Path Session:
/// an artifact is a Recovery Reference a caller may hold, a scratch Resource is
/// caller-authored state, but a cached response can always be fetched again.
/// So when the shared object or byte ceiling is reached, the cache yields —
/// least recently used first — before an authoritative object is refused.
#[derive(Debug, Clone)]
struct CachedObject {
    entry: SessionCacheEntry,
    last_used: u64,
}

/// One caller-authored Session Scratch Resource.
///
/// The name indexes this entry; the allocated `id` is the only key the backing
/// store ever sees, so a scratch name never becomes a storage path.
#[derive(Debug, Clone)]
struct ScratchEntry {
    id: ArtifactId,
    version_tag: VersionTag,
    bytes: usize,
}

/// Current content and Version Tag of one Session Scratch Resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchState {
    pub content: String,
    pub version_tag: VersionTag,
}

/// Objects charged against the Path Session's shared object ceiling.
fn session_object_count(state: &SessionState) -> usize {
    state
        .records
        .len()
        .saturating_add(state.scratch.len())
        .saturating_add(state.cache.len())
}

/// Hands out the next recency stamp for a cache access.
fn next_cache_stamp(state: &mut SessionState) -> u64 {
    state.cache_clock = state.cache_clock.wrapping_add(1);
    state.cache_clock
}

/// Evicts least-recently-used cache entries until `admissible` holds or the
/// cache (minus `keep`) is empty. Returns whether `admissible` holds now.
///
/// Callers pass the same predicate they are about to enforce, so an admission
/// that fails after this returns `false` fails for a reason the cache cannot
/// fix: authoritative objects alone exceed the ceiling.
fn yield_cache_until(
    state: &mut SessionState,
    keep: Option<&SessionCacheKey>,
    admissible: impl Fn(&SessionState) -> bool,
) -> bool {
    while !admissible(state) {
        let victim = state
            .cache
            .iter()
            .filter(|(key, _)| keep.is_none_or(|kept| kept != *key))
            .min_by_key(|(_, object)| object.last_used)
            .map(|(key, _)| key.clone());
        let Some(key) = victim else {
            return false;
        };
        release_cache_entry(state, &key);
    }
    true
}

/// Removes one cache entry and credits its bytes back to the session.
fn release_cache_entry(state: &mut SessionState, key: &SessionCacheKey) -> bool {
    let Some(object) = state.cache.remove(key) else {
        return false;
    };
    let released_bytes = key
        .retained_bytes()
        .checked_add(object.entry.retained_bytes())
        .expect("published cache entry byte count fits");
    state.used_bytes = state
        .used_bytes
        .checked_sub(released_bytes)
        .expect("cache bytes are charged before release");
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SnapshotResourceKey(String);

impl SnapshotResourceKey {
    /// Accepts the two families that own mutable, seen-region-bearing content:
    /// canonical workspace Resources and canonical `local://` Session Scratch.
    /// Every other family — immutable `artifact://`, synthetic catalogs — and
    /// every non-canonical spelling is refused, so a snapshot can never be
    /// keyed by a reference that does not identify exactly one Resource.
    fn parse(value: &str) -> Result<Self, ResourceError> {
        let reference = PathReference::parse(value.to_owned())?;
        if reference.requested() != value
            || !matches!(
                reference.address(),
                ResourceAddress::Workspace(WorkspaceAddress::Canonical { .. })
                    | ResourceAddress::Local(LocalAddress::Named(_))
            )
        {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "snapshot identity must be a canonical workspace or local:// reference",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SnapshotKey {
    canonical_reference: SnapshotResourceKey,
    version_tag: VersionTag,
}

#[derive(Debug, Clone)]
struct SeenSnapshot {
    ranges: Vec<DisplayedLineRange>,
    displayed_eof: bool,
}

struct SeenRecord<'a> {
    canonical_reference: &'a SnapshotResourceKey,
    version_tag: &'a VersionTag,
    ranges: &'a [DisplayedLineRange],
    displayed_eof: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SeenSnapshotData {
    pub version_tag: VersionTag,
    pub ranges: Vec<DisplayedLineRange>,
    pub displayed_eof: bool,
}

struct SnapshotUpdate {
    key: SnapshotKey,
    snapshot: SeenSnapshot,
    next_snapshot_bytes: usize,
}

pub(crate) struct SeenReservation {
    session: PathSession,
    update: Option<SnapshotUpdate>,
    reserved_bytes: usize,
}

impl Drop for SeenReservation {
    fn drop(&mut self) {
        if self.reserved_bytes == 0 {
            return;
        }
        let session = self.session.clone();
        let reserved_bytes = self.reserved_bytes;
        tokio::spawn(async move {
            session.release_seen_reservation(reserved_bytes).await;
        });
    }
}

impl PathSession {
    pub fn new(
        token: SessionToken,
        storage: Arc<dyn SessionStorage>,
        limits: ServerLimits,
    ) -> Self {
        Self {
            inner: Arc::new(PathSessionInner {
                token,
                active: AtomicBool::new(true),
                admission: Mutex::new(SessionState {
                    next_object_id: 1,
                    used_bytes: 0,
                    records: HashSet::new(),
                    digest_index: HashMap::new(),
                    scratch: HashMap::new(),
                    cache: HashMap::new(),
                    cache_generations: HashMap::new(),
                    cache_clock: 0,
                    snapshots: HashMap::new(),
                    snapshot_bytes: 0,
                    reserved_snapshot_bytes: 0,
                    mutation_operations: HashMap::new(),
                }),
                storage,
                limits,
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

    /// Binds one Creation Target identity/content pair or joins its current
    /// attempt. Existing success/unknown states replay without new ownership.
    pub async fn begin_mutation_operation(
        &self,
        id: OperationId,
        target: PathReference,
        content: Arc<str>,
    ) -> Result<MutationOperationStart, ResourceError> {
        let fingerprint: [u8; 32] = Sha256::digest(content.as_bytes()).into();
        self.begin_mutation_operation_with_fingerprint(id, target, content, fingerprint)
            .await
    }

    /// Test-only collision injection; exact bytes remain authoritative.
    #[cfg(feature = "test-support")]
    pub async fn begin_mutation_operation_with_fingerprint_for_test(
        &self,
        id: OperationId,
        target: PathReference,
        content: Arc<str>,
        fingerprint: [u8; 32],
    ) -> Result<MutationOperationStart, ResourceError> {
        self.begin_mutation_operation_with_fingerprint(id, target, content, fingerprint)
            .await
    }

    async fn begin_mutation_operation_with_fingerprint(
        &self,
        id: OperationId,
        target: PathReference,
        content: Arc<str>,
        fingerprint: [u8; 32],
    ) -> Result<MutationOperationStart, ResourceError> {
        if !self.is_active() {
            return Err(inactive_mutation_journal_error());
        }
        if content.len() > self.inner.limits.object_bytes() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "mutation operation content exceeds the configured {}-byte object ceiling",
                    self.inner.limits.object_bytes()
                ),
            ));
        }
        let target = target.requested().to_owned();
        let mut state = self.inner.admission.lock().await;
        if !self.is_active() {
            return Err(inactive_mutation_journal_error());
        }
        if let Some(entry) = state.mutation_operations.get_mut(&id) {
            if entry.target != target
                || entry.fingerprint != fingerprint
                || entry.content.as_ref() != content.as_ref()
            {
                return Err(ResourceError::new(
                    ErrorCategory::VersionConflict,
                    "operationId is already bound to a different Creation Target or exact content",
                ));
            }
            return Ok(match entry.attempt.outcome.get() {
                None => MutationOperationStart::Wait(MutationOperationWaiter {
                    attempt: Arc::clone(&entry.attempt),
                }),
                Some(MutationOperationOutcome::Succeeded(reference)) => {
                    MutationOperationStart::Replay(reference.clone())
                }
                Some(MutationOperationOutcome::Unknown(error)) => {
                    MutationOperationStart::Unknown(error.clone())
                }
                Some(MutationOperationOutcome::Conclusive(_)) => {
                    let attempt = Arc::new(MutationAttempt::new());
                    entry.attempt = Arc::clone(&attempt);
                    MutationOperationStart::Owner(MutationOperationLease {
                        attempt,
                        completed: false,
                    })
                }
            });
        }
        if state.mutation_operations.len() >= MAX_MUTATION_OPERATIONS {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "Path Session mutation operation journal exceeds the {MAX_MUTATION_OPERATIONS}-entry ceiling"
                ),
            ));
        }
        let retained_bytes = id
            .as_str()
            .len()
            .checked_add(target.len())
            .and_then(|bytes| bytes.checked_add(content.len()))
            .ok_or_else(|| session_quota_error(self.inner.limits.session_bytes()))?;
        let session_limit = self.inner.limits.session_bytes();
        yield_cache_until(&mut state, None, |state| {
            scratch_resulting_bytes(state, 0, retained_bytes, session_limit).is_ok()
        });
        let resulting_bytes = scratch_resulting_bytes(&state, 0, retained_bytes, session_limit)?;
        let attempt = Arc::new(MutationAttempt::new());
        state.mutation_operations.insert(
            id,
            MutationJournalEntry {
                target,
                content,
                fingerprint,
                attempt: Arc::clone(&attempt),
            },
        );
        state.used_bytes = resulting_bytes;
        Ok(MutationOperationStart::Owner(MutationOperationLease {
            attempt,
            completed: false,
        }))
    }

    pub async fn retain(
        &self,
        content: &str,
        operation: &OperationGuard,
    ) -> Result<ArtifactAddress, ResourceError> {
        let digest: [u8; 32] = Sha256::digest(content.as_bytes()).into();
        self.retain_with_digest(content, digest, operation, None)
            .await
    }

    pub(crate) async fn retain_with_seen(
        &self,
        content: &str,
        operation: &OperationGuard,
        canonical_reference: &str,
        version_tag: &VersionTag,
        ranges: &[DisplayedLineRange],
        displayed_eof: bool,
    ) -> Result<ArtifactAddress, ResourceError> {
        let canonical_reference = SnapshotResourceKey::parse(canonical_reference)?;
        let digest: [u8; 32] = Sha256::digest(content.as_bytes()).into();
        self.retain_with_digest(
            content,
            digest,
            operation,
            Some(SeenRecord {
                canonical_reference: &canonical_reference,
                version_tag,
                ranges,
                displayed_eof,
            }),
        )
        .await
    }

    #[cfg(feature = "test-support")]
    pub async fn retain_with_digest_for_test(
        &self,
        content: &str,
        digest: [u8; 32],
        operation: &OperationGuard,
    ) -> Result<ArtifactAddress, ResourceError> {
        self.retain_with_digest(content, digest, operation, None)
            .await
    }

    async fn retain_with_digest(
        &self,
        content: &str,
        digest: [u8; 32],
        operation: &OperationGuard,
        seen: Option<SeenRecord<'_>>,
    ) -> Result<ArtifactAddress, ResourceError> {
        let content_bytes = content.as_bytes();
        if content_bytes.len() > self.inner.limits.object_bytes() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "selected projection exceeds the configured {}-byte artifact ceiling; narrow the selector",
                    self.inner.limits.object_bytes()
                ),
            ));
        }
        ensure_commit_live(self, operation)?;

        let mut state = self.inner.admission.lock().await;
        ensure_commit_live(self, operation)?;
        let snapshot_update = seen
            .map(|record| {
                prepare_snapshot_update(&state, record, self.inner.limits.session_bytes())
            })
            .transpose()?;
        let next_snapshot_bytes = snapshot_update
            .as_ref()
            .map_or(state.snapshot_bytes, |update| update.next_snapshot_bytes);

        if let Some(candidates) = state.digest_index.get(&digest).cloned() {
            for id in candidates {
                if self.inner.storage.content_equals(id, content_bytes).await? {
                    let next_used_bytes = combined_session_bytes(
                        &state,
                        next_snapshot_bytes,
                        0,
                        self.inner.limits.session_bytes(),
                    )?;
                    apply_snapshot_update(&mut state, snapshot_update);
                    state.used_bytes = next_used_bytes;
                    return self.address(id);
                }
            }
        }

        // A Recovery Reference outranks cached upstream bytes: the cache yields
        // before an artifact is refused for room the cache is holding.
        let session_limit = self.inner.limits.session_bytes();
        let content_len = content_bytes.len();
        yield_cache_until(&mut state, None, |state| {
            session_object_count(state) < MAX_SESSION_ARTIFACTS
                && combined_session_bytes(state, next_snapshot_bytes, content_len, session_limit)
                    .is_ok()
        });
        if session_object_count(&state) >= MAX_SESSION_ARTIFACTS {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "Path Session exceeds the {MAX_SESSION_ARTIFACTS}-artifact ceiling; reuse an existing Recovery Reference"
                ),
            ));
        }
        let resulting_bytes =
            combined_session_bytes(&state, next_snapshot_bytes, content_len, session_limit)?;

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
        apply_snapshot_update(&mut state, snapshot_update);
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

    pub(crate) async fn record_seen(
        &self,
        canonical_reference: &str,
        version_tag: &VersionTag,
        ranges: &[DisplayedLineRange],
        displayed_eof: bool,
    ) -> Result<(), ResourceError> {
        let canonical_reference = SnapshotResourceKey::parse(canonical_reference)?;
        if !self.is_active() {
            return Err(inactive_catalog_error());
        }
        let mut state = self.inner.admission.lock().await;
        if !self.is_active() {
            return Err(inactive_catalog_error());
        }
        let update = prepare_snapshot_update(
            &state,
            SeenRecord {
                canonical_reference: &canonical_reference,
                version_tag,
                ranges,
                displayed_eof,
            },
            self.inner.limits.session_bytes(),
        )?;
        let next_used_bytes = combined_session_bytes(
            &state,
            update.next_snapshot_bytes,
            0,
            self.inner.limits.session_bytes(),
        )?;
        apply_snapshot_update(&mut state, Some(update));
        state.used_bytes = next_used_bytes;
        Ok(())
    }

    pub(crate) async fn reserve_seen(
        &self,
        canonical_reference: &str,
        version_tag: &VersionTag,
        ranges: &[DisplayedLineRange],
        displayed_eof: bool,
    ) -> Result<SeenReservation, ResourceError> {
        let canonical_reference = SnapshotResourceKey::parse(canonical_reference)?;
        let mut state = self.inner.admission.lock().await;
        let key = SnapshotKey {
            canonical_reference: canonical_reference.clone(),
            version_tag: version_tag.clone(),
        };
        let key_bytes = if state.snapshots.contains_key(&key) {
            0
        } else {
            canonical_reference
                .as_str()
                .len()
                .checked_add(version_tag.as_str().len())
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<bool>()))
                .ok_or_else(|| session_quota_error(self.inner.limits.session_bytes()))?
        };
        let reserved_bytes = ranges
            .len()
            .checked_mul(std::mem::size_of::<DisplayedLineRange>())
            .and_then(|bytes| bytes.checked_add(key_bytes))
            .ok_or_else(|| session_quota_error(self.inner.limits.session_bytes()))?;
        let projected = state
            .used_bytes
            .checked_add(state.reserved_snapshot_bytes)
            .and_then(|bytes| bytes.checked_add(reserved_bytes))
            .ok_or_else(|| session_quota_error(self.inner.limits.session_bytes()))?;
        if projected > self.inner.limits.session_bytes() {
            return Err(session_quota_error(self.inner.limits.session_bytes()));
        }
        let update = prepare_snapshot_update(
            &state,
            SeenRecord {
                canonical_reference: &canonical_reference,
                version_tag,
                ranges,
                displayed_eof,
            },
            self.inner.limits.session_bytes(),
        )?;
        state.reserved_snapshot_bytes += reserved_bytes;
        Ok(SeenReservation {
            session: self.clone(),
            update: Some(update),
            reserved_bytes,
        })
    }

    pub(crate) async fn publish_seen(&self, mut reservation: SeenReservation) {
        let mut state = self.inner.admission.lock().await;
        state.reserved_snapshot_bytes = state
            .reserved_snapshot_bytes
            .checked_sub(reservation.reserved_bytes)
            .expect("published reservation must remain charged");
        let update = reservation
            .update
            .take()
            .expect("unpublished reservation carries one snapshot update");
        let artifact_bytes = state
            .used_bytes
            .checked_sub(state.snapshot_bytes)
            .expect("session snapshot accounting cannot exceed used bytes");
        state.used_bytes = artifact_bytes
            .checked_add(update.next_snapshot_bytes)
            .expect("reserved snapshot update fits the session byte ceiling");
        apply_snapshot_update(&mut state, Some(update));
        reservation.reserved_bytes = 0;
    }

    pub(crate) async fn cancel_seen(&self, mut reservation: SeenReservation) {
        self.release_seen_reservation(reservation.reserved_bytes)
            .await;
        reservation.update.take();
        reservation.reserved_bytes = 0;
    }

    async fn release_seen_reservation(&self, reserved_bytes: usize) {
        let mut state = self.inner.admission.lock().await;
        state.reserved_snapshot_bytes = state
            .reserved_snapshot_bytes
            .checked_sub(reserved_bytes)
            .expect("seen reservation release matches its charge");
    }

    #[cfg(feature = "test-support")]
    pub async fn record_seen_for_test(
        &self,
        canonical_reference: &str,
        version_tag: &VersionTag,
        ranges: &[DisplayedLineRange],
        displayed_eof: bool,
    ) -> Result<(), ResourceError> {
        self.record_seen(canonical_reference, version_tag, ranges, displayed_eof)
            .await
    }

    #[cfg(feature = "test-support")]
    pub async fn seen_snapshot_for_test(
        &self,
        canonical_reference: &str,
        version_tag: &VersionTag,
    ) -> Result<Option<(Vec<DisplayedLineRange>, bool)>, ResourceError> {
        let canonical_reference = SnapshotResourceKey::parse(canonical_reference)?;
        let state = self.inner.admission.lock().await;
        let key = SnapshotKey {
            canonical_reference,
            version_tag: version_tag.clone(),
        };
        Ok(state
            .snapshots
            .get(&key)
            .map(|snapshot| (snapshot.ranges.clone(), snapshot.displayed_eof)))
    }

    /// Reads one Session Scratch Resource, or `None` when the name is unused.
    pub async fn scratch_load(
        &self,
        name: &LocalName,
    ) -> Result<Option<ScratchState>, ResourceError> {
        if !self.is_active() {
            return Err(inactive_scratch_error());
        }
        let entry = {
            let state = self.inner.admission.lock().await;
            state.scratch.get(name).cloned()
        };
        let Some(entry) = entry else {
            return Ok(None);
        };
        let content = self.inner.storage.read(entry.id).await?;
        if !self.is_active() {
            return Err(inactive_scratch_error());
        }
        Ok(Some(ScratchState {
            content,
            version_tag: entry.version_tag,
        }))
    }

    /// Creates or replaces one Session Scratch Resource under Path Session
    /// authority, charging its bytes to the shared session ceilings.
    ///
    /// Quota is checked before any write, so a refused put leaves every existing
    /// Resource — scratch or artifact — byte-for-byte unchanged.
    pub async fn scratch_put(
        &self,
        name: &LocalName,
        content: &str,
        operation: &OperationGuard,
    ) -> Result<VersionTag, ResourceError> {
        let content_bytes = content.as_bytes();
        if content_bytes.len() > self.inner.limits.object_bytes() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "Session Scratch Resource exceeds the configured {}-byte object ceiling; write less content",
                    self.inner.limits.object_bytes()
                ),
            ));
        }
        ensure_commit_live(self, operation)?;

        let mut state = self.inner.admission.lock().await;
        ensure_commit_live(self, operation)?;

        let existing = state.scratch.get(name).cloned();
        let released_bytes = existing.as_ref().map_or(0, |entry| entry.bytes);
        let session_limit = self.inner.limits.session_bytes();
        let content_len = content_bytes.len();
        // Caller-authored state outranks cached upstream bytes: the cache
        // yields before a scratch write is refused for room the cache holds.
        yield_cache_until(&mut state, None, |state| {
            (existing.is_some() || session_object_count(state) < MAX_SESSION_ARTIFACTS)
                && scratch_resulting_bytes(state, released_bytes, content_len, session_limit)
                    .is_ok()
        });
        if existing.is_none() && session_object_count(&state) >= MAX_SESSION_ARTIFACTS {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "Path Session exceeds the {MAX_SESSION_ARTIFACTS}-object ceiling; remove a Session Scratch Resource first"
                ),
            ));
        }
        let resulting_bytes =
            scratch_resulting_bytes(&state, released_bytes, content_len, session_limit)?;

        // Replacement reuses the existing object id: `write_atomic` swaps the
        // content atomically, so a failed write leaves the previous content
        // readable under the same name.
        let id = match existing.as_ref() {
            Some(entry) => entry.id,
            None => {
                let id = ArtifactId::new(state.next_object_id)?;
                state.next_object_id = state.next_object_id.checked_add(1).ok_or_else(|| {
                    ResourceError::new(
                        ErrorCategory::LimitExceeded,
                        "Session Scratch object ID space exhausted",
                    )
                })?;
                id
            }
        };
        self.inner.storage.write_atomic(id, content_bytes).await?;

        let version_tag = VersionTag::from_content(content_bytes);
        state.scratch.insert(
            name.clone(),
            ScratchEntry {
                id,
                version_tag: version_tag.clone(),
                bytes: content_bytes.len(),
            },
        );
        state.used_bytes = resulting_bytes;
        Ok(version_tag)
    }

    /// Deletes one Session Scratch Resource and returns its bytes to the session.
    pub async fn scratch_remove(&self, name: &LocalName) -> Result<(), ResourceError> {
        if !self.is_active() {
            return Err(inactive_scratch_error());
        }
        let mut state = self.inner.admission.lock().await;
        let Some(entry) = state.scratch.remove(name) else {
            return Err(scratch_not_found(name));
        };
        state.used_bytes = state
            .used_bytes
            .checked_sub(entry.bytes)
            .expect("scratch bytes are charged before they are released");
        self.inner.storage.remove(entry.id).await
    }

    /// Renames one Session Scratch Resource without clobbering an existing name.
    ///
    /// The backing object is untouched: only the name index moves, so the
    /// Version Tag is unchanged.
    pub async fn scratch_rename(
        &self,
        from: &LocalName,
        to: &LocalName,
    ) -> Result<(), ResourceError> {
        if !self.is_active() {
            return Err(inactive_scratch_error());
        }
        let mut state = self.inner.admission.lock().await;
        if !state.scratch.contains_key(from) {
            return Err(scratch_not_found(from));
        }
        if from != to && state.scratch.contains_key(to) {
            return Err(ResourceError::new(
                ErrorCategory::VersionConflict,
                format!(
                    "Session Scratch Resource '{to}' already exists; move is no-clobber within one Path Session"
                ),
            ));
        }
        let entry = state
            .scratch
            .remove(from)
            .expect("scratch entry presence was checked under this lock");
        state.scratch.insert(to.clone(), entry);
        Ok(())
    }

    pub async fn cache_get(
        &self,
        key: &SessionCacheKey,
    ) -> Result<Option<SessionCacheEntry>, ResourceError> {
        if !self.is_active() {
            return Err(inactive_cache_error());
        }
        let mut state = self.inner.admission.lock().await;
        let stamp = next_cache_stamp(&mut state);
        Ok(state.cache.get_mut(key).map(|object| {
            object.last_used = stamp;
            object.entry.clone()
        }))
    }

    /// Admits one cached upstream representation under the shared ceilings.
    ///
    /// A new entry that would breach the object or byte ceiling first evicts
    /// other cache entries, least recently used first; only when the ceiling is
    /// held by authoritative objects — artifacts, scratch, snapshots — is the
    /// put refused, and the caller decides whether serving uncached is fine.
    pub async fn cache_put(
        &self,
        key: SessionCacheKey,
        entry: SessionCacheEntry,
    ) -> Result<(), ResourceError> {
        self.cache_put_inner(key, entry, None).await.map(|_| ())
    }

    /// Admits a fetch result only if no namespace invalidation happened since
    /// the caller captured `generation`.
    pub async fn cache_put_if_generation(
        &self,
        key: SessionCacheKey,
        entry: SessionCacheEntry,
        generation: u64,
    ) -> Result<bool, ResourceError> {
        self.cache_put_inner(key, entry, Some(generation)).await
    }

    async fn cache_put_inner(
        &self,
        key: SessionCacheKey,
        entry: SessionCacheEntry,
        expected_generation: Option<u64>,
    ) -> Result<bool, ResourceError> {
        if !self.is_active() {
            return Err(inactive_cache_error());
        }
        if entry.retained_bytes() > self.inner.limits.object_bytes() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "session cache entry exceeds the configured {}-byte object ceiling",
                    self.inner.limits.object_bytes()
                ),
            ));
        }
        let session_limit = self.inner.limits.session_bytes();
        let added_bytes = key
            .retained_bytes()
            .checked_add(entry.retained_bytes())
            .ok_or_else(|| session_quota_error(session_limit))?;
        let mut state = self.inner.admission.lock().await;
        if expected_generation.is_some_and(|expected| {
            state
                .cache_generations
                .get(key.namespace())
                .copied()
                .unwrap_or(0)
                != expected
        }) {
            return Ok(false);
        }
        let released_bytes = state.cache.get(&key).map_or(0, |previous| {
            key.retained_bytes()
                .saturating_add(previous.entry.retained_bytes())
        });
        let replacing = released_bytes != 0;
        yield_cache_until(&mut state, Some(&key), |state| {
            (replacing || session_object_count(state) < MAX_SESSION_ARTIFACTS)
                && scratch_resulting_bytes(state, released_bytes, added_bytes, session_limit)
                    .is_ok()
        });
        if !replacing && session_object_count(&state) >= MAX_SESSION_ARTIFACTS {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "Path Session object count exceeds the {MAX_SESSION_ARTIFACTS}-object ceiling"
                ),
            ));
        }
        let resulting_bytes =
            scratch_resulting_bytes(&state, released_bytes, added_bytes, session_limit)?;
        let last_used = next_cache_stamp(&mut state);
        state.cache.insert(key, CachedObject { entry, last_used });
        state.used_bytes = resulting_bytes;
        Ok(true)
    }

    /// Captures the invalidation generation for one cache namespace.
    pub async fn cache_generation(&self, namespace: &str) -> Result<u64, ResourceError> {
        if !self.is_active() {
            return Err(inactive_cache_error());
        }
        validate_cache_namespace(namespace)?;
        let state = self.inner.admission.lock().await;
        Ok(state.cache_generations.get(namespace).copied().unwrap_or(0))
    }

    /// Removes an exact entry only if its fetch generation is still current.
    pub async fn cache_remove_if_generation(
        &self,
        key: &SessionCacheKey,
        generation: u64,
    ) -> Result<bool, ResourceError> {
        if !self.is_active() {
            return Err(inactive_cache_error());
        }
        let mut state = self.inner.admission.lock().await;
        if state
            .cache_generations
            .get(key.namespace())
            .copied()
            .unwrap_or(0)
            != generation
        {
            return Ok(false);
        }
        Ok(release_cache_entry(&mut state, key))
    }

    pub async fn cache_remove(&self, key: &SessionCacheKey) -> Result<bool, ResourceError> {
        if !self.is_active() {
            return Err(inactive_cache_error());
        }
        let mut state = self.inner.admission.lock().await;
        Ok(release_cache_entry(&mut state, key))
    }

    /// Removes every reconstructible cache entry in one source namespace.
    pub async fn cache_remove_namespace(&self, namespace: &str) -> Result<usize, ResourceError> {
        if !self.is_active() {
            return Err(inactive_cache_error());
        }
        validate_cache_namespace(namespace)?;
        let mut state = self.inner.admission.lock().await;
        let generation = state
            .cache_generations
            .get(namespace)
            .copied()
            .unwrap_or(0)
            .wrapping_add(1);
        state
            .cache_generations
            .insert(namespace.to_owned(), generation);
        let keys = state
            .cache
            .keys()
            .filter(|key| key.namespace() == namespace)
            .cloned()
            .collect::<Vec<_>>();
        let removed = keys.len();
        for key in keys {
            let released = release_cache_entry(&mut state, &key);
            debug_assert!(
                released,
                "collected cache key remains present until release"
            );
        }
        Ok(removed)
    }
    /// Enumerates this Path Session's Session Scratch names in sorted order.
    pub async fn scratch_names(&self) -> Result<Vec<LocalName>, ResourceError> {
        if !self.is_active() {
            return Err(inactive_scratch_error());
        }
        let state = self.inner.admission.lock().await;
        let mut names = state.scratch.keys().cloned().collect::<Vec<_>>();
        names.sort_unstable();
        Ok(names)
    }

    pub async fn used_bytes(&self) -> usize {
        self.inner.admission.lock().await.used_bytes
    }

    pub async fn artifact_count(&self) -> usize {
        self.inner.admission.lock().await.records.len()
    }

    pub(crate) async fn resolve_seen(
        &self,
        canonical_reference: &str,
        selector: &VersionSelector,
    ) -> Result<SeenSnapshotData, ResourceError> {
        if !self.is_active() {
            return Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "Path Session is inactive",
            ));
        }
        let canonical_reference = SnapshotResourceKey::parse(canonical_reference)?;
        let state = self.inner.admission.lock().await;
        let matches = state
            .snapshots
            .iter()
            .filter(|(key, _)| key.canonical_reference == canonical_reference)
            .filter(|(key, _)| selector.matches(&key.version_tag))
            .collect::<Vec<_>>();
        let [(key, snapshot)] = matches.as_slice() else {
            let error = if matches.is_empty() {
                ResourceError::new(
                    ErrorCategory::VersionConflict,
                    "no read snapshot matches the Resource and Version Tag",
                )
            } else {
                ResourceError::new(
                    ErrorCategory::InvalidPatch,
                    "Version Tag prefix is ambiguous in this Path Session",
                )
            };
            return Err(error);
        };
        Ok(SeenSnapshotData {
            version_tag: key.version_tag.clone(),
            ranges: snapshot.ranges.clone(),
            displayed_eof: snapshot.displayed_eof,
        })
    }

    #[cfg(feature = "test-support")]
    pub async fn resolve_seen_for_test(
        &self,
        canonical_reference: &str,
        selector: &VersionSelector,
    ) -> Result<(VersionTag, Vec<DisplayedLineRange>, bool), ResourceError> {
        let snapshot = self.resolve_seen(canonical_reference, selector).await?;
        Ok((
            snapshot.version_tag,
            snapshot.ranges,
            snapshot.displayed_eof,
        ))
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

fn snapshot_allocation_bytes(snapshot: &SeenSnapshot) -> usize {
    snapshot.ranges.capacity() * std::mem::size_of::<DisplayedLineRange>()
        + std::mem::size_of::<bool>()
}

fn merge_seen_ranges(
    existing: &[DisplayedLineRange],
    incoming: &[DisplayedLineRange],
) -> Result<Vec<DisplayedLineRange>, ResourceError> {
    let mut ranges = Vec::with_capacity(existing.len().saturating_add(incoming.len()));
    ranges.extend_from_slice(existing);
    ranges.extend_from_slice(incoming);
    ranges.sort_unstable_by_key(|range| range.start_line());
    let mut merged: Vec<DisplayedLineRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && last
                .end_line()
                .checked_add(1)
                .is_some_and(|next| next >= range.start_line())
        {
            *last =
                DisplayedLineRange::new(last.start_line(), last.end_line().max(range.end_line()))?;
        } else {
            merged.push(range);
        }
    }
    merged.shrink_to_fit();
    Ok(merged)
}

fn prepare_snapshot_update(
    state: &SessionState,
    seen: SeenRecord<'_>,
    session_limit: usize,
) -> Result<SnapshotUpdate, ResourceError> {
    let key = SnapshotKey {
        canonical_reference: seen.canonical_reference.clone(),
        version_tag: seen.version_tag.clone(),
    };
    let existing = state.snapshots.get(&key);
    let snapshot = SeenSnapshot {
        ranges: merge_seen_ranges(
            existing.map_or(&[], |snapshot| snapshot.ranges.as_slice()),
            seen.ranges,
        )?,
        displayed_eof: seen.displayed_eof
            || existing.is_some_and(|snapshot| snapshot.displayed_eof),
    };
    let previous_bytes = existing.map_or(0, snapshot_allocation_bytes);
    let key_bytes = if existing.is_none() {
        seen.canonical_reference
            .as_str()
            .len()
            .checked_add(seen.version_tag.as_str().len())
            .ok_or_else(|| session_quota_error(session_limit))?
    } else {
        0
    };
    let next_snapshot_bytes = state
        .snapshot_bytes
        .checked_sub(previous_bytes)
        .and_then(|bytes| bytes.checked_add(key_bytes))
        .and_then(|bytes| bytes.checked_add(snapshot_allocation_bytes(&snapshot)))
        .ok_or_else(|| session_quota_error(session_limit))?;
    Ok(SnapshotUpdate {
        key,
        snapshot,
        next_snapshot_bytes,
    })
}

fn combined_session_bytes(
    state: &SessionState,
    next_snapshot_bytes: usize,
    added_artifact_bytes: usize,
    session_limit: usize,
) -> Result<usize, ResourceError> {
    let artifact_bytes = state
        .used_bytes
        .checked_sub(state.snapshot_bytes)
        .ok_or_else(|| session_quota_error(session_limit))?;
    let resulting = artifact_bytes
        .checked_add(added_artifact_bytes)
        .and_then(|bytes| bytes.checked_add(next_snapshot_bytes))
        .ok_or_else(|| session_quota_error(session_limit))?;
    let with_reservations = resulting
        .checked_add(state.reserved_snapshot_bytes)
        .ok_or_else(|| session_quota_error(session_limit))?;
    if with_reservations > session_limit {
        return Err(session_quota_error(session_limit));
    }
    Ok(resulting)
}

/// Session bytes after releasing one Session Scratch Resource's previous bytes
/// and charging its replacement, refusing anything past the session ceiling.
fn scratch_resulting_bytes(
    state: &SessionState,
    released_bytes: usize,
    added_bytes: usize,
    session_limit: usize,
) -> Result<usize, ResourceError> {
    let resulting = state
        .used_bytes
        .checked_sub(released_bytes)
        .and_then(|bytes| bytes.checked_add(added_bytes))
        .ok_or_else(|| session_quota_error(session_limit))?;
    let with_reservations = resulting
        .checked_add(state.reserved_snapshot_bytes)
        .ok_or_else(|| session_quota_error(session_limit))?;
    if with_reservations > session_limit {
        return Err(session_quota_error(session_limit));
    }
    Ok(resulting)
}

fn apply_snapshot_update(state: &mut SessionState, update: Option<SnapshotUpdate>) {
    let Some(update) = update else {
        return;
    };
    state.snapshot_bytes = update.next_snapshot_bytes;
    state.snapshots.insert(update.key, update.snapshot);
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

fn session_quota_error(limit: usize) -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        format!("Path Session exceeds the {limit}-byte storage quota; narrow or remove content"),
    )
}

fn inactive_scratch_error() -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        "Path Session disconnected before the Session Scratch operation",
    )
}

fn inactive_cache_error() -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        "Path Session disconnected before the cache operation",
    )
}

fn inactive_mutation_journal_error() -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        "Path Session disconnected before the mutation operation journal access",
    )
}

fn scratch_not_found(name: &LocalName) -> ResourceError {
    ResourceError::new(
        ErrorCategory::NotFound,
        format!("Session Scratch Resource '{name}' is not available in this Path Session"),
    )
}

fn artifact_not_found() -> ResourceError {
    ResourceError::new(
        ErrorCategory::NotFound,
        "artifact is not available in this Path Session",
    )
}
