use url::Url;

use crate::{
    ArtifactAddress, ErrorCategory, PathReference, ResourceAddress, ResourceError, VersionTag,
    WorkspaceAddress,
};

pub const BEHAVIOR_CONTRACT_VERSION: &str = "1.0.0";
pub const MAX_TEXT_BYTES: usize = 48 * 1024;
pub const MAX_TEXT_LINES: usize = 3_000;
pub const MAX_TEXT_COLUMNS: usize = 512;
pub const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
pub const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// Position of a contiguous artifact projection in its immutable root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactProjectionOrigin {
    byte_offset: usize,
    line_number: u64,
}

impl ArtifactProjectionOrigin {
    pub const fn new(byte_offset: usize, line_number: u64) -> Self {
        Self {
            byte_offset,
            line_number,
        }
    }

    pub const fn byte_offset(self) -> usize {
        self.byte_offset
    }

    pub const fn line_number(self) -> u64 {
        self.line_number
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayedLineRange {
    start_line: u64,
    end_line: u64,
}

impl DisplayedLineRange {
    pub fn new(start_line: u64, end_line: u64) -> Result<Self, ResourceError> {
        if start_line == 0 || end_line < start_line {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "displayed line ranges must be positive and ascending",
            ));
        }
        Ok(Self {
            start_line,
            end_line,
        })
    }

    pub const fn start_line(self) -> u64 {
        self.start_line
    }

    pub const fn end_line(self) -> u64 {
        self.end_line
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectionSpan {
    output_start: usize,
    output_end: usize,
    start_line: u64,
    end_line: u64,
    reaches_eof: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionMetadata {
    spans: Vec<ProjectionSpan>,
    source_eof: bool,
}

impl ProjectionMetadata {
    fn complete(content: &str) -> Self {
        let lines = line_count(content);
        Self {
            spans: (lines != 0)
                .then_some(ProjectionSpan {
                    output_start: 0,
                    output_end: content.len(),
                    start_line: 1,
                    end_line: lines,
                    reaches_eof: true,
                })
                .into_iter()
                .collect(),
            source_eof: true,
        }
    }

    fn displayed_prefix(
        &self,
        content: &str,
        prefix: usize,
    ) -> (Vec<DisplayedLineRange>, Vec<u64>, bool) {
        let mut ranges = Vec::new();
        let mut line_numbers = Vec::new();
        for span in &self.spans {
            if span.output_start >= prefix {
                break;
            }
            let visible_end = span.output_end.min(prefix);
            let visible = &content[span.output_start..visible_end];
            if visible.is_empty() {
                continue;
            }
            let terminators = visible.bytes().filter(|byte| *byte == b'\n').count() as u64;
            let rendered_lines = terminators + u64::from(!visible.ends_with('\n'));
            line_numbers.extend(span.start_line..span.start_line + rendered_lines);

            let complete_lines = terminators
                + u64::from(
                    visible_end == span.output_end && span.reaches_eof && !visible.ends_with('\n'),
                );
            if complete_lines != 0 {
                push_merged_range(
                    &mut ranges,
                    DisplayedLineRange {
                        start_line: span.start_line,
                        end_line: span.start_line + complete_lines - 1,
                    },
                );
            }
        }
        let displayed_eof = self.source_eof
            && self
                .spans
                .last()
                .is_none_or(|span| prefix >= span.output_end);
        (ranges, line_numbers, displayed_eof)
    }
}

fn push_merged_range(ranges: &mut Vec<DisplayedLineRange>, incoming: DisplayedLineRange) {
    if let Some(last) = ranges.last_mut()
        && last.end_line.checked_add(1) == Some(incoming.start_line)
    {
        last.end_line = incoming.end_line;
        return;
    }
    ranges.push(incoming);
}

fn line_count(content: &str) -> u64 {
    content.bytes().filter(|byte| *byte == b'\n').count() as u64
        + u64::from(!content.is_empty() && !content.ends_with('\n'))
}

/// Complete authoritative UTF-8 projection returned by a Source Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceResource {
    canonical_reference: String,
    content_type: &'static str,
    version_tag: VersionTag,
    mutable: bool,
    backing_file_uri: Option<String>,
    artifact_identity: bool,
    artifact_origin: Option<ArtifactProjectionOrigin>,
    content: String,
    projection: ProjectionMetadata,
}

impl SourceResource {
    pub fn text(reference: PathReference, content: String) -> Result<Self, ResourceError> {
        let version_tag = VersionTag::from_content(content.as_bytes());
        Self::text_projection(reference, content, version_tag)
    }

    /// Build a selected projection carrying the authoritative whole-Resource Version Tag.
    pub fn text_projection(
        reference: PathReference,
        content: String,
        version_tag: VersionTag,
    ) -> Result<Self, ResourceError> {
        validate_canonical_identity(&reference)?;
        let artifact_identity = matches!(reference.address(), ResourceAddress::Artifact(_));
        let projection = ProjectionMetadata::complete(&content);
        Ok(Self {
            canonical_reference: reference.requested().to_owned(),
            content_type: TEXT_CONTENT_TYPE,
            version_tag,
            mutable: false,
            backing_file_uri: None,
            artifact_identity,
            artifact_origin: None,
            content,
            projection,
        })
    }

    pub fn selected_text(
        reference: PathReference,
        selected: crate::selector::SelectedText,
    ) -> Result<Self, ResourceError> {
        validate_canonical_identity(&reference)?;
        let artifact_identity = matches!(reference.address(), ResourceAddress::Artifact(_));
        let (content, version_tag, spans, source_eof) = selected.into_projected_parts();
        let projection = ProjectionMetadata {
            spans: spans
                .into_iter()
                .map(|span| ProjectionSpan {
                    output_start: span.output_start,
                    output_end: span.output_end,
                    start_line: span.start_line,
                    end_line: span.end_line,
                    reaches_eof: span.reaches_eof,
                })
                .collect(),
            source_eof,
        };
        Ok(Self {
            canonical_reference: reference.requested().to_owned(),
            content_type: TEXT_CONTENT_TYPE,
            version_tag,
            mutable: false,
            backing_file_uri: None,
            artifact_identity,
            artifact_origin: None,
            content,
            projection,
        })
    }

    pub fn with_artifact_origin(
        mut self,
        origin: ArtifactProjectionOrigin,
    ) -> Result<Self, ResourceError> {
        if !self.artifact_identity {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "only an artifact projection can carry an artifact origin",
            ));
        }
        self.artifact_origin = Some(origin);
        Ok(self)
    }

    pub fn with_mutability(mut self, mutable: bool) -> Self {
        self.mutable = mutable;
        self
    }

    pub fn with_backing_file_uri(
        mut self,
        backing_file_uri: impl Into<String>,
    ) -> Result<Self, ResourceError> {
        self.backing_file_uri = Some(validate_backing_file_uri(backing_file_uri.into())?);
        Ok(self)
    }

    pub fn canonical_reference(&self) -> &str {
        &self.canonical_reference
    }

    pub const fn content_type(&self) -> &'static str {
        self.content_type
    }

    pub const fn version_tag(&self) -> &VersionTag {
        &self.version_tag
    }

    pub const fn is_mutable(&self) -> bool {
        self.mutable
    }

    pub fn backing_file_uri(&self) -> Option<&str> {
        self.backing_file_uri.as_deref()
    }

    pub const fn artifact_origin(&self) -> Option<ArtifactProjectionOrigin> {
        self.artifact_origin
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub(crate) fn into_parts(self) -> SourceResourceParts {
        SourceResourceParts {
            canonical_reference: self.canonical_reference,
            content_type: self.content_type,
            version_tag: self.version_tag,
            mutable: self.mutable,
            backing_file_uri: self.backing_file_uri,
            artifact_origin: self.artifact_origin,
            content: self.content,
            projection: self.projection,
        }
    }
}

pub(crate) struct SourceResourceParts {
    pub canonical_reference: String,
    pub content_type: &'static str,
    pub version_tag: VersionTag,
    pub mutable: bool,
    pub backing_file_uri: Option<String>,
    pub artifact_origin: Option<ArtifactProjectionOrigin>,
    pub content: String,
    pub projection: ProjectionMetadata,
}

impl SourceResourceParts {
    pub(crate) fn displayed_prefix(
        &self,
        prefix: usize,
    ) -> (Vec<DisplayedLineRange>, Vec<u64>, bool) {
        self.projection.displayed_prefix(&self.content, prefix)
    }
}

/// Bounded source-neutral result returned to an MCP renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadResource {
    canonical_reference: String,
    content_type: &'static str,
    version_tag: VersionTag,
    mutable: bool,
    bounded: bool,
    backing_file_uri: Option<String>,
    recovery_reference: Option<String>,
    continuation_reference: Option<String>,
    content: String,
    displayed_ranges: Vec<DisplayedLineRange>,
    displayed_eof: bool,
    display_line_numbers: Vec<u64>,
    numbered: bool,
}

impl ReadResource {
    pub fn text(reference: PathReference, content: String) -> Result<Self, ResourceError> {
        Self::from_source(SourceResource::text(reference, content)?)
    }

    pub fn text_projection(
        reference: PathReference,
        content: String,
        version_tag: VersionTag,
    ) -> Result<Self, ResourceError> {
        Self::from_source(SourceResource::text_projection(
            reference,
            content,
            version_tag,
        )?)
    }

    fn from_source(source: SourceResource) -> Result<Self, ResourceError> {
        let content_len = source.content().len();
        Self::from_parts(source.into_parts(), false, None, None, content_len, false)
    }

    pub(crate) fn from_parts(
        source: SourceResourceParts,
        bounded: bool,
        recovery_reference: Option<String>,
        continuation_reference: Option<String>,
        page_end: usize,
        numbered: bool,
    ) -> Result<Self, ResourceError> {
        if bounded != continuation_reference.is_some() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "bounded reads must carry exactly one progressing continuation",
            ));
        }
        let (displayed_ranges, display_line_numbers, displayed_eof) = source
            .projection
            .displayed_prefix(&source.content, page_end);
        Ok(Self {
            canonical_reference: source.canonical_reference,
            content_type: source.content_type,
            version_tag: source.version_tag,
            mutable: source.mutable,
            bounded,
            backing_file_uri: source.backing_file_uri,
            recovery_reference,
            continuation_reference,
            content: source.content,
            displayed_ranges,
            displayed_eof,
            display_line_numbers,
            numbered,
        })
    }

    pub fn canonical_reference(&self) -> &str {
        &self.canonical_reference
    }

    pub const fn content_type(&self) -> &'static str {
        self.content_type
    }

    pub const fn version_tag(&self) -> &VersionTag {
        &self.version_tag
    }

    pub const fn is_mutable(&self) -> bool {
        self.mutable
    }

    pub const fn is_bounded(&self) -> bool {
        self.bounded
    }

    pub fn displayed_ranges(&self) -> &[DisplayedLineRange] {
        &self.displayed_ranges
    }

    pub const fn displayed_eof(&self) -> bool {
        self.displayed_eof
    }

    pub fn display_line_numbers(&self) -> &[u64] {
        &self.display_line_numbers
    }

    pub const fn is_numbered(&self) -> bool {
        self.numbered
    }

    pub fn with_backing_file_uri(
        mut self,
        backing_file_uri: impl Into<String>,
    ) -> Result<Self, ResourceError> {
        self.backing_file_uri = Some(validate_backing_file_uri(backing_file_uri.into())?);
        Ok(self)
    }

    pub fn backing_file_uri(&self) -> Option<&str> {
        self.backing_file_uri.as_deref()
    }

    pub fn recovery_reference(&self) -> Option<&str> {
        self.recovery_reference.as_deref()
    }

    pub fn continuation_reference(&self) -> Option<&str> {
        self.continuation_reference.as_deref()
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    #[cfg(feature = "test-support")]
    pub fn content_capacity_for_test(&self) -> usize {
        self.content.capacity()
    }
}

fn validate_canonical_identity(reference: &PathReference) -> Result<(), ResourceError> {
    let canonical = match reference.address() {
        ResourceAddress::Catalog(address) => {
            reference.projection().is_none()
                && reference.requested() == address.canonical_reference()
        }
        ResourceAddress::Workspace(WorkspaceAddress::Canonical { .. }) => true,
        ResourceAddress::Artifact(address) => {
            reference.projection().is_none() && canonical_artifact_reference(reference, address)
        }
        ResourceAddress::Local(name) => {
            reference.projection().is_none() && reference.requested() == name.canonical_reference()
        }
        ResourceAddress::Workspace(_) => false,
    };
    if !canonical {
        return Err(ResourceError::new(
            ErrorCategory::InvalidReference,
            "SourceResource identity must be a canonical Resource reference",
        ));
    }
    Ok(())
}

fn canonical_artifact_reference(reference: &PathReference, address: &ArtifactAddress) -> bool {
    PathReference::artifact(address.clone(), None)
        .is_ok_and(|canonical| canonical.requested() == reference.requested())
}

fn validate_backing_file_uri(backing_file_uri: String) -> Result<String, ResourceError> {
    let parsed = Url::parse(&backing_file_uri).map_err(|_| invalid_backing_uri())?;
    if parsed.scheme() != "file"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.to_file_path().is_err()
    {
        return Err(invalid_backing_uri());
    }
    Ok(parsed.into())
}

fn invalid_backing_uri() -> ResourceError {
    ResourceError::new(
        ErrorCategory::InvalidReference,
        "backing-file metadata must be a valid local file URI",
    )
}
