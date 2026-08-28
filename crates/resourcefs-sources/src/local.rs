use std::{fmt, io::Cursor};

use async_trait::async_trait;
use resourcefs_core::{
    DiscoveryAdapter, ErrorCategory, GlobEntry, GlobKind, GlobOptions, GlobTarget, LocalAddress,
    LocalName, MutationAccess, MutationAdapter, MutationCommitFailure, MutationCommitOutcome,
    MutationSourceKey, MutationState, MutationTarget, MutationTargetMode, OperationGuard,
    PathReference, PathSession, ResourceAddress, ResourceError, SearchOptions, SearchRecord,
    SearchSourceResult, SearchTarget, SourceAdapter, SourceGlobResult, SourceMutation,
    SourceResource, VersionTag, select_utf8,
};

use crate::{
    catalog::{SourceCatalogEntry, SourceCatalogMetadata},
    pattern::{GlobMatcher, SearchMatcher},
};

/// Stable mutation source key for Session Scratch.
///
/// Distinct from every other source's key, so per-canonical-Resource lock
/// identity never collides a scratch name with a workspace path.
pub(crate) const LOCAL_MUTATION_SOURCE_KEY: &str = "local";

/// Read, discovery, and mutation adapter for `local://` Session Scratch.
///
/// Scratch is the one writable Path-Session-owned family: authority is session
/// ownership, so no `MutationGrants` is consulted here. Quotas remain the only
/// bound, and they are enforced by `PathSession` itself.
#[derive(Clone)]
pub struct LocalSource {
    session: PathSession,
}

impl LocalSource {
    pub fn new(session: PathSession) -> Self {
        Self { session }
    }

    /// Resolves a scratch reference to its content under the literal-wins rule.
    ///
    /// A colon is a legal scratch-name character, so `local://plan.md:1-5` is
    /// first a candidate *name*. Only when no Resource carries that literal name
    /// does the trailing text become a projection selector — the same rule the
    /// Workspace family applies in `filesystem::read_from_view`.
    async fn read_named(&self, reference: &PathReference) -> Result<SourceResource, ResourceError> {
        let ResourceAddress::Local(LocalAddress::Named(name)) = reference.address() else {
            return Err(unsupported_local_target());
        };

        if let Some(state) = self.session.scratch_load(name).await? {
            return SourceResource::text_projection(
                PathReference::local(name.as_str())?,
                state.content,
                state.version_tag,
            )
            .map(|resource| resource.with_mutability(true));
        }

        // The literal name is absent: fall back to the selector reading.
        let Some(candidate) = reference.local_selector_candidate() else {
            return Err(scratch_not_found(name));
        };
        let Some(state) = self.session.scratch_load(candidate.base()).await? else {
            return Err(scratch_not_found(name));
        };
        let selected = select_utf8(Cursor::new(state.content), Some(candidate.selector()))?;
        let (content, version_tag, _) = selected.into_parts();
        // Identity is the RESOLVED Resource, never the requested spelling, so a
        // later edit finds this read's seen region under `local://<base>`.
        SourceResource::text_projection(
            PathReference::local(candidate.base().as_str())?,
            content,
            version_tag,
        )
        .map(|resource| resource.with_mutability(true))
    }

    /// Renders the scratch family root: a bounded, sorted, self-describing
    /// listing of this Path Session's scratch Resources.
    async fn read_root(&self) -> Result<SourceResource, ResourceError> {
        let names = self.session.scratch_names().await?;
        let mut content = String::from(
            "Session Scratch — caller-owned Resources for this Path Session.\n\
             Write with rfs_write local://<name>; read one with rfs_read local://<name>.\n\n",
        );
        if names.is_empty() {
            content.push_str("No Session Scratch Resources in this Path Session.\n");
        } else {
            for name in &names {
                content.push_str(PathReference::local(name.as_str())?.requested());
                content.push('\n');
            }
        }
        let version_tag = VersionTag::from_content(content.as_bytes());
        // The listing is synthetic and read-only: only named scratch is mutable.
        SourceResource::text_projection(PathReference::local_root(), content, version_tag)
    }

    async fn scratch_state(&self, name: &LocalName) -> Result<MutationState, ResourceError> {
        Ok(match self.session.scratch_load(name).await? {
            Some(state) => MutationState::Text {
                content: state.content,
                version_tag: state.version_tag,
            },
            None => MutationState::Missing,
        })
    }
}

impl fmt::Debug for LocalSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalSource")
            .finish_non_exhaustive()
    }
}

impl SourceCatalogMetadata for LocalSource {
    fn catalog_entries(&self) -> Result<Vec<SourceCatalogEntry>, ResourceError> {
        Ok(vec![SourceCatalogEntry::new(
            "local://",
            "local://<name>[:selector] (flat names; bare local:// lists this Path Session's scratch)",
            "local://plan.md",
            None,
        )?])
    }
}

#[async_trait]
impl SourceAdapter for LocalSource {
    async fn read(
        &self,
        reference: &PathReference,
        _operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        match reference.address() {
            ResourceAddress::Local(LocalAddress::Root) => self.read_root().await,
            ResourceAddress::Local(LocalAddress::Named(_)) => self.read_named(reference).await,
            _ => Err(unsupported_local_target()),
        }
    }
}

#[async_trait]
impl MutationAdapter for LocalSource {
    async fn resolve(
        &self,
        reference: &PathReference,
        _access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError> {
        // Path Session authority alone authorizes scratch mutation: no grant is
        // read here, and none exists for this family.
        match reference.address() {
            ResourceAddress::Local(LocalAddress::Named(name)) => MutationTarget::new(
                PathReference::local(name.as_str())?,
                MutationSourceKey::new(LOCAL_MUTATION_SOURCE_KEY)?,
                MutationTargetMode::AuthoredText,
            ),
            ResourceAddress::Local(LocalAddress::Root) => Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "the Session Scratch listing is a read-only projection; mutate one local://<name> Resource",
            )),
            _ => Err(unsupported_local_target()),
        }
    }

    async fn load(
        &self,
        target: &MutationTarget,
        _access: MutationAccess,
        operation: &OperationGuard,
    ) -> Result<MutationState, ResourceError> {
        ensure_live(&self.session, operation)?;
        let name = target_name(target)?;
        self.scratch_state(name).await
    }

    async fn commit(
        &self,
        mutation: SourceMutation,
        operation: &OperationGuard,
    ) -> Result<MutationCommitOutcome, MutationCommitFailure> {
        let committed: Result<(), ResourceError> = async {
            match mutation {
                SourceMutation::Create { target, content } => {
                    let name = target_name(&target)?;
                    if self.session.scratch_load(name).await?.is_some() {
                        return Err(ResourceError::new(
                            ErrorCategory::VersionConflict,
                            format!(
                                "Session Scratch Resource '{name}' already exists; supply ifVersion to replace it"
                            ),
                        ));
                    }
                    self.session.scratch_put(name, &content, operation).await?;
                    Ok(())
                }
                SourceMutation::Replace {
                    target,
                    expected,
                    content,
                } => {
                    let name = target_name(&target)?;
                    expect_current(self, name, &expected).await?;
                    self.session.scratch_put(name, &content, operation).await?;
                    Ok(())
                }
                SourceMutation::Delete { target, expected } => {
                    let name = target_name(&target)?;
                    expect_current(self, name, &expected).await?;
                    self.session.scratch_remove(name).await
                }
                SourceMutation::Move {
                    source,
                    destination,
                    expected,
                } => {
                    let from = target_name(&source)?;
                    let to = target_name(&destination)?;
                    expect_current(self, from, &expected).await?;
                    self.session.scratch_rename(from, to).await
                }
            }
        }
        .await;
        committed
            .map(|()| MutationCommitOutcome::AuthoredText)
            .map_err(Into::into)
    }
}

#[async_trait]
impl DiscoveryAdapter for LocalSource {
    async fn search(
        &self,
        target: &SearchTarget,
        pattern: &str,
        options: SearchOptions,
        operation: &OperationGuard,
    ) -> Result<SearchSourceResult, ResourceError> {
        let reference = target.reference().ok_or_else(unsupported_local_target)?;
        let names = match reference.address() {
            // The family root searches every scratch Resource in this session.
            ResourceAddress::Local(LocalAddress::Root) => self.session.scratch_names().await?,
            ResourceAddress::Local(LocalAddress::Named(name)) => vec![name.clone()],
            _ => return Err(unsupported_local_target()),
        };
        ensure_live(&self.session, operation)?;

        // Load every subject before matching: a compiled PCRE2 matcher is not
        // `Send`, so it may not be held across an await. Same structure as the
        // Artifact adapter's search.
        let mut subjects = Vec::with_capacity(names.len());
        for name in names {
            ensure_live(&self.session, operation)?;
            if let Some(state) = self.session.scratch_load(&name).await? {
                subjects.push((name, state.content));
            }
        }
        ensure_live(&self.session, operation)?;

        let pattern = pattern.to_owned();
        let case_sensitive = options.case_sensitive();
        let session = self.session.clone();
        let operation = operation.clone();
        tokio::task::spawn_blocking(move || {
            search_scratch_subjects(subjects, &pattern, case_sensitive, &session, &operation)
        })
        .await
        .map_err(discovery_worker_error)?
    }

    async fn glob(
        &self,
        target: &GlobTarget,
        options: GlobOptions,
        operation: &OperationGuard,
    ) -> Result<SourceGlobResult, ResourceError> {
        ensure_live(&self.session, operation)?;
        let names = self.session.scratch_names().await?;
        let matcher = GlobMatcher::compile(target.pattern(), options.case_sensitive())?;
        let mut entries = Vec::new();
        for name in names {
            ensure_live(&self.session, operation)?;
            let reference = PathReference::local(name.as_str())?;
            if matcher.is_match(reference.requested()) {
                entries.push(GlobEntry::new(reference, GlobKind::LocalScratch)?);
            }
        }
        ensure_live(&self.session, operation)?;
        Ok(SourceGlobResult::new(entries, Vec::new()))
    }
}

fn search_scratch_subjects(
    subjects: Vec<(LocalName, String)>,
    pattern: &str,
    case_sensitive: bool,
    session: &PathSession,
    operation: &OperationGuard,
) -> Result<SearchSourceResult, ResourceError> {
    ensure_live(session, operation)?;
    let mut matcher = SearchMatcher::compile(pattern, case_sensitive)?;
    let engine = matcher.engine();
    let mut records = Vec::new();
    for (name, content) in subjects {
        ensure_live(session, operation)?;
        let canonical = PathReference::local(name.as_str())?;
        for (index, line) in content.lines().enumerate() {
            ensure_live(session, operation)?;
            if matcher.is_match(line)? {
                let line_number = u64::try_from(index)
                    .ok()
                    .and_then(|index| index.checked_add(1))
                    .ok_or_else(search_line_overflow)?;
                records.push(SearchRecord::new(canonical.clone(), line_number, line)?);
            }
        }
    }
    ensure_live(session, operation)?;
    Ok(SearchSourceResult::new(engine, records, Vec::new()))
}

fn discovery_worker_error(error: tokio::task::JoinError) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!("Session Scratch discovery worker failed: {error}"),
    )
}

async fn expect_current(
    source: &LocalSource,
    name: &LocalName,
    expected: &VersionTag,
) -> Result<(), ResourceError> {
    let current = source
        .session
        .scratch_load(name)
        .await?
        .ok_or_else(|| scratch_not_found(name))?;
    if &current.version_tag != expected {
        return Err(ResourceError::new(
            ErrorCategory::VersionConflict,
            format!("Session Scratch Resource '{name}' changed since it was read"),
        ));
    }
    Ok(())
}

fn target_name(target: &MutationTarget) -> Result<&LocalName, ResourceError> {
    match target.canonical_reference().address() {
        ResourceAddress::Local(LocalAddress::Named(name)) => Ok(name),
        _ => Err(unsupported_local_target()),
    }
}

fn ensure_live(session: &PathSession, operation: &OperationGuard) -> Result<(), ResourceError> {
    if !operation.is_active() {
        return Err(ResourceError::new(
            ErrorCategory::Cancelled,
            "Session Scratch operation was cancelled",
        ));
    }
    if !session.is_active() {
        return Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "Path Session disconnected during a Session Scratch operation",
        ));
    }
    Ok(())
}

fn search_line_overflow() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        "Session Scratch search line number is not representable",
    )
}

fn scratch_not_found(name: &LocalName) -> ResourceError {
    ResourceError::new(
        ErrorCategory::NotFound,
        format!("Session Scratch Resource '{name}' does not exist in this Path Session"),
    )
}

fn unsupported_local_target() -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        "Local Source Adapter requires one Session Scratch Resource",
    )
}
