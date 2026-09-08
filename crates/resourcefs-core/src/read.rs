use std::{fmt, sync::Arc};

use crate::{
    ArtifactAddress, ArtifactProjectionOrigin, ErrorCategory, LocalAddress, MAX_ARTIFACT_BYTES,
    MAX_TEXT_BYTES, MAX_TEXT_COLUMNS, MAX_TEXT_LINES, OperationGuard, PathReference, PathSession,
    ProjectionSelector, ReadResource, ResourceAddress, ResourceError, ServerLimits, SourceAdapter,
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
    pub(crate) const fn from_validated(bytes: usize, lines: usize, columns: usize) -> Self {
        Self {
            bytes,
            lines,
            columns,
        }
    }

    fn lowered_by(self, base: Self) -> Self {
        Self {
            bytes: self.bytes.min(base.bytes),
            lines: self.lines.min(base.lines),
            columns: self.columns.min(base.columns),
        }
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
    pub numbered: bool,
    pub acquisition: Option<crate::ReadAcquisitionLimits>,
}

#[derive(Clone)]
pub struct ReadEngine {
    sources: Arc<dyn SourceAdapter>,
    session: PathSession,
    limits: ServerLimits,
}

impl fmt::Debug for ReadEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("ReadEngine").finish_non_exhaustive()
    }
}

impl ReadEngine {
    pub fn new(
        sources: Arc<dyn SourceAdapter>,
        session: PathSession,
        limits: ServerLimits,
    ) -> Self {
        Self {
            sources,
            session,
            limits,
        }
    }

    pub async fn read(
        &self,
        request: ReadRequest,
        operation: &OperationGuard,
    ) -> Result<ReadResource, ResourceError> {
        let limits = request.limits.lowered_by(self.limits.text_limits());
        ensure_live(&self.session, operation)?;
        let source = self
            .sources
            .read(&request.reference, operation, request.acquisition.as_ref())
            .await?;
        ensure_live(&self.session, operation)?;

        let requested_artifact = match request.reference.address() {
            ResourceAddress::Artifact(address) => Some(address.clone()),
            ResourceAddress::Catalog(_)
            | ResourceAddress::Workspace(_)
            | ResourceAddress::Local(_)
            | ResourceAddress::Https(_)
            | ResourceAddress::Issue(_)
            | ResourceAddress::Jira(_)
            | ResourceAddress::PullRequest(_) => None,
        };
        let requested_artifact_projection = request.reference.projection().is_some();
        // Seen regions exist for editable Resources: Workspace files and named
        // Session Scratch. The scratch family root is a synthetic listing and
        // records nothing.
        let requested_workspace = matches!(
            request.reference.address(),
            ResourceAddress::Workspace(_) | ResourceAddress::Local(LocalAddress::Named(_))
        );
        let mut parts = source.into_parts();
        if parts.content.len() > MAX_ARTIFACT_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "Source Adapter selected projection exceeds the {MAX_ARTIFACT_BYTES}-byte artifact ceiling; narrow the selector"
                ),
            ));
        }

        let page_end = page_prefix_len(&parts.content, limits)?;
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
            // Nothing was cut from this page, so the only thing left to reach
            // is what the source itself omitted: its typed continuation names
            // the next upstream page, and a read that carries one is bounded.
            let continuation_reference = parts.continuation.take();
            let resource = ReadResource::from_parts(
                parts,
                continuation_reference.is_some(),
                recovery_reference,
                continuation_reference,
                None,
                page_end,
                request.numbered,
            )?;
            self.record_seen(&request.reference, &resource).await?;
            return Ok(resource);
        }

        // The page overflowed, so the artifact continuation must win: it is
        // the only reference that reaches the bytes cut from this page. The
        // source continuation travels alongside it as the source continuation
        // reference (ADR-0006), so the upstream page is addressable from this
        // response and not only from the end of the artifact chain.
        let source_continuation_reference = parts.continuation.take();

        let (recovery_address, origin) = match (requested_artifact, parts.artifact_origin) {
            (Some(address), Some(origin)) => (address, origin),
            _ => {
                let (ranges, _, displayed_eof) = parts.displayed_prefix(page_end);
                let address = if requested_workspace && (!ranges.is_empty() || displayed_eof) {
                    self.session
                        .retain_with_seen(
                            &parts.content,
                            operation,
                            &parts.canonical_reference,
                            &parts.version_tag,
                            &ranges,
                            displayed_eof,
                        )
                        .await?
                } else {
                    self.session.retain(&parts.content, operation).await?
                };
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
        let resource = ReadResource::from_parts(
            parts,
            true,
            Some(recovery_reference),
            Some(continuation_reference),
            source_continuation_reference,
            page_end,
            request.numbered,
        )?;
        Ok(resource)
    }
    async fn record_seen(
        &self,
        requested: &PathReference,
        resource: &ReadResource,
    ) -> Result<(), ResourceError> {
        // The recorded identity is the RESOLVED Resource's canonical reference,
        // never the requested spelling: a selector-bearing scratch read
        // (`local://plan.md:1-5`) must reserve its seen region under
        // `local://plan.md` so a later edit can find the snapshot.
        if matches!(
            requested.address(),
            ResourceAddress::Workspace(_) | ResourceAddress::Local(LocalAddress::Named(_))
        ) {
            self.session
                .record_seen(
                    resource.canonical_reference(),
                    resource.version_tag(),
                    resource.displayed_ranges(),
                    resource.displayed_eof(),
                )
                .await?;
        }
        Ok(())
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

pub(crate) fn page_prefix_len(content: &str, limits: TextLimits) -> Result<usize, ResourceError> {
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

pub(crate) fn canonical_artifact_reference(
    address: &ArtifactAddress,
) -> Result<String, ResourceError> {
    PathReference::artifact(address.clone(), None).map(|reference| reference.requested().to_owned())
}

pub(crate) fn artifact_line_continuation(
    address: &ArtifactAddress,
    next_line: u64,
) -> Result<String, ResourceError> {
    let selector = ProjectionSelector::parse(format!("{next_line}-"))?;
    PathReference::artifact(address.clone(), Some(selector))
        .map(|reference| reference.requested().to_owned())
}

pub(crate) fn artifact_page_continuation(
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
