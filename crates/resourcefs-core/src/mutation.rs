use std::{
    collections::HashMap,
    fmt,
    num::NonZeroU64,
    sync::{Arc, Mutex as StdMutex, Weak},
};

use async_trait::async_trait;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::{
    ErrorCategory, MAX_ARTIFACT_BYTES, OperationGuard, PathReference, PathSession, ResourceAddress,
    ResourceError, VersionTag, WorkspaceAddress,
};

pub const MAX_HASHLINE_PATCH_BYTES: usize = MAX_ARTIFACT_BYTES;
pub const MIN_VERSION_PREFIX_HEX: usize = 12;

const ACCEPTED_GRAMMAR: &str =
    "accepted forms: PUT N.=M:, PUT <N:, PUT >N:, PUT >$:, CUT N.=M, REM, or MV <destination>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionSelector(VersionSelectorKind);

#[derive(Debug, Clone, PartialEq, Eq)]
enum VersionSelectorKind {
    Full(VersionTag),
    Prefix(String),
}

impl VersionSelector {
    pub fn full(tag: VersionTag) -> Self {
        Self(VersionSelectorKind::Full(tag))
    }

    pub fn prefix(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        if value.len() < MIN_VERSION_PREFIX_HEX || value.len() > 64 || !is_lower_hex(&value) {
            return Err(invalid_patch(format!(
                "Version Tag prefix must contain {MIN_VERSION_PREFIX_HEX}-64 lowercase hexadecimal characters"
            )));
        }
        Ok(Self(VersionSelectorKind::Prefix(value)))
    }

    pub fn as_str(&self) -> &str {
        match &self.0 {
            VersionSelectorKind::Full(tag) => tag.as_str(),
            VersionSelectorKind::Prefix(prefix) => prefix,
        }
    }

    pub fn matches(&self, tag: &VersionTag) -> bool {
        match &self.0 {
            VersionSelectorKind::Full(expected) => expected == tag,
            VersionSelectorKind::Prefix(prefix) => tag
                .as_str()
                .strip_prefix("sha256:")
                .is_some_and(|hex| hex.starts_with(prefix)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineNumber(NonZeroU64);

impl LineNumber {
    pub fn new(value: u64) -> Result<Self, ResourceError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or_else(|| invalid_patch("line coordinates must be positive"))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginalLineRange {
    start: LineNumber,
    end: LineNumber,
}

impl OriginalLineRange {
    pub fn new(start: LineNumber, end: LineNumber) -> Result<Self, ResourceError> {
        if end < start {
            return Err(invalid_patch("line range end must not precede its start"));
        }
        Ok(Self { start, end })
    }

    pub const fn start(self) -> LineNumber {
        self.start
    }

    pub const fn end(self) -> LineNumber {
        self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutTarget {
    Range(OriginalLineRange),
    Before(LineNumber),
    After(LineNumber),
    Tail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchOperation(PatchOperationKind);

#[derive(Debug, Clone, PartialEq, Eq)]
enum PatchOperationKind {
    Put {
        target: PutTarget,
        body: Vec<String>,
    },
    Cut {
        range: OriginalLineRange,
    },
    Remove,
    Move {
        destination: Box<PathReference>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOperationRef<'a> {
    Put {
        target: PutTarget,
        body: &'a [String],
    },
    Cut {
        range: OriginalLineRange,
    },
    Remove,
    Move {
        destination: &'a PathReference,
    },
}

impl PatchOperation {
    pub fn kind(&self) -> PatchOperationRef<'_> {
        match &self.0 {
            PatchOperationKind::Put { target, body } => PatchOperationRef::Put {
                target: *target,
                body,
            },
            PatchOperationKind::Cut { range } => PatchOperationRef::Cut { range: *range },
            PatchOperationKind::Remove => PatchOperationRef::Remove,
            PatchOperationKind::Move { destination } => PatchOperationRef::Move { destination },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashlinePatch {
    target: PathReference,
    version: VersionSelector,
    operations: Vec<PatchOperation>,
}

impl HashlinePatch {
    pub fn parse(document: &str) -> Result<Self, ResourceError> {
        if document.len() > MAX_HASHLINE_PATCH_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!("hashline patch exceeds the {MAX_HASHLINE_PATCH_BYTES}-byte ceiling"),
            ));
        }
        let mut lines = document.lines();
        let header = lines
            .next()
            .ok_or_else(|| invalid_patch("hashline patch is empty"))?;
        let (target, version) = parse_header(header)?;
        let mut body = lines.peekable();
        if body.peek().is_none() {
            return Err(invalid_patch("hashline patch contains no operation"));
        }

        let mut operations = Vec::new();
        while let Some(line) = body.next() {
            if line.starts_with('[') {
                return Err(invalid_patch(
                    "one rfs_edit document may contain exactly one Resource header",
                ));
            }
            if line == "REM" {
                if !operations.is_empty() || body.peek().is_some() {
                    return Err(invalid_patch("REM must be the sole operation"));
                }
                operations.push(PatchOperation(PatchOperationKind::Remove));
                continue;
            }
            if let Some(destination) = line.strip_prefix("MV ") {
                if !operations.is_empty() || body.peek().is_some() || destination.is_empty() {
                    return Err(invalid_patch(
                        "MV must be the sole operation with one destination",
                    ));
                }
                let destination = PathReference::parse(destination.to_owned())
                    .map_err(|error| invalid_patch(format!("invalid MV destination: {error}")))?;
                operations.push(PatchOperation(PatchOperationKind::Move {
                    destination: Box::new(destination),
                }));
                continue;
            }
            if line.starts_with("PUT ") {
                reject_reserved_form(line)?;
                let locator = line
                    .strip_prefix("PUT ")
                    .and_then(|line| line.strip_suffix(':'))
                    .ok_or_else(|| invalid_patch(format!("malformed PUT; {ACCEPTED_GRAMMAR}")))?;
                let target = parse_put_target(locator)?;
                let mut replacement = Vec::new();
                while let Some(row) = body.peek().and_then(|row| row.strip_prefix('+')) {
                    replacement.push(row.to_owned());
                    body.next();
                }
                if replacement.is_empty() {
                    return Err(invalid_patch(
                        "PUT requires one or more +TEXT body rows; use + alone for a blank line",
                    ));
                }
                operations.push(PatchOperation(PatchOperationKind::Put {
                    target,
                    body: replacement,
                }));
                continue;
            }
            if line.starts_with("CUT ") {
                reject_reserved_form(line)?;
                if line.contains('@') {
                    return Err(reserved_form("register capture"));
                }
                let range = parse_original_range(
                    line.strip_prefix("CUT ")
                        .ok_or_else(|| invalid_patch("malformed CUT"))?,
                )?;
                operations.push(PatchOperation(PatchOperationKind::Cut { range }));
                continue;
            }
            if line.starts_with("MV") {
                return Err(invalid_patch("malformed MV; expected MV <destination>"));
            }
            if line.starts_with("REM") {
                return Err(invalid_patch("malformed REM; REM takes no arguments"));
            }
            if line.contains('@') {
                return Err(reserved_form("register paste"));
            }
            return Err(invalid_patch(format!(
                "unrecognized hashline operation {line:?}; {ACCEPTED_GRAMMAR}"
            )));
        }
        validate_operation_conflicts(&operations)?;
        Ok(Self {
            target,
            version,
            operations,
        })
    }

    pub fn target(&self) -> &PathReference {
        &self.target
    }

    pub const fn version(&self) -> &VersionSelector {
        &self.version
    }

    pub fn operations(&self) -> &[PatchOperation] {
        &self.operations
    }
}

fn parse_header(header: &str) -> Result<(PathReference, VersionSelector), ResourceError> {
    let inner = header
        .strip_prefix('[')
        .and_then(|header| header.strip_suffix(']'))
        .ok_or_else(|| {
            invalid_patch("patch must begin with [<canonical-reference>#<Version-Tag>]")
        })?;
    let (reference, version) = inner.rsplit_once('#').ok_or_else(|| {
        invalid_patch("patch header must separate reference and Version Tag with #")
    })?;
    if reference.is_empty() || version.is_empty() {
        return Err(invalid_patch(
            "patch header reference and Version Tag must be non-empty",
        ));
    }
    let target = PathReference::parse(reference.to_owned())
        .map_err(|error| invalid_patch(format!("invalid patch header reference: {error}")))?;
    if let ResourceAddress::Workspace(address) = target.address()
        && !matches!(address, WorkspaceAddress::Canonical { .. })
    {
        return Err(invalid_patch(
            "patch header must use the canonical workspace reference rendered by rfs_read",
        ));
    }
    let version = parse_version_selector(version)?;
    Ok((target, version))
}

fn parse_version_selector(value: &str) -> Result<VersionSelector, ResourceError> {
    if let Some(hex) = value.strip_prefix("sha256:") {
        if hex.len() != 64 || !is_lower_hex(hex) {
            return Err(invalid_patch(
                "full Version Tag must be sha256: followed by 64 lowercase hexadecimal characters",
            ));
        }
        return Ok(VersionSelector::full(VersionTag::from_sha256_hex(hex)));
    }
    VersionSelector::prefix(value)
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn reject_reserved_form(line: &str) -> Result<(), ResourceError> {
    if line.contains('*') {
        return Err(reserved_form("block-star operator"));
    }
    if line.contains('@') {
        return Err(reserved_form("register form"));
    }
    Ok(())
}

fn reserved_form(name: &str) -> ResourceError {
    invalid_patch(format!(
        "{name} is reserved but unsupported in ResourceFS 1.0; {ACCEPTED_GRAMMAR}"
    ))
}

fn parse_put_target(locator: &str) -> Result<PutTarget, ResourceError> {
    if locator == ">$" {
        return Ok(PutTarget::Tail);
    }
    if let Some(line) = locator.strip_prefix('<') {
        return positive_line(line).map(PutTarget::Before);
    }
    if let Some(line) = locator.strip_prefix('>') {
        return positive_line(line).map(PutTarget::After);
    }
    parse_original_range(locator).map(PutTarget::Range)
}

fn parse_original_range(value: &str) -> Result<OriginalLineRange, ResourceError> {
    let (start, end) = value
        .split_once(".=")
        .ok_or_else(|| invalid_patch("line ranges must use N.=M inclusive spelling"))?;
    let start = positive_line(start)?;
    let end = positive_line(end)?;
    OriginalLineRange::new(start, end)
}

fn positive_line(value: &str) -> Result<LineNumber, ResourceError> {
    let value = value
        .parse::<u64>()
        .ok()
        .filter(|line| line.to_string() == value)
        .ok_or_else(|| invalid_patch("line coordinates must be canonical positive integers"))?;
    LineNumber::new(value)
}
fn validate_operation_conflicts(operations: &[PatchOperation]) -> Result<(), ResourceError> {
    let mut ranges: Vec<OriginalLineRange> = Vec::new();
    let mut gaps = Vec::new();
    for operation in operations {
        match &operation.0 {
            PatchOperationKind::Put {
                target: PutTarget::Range(range),
                ..
            }
            | PatchOperationKind::Cut { range } => ranges.push(*range),
            PatchOperationKind::Put {
                target: PutTarget::Before(line),
                ..
            } => gaps.push(line.get() - 1),
            PatchOperationKind::Put {
                target: PutTarget::After(line),
                ..
            } => gaps.push(line.get()),
            PatchOperationKind::Put {
                target: PutTarget::Tail,
                ..
            } => gaps.push(u64::MAX),
            PatchOperationKind::Remove | PatchOperationKind::Move { .. } => {}
        }
    }
    ranges.sort_unstable_by_key(|range| (range.start(), range.end()));
    for pair in ranges.windows(2) {
        if pair[1].start() <= pair[0].end() {
            return Err(invalid_patch(
                "patch ranges overlap in the original tagged snapshot",
            ));
        }
    }
    gaps.sort_unstable();
    if gaps.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid_patch(
            "patch inserts more than once at the same original gap",
        ));
    }
    Ok(())
}

fn invalid_patch(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::InvalidPatch, message)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationAccess {
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MutationSourceKey(String);

impl MutationSourceKey {
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "mutation source key must not be empty",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationTarget {
    canonical_reference: PathReference,
    source_key: MutationSourceKey,
}

impl MutationTarget {
    pub fn new(
        canonical_reference: PathReference,
        source_key: MutationSourceKey,
    ) -> Result<Self, ResourceError> {
        if let ResourceAddress::Workspace(address) = canonical_reference.address()
            && !matches!(address, WorkspaceAddress::Canonical { .. })
        {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "mutation target must use a canonical workspace reference",
            ));
        }
        Ok(Self {
            canonical_reference,
            source_key,
        })
    }

    pub const fn canonical_reference(&self) -> &PathReference {
        &self.canonical_reference
    }

    pub const fn source_key(&self) -> &MutationSourceKey {
        &self.source_key
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationState {
    Missing,
    Text {
        content: String,
        version_tag: VersionTag,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceMutation {
    Create {
        target: MutationTarget,
        content: String,
    },
    Replace {
        target: MutationTarget,
        expected: VersionTag,
        content: String,
    },
    Delete {
        target: MutationTarget,
        expected: VersionTag,
    },
    Move {
        source: MutationTarget,
        destination: Box<MutationTarget>,
        expected: VersionTag,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationOutcome {
    Created {
        target: MutationTarget,
        version_tag: VersionTag,
    },
    Replaced {
        target: MutationTarget,
        version_tag: VersionTag,
    },
    Deleted {
        target: MutationTarget,
    },
    Moved {
        source: MutationTarget,
        destination: Box<MutationTarget>,
        version_tag: VersionTag,
    },
}

#[async_trait]
pub trait MutationAdapter: Send + Sync {
    async fn resolve(
        &self,
        reference: &PathReference,
        access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError>;

    async fn load(
        &self,
        target: &MutationTarget,
        access: MutationAccess,
        operation: &OperationGuard,
    ) -> Result<MutationState, ResourceError>;

    async fn commit(
        &self,
        mutation: SourceMutation,
        operation: &OperationGuard,
    ) -> Result<MutationOutcome, ResourceError>;
}

#[derive(Clone)]
pub struct MutationEngine {
    adapter: Arc<dyn MutationAdapter>,
    session: PathSession,
    locks: Arc<MutationLocks>,
}

impl fmt::Debug for MutationEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MutationEngine")
            .finish_non_exhaustive()
    }
}

impl MutationEngine {
    pub fn new(adapter: Arc<dyn MutationAdapter>, session: PathSession) -> Self {
        Self {
            adapter,
            session,
            locks: Arc::new(MutationLocks::default()),
        }
    }

    pub fn parse_edit(&self, document: &str) -> Result<HashlinePatch, ResourceError> {
        HashlinePatch::parse(document)
    }

    pub fn adapter(&self) -> &Arc<dyn MutationAdapter> {
        &self.adapter
    }

    pub fn session(&self) -> &PathSession {
        &self.session
    }

    pub(crate) async fn lock_resources(
        &self,
        keys: impl IntoIterator<Item = String>,
    ) -> Result<MutationLockSet, ResourceError> {
        self.locks.acquire(keys).await
    }

    #[cfg(feature = "test-support")]
    pub async fn lock_resources_for_test(
        &self,
        keys: Vec<String>,
    ) -> Result<MutationLockSet, ResourceError> {
        self.lock_resources(keys).await
    }
}

#[derive(Default)]
struct MutationLocks {
    entries: StdMutex<HashMap<String, Weak<Mutex<()>>>>,
}

impl MutationLocks {
    async fn acquire(
        &self,
        keys: impl IntoIterator<Item = String>,
    ) -> Result<MutationLockSet, ResourceError> {
        let mut keys = keys.into_iter().collect::<Vec<_>>();
        keys.sort_unstable();
        keys.dedup();
        if keys.is_empty() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "mutation lock set must not be empty",
            ));
        }
        let locks = {
            let mut entries = self.entries.lock().map_err(|_| {
                ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "mutation lock registry is poisoned",
                )
            })?;
            entries.retain(|_, lock| lock.strong_count() != 0);
            keys.into_iter()
                .map(|key| match entries.entry(key) {
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        entry.get().upgrade().unwrap_or_else(|| {
                            let lock = Arc::new(Mutex::new(()));
                            entry.insert(Arc::downgrade(&lock));
                            lock
                        })
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let lock = Arc::new(Mutex::new(()));
                        entry.insert(Arc::downgrade(&lock));
                        lock
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut guards = Vec::with_capacity(locks.len());
        for lock in locks {
            guards.push(lock.lock_owned().await);
            #[cfg(feature = "test-support")]
            tokio::task::yield_now().await;
        }
        Ok(MutationLockSet { _guards: guards })
    }
}

pub struct MutationLockSet {
    _guards: Vec<OwnedMutexGuard<()>>,
}

impl fmt::Debug for MutationLockSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MutationLockSet")
            .field("resources", &self._guards.len())
            .finish()
    }
}
