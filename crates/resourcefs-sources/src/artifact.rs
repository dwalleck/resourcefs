use std::{fmt, io::Cursor};

use async_trait::async_trait;
use resourcefs_core::{
    ArtifactProjectionOrigin, ErrorCategory, LineSelector, PathReference, PathSession,
    ResourceAddress, ResourceError, SourceAdapter, SourceResource, VersionTag, select_utf8,
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

impl ArtifactSource {
    pub fn new(session: PathSession) -> Self {
        Self { session }
    }
}

impl fmt::Debug for ArtifactSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactSource")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SourceAdapter for ArtifactSource {
    async fn read(&self, reference: &PathReference) -> Result<SourceResource, ResourceError> {
        let ResourceAddress::Artifact(address) = reference.address() else {
            return Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "Artifact Source Adapter cannot read non-artifact Resources",
            ));
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

        let resource = SourceResource::text_projection(canonical, content, version_tag)?;
        match origin {
            Some(origin) => resource.with_artifact_origin(origin),
            None => Ok(resource),
        }
    }
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
