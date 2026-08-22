use std::{
    collections::{HashMap, HashSet},
    fmt,
    num::NonZeroU64,
    sync::{Arc, Mutex as StdMutex, Weak},
};

use async_trait::async_trait;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::{
    ErrorCategory, MAX_ARTIFACT_BYTES, OperationGuard, PathReference, PathSession, ResourceAddress,
    ResourceError, VersionTag, WorkspaceAddress, session::SeenSnapshotData,
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

#[derive(Debug, Clone, Copy)]
enum NewlineStyle {
    Lf,
    CrLf,
}

impl NewlineStyle {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

struct TextAnalysis {
    total_lines: u64,
    offsets: HashMap<u64, usize>,
    styles: HashMap<u64, NewlineStyle>,
    dominant_style: NewlineStyle,
    last_style: Option<NewlineStyle>,
    ends_with_newline: bool,
}

struct ByteEdit {
    byte_start: usize,
    byte_end: usize,
    start_line: u64,
    end_line: u64,
    replacement: String,
    inserted_lines: u64,
}

struct PatchApplication {
    content: String,
    seen_ranges: Vec<crate::DisplayedLineRange>,
    seen_eof: bool,
}

fn apply_patch(
    content: &str,
    operations: &[PatchOperation],
    snapshot: &SeenSnapshotData,
) -> Result<PatchApplication, ResourceError> {
    let (wanted_offsets, wanted_styles) = wanted_line_metadata(operations)?;
    let analysis = analyze_text(content, &wanted_offsets, &wanted_styles);
    let mut edits = operations
        .iter()
        .map(|operation| build_byte_edit(content, operation, snapshot, &analysis))
        .collect::<Result<Vec<_>, _>>()?;
    edits.sort_unstable_by_key(|edit| {
        (
            edit.start_line,
            u8::from(edit.end_line != edit.start_line),
            edit.end_line,
        )
    });

    let capacity = edits.iter().try_fold(content.len(), |capacity, edit| {
        capacity
            .checked_sub(edit.byte_end - edit.byte_start)
            .and_then(|bytes| bytes.checked_add(edit.replacement.len()))
            .ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::LimitExceeded,
                    "edited content length overflows",
                )
            })
    })?;
    if capacity > MAX_ARTIFACT_BYTES {
        return Err(ResourceError::new(
            ErrorCategory::LimitExceeded,
            format!("edited content exceeds the {MAX_ARTIFACT_BYTES}-byte ceiling"),
        ));
    }
    let mut output = String::with_capacity(capacity);
    let mut byte_cursor = 0;
    for edit in &edits {
        if edit.byte_start < byte_cursor {
            return Err(invalid_patch(
                "patch operations conflict in the original tagged snapshot",
            ));
        }
        output.push_str(&content[byte_cursor..edit.byte_start]);
        output.push_str(&edit.replacement);
        byte_cursor = edit.byte_end;
    }
    output.push_str(&content[byte_cursor..]);

    let seen_ranges = remap_seen_ranges(snapshot, analysis.total_lines, &edits)?;
    let seen_eof = snapshot.displayed_eof
        || edits
            .iter()
            .any(|edit| edit.end_line == analysis.total_lines.saturating_add(1));
    Ok(PatchApplication {
        content: output,
        seen_ranges,
        seen_eof,
    })
}

fn wanted_line_metadata(
    operations: &[PatchOperation],
) -> Result<(HashSet<u64>, HashSet<u64>), ResourceError> {
    let mut offsets = HashSet::new();
    let mut styles = HashSet::new();
    offsets.insert(1);
    for operation in operations {
        match &operation.0 {
            PatchOperationKind::Put {
                target: PutTarget::Range(range),
                ..
            }
            | PatchOperationKind::Cut { range } => {
                let start = range.start().get();
                let end = range.end().get();
                offsets.insert(start);
                offsets.insert(
                    end.checked_add(1)
                        .ok_or_else(|| invalid_patch("line range boundary overflows"))?,
                );
                styles.insert(start);
                if start > 1 {
                    styles.insert(start - 1);
                }
            }
            PatchOperationKind::Put {
                target: PutTarget::Before(line),
                ..
            } => {
                let line = line.get();
                offsets.insert(line);
                styles.insert(line);
                if line > 1 {
                    styles.insert(line - 1);
                }
            }
            PatchOperationKind::Put {
                target: PutTarget::After(line),
                ..
            } => {
                let line = line.get();
                offsets.insert(
                    line.checked_add(1)
                        .ok_or_else(|| invalid_patch("line gap boundary overflows"))?,
                );
                styles.insert(line);
            }
            PatchOperationKind::Put {
                target: PutTarget::Tail,
                ..
            }
            | PatchOperationKind::Remove
            | PatchOperationKind::Move { .. } => {}
        }
    }
    Ok((offsets, styles))
}

fn analyze_text(
    content: &str,
    wanted_offsets: &HashSet<u64>,
    wanted_styles: &HashSet<u64>,
) -> TextAnalysis {
    let mut offsets = HashMap::with_capacity(wanted_offsets.len());
    let mut styles = HashMap::with_capacity(wanted_styles.len());
    if wanted_offsets.contains(&1) {
        offsets.insert(1, 0);
    }
    let mut line = 1_u64;
    let mut lf = 0_u64;
    let mut crlf = 0_u64;
    let mut last_style = None;
    for (index, byte) in content.bytes().enumerate() {
        if byte != b'\n' {
            continue;
        }
        let style = if index != 0 && content.as_bytes()[index - 1] == b'\r' {
            crlf += 1;
            NewlineStyle::CrLf
        } else {
            lf += 1;
            NewlineStyle::Lf
        };
        if wanted_styles.contains(&line) {
            styles.insert(line, style);
        }
        last_style = Some(style);
        line += 1;
        if wanted_offsets.contains(&line) {
            offsets.insert(line, index + 1);
        }
    }
    let ends_with_newline = content.ends_with('\n');
    let total_lines = if content.is_empty() {
        0
    } else if ends_with_newline {
        line - 1
    } else {
        line
    };
    if wanted_offsets.contains(&total_lines.saturating_add(1)) {
        offsets.insert(total_lines.saturating_add(1), content.len());
    }
    TextAnalysis {
        total_lines,
        offsets,
        styles,
        dominant_style: if crlf > lf {
            NewlineStyle::CrLf
        } else {
            NewlineStyle::Lf
        },
        last_style,
        ends_with_newline,
    }
}

fn build_byte_edit(
    content: &str,
    operation: &PatchOperation,
    snapshot: &SeenSnapshotData,
    analysis: &TextAnalysis,
) -> Result<ByteEdit, ResourceError> {
    match &operation.0 {
        PatchOperationKind::Put {
            target: PutTarget::Range(range),
            body,
        } => {
            validate_seen_range(snapshot, *range, analysis.total_lines)?;
            let start = range.start().get();
            let end = range.end().get();
            let style = line_style(analysis, start);
            let terminates_last = end < analysis.total_lines || analysis.ends_with_newline;
            Ok(ByteEdit {
                byte_start: line_offset(analysis, start)?,
                byte_end: line_offset(
                    analysis,
                    end.checked_add(1)
                        .ok_or_else(|| invalid_patch("line range boundary overflows"))?,
                )?,
                start_line: start,
                end_line: end + 1,
                replacement: build_body(body, style, false, terminates_last)?,
                inserted_lines: body.len() as u64,
            })
        }
        PatchOperationKind::Cut { range } => {
            validate_seen_range(snapshot, *range, analysis.total_lines)?;
            let start = range.start().get();
            let end = range.end().get();
            Ok(ByteEdit {
                byte_start: line_offset(analysis, start)?,
                byte_end: line_offset(
                    analysis,
                    end.checked_add(1)
                        .ok_or_else(|| invalid_patch("line range boundary overflows"))?,
                )?,
                start_line: start,
                end_line: end + 1,
                replacement: String::new(),
                inserted_lines: 0,
            })
        }
        PatchOperationKind::Put {
            target: PutTarget::Before(line),
            body,
        } => {
            let line = line.get();
            validate_seen_line(snapshot, line, analysis.total_lines)?;
            let style = adjacent_style(analysis, line);
            Ok(ByteEdit {
                byte_start: line_offset(analysis, line)?,
                byte_end: line_offset(analysis, line)?,
                start_line: line,
                end_line: line,
                replacement: build_body(body, style, false, true)?,
                inserted_lines: body.len() as u64,
            })
        }
        PatchOperationKind::Put {
            target: PutTarget::After(line),
            body,
        } => {
            let line = line.get();
            validate_seen_line(snapshot, line, analysis.total_lines)?;
            let gap = line
                .checked_add(1)
                .ok_or_else(|| invalid_patch("line gap boundary overflows"))?;
            let at_unterminated_eof = line == analysis.total_lines && !analysis.ends_with_newline;
            let style = line_style(analysis, line);
            Ok(ByteEdit {
                byte_start: line_offset(analysis, gap)?,
                byte_end: line_offset(analysis, gap)?,
                start_line: gap,
                end_line: gap,
                replacement: build_body(body, style, at_unterminated_eof, !at_unterminated_eof)?,
                inserted_lines: body.len() as u64,
            })
        }
        PatchOperationKind::Put {
            target: PutTarget::Tail,
            body,
        } => {
            if !snapshot.displayed_eof {
                return Err(invalid_patch(
                    "PUT >$: requires the same tagged snapshot to display EOF",
                ));
            }
            let gap = analysis.total_lines.saturating_add(1);
            let at_unterminated_eof = analysis.total_lines != 0 && !analysis.ends_with_newline;
            let style = analysis.last_style.unwrap_or(analysis.dominant_style);
            Ok(ByteEdit {
                byte_start: content.len(),
                byte_end: content.len(),
                start_line: gap,
                end_line: gap,
                replacement: build_body(
                    body,
                    style,
                    at_unterminated_eof,
                    analysis.ends_with_newline,
                )?,
                inserted_lines: body.len() as u64,
            })
        }
        PatchOperationKind::Remove | PatchOperationKind::Move { .. } => Err(ResourceError::new(
            ErrorCategory::UnsupportedMutation,
            "REM and MV are not PUT/CUT byte edits",
        )),
    }
}

fn line_offset(analysis: &TextAnalysis, line: u64) -> Result<usize, ResourceError> {
    analysis.offsets.get(&line).copied().ok_or_else(|| {
        invalid_patch(format!(
            "line coordinate {line} is outside the authoritative Resource"
        ))
    })
}

fn line_style(analysis: &TextAnalysis, line: u64) -> NewlineStyle {
    analysis
        .styles
        .get(&line)
        .copied()
        .or(analysis.last_style)
        .unwrap_or(analysis.dominant_style)
}

fn adjacent_style(analysis: &TextAnalysis, line: u64) -> NewlineStyle {
    line.checked_sub(1)
        .and_then(|previous| analysis.styles.get(&previous).copied())
        .or_else(|| analysis.styles.get(&line).copied())
        .or(analysis.last_style)
        .unwrap_or(analysis.dominant_style)
}

fn build_body(
    body: &[String],
    style: NewlineStyle,
    leading_separator: bool,
    terminate_last: bool,
) -> Result<String, ResourceError> {
    let newline = style.as_str();
    let capacity = body.iter().try_fold(
        usize::from(leading_separator) * newline.len(),
        |capacity, row| {
            capacity
                .checked_add(row.len())
                .and_then(|bytes| bytes.checked_add(newline.len()))
                .ok_or_else(|| {
                    ResourceError::new(ErrorCategory::LimitExceeded, "patch body length overflows")
                })
        },
    )?;
    let mut output = String::with_capacity(capacity);
    if leading_separator {
        output.push_str(newline);
    }
    for (index, row) in body.iter().enumerate() {
        output.push_str(row);
        if index + 1 != body.len() || terminate_last {
            output.push_str(newline);
        }
    }
    Ok(output)
}

fn validate_seen_range(
    snapshot: &SeenSnapshotData,
    range: OriginalLineRange,
    total_lines: u64,
) -> Result<(), ResourceError> {
    let start = range.start().get();
    let end = range.end().get();
    if end > total_lines {
        return Err(invalid_patch(format!(
            "line range {start}.={end} extends past EOF"
        )));
    }
    if snapshot
        .ranges
        .iter()
        .any(|seen| seen.start_line() <= start && seen.end_line() >= end)
    {
        Ok(())
    } else {
        Err(invalid_patch(format!(
            "line range {start}.={end} was not fully displayed; call rfs_read first"
        )))
    }
}

fn validate_seen_line(
    snapshot: &SeenSnapshotData,
    line: u64,
    total_lines: u64,
) -> Result<(), ResourceError> {
    if line == 0 || line > total_lines {
        return Err(invalid_patch(format!("line coordinate {line} is past EOF")));
    }
    if snapshot
        .ranges
        .iter()
        .any(|seen| seen.start_line() <= line && seen.end_line() >= line)
    {
        Ok(())
    } else {
        Err(invalid_patch(format!(
            "line {line} was not displayed; call rfs_read first"
        )))
    }
}

fn remap_seen_ranges(
    snapshot: &SeenSnapshotData,
    total_lines: u64,
    edits: &[ByteEdit],
) -> Result<Vec<crate::DisplayedLineRange>, ResourceError> {
    let mut output = Vec::new();
    let mut original_cursor = 1_u64;
    let mut output_cursor = 1_u64;
    for edit in edits {
        if edit.start_line < original_cursor {
            return Err(invalid_patch(
                "patch operations conflict in original line coordinates",
            ));
        }
        append_shifted_seen(
            &mut output,
            &snapshot.ranges,
            original_cursor,
            edit.start_line,
            output_cursor,
        )?;
        output_cursor += edit.start_line - original_cursor;
        if edit.inserted_lines != 0 {
            push_seen_range(
                &mut output,
                crate::DisplayedLineRange::new(
                    output_cursor,
                    output_cursor + edit.inserted_lines - 1,
                )?,
            );
            output_cursor += edit.inserted_lines;
        }
        original_cursor = edit.end_line;
    }
    let source_end = total_lines.saturating_add(1);
    append_shifted_seen(
        &mut output,
        &snapshot.ranges,
        original_cursor,
        source_end,
        output_cursor,
    )?;
    Ok(output)
}

fn append_shifted_seen(
    output: &mut Vec<crate::DisplayedLineRange>,
    seen: &[crate::DisplayedLineRange],
    source_start: u64,
    source_end: u64,
    output_start: u64,
) -> Result<(), ResourceError> {
    for range in seen {
        let start = range.start_line().max(source_start);
        let end = range.end_line().min(source_end.saturating_sub(1));
        if start > end {
            continue;
        }
        let shifted_start = output_start + (start - source_start);
        let shifted_end = output_start + (end - source_start);
        push_seen_range(
            output,
            crate::DisplayedLineRange::new(shifted_start, shifted_end)?,
        );
    }
    Ok(())
}

fn push_seen_range(
    ranges: &mut Vec<crate::DisplayedLineRange>,
    incoming: crate::DisplayedLineRange,
) {
    if let Some(last) = ranges.last_mut()
        && last.end_line().checked_add(1) == Some(incoming.start_line())
    {
        *last = crate::DisplayedLineRange::new(last.start_line(), incoming.end_line())
            .expect("merged seen ranges remain positive and ascending");
    } else {
        ranges.push(incoming);
    }
}

fn complete_content_ranges(content: &str) -> Result<Vec<crate::DisplayedLineRange>, ResourceError> {
    let lines = content.bytes().filter(|byte| *byte == b'\n').count() as u64
        + u64::from(!content.is_empty() && !content.ends_with('\n'));
    if lines == 0 {
        Ok(Vec::new())
    } else {
        crate::DisplayedLineRange::new(1, lines).map(|range| vec![range])
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    reference: PathReference,
    content: String,
    if_version: Option<VersionTag>,
}

impl WriteRequest {
    pub fn new(
        reference: PathReference,
        content: String,
        if_version: Option<VersionTag>,
    ) -> Result<Self, ResourceError> {
        if content.len() > MAX_ARTIFACT_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!("mutation content exceeds the {MAX_ARTIFACT_BYTES}-byte ceiling"),
            ));
        }
        Ok(Self {
            reference,
            content,
            if_version,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationOperation {
    Created,
    Replaced,
    Edited,
    Deleted,
    Moved,
}
impl MutationOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Replaced => "replaced",
            Self::Edited => "edited",
            Self::Deleted => "deleted",
            Self::Moved => "moved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationReceipt {
    operation: MutationOperation,
    canonical_reference: PathReference,
    source_reference: Option<PathReference>,
    version_tag: Option<VersionTag>,
    displayed_ranges: Vec<crate::DisplayedLineRange>,
    displayed_eof: bool,
}

impl MutationReceipt {
    pub const fn operation(&self) -> MutationOperation {
        self.operation
    }

    pub const fn canonical_reference(&self) -> &PathReference {
        &self.canonical_reference
    }

    pub const fn source_reference(&self) -> Option<&PathReference> {
        self.source_reference.as_ref()
    }

    pub const fn version_tag(&self) -> Option<&VersionTag> {
        self.version_tag.as_ref()
    }

    pub fn displayed_ranges(&self) -> &[crate::DisplayedLineRange] {
        &self.displayed_ranges
    }

    pub const fn displayed_eof(&self) -> bool {
        self.displayed_eof
    }
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

    fn lock_key(&self) -> String {
        let mut key = String::with_capacity(
            self.source_key.as_str().len() + self.canonical_reference.requested().len() + 1,
        );
        key.push_str(self.source_key.as_str());
        key.push('\0');
        key.push_str(self.canonical_reference.requested());
        key
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
    ) -> Result<(), ResourceError>;
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

    pub async fn write(
        &self,
        request: WriteRequest,
        operation: &OperationGuard,
    ) -> Result<MutationReceipt, ResourceError> {
        let WriteRequest {
            reference,
            content,
            if_version,
        } = request;
        let access = if if_version.is_some() {
            MutationAccess::Update
        } else {
            MutationAccess::Create
        };
        let target = self.adapter.resolve(&reference, access).await?;
        let _locks = self.lock_resources([target.lock_key()]).await?;
        let state = self.adapter.load(&target, access, operation).await?;
        let operation_kind = match (&if_version, &state) {
            (None, MutationState::Missing) => MutationOperation::Created,
            (None, MutationState::Text { .. }) => {
                return Err(ResourceError::new(
                    ErrorCategory::VersionConflict,
                    "create requires a missing destination",
                ));
            }
            (Some(_), MutationState::Missing) => {
                return Err(ResourceError::new(
                    ErrorCategory::VersionConflict,
                    "replacement requires an existing Resource",
                ));
            }
            (Some(expected), MutationState::Text { version_tag, .. })
                if expected != version_tag =>
            {
                return Err(ResourceError::new(
                    ErrorCategory::VersionConflict,
                    "replacement Version Tag does not match authoritative content",
                ));
            }
            (Some(_), MutationState::Text { .. }) => MutationOperation::Replaced,
        };
        let version_tag = VersionTag::from_content(content.as_bytes());
        let displayed_ranges = complete_content_ranges(&content)?;
        let reservation = self
            .session
            .reserve_seen(
                target.canonical_reference().requested(),
                &version_tag,
                &displayed_ranges,
                true,
            )
            .await?;
        if let Err(error) = operation.begin_commit() {
            self.session.cancel_seen(reservation).await;
            return Err(error);
        }
        let mutation = match (&if_version, operation_kind) {
            (None, MutationOperation::Created) => SourceMutation::Create {
                target: target.clone(),
                content,
            },
            (Some(expected), MutationOperation::Replaced) => SourceMutation::Replace {
                target: target.clone(),
                expected: expected.clone(),
                content,
            },
            _ => unreachable!("write state matrix chooses one matching mutation"),
        };
        let committed = self.adapter.commit(mutation, operation).await;
        debug_assert!(
            operation.finish_commit(),
            "write commit transition must complete"
        );
        if let Err(error) = committed {
            self.session.cancel_seen(reservation).await;
            return Err(error);
        }
        self.session.publish_seen(reservation).await;
        Ok(MutationReceipt {
            operation: operation_kind,
            canonical_reference: target.canonical_reference().clone(),
            source_reference: None,
            version_tag: Some(version_tag),
            displayed_ranges,
            displayed_eof: true,
        })
    }

    pub async fn edit(
        &self,
        document: &str,
        operation: &OperationGuard,
    ) -> Result<MutationReceipt, ResourceError> {
        let patch = HashlinePatch::parse(document)?;
        if patch.operations().iter().any(|operation| {
            matches!(
                operation.0,
                PatchOperationKind::Remove | PatchOperationKind::Move { .. }
            )
        }) {
            return Err(ResourceError::new(
                ErrorCategory::UnsupportedMutation,
                "REM and MV execution are not enabled in this increment",
            ));
        }
        let target = self
            .adapter
            .resolve(patch.target(), MutationAccess::Update)
            .await?;
        let _locks = self.lock_resources([target.lock_key()]).await?;
        let snapshot = self
            .session
            .resolve_seen(target.canonical_reference().requested(), patch.version())
            .await?;
        let state = self
            .adapter
            .load(&target, MutationAccess::Update, operation)
            .await?;
        let MutationState::Text {
            content,
            version_tag,
        } = state
        else {
            return Err(ResourceError::new(
                ErrorCategory::VersionConflict,
                "hashline edit requires an existing text Resource",
            ));
        };
        if version_tag != snapshot.version_tag {
            return Err(ResourceError::new(
                ErrorCategory::VersionConflict,
                "hashline edit Version Tag no longer matches authoritative content",
            ));
        }
        let applied = apply_patch(&content, patch.operations(), &snapshot)?;
        if applied.content.len() > MAX_ARTIFACT_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!("edited content exceeds the {MAX_ARTIFACT_BYTES}-byte ceiling"),
            ));
        }
        let PatchApplication {
            content: edited_content,
            seen_ranges,
            seen_eof,
        } = applied;
        let new_version = VersionTag::from_content(edited_content.as_bytes());
        let reservation = self
            .session
            .reserve_seen(
                target.canonical_reference().requested(),
                &new_version,
                &seen_ranges,
                seen_eof,
            )
            .await?;
        if let Err(error) = operation.begin_commit() {
            self.session.cancel_seen(reservation).await;
            return Err(error);
        }
        let committed = self
            .adapter
            .commit(
                SourceMutation::Replace {
                    target: target.clone(),
                    expected: snapshot.version_tag,
                    content: edited_content,
                },
                operation,
            )
            .await;
        debug_assert!(
            operation.finish_commit(),
            "edit commit transition must complete"
        );
        if let Err(error) = committed {
            self.session.cancel_seen(reservation).await;
            return Err(error);
        }
        self.session.publish_seen(reservation).await;
        Ok(MutationReceipt {
            operation: MutationOperation::Edited,
            canonical_reference: target.canonical_reference().clone(),
            source_reference: None,
            version_tag: Some(new_version),
            displayed_ranges: seen_ranges,
            displayed_eof: seen_eof,
        })
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
