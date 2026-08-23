use std::{fmt, io::Cursor};

use async_trait::async_trait;
use resourcefs_core::{
    ArtifactAddress, ArtifactProjectionOrigin, DiscoveryAdapter, ErrorCategory, GlobEntry,
    GlobKind, GlobOptions, GlobTarget, LineSelector, OperationGuard, PathReference, PathSession,
    ResourceAddress, ResourceError, SearchOptions, SearchRecord, SearchSourceResult, SearchTarget,
    SourceAdapter, SourceGlobResult, SourceResource, VersionTag, select_utf8,
};

use crate::{
    catalog::{SourceCatalogEntry, SourceCatalogMetadata},
    pattern::{GlobMatcher, SearchMatcher},
};

/// Read-only adapter for immutable Path Session artifacts.
///
/// Artifact mutation is intentionally absent:
/// ```compile_fail
/// use resourcefs_core::{PathReference, ResourceError};
/// use resourcefs_sources::ArtifactSource;
///
/// async fn forbidden_mutation(
///     source: &ArtifactSource,
///     reference: &PathReference,
/// ) -> Result<(), ResourceError> {
///     source.write(reference, "changed").await
/// }
/// ```
#[derive(Clone)]
pub struct ArtifactSource {
    session: PathSession,
}

#[derive(Debug)]
struct SelectedArtifact {
    canonical: PathReference,
    content: String,
    version_tag: VersionTag,
    origin: Option<ArtifactProjectionOrigin>,
}

impl ArtifactSource {
    pub fn new(session: PathSession) -> Self {
        Self { session }
    }

    async fn select(&self, reference: &PathReference) -> Result<SelectedArtifact, ResourceError> {
        let ResourceAddress::Artifact(address) = reference.address() else {
            return Err(unsupported_artifact_target());
        };
        let canonical = PathReference::artifact(address.clone(), None)?;
        let mut content = self.session.read_artifact(address).await?;
        let projection = reference.projection();

        let (content, version_tag, origin) = if let Some(offset) =
            projection.and_then(resourcefs_core::ProjectionSelector::page_offset)
        {
            let offset = usize::try_from(offset).map_err(|_| invalid_page_offset())?;
            if offset >= content.len() || !content.is_char_boundary(offset) {
                return Err(invalid_page_offset());
            }
            let line_number = line_number_at_offset(&content, offset);
            let version_tag = VersionTag::from_content(content.as_bytes());
            content.drain(..offset);
            (
                content,
                version_tag,
                Some(ArtifactProjectionOrigin::new(offset, line_number)),
            )
        } else if let Some(lines) = projection.and_then(|selector| selector.line_selection()) {
            let origin = contiguous_suffix_origin(&content, lines);
            let selected = select_utf8(Cursor::new(content), projection)?;
            let (content, version_tag, _) = selected.into_parts();
            (content, version_tag, origin)
        } else {
            let version_tag = VersionTag::from_content(content.as_bytes());
            (
                content,
                version_tag,
                Some(ArtifactProjectionOrigin::new(0, 1)),
            )
        };

        Ok(SelectedArtifact {
            canonical,
            content,
            version_tag,
            origin,
        })
    }
}

impl fmt::Debug for ArtifactSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactSource")
            .finish_non_exhaustive()
    }
}

impl SourceCatalogMetadata for ArtifactSource {
    fn catalog_entries(&self) -> Result<Vec<SourceCatalogEntry>, ResourceError> {
        Ok(vec![SourceCatalogEntry::new(
            "artifact://",
            "artifact://<session>-<id>[:selector]",
            "artifact://00000000000000000000000000000000-1",
            None,
        )?])
    }
}

#[async_trait]
impl SourceAdapter for ArtifactSource {
    async fn read(
        &self,
        reference: &PathReference,
        _operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        let selected = self.select(reference).await?;
        let resource = SourceResource::text_projection(
            selected.canonical,
            selected.content,
            selected.version_tag,
        )?;
        match selected.origin {
            Some(origin) => resource.with_artifact_origin(origin),
            None => Ok(resource),
        }
    }
}

#[async_trait]
impl DiscoveryAdapter for ArtifactSource {
    async fn search(
        &self,
        target: &SearchTarget,
        pattern: &str,
        options: SearchOptions,
        operation: &OperationGuard,
    ) -> Result<SearchSourceResult, ResourceError> {
        let reference = target.reference().ok_or_else(unsupported_artifact_target)?;
        if !matches!(reference.address(), ResourceAddress::Artifact(_)) {
            return Err(unsupported_artifact_target());
        }
        ensure_discovery_live(&self.session, operation)?;
        let selected = self.select(reference).await?;
        ensure_discovery_live(&self.session, operation)?;

        let pattern = pattern.to_owned();
        let case_sensitive = options.case_sensitive();
        let session = self.session.clone();
        let operation = operation.clone();
        tokio::task::spawn_blocking(move || {
            search_selected_artifact(selected, &pattern, case_sensitive, &session, &operation)
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
        ensure_discovery_live(&self.session, operation)?;
        let catalog = self.session.artifact_catalog().await?;
        ensure_discovery_live(&self.session, operation)?;

        let pattern = target.pattern().to_owned();
        let case_sensitive = options.case_sensitive();
        let session = self.session.clone();
        let operation = operation.clone();
        tokio::task::spawn_blocking(move || {
            glob_artifact_catalog(catalog, &pattern, case_sensitive, &session, &operation)
        })
        .await
        .map_err(discovery_worker_error)?
    }
}

fn search_selected_artifact(
    selected: SelectedArtifact,
    pattern: &str,
    case_sensitive: bool,
    session: &PathSession,
    operation: &OperationGuard,
) -> Result<SearchSourceResult, ResourceError> {
    ensure_discovery_live(session, operation)?;
    let mut matcher = SearchMatcher::compile(pattern, case_sensitive)?;
    let engine = matcher.engine();
    let SelectedArtifact {
        canonical,
        content,
        origin,
        ..
    } = selected;
    let first_line = origin.map_or(1, ArtifactProjectionOrigin::line_number);
    let mut records = Vec::new();
    for (index, line) in content.lines().enumerate() {
        ensure_discovery_live(session, operation)?;
        if matcher.is_match(line)? {
            let index = u64::try_from(index).map_err(|_| search_line_overflow())?;
            let line_number = first_line
                .checked_add(index)
                .ok_or_else(search_line_overflow)?;
            records.push(SearchRecord::new(canonical.clone(), line_number, line)?);
        }
    }
    ensure_discovery_live(session, operation)?;
    Ok(SearchSourceResult::new(engine, records, Vec::new()))
}

fn glob_artifact_catalog(
    catalog: Vec<ArtifactAddress>,
    pattern: &str,
    case_sensitive: bool,
    session: &PathSession,
    operation: &OperationGuard,
) -> Result<SourceGlobResult, ResourceError> {
    ensure_discovery_live(session, operation)?;
    let matcher = GlobMatcher::compile(pattern, case_sensitive)?;
    let mut entries = Vec::new();
    for address in catalog {
        ensure_discovery_live(session, operation)?;
        let reference = PathReference::artifact(address, None)?;
        if matcher.is_match(reference.requested()) {
            entries.push(GlobEntry::new(reference, GlobKind::Artifact)?);
        }
    }
    ensure_discovery_live(session, operation)?;
    Ok(SourceGlobResult::new(entries, Vec::new()))
}

fn ensure_discovery_live(
    session: &PathSession,
    operation: &OperationGuard,
) -> Result<(), ResourceError> {
    if !operation.is_active() {
        return Err(ResourceError::new(
            ErrorCategory::Cancelled,
            "Artifact discovery operation was cancelled",
        ));
    }
    if !session.is_active() {
        return Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "Path Session disconnected during Artifact discovery",
        ));
    }
    Ok(())
}

fn unsupported_artifact_target() -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        "Artifact Source Adapter requires one Artifact Resource",
    )
}

fn search_line_overflow() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        "Artifact search line number is not representable",
    )
}

fn discovery_worker_error(error: tokio::task::JoinError) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!("Artifact discovery worker failed: {error}"),
    )
}

fn contiguous_suffix_origin(
    content: &str,
    selection: &LineSelector,
) -> Option<ArtifactProjectionOrigin> {
    let [range] = selection.ranges() else {
        return None;
    };
    if range.inclusive_end().is_some() {
        return None;
    }
    line_start_offset(content, range.start())
        .map(|offset| ArtifactProjectionOrigin::new(offset, range.start()))
}

fn line_start_offset(content: &str, line_number: u64) -> Option<usize> {
    if line_number == 1 {
        return (!content.is_empty()).then_some(0);
    }
    let mut current = 1_u64;
    for (offset, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            current += 1;
            if current == line_number && offset + 1 < content.len() {
                return Some(offset + 1);
            }
        }
    }
    None
}

fn line_number_at_offset(content: &str, offset: usize) -> u64 {
    content.as_bytes()[..offset]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count() as u64
        + 1
}

fn invalid_page_offset() -> ResourceError {
    ResourceError::new(
        ErrorCategory::InvalidReference,
        "artifact page offset must name a UTF-8 boundary before end of content",
    )
}
