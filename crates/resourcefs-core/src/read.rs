use std::{fmt, sync::Arc};

use crate::{
    ArtifactAddress, ArtifactProjectionOrigin, ErrorCategory, MAX_ARTIFACT_BYTES, MAX_TEXT_BYTES,
    MAX_TEXT_COLUMNS, MAX_TEXT_LINES, OperationGuard, PathReference, PathSession,
    ProjectionSelector, ReadResource, ResourceAddress, ResourceError, SourceAdapter,
};

/// Valid lower-only ceilings for one inline text page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextLimits {
    bytes: usize,
    lines: usize,
    columns: usize,
}

impl TextLimits {
    pub fn new(
        bytes: Option<usize>,
        lines: Option<usize>,
        columns: Option<usize>,
    ) -> Result<Self, ResourceError> {
        Ok(Self {
            bytes: validate_limit("bytes", bytes.unwrap_or(MAX_TEXT_BYTES), MAX_TEXT_BYTES)?,
            lines: validate_limit("lines", lines.unwrap_or(MAX_TEXT_LINES), MAX_TEXT_LINES)?,
            columns: validate_limit(
                "columns",
                columns.unwrap_or(MAX_TEXT_COLUMNS),
                MAX_TEXT_COLUMNS,
            )?,
        })
    }

    pub const fn bytes(self) -> usize {
        self.bytes
    }

    pub const fn lines(self) -> usize {
        self.lines
    }

    pub const fn columns(self) -> usize {
        self.columns
    }
}

impl Default for TextLimits {
    fn default() -> Self {
        Self {
            bytes: MAX_TEXT_BYTES,
            lines: MAX_TEXT_LINES,
            columns: MAX_TEXT_COLUMNS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReadRequest {
    pub reference: PathReference,
    pub limits: TextLimits,
}

#[derive(Clone)]
pub struct ReadEngine {
    sources: Arc<dyn SourceAdapter>,
    session: PathSession,
}

impl fmt::Debug for ReadEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("ReadEngine").finish_non_exhaustive()
    }
}

impl ReadEngine {
    pub fn new(sources: Arc<dyn SourceAdapter>, session: PathSession) -> Self {
        Self { sources, session }
    }

    pub async fn read(
        &self,
        request: ReadRequest,
        operation: &OperationGuard,
    ) -> Result<ReadResource, ResourceError> {
        ensure_live(&self.session, operation)?;
        let source = self.sources.read(&request.reference).await?;
        ensure_live(&self.session, operation)?;

        let requested_artifact = match request.reference.address() {
            ResourceAddress::Artifact(address) => Some(address.clone()),
            ResourceAddress::Workspace(_) => None,
        };
        let requested_artifact_projection = request.reference.projection().is_some();
        let mut parts = source.into_parts();
        if parts.content.len() > MAX_ARTIFACT_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "Source Adapter selected projection exceeds the {MAX_ARTIFACT_BYTES}-byte artifact ceiling; narrow the selector"
                ),
            ));
        }

        let page_end = page_prefix_len(&parts.content, request.limits)?;
        if page_end == parts.content.len() {
            let recovery_reference = if requested_artifact_projection {
                requested_artifact
                    .as_ref()
                    .map(canonical_artifact_reference)
                    .transpose()?
            } else {
                None
            };
            ensure_live(&self.session, operation)?;
            return ReadResource::from_parts(parts, false, recovery_reference, None);
        }

        let (recovery_address, origin) = match (requested_artifact, parts.artifact_origin) {
            (Some(address), Some(origin)) => (address, origin),
            _ => {
                let address = self.session.retain(&parts.content, operation).await?;
                (address, ArtifactProjectionOrigin::new(0, 1))
            }
        };
        let recovery_reference = canonical_artifact_reference(&recovery_address)?;
        let absolute_end = origin
            .byte_offset()
            .checked_add(page_end)
            .ok_or_else(continuation_overflow)?;
        let next_line = origin
            .line_number()
            .checked_add(count_line_terminators(&parts.content[..page_end]))
            .ok_or_else(continuation_overflow)?;
        let continuation_reference = if parts.content[..page_end].ends_with('\n') {
            artifact_line_continuation(&recovery_address, next_line)?
        } else {
            artifact_page_continuation(&recovery_address, absolute_end)?
        };

        parts.content.truncate(page_end);
        ensure_live(&self.session, operation)?;
        ReadResource::from_parts(
            parts,
            true,
            Some(recovery_reference),
            Some(continuation_reference),
        )
    }
}

fn validate_limit(name: &str, value: usize, maximum: usize) -> Result<usize, ResourceError> {
    if value == 0 || value > maximum {
        return Err(ResourceError::new(
            ErrorCategory::LimitExceeded,
            format!("text {name} limit must be between 1 and {maximum}"),
        ));
    }
    Ok(value)
}

fn page_prefix_len(content: &str, limits: TextLimits) -> Result<usize, ResourceError> {
    if content.is_empty() {
        return Ok(0);
    }

    let bytes = content.as_bytes();
    let mut offset = 0_usize;
    let mut lines = 0_usize;
    let mut columns = 0_usize;
    let mut at_line_start = true;
    while offset < bytes.len() {
        if at_line_start {
            if lines == limits.lines {
                break;
            }
            lines += 1;
            at_line_start = false;
        }

        let remaining = &content[offset..];
        let (unit_bytes, is_terminator) = if remaining.starts_with("\r\n") {
            (2, true)
        } else if remaining.starts_with('\n') {
            (1, true)
        } else {
            (
                remaining
                    .chars()
                    .next()
                    .expect("non-empty UTF-8 remainder")
                    .len_utf8(),
                false,
            )
        };
        if offset + unit_bytes > limits.bytes {
            break;
        }
        if !is_terminator && columns == limits.columns {
            break;
        }

        offset += unit_bytes;
        if is_terminator {
            columns = 0;
            at_line_start = true;
        } else {
            columns += 1;
        }
    }

    if offset == 0 {
        return Err(ResourceError::new(
            ErrorCategory::LimitExceeded,
            "text byte limit is too small for the next complete UTF-8 scalar or line terminator",
        ));
    }
    Ok(offset)
}

fn canonical_artifact_reference(address: &ArtifactAddress) -> Result<String, ResourceError> {
    PathReference::artifact(address.clone(), None).map(|reference| reference.requested().to_owned())
}

fn artifact_line_continuation(
    address: &ArtifactAddress,
    next_line: u64,
) -> Result<String, ResourceError> {
    let selector = ProjectionSelector::parse(format!("{next_line}-"))?;
    PathReference::artifact(address.clone(), Some(selector))
        .map(|reference| reference.requested().to_owned())
}

fn artifact_page_continuation(
    address: &ArtifactAddress,
    offset: usize,
) -> Result<String, ResourceError> {
    let selector = ProjectionSelector::parse(format!("page:{offset}"))?;
    PathReference::artifact(address.clone(), Some(selector))
        .map(|reference| reference.requested().to_owned())
}

fn count_line_terminators(content: &str) -> u64 {
    content.bytes().filter(|byte| *byte == b'\n').count() as u64
}

fn ensure_live(session: &PathSession, operation: &OperationGuard) -> Result<(), ResourceError> {
    if !operation.is_active() || !session.is_active() {
        return Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "Path Session is no longer active",
        ));
    }
    Ok(())
}

fn continuation_overflow() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        "artifact continuation offset is not representable",
    )
}
