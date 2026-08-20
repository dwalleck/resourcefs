use std::{
    collections::{HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
};

use sha2::{Digest, Sha256};

use crate::{ErrorCategory, MAX_ARTIFACT_BYTES, ProjectionSelector, ResourceError, VersionTag};

const SCAN_BUFFER_BYTES: usize = 64 * 1024;

/// Complete selected UTF-8 projection and the authoritative source identity observed around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedText {
    content: String,
    version_tag: VersionTag,
    source_bytes: u64,
}

impl SelectedText {
    pub fn content(&self) -> &str {
        &self.content
    }

    pub const fn version_tag(&self) -> &VersionTag {
        &self.version_tag
    }

    pub const fn source_bytes(&self) -> u64 {
        self.source_bytes
    }

    pub fn into_parts(self) -> (String, VersionTag, u64) {
        (self.content, self.version_tag, self.source_bytes)
    }
}

/// Select exact UTF-8 line spans while hashing and validating the complete seekable source.
///
/// The source is scanned before and after the selected spans are copied. A changed digest rejects
/// delivery rather than pairing bytes from one state with the Version Tag of another.
pub fn select_utf8<R>(
    mut reader: R,
    projection: Option<&ProjectionSelector>,
) -> Result<SelectedText, ResourceError>
where
    R: Read + Seek,
{
    if projection.is_some_and(|selector| selector.page_offset().is_some()) {
        return Err(ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            "artifact page selectors are executed by the bounded read engine",
        ));
    }

    let ranges = projection.and_then(ProjectionSelector::line_selection);
    let boundaries = requested_boundaries(ranges);
    let first = scan_source(&mut reader, &boundaries)?;
    let spans = selected_spans(ranges, &first)?;
    let selected_bytes = spans.iter().try_fold(0_u64, |total, (start, end)| {
        total
            .checked_add(end - start)
            .ok_or_else(selection_too_large)
    })?;
    if selected_bytes > MAX_ARTIFACT_BYTES as u64 {
        return Err(selection_too_large());
    }

    let mut content = Vec::with_capacity(selected_bytes as usize);
    let mut buffer = vec![0_u8; SCAN_BUFFER_BYTES];
    for (start, end) in spans {
        reader
            .seek(SeekFrom::Start(start))
            .map_err(selection_io_error)?;
        let mut remaining = end - start;
        while remaining != 0 {
            let wanted = remaining.min(buffer.len() as u64) as usize;
            let read = reader
                .read(&mut buffer[..wanted])
                .map_err(selection_io_error)?;
            if read == 0 {
                return Err(ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "Resource changed while its selected projection was read",
                ));
            }
            content.extend_from_slice(&buffer[..read]);
            remaining -= read as u64;
        }
    }

    let second = scan_source(&mut reader, &HashSet::new())?;
    if first.source_bytes != second.source_bytes || first.digest != second.digest {
        return Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "Resource changed while its selected projection was read",
        ));
    }
    let content = String::from_utf8(content).map_err(|_| {
        ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            "Resource is not valid UTF-8 text",
        )
    })?;
    Ok(SelectedText {
        content,
        version_tag: VersionTag::from_sha256_digest(second.digest),
        source_bytes: second.source_bytes,
    })
}

#[derive(Debug)]
struct SourceScan {
    digest: [u8; 32],
    source_bytes: u64,
    total_lines: u64,
    line_offsets: HashMap<u64, u64>,
}

fn requested_boundaries(ranges: Option<&crate::LineSelector>) -> HashSet<u64> {
    let mut boundaries = HashSet::new();
    let Some(ranges) = ranges else {
        return boundaries;
    };
    for range in ranges.ranges() {
        boundaries.insert(range.start());
        if let Some(end) = range.inclusive_end().and_then(|end| end.checked_add(1)) {
            boundaries.insert(end);
        }
    }
    boundaries
}

fn scan_source<R: Read + Seek>(
    reader: &mut R,
    requested_boundaries: &HashSet<u64>,
) -> Result<SourceScan, ResourceError> {
    reader
        .seek(SeekFrom::Start(0))
        .map_err(selection_io_error)?;
    let mut digest = Sha256::new();
    let mut validator = Utf8Validator::default();
    let mut buffer = vec![0_u8; SCAN_BUFFER_BYTES];
    let mut source_bytes = 0_u64;
    let mut newline_count = 0_u64;
    let mut last_was_newline = false;
    let mut line_offsets = HashMap::with_capacity(requested_boundaries.len());
    if requested_boundaries.contains(&1) {
        line_offsets.insert(1, 0);
    }

    loop {
        let read = reader.read(&mut buffer).map_err(selection_io_error)?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        validator.push(chunk)?;
        digest.update(chunk);
        for (index, byte) in chunk.iter().enumerate() {
            if *byte == b'\n' {
                newline_count = newline_count.checked_add(1).ok_or_else(|| {
                    ResourceError::new(ErrorCategory::LimitExceeded, "Resource has too many lines")
                })?;
                let next_line = newline_count + 1;
                if requested_boundaries.contains(&next_line) {
                    line_offsets.insert(next_line, source_bytes + index as u64 + 1);
                }
            }
        }
        source_bytes = source_bytes.checked_add(read as u64).ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::LimitExceeded,
                "Resource byte length overflows",
            )
        })?;
        last_was_newline = chunk.last() == Some(&b'\n');
    }
    validator.finish()?;
    let total_lines = newline_count + u64::from(source_bytes != 0 && !last_was_newline);
    Ok(SourceScan {
        digest: digest.finalize().into(),
        source_bytes,
        total_lines,
        line_offsets,
    })
}

fn selected_spans(
    ranges: Option<&crate::LineSelector>,
    source: &SourceScan,
) -> Result<Vec<(u64, u64)>, ResourceError> {
    let Some(ranges) = ranges else {
        return Ok(vec![(0, source.source_bytes)]);
    };
    let mut spans = Vec::with_capacity(ranges.ranges().len());
    for range in ranges.ranges() {
        let start_line = range.start();
        if start_line > source.total_lines {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                format!("line selector start {start_line} is past EOF"),
            ));
        }
        let requested_end = range.inclusive_end().unwrap_or(source.total_lines);
        let end_line = requested_end.min(source.total_lines);
        let start = *source.line_offsets.get(&start_line).ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "Resource line index changed during selection",
            )
        })?;
        let end = if end_line == source.total_lines {
            source.source_bytes
        } else {
            *source.line_offsets.get(&(end_line + 1)).ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "Resource line index changed during selection",
                )
            })?
        };
        spans.push((start, end));
    }
    Ok(spans)
}

#[derive(Debug, Default)]
struct Utf8Validator {
    pending: Vec<u8>,
}

impl Utf8Validator {
    fn push(&mut self, mut bytes: &[u8]) -> Result<(), ResourceError> {
        if !self.pending.is_empty() {
            let take = (4 - self.pending.len()).min(bytes.len());
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            match std::str::from_utf8(&self.pending) {
                Ok(_) => self.pending.clear(),
                Err(error) if error.error_len().is_some() => return Err(invalid_utf8()),
                Err(_) if bytes.is_empty() => return Ok(()),
                Err(_) => return Err(invalid_utf8()),
            }
        }
        if bytes.is_empty() {
            return Ok(());
        }
        match std::str::from_utf8(bytes) {
            Ok(_) => Ok(()),
            Err(error) if error.error_len().is_some() => Err(invalid_utf8()),
            Err(error) => {
                self.pending
                    .extend_from_slice(&bytes[error.valid_up_to()..]);
                Ok(())
            }
        }
    }

    fn finish(self) -> Result<(), ResourceError> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(invalid_utf8())
        }
    }
}

fn invalid_utf8() -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        "Resource is not valid UTF-8 text",
    )
}

fn selection_io_error(error: std::io::Error) -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!("reading Resource text failed: {error}"),
    )
}

fn selection_too_large() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        format!(
            "selected projection exceeds the {MAX_ARTIFACT_BYTES}-byte artifact ceiling; use a narrower selector"
        ),
    )
}
