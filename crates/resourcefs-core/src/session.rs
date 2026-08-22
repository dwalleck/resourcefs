use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};

use crate::{
    ArtifactAddress, DisplayedLineRange, ErrorCategory, ResourceError, ServerLimits,
    VersionSelector, VersionTag,
};

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
    snapshots: HashMap<SnapshotKey, SeenSnapshot>,
    snapshot_bytes: usize,
    reserved_snapshot_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SnapshotKey {
    canonical_reference: String,
    version_tag: VersionTag,
}

#[derive(Debug, Clone)]
struct SeenSnapshot {
    ranges: Vec<DisplayedLineRange>,
    displayed_eof: bool,
}

struct SeenRecord<'a> {
    canonical_reference: &'a str,
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
                    snapshots: HashMap::new(),
                    snapshot_bytes: 0,
                    reserved_snapshot_bytes: 0,
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
        let digest: [u8; 32] = Sha256::digest(content.as_bytes()).into();
        self.retain_with_digest(
            content,
            digest,
            operation,
            Some(SeenRecord {
                canonical_reference,
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

        if state.records.len() >= MAX_SESSION_ARTIFACTS {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "Path Session exceeds the {MAX_SESSION_ARTIFACTS}-artifact ceiling; reuse an existing Recovery Reference"
                ),
            ));
        }
        let resulting_bytes = combined_session_bytes(
            &state,
            next_snapshot_bytes,
            content_bytes.len(),
            self.inner.limits.session_bytes(),
        )?;

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
                canonical_reference,
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
        let mut state = self.inner.admission.lock().await;
        let key = SnapshotKey {
            canonical_reference: canonical_reference.to_owned(),
            version_tag: version_tag.clone(),
        };
        let key_bytes = if state.snapshots.contains_key(&key) {
            0
        } else {
            canonical_reference
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
                canonical_reference,
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
    ) -> Option<(Vec<DisplayedLineRange>, bool)> {
        let state = self.inner.admission.lock().await;
        let key = SnapshotKey {
            canonical_reference: canonical_reference.to_owned(),
            version_tag: version_tag.clone(),
        };
        state
            .snapshots
            .get(&key)
            .map(|snapshot| (snapshot.ranges.clone(), snapshot.displayed_eof))
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
        canonical_reference: seen.canonical_reference.to_owned(),
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
        format!("Path Session exceeds the {limit}-byte artifact quota; narrow the selector"),
    )
}

fn artifact_not_found() -> ResourceError {
    ResourceError::new(
        ErrorCategory::NotFound,
        "artifact is not available in this Path Session",
    )
}
