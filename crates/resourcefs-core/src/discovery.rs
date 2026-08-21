use std::{
    cmp::Ordering,
    fmt::{self, Write as _},
    ops::Range,
    sync::Arc,
};

use async_trait::async_trait;
#[cfg(feature = "test-support")]
use tokio::sync::Notify;

use crate::{
    ErrorCategory, MAX_ARTIFACT_BYTES, MAX_PATH_REFERENCE_BYTES, OperationGuard, PathReference,
    PathSession, ResourceAddress, ResourceError, TextLimits, WorkspaceAddress,
    read::{artifact_line_continuation, canonical_artifact_reference, page_prefix_len},
};
pub const MAX_DISCOVERY_PATTERN_BYTES: usize = 64 * 1024;
pub const MAX_DISCOVERY_RESULTS: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchTarget {
    reference: Option<PathReference>,
}

impl SearchTarget {
    pub const fn primary() -> Self {
        Self { reference: None }
    }

    pub fn resource(reference: PathReference) -> Self {
        Self {
            reference: Some(reference),
        }
    }

    pub const fn reference(&self) -> Option<&PathReference> {
        self.reference.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobSource {
    Workspace,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobTarget {
    pattern: String,
    source: GlobSource,
}

impl GlobTarget {
    pub fn new(pattern: impl Into<String>) -> Result<Self, ResourceError> {
        let pattern = pattern.into();
        if pattern.is_empty() {
            return Err(invalid_pattern("glob pattern must not be empty"));
        }
        if pattern.len() > MAX_PATH_REFERENCE_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "glob pattern exceeds the {MAX_PATH_REFERENCE_BYTES}-byte Path Reference ceiling"
                ),
            ));
        }
        let source = if pattern.starts_with(crate::reference::ARTIFACT_PREFIX) {
            GlobSource::Artifact
        } else {
            GlobSource::Workspace
        };
        Ok(Self { pattern, source })
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub const fn source(&self) -> GlobSource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOptions {
    case_sensitive: bool,
    gitignore: bool,
    hidden: bool,
}

impl SearchOptions {
    pub const fn new(case_sensitive: bool, gitignore: bool, hidden: bool) -> Self {
        Self {
            case_sensitive,
            gitignore,
            hidden,
        }
    }

    pub const fn case_sensitive(self) -> bool {
        self.case_sensitive
    }

    pub const fn gitignore(self) -> bool {
        self.gitignore
    }

    pub const fn hidden(self) -> bool {
        self.hidden
    }
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self::new(true, true, false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobOptions {
    case_sensitive: bool,
    gitignore: bool,
    hidden: bool,
}

impl GlobOptions {
    pub const fn new(case_sensitive: bool, gitignore: bool, hidden: bool) -> Self {
        Self {
            case_sensitive,
            gitignore,
            hidden,
        }
    }

    pub const fn case_sensitive(self) -> bool {
        self.case_sensitive
    }

    pub const fn gitignore(self) -> bool {
        self.gitignore
    }

    pub const fn hidden(self) -> bool {
        self.hidden
    }
}

impl Default for GlobOptions {
    fn default() -> Self {
        Self::new(true, true, false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchLimits {
    max_results: usize,
    text: TextLimits,
}

impl SearchLimits {
    pub fn new(
        max_results: Option<usize>,
        max_bytes: Option<usize>,
        max_lines: Option<usize>,
        max_columns: Option<usize>,
    ) -> Result<Self, ResourceError> {
        Ok(Self {
            max_results: validate_result_limit(max_results)?,
            text: TextLimits::new(max_bytes, max_lines, max_columns)?,
        })
    }
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_results: MAX_DISCOVERY_RESULTS,
            text: TextLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobLimits {
    max_results: usize,
}

impl GlobLimits {
    pub fn new(max_results: Option<usize>) -> Result<Self, ResourceError> {
        Ok(Self {
            max_results: validate_result_limit(max_results)?,
        })
    }
}

impl Default for GlobLimits {
    fn default() -> Self {
        Self {
            max_results: MAX_DISCOVERY_RESULTS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    target: SearchTarget,
    pattern: String,
    options: SearchOptions,
    skip: usize,
    limits: SearchLimits,
}

impl SearchRequest {
    pub fn new(
        target: SearchTarget,
        pattern: impl Into<String>,
        options: SearchOptions,
        skip: usize,
        limits: SearchLimits,
    ) -> Result<Self, ResourceError> {
        let pattern = pattern.into();
        if pattern.is_empty() {
            return Err(invalid_pattern("search pattern must not be empty"));
        }
        if pattern.len() > MAX_DISCOVERY_PATTERN_BYTES {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                format!(
                    "search pattern exceeds the {MAX_DISCOVERY_PATTERN_BYTES}-byte compiler-input ceiling"
                ),
            ));
        }
        Ok(Self {
            target,
            pattern,
            options,
            skip,
            limits,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobRequest {
    target: GlobTarget,
    options: GlobOptions,
    skip: usize,
    limits: GlobLimits,
}

impl GlobRequest {
    pub const fn new(
        target: GlobTarget,
        options: GlobOptions,
        skip: usize,
        limits: GlobLimits,
    ) -> Self {
        Self {
            target,
            options,
            skip,
            limits,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEngine {
    RustRegex,
    Pcre2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRecord {
    reference: String,
    line: u64,
    text: String,
}

impl SearchRecord {
    pub fn new(
        reference: PathReference,
        line: u64,
        text: impl Into<String>,
    ) -> Result<Self, ResourceError> {
        if line == 0 {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "search record line number must be positive",
            ));
        }
        Ok(Self {
            reference: canonical_identity(&reference)?,
            line,
            text: text.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryDiagnostic {
    reference: Option<PathReference>,
    category: ErrorCategory,
    message: String,
}

impl DiscoveryDiagnostic {
    pub fn new(
        reference: Option<PathReference>,
        category: ErrorCategory,
        message: impl Into<String>,
    ) -> Self {
        Self {
            reference,
            category,
            message: message.into(),
        }
    }

    pub fn reference(&self) -> Option<&str> {
        self.reference.as_ref().map(PathReference::requested)
    }

    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobKind {
    File,
    Directory,
    Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobEntry {
    reference: String,
    kind: GlobKind,
}

impl GlobEntry {
    pub fn new(reference: PathReference, kind: GlobKind) -> Result<Self, ResourceError> {
        let kind_matches_source = match reference.address() {
            ResourceAddress::Workspace(_) => !matches!(kind, GlobKind::Artifact),
            ResourceAddress::Artifact(_) => matches!(kind, GlobKind::Artifact),
        };
        if !kind_matches_source {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "glob entry kind must match its canonical Resource source",
            ));
        }
        Ok(Self {
            reference: canonical_identity(&reference)?,
            kind,
        })
    }

    pub fn reference(&self) -> &str {
        &self.reference
    }

    pub const fn kind(&self) -> GlobKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchSourceResult {
    engine: SearchEngine,
    records: Vec<SearchRecord>,
    diagnostics: Vec<DiscoveryDiagnostic>,
}

impl SearchSourceResult {
    pub const fn new(
        engine: SearchEngine,
        records: Vec<SearchRecord>,
        diagnostics: Vec<DiscoveryDiagnostic>,
    ) -> Self {
        Self {
            engine,
            records,
            diagnostics,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceGlobResult {
    entries: Vec<GlobEntry>,
    diagnostics: Vec<DiscoveryDiagnostic>,
}

impl SourceGlobResult {
    pub const fn new(entries: Vec<GlobEntry>, diagnostics: Vec<DiscoveryDiagnostic>) -> Self {
        Self {
            entries,
            diagnostics,
        }
    }
}

#[async_trait]
pub trait DiscoveryAdapter: Send + Sync {
    async fn search(
        &self,
        target: &SearchTarget,
        pattern: &str,
        options: SearchOptions,
        operation: &OperationGuard,
    ) -> Result<SearchSourceResult, ResourceError>;

    async fn glob(
        &self,
        target: &GlobTarget,
        options: GlobOptions,
        operation: &OperationGuard,
    ) -> Result<SourceGlobResult, ResourceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchLine {
    line: u64,
    text: String,
}

impl SearchLine {
    pub const fn line(&self) -> u64 {
        self.line
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchGroup {
    reference: String,
    lines: Vec<SearchLine>,
}

impl SearchGroup {
    pub fn reference(&self) -> &str {
        &self.reference
    }

    pub fn lines(&self) -> &[SearchLine] {
        &self.lines
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    engine: SearchEngine,
    groups: Vec<SearchGroup>,
    diagnostics: Vec<DiscoveryDiagnostic>,
    returned_records: usize,
    total_records: usize,
    text: String,
    recovery_reference: Option<String>,
    continuation_reference: Option<String>,
}

impl SearchResult {
    pub const fn engine(&self) -> SearchEngine {
        self.engine
    }

    pub fn groups(&self) -> &[SearchGroup] {
        &self.groups
    }

    pub fn diagnostics(&self) -> &[DiscoveryDiagnostic] {
        &self.diagnostics
    }

    pub const fn returned_records(&self) -> usize {
        self.returned_records
    }

    pub const fn total_records(&self) -> usize {
        self.total_records
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn recovery_reference(&self) -> Option<&str> {
        self.recovery_reference.as_deref()
    }

    pub fn continuation_reference(&self) -> Option<&str> {
        self.continuation_reference.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobResult {
    entries: Vec<GlobEntry>,
    diagnostics: Vec<DiscoveryDiagnostic>,
    returned_records: usize,
    total_records: usize,
    text: String,
    recovery_reference: Option<String>,
    continuation_reference: Option<String>,
}

impl GlobResult {
    pub fn entries(&self) -> &[GlobEntry] {
        &self.entries
    }

    pub fn diagnostics(&self) -> &[DiscoveryDiagnostic] {
        &self.diagnostics
    }

    pub const fn returned_records(&self) -> usize {
        self.returned_records
    }

    pub const fn total_records(&self) -> usize {
        self.total_records
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn recovery_reference(&self) -> Option<&str> {
        self.recovery_reference.as_deref()
    }

    pub fn continuation_reference(&self) -> Option<&str> {
        self.continuation_reference.as_deref()
    }
}

#[cfg(feature = "test-support")]
#[derive(Debug, Default)]
pub struct DiscoveryRetainGate {
    entered: Notify,
    release: Notify,
}

#[cfg(feature = "test-support")]
impl DiscoveryRetainGate {
    pub const fn new() -> Self {
        Self {
            entered: Notify::const_new(),
            release: Notify::const_new(),
        }
    }

    pub async fn wait_until_entered(&self) {
        self.entered.notified().await;
    }

    pub fn release(&self) {
        self.release.notify_one();
    }

    async fn block(&self) {
        self.entered.notify_one();
        self.release.notified().await;
    }
}

#[derive(Clone)]
pub struct DiscoveryEngine {
    sources: Arc<dyn DiscoveryAdapter>,
    session: PathSession,
    #[cfg(feature = "test-support")]
    retain_gate: Option<Arc<DiscoveryRetainGate>>,
}

impl fmt::Debug for DiscoveryEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveryEngine")
            .finish_non_exhaustive()
    }
}

impl DiscoveryEngine {
    pub fn new(sources: Arc<dyn DiscoveryAdapter>, session: PathSession) -> Self {
        Self {
            sources,
            session,
            #[cfg(feature = "test-support")]
            retain_gate: None,
        }
    }

    #[cfg(feature = "test-support")]
    pub fn with_retain_gate_for_test(mut self, gate: Arc<DiscoveryRetainGate>) -> Self {
        self.retain_gate = Some(gate);
        self
    }

    pub async fn search(
        &self,
        request: SearchRequest,
        operation: &OperationGuard,
    ) -> Result<SearchResult, ResourceError> {
        ensure_discovery_live(&self.session, operation)?;
        let source = self
            .sources
            .search(
                &request.target,
                &request.pattern,
                request.options,
                operation,
            )
            .await?;
        ensure_discovery_live(&self.session, operation)?;

        let SearchSourceResult {
            engine,
            mut records,
            mut diagnostics,
        } = source;
        normalize_search_records(&mut records);
        normalize_diagnostics(&mut diagnostics)?;
        let document = render_search_document(&records, &diagnostics)?;
        let page = select_search_page(
            &document,
            &records,
            &diagnostics,
            request.skip,
            request.limits,
        )?;
        let recovery = self
            .retain_omitted(&document, page.omitted, page.next_record, operation)
            .await?;

        Ok(SearchResult {
            engine,
            groups: group_search_records(&records[page.record_range.clone()]),
            diagnostics: diagnostics[page.diagnostic_range].to_vec(),
            returned_records: page.record_range.len(),
            total_records: records.len(),
            text: page.text,
            recovery_reference: recovery.reference,
            continuation_reference: recovery.continuation,
        })
    }

    pub async fn glob(
        &self,
        request: GlobRequest,
        operation: &OperationGuard,
    ) -> Result<GlobResult, ResourceError> {
        ensure_discovery_live(&self.session, operation)?;
        let source = self
            .sources
            .glob(&request.target, request.options, operation)
            .await?;
        ensure_discovery_live(&self.session, operation)?;

        let SourceGlobResult {
            mut entries,
            mut diagnostics,
        } = source;
        normalize_glob_entries(&mut entries);
        normalize_diagnostics(&mut diagnostics)?;
        let document = render_glob_document(&entries, &diagnostics)?;
        let page = select_glob_page(
            &document,
            entries.len(),
            diagnostics.len(),
            request.skip,
            request.limits,
        );
        let recovery = self
            .retain_omitted(&document, page.omitted, page.next_record, operation)
            .await?;

        Ok(GlobResult {
            entries: entries[page.record_range.clone()].to_vec(),
            diagnostics: diagnostics[page.diagnostic_range].to_vec(),
            returned_records: page.record_range.len(),
            total_records: entries.len(),
            text: page.text,
            recovery_reference: recovery.reference,
            continuation_reference: recovery.continuation,
        })
    }

    async fn retain_omitted(
        &self,
        document: &CanonicalDocument,
        omitted: bool,
        next_record: Option<usize>,
        operation: &OperationGuard,
    ) -> Result<Recovery, ResourceError> {
        if !omitted {
            return Ok(Recovery::default());
        }
        #[cfg(feature = "test-support")]
        if let Some(gate) = self.retain_gate.as_ref() {
            gate.block().await;
        }
        ensure_discovery_live(&self.session, operation)?;
        let retain_started_active = operation.is_active();
        let address = match self.session.retain(&document.text, operation).await {
            Ok(address) => address,
            Err(_) if retain_started_active && !operation.is_active() => return Err(cancelled()),
            Err(error) => return Err(error),
        };
        let reference = canonical_artifact_reference(&address)?;
        let continuation = next_record
            .map(|index| {
                let next_line = u64::try_from(index)
                    .ok()
                    .and_then(|line| line.checked_add(1))
                    .ok_or_else(continuation_overflow)?;
                artifact_line_continuation(&address, next_line)
            })
            .transpose()?;
        Ok(Recovery {
            reference: Some(reference),
            continuation,
        })
    }
}

#[derive(Default)]
struct Recovery {
    reference: Option<String>,
    continuation: Option<String>,
}

struct CanonicalDocument {
    text: String,
    record_ranges: Vec<Range<usize>>,
    diagnostic_ranges: Vec<Range<usize>>,
}

struct SelectedPage {
    record_range: Range<usize>,
    diagnostic_range: Range<usize>,
    next_record: Option<usize>,
    omitted: bool,
    text: String,
}

fn normalize_search_records(records: &mut Vec<SearchRecord>) {
    records.sort_unstable_by(|left, right| {
        left.reference
            .as_bytes()
            .cmp(right.reference.as_bytes())
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.text.as_bytes().cmp(right.text.as_bytes()))
    });
    records.dedup_by(|current, previous| {
        current.reference == previous.reference && current.line == previous.line
    });
}

fn normalize_glob_entries(entries: &mut Vec<GlobEntry>) {
    entries.sort_unstable_by(|left, right| {
        left.reference
            .as_bytes()
            .cmp(right.reference.as_bytes())
            .then_with(|| glob_kind_rank(left.kind).cmp(&glob_kind_rank(right.kind)))
    });
    entries.dedup_by(|current, previous| current.reference == previous.reference);
}

fn glob_kind_rank(kind: GlobKind) -> u8 {
    match kind {
        GlobKind::File => 0,
        GlobKind::Directory => 1,
        GlobKind::Artifact => 2,
    }
}

fn normalize_diagnostics(diagnostics: &mut Vec<DiscoveryDiagnostic>) -> Result<(), ResourceError> {
    for diagnostic in diagnostics.iter() {
        if let Some(reference) = diagnostic.reference.as_ref() {
            canonical_identity(reference)?;
        }
    }
    diagnostics.sort_unstable_by(compare_diagnostics);
    diagnostics.dedup_by(|current, previous| current == previous);
    Ok(())
}

fn compare_diagnostics(left: &DiscoveryDiagnostic, right: &DiscoveryDiagnostic) -> Ordering {
    left.reference()
        .unwrap_or_default()
        .as_bytes()
        .cmp(right.reference().unwrap_or_default().as_bytes())
        .then_with(|| left.category.as_str().cmp(right.category.as_str()))
        .then_with(|| left.message.as_bytes().cmp(right.message.as_bytes()))
}

fn render_search_document(
    records: &[SearchRecord],
    diagnostics: &[DiscoveryDiagnostic],
) -> Result<CanonicalDocument, ResourceError> {
    if records.is_empty() && diagnostics.is_empty() {
        return Ok(CanonicalDocument {
            text: "No matches found.\n".to_owned(),
            record_ranges: Vec::new(),
            diagnostic_ranges: Vec::new(),
        });
    }

    let mut total = 0_usize;
    for record in records {
        total = checked_document_add(total, search_record_len(record)?)?;
    }
    for diagnostic in diagnostics {
        total = checked_document_add(total, diagnostic_len(diagnostic)?)?;
    }

    let mut text = String::with_capacity(total);
    let mut record_ranges = Vec::with_capacity(records.len());
    for record in records {
        let start = text.len();
        write!(&mut text, "{}\t{}\t", record.reference, record.line)
            .expect("writing to a String cannot fail");
        push_escaped_field(&mut text, &record.text);
        text.push('\n');
        record_ranges.push(start..text.len());
    }
    let mut diagnostic_ranges = Vec::with_capacity(diagnostics.len());
    for diagnostic in diagnostics {
        let start = text.len();
        write!(
            &mut text,
            "!{}\t{}\t",
            diagnostic.reference().unwrap_or("-"),
            diagnostic.category.as_str()
        )
        .expect("writing to a String cannot fail");
        push_escaped_field(&mut text, &diagnostic.message);
        text.push('\n');
        diagnostic_ranges.push(start..text.len());
    }
    debug_assert_eq!(text.len(), total);
    Ok(CanonicalDocument {
        text,
        record_ranges,
        diagnostic_ranges,
    })
}

fn render_glob_document(
    entries: &[GlobEntry],
    diagnostics: &[DiscoveryDiagnostic],
) -> Result<CanonicalDocument, ResourceError> {
    if entries.is_empty() && diagnostics.is_empty() {
        return Ok(CanonicalDocument {
            text: "No matches found.\n".to_owned(),
            record_ranges: Vec::new(),
            diagnostic_ranges: Vec::new(),
        });
    }

    let mut total = 0_usize;
    for entry in entries {
        let suffix = usize::from(matches!(entry.kind, GlobKind::Directory));
        total = checked_document_add(total, entry.reference.len() + suffix + 1)?;
    }
    for diagnostic in diagnostics {
        total = checked_document_add(total, diagnostic_len(diagnostic)?)?;
    }

    let mut text = String::with_capacity(total);
    let mut record_ranges = Vec::with_capacity(entries.len());
    for entry in entries {
        let start = text.len();
        text.push_str(&entry.reference);
        if matches!(entry.kind, GlobKind::Directory) {
            text.push('/');
        }
        text.push('\n');
        record_ranges.push(start..text.len());
    }
    let mut diagnostic_ranges = Vec::with_capacity(diagnostics.len());
    for diagnostic in diagnostics {
        let start = text.len();
        write!(
            &mut text,
            "!{}\t{}\t",
            diagnostic.reference().unwrap_or("-"),
            diagnostic.category.as_str()
        )
        .expect("writing to a String cannot fail");
        push_escaped_field(&mut text, &diagnostic.message);
        text.push('\n');
        diagnostic_ranges.push(start..text.len());
    }
    debug_assert_eq!(text.len(), total);
    Ok(CanonicalDocument {
        text,
        record_ranges,
        diagnostic_ranges,
    })
}

fn search_record_len(record: &SearchRecord) -> Result<usize, ResourceError> {
    let mut length = record.reference.len();
    length = checked_document_add(length, 1 + decimal_len(record.line) + 1)?;
    length = checked_document_add(length, escaped_field_len(&record.text)?)?;
    checked_document_add(length, 1)
}

fn diagnostic_len(diagnostic: &DiscoveryDiagnostic) -> Result<usize, ResourceError> {
    let reference_length = diagnostic.reference().map_or(1, str::len);
    let mut length = 1 + reference_length + 1 + diagnostic.category.as_str().len() + 1;
    length = checked_document_add(length, escaped_field_len(&diagnostic.message)?)?;
    checked_document_add(length, 1)
}

fn checked_document_add(current: usize, additional: usize) -> Result<usize, ResourceError> {
    let resulting = current
        .checked_add(additional)
        .ok_or_else(discovery_document_too_large)?;
    if resulting > MAX_ARTIFACT_BYTES {
        return Err(discovery_document_too_large());
    }
    Ok(resulting)
}

fn discovery_document_too_large() -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        format!(
            "complete discovery result exceeds the {MAX_ARTIFACT_BYTES}-byte Recovery Artifact ceiling"
        ),
    )
}

fn decimal_len(mut value: u64) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

fn escaped_field_len(value: &str) -> Result<usize, ResourceError> {
    let mut length = 0_usize;
    for character in value.chars() {
        let bytes = match character {
            '\\' | '\n' | '\r' | '\t' => 2,
            other => other.len_utf8(),
        };
        length = checked_document_add(length, bytes)?;
    }
    Ok(length)
}

fn push_escaped_field(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            other => output.push(other),
        }
    }
}

fn select_search_page(
    document: &CanonicalDocument,
    records: &[SearchRecord],
    diagnostics: &[DiscoveryDiagnostic],
    skip: usize,
    limits: SearchLimits,
) -> Result<SelectedPage, ResourceError> {
    let start = skip.min(records.len());
    let mut end = start;
    let mut page = String::new();
    let mut used_lines = 0_usize;
    while end < records.len() && end - start < limits.max_results {
        let range = &document.record_ranges[end];
        if !append_search_item(
            &mut page,
            &document.text[range.clone()],
            &mut used_lines,
            limits.text,
        ) {
            break;
        }
        end += 1;
    }

    let mut diagnostic_end = 0_usize;
    if end == records.len() {
        while diagnostic_end < diagnostics.len() {
            let range = &document.diagnostic_ranges[diagnostic_end];
            if !append_search_item(
                &mut page,
                &document.text[range.clone()],
                &mut used_lines,
                limits.text,
            ) {
                break;
            }
            diagnostic_end += 1;
        }
    }

    let omitted = start > 0 || end < records.len() || diagnostic_end < diagnostics.len();
    if page.is_empty() {
        let status = if records.is_empty() && diagnostics.is_empty() {
            "No matches found.\n"
        } else if start == records.len() && !records.is_empty() {
            "No matches remain after skip.\n"
        } else {
            "Results omitted; use recovery.\n"
        };
        let prefix = page_prefix_len(status, limits.text)?;
        page.push_str(&status[..prefix]);
    }

    Ok(SelectedPage {
        record_range: start..end,
        diagnostic_range: 0..diagnostic_end,
        next_record: (end < records.len()).then_some(end),
        omitted,
        text: page,
    })
}

fn append_search_item(
    page: &mut String,
    item: &str,
    used_lines: &mut usize,
    limits: TextLimits,
) -> bool {
    let columns = item.strip_suffix('\n').unwrap_or(item).chars().count();
    if page.len() + item.len() > limits.bytes()
        || *used_lines == limits.lines()
        || columns > limits.columns()
    {
        return false;
    }
    page.push_str(item);
    *used_lines += 1;
    true
}

fn select_glob_page(
    document: &CanonicalDocument,
    record_count: usize,
    diagnostic_count: usize,
    skip: usize,
    limits: GlobLimits,
) -> SelectedPage {
    let start = skip.min(record_count);
    let end = start.saturating_add(limits.max_results).min(record_count);
    let diagnostic_end = if end == record_count {
        diagnostic_count
    } else {
        0
    };
    let omitted = start > 0 || end < record_count || diagnostic_end < diagnostic_count;
    let mut text = String::new();
    for range in &document.record_ranges[start..end] {
        text.push_str(&document.text[range.clone()]);
    }
    for range in &document.diagnostic_ranges[..diagnostic_end] {
        text.push_str(&document.text[range.clone()]);
    }
    if text.is_empty() {
        if record_count == 0 && diagnostic_count == 0 {
            text.push_str("No matches found.\n");
        } else if start == record_count && record_count > 0 {
            text.push_str("No matches remain after skip.\n");
        } else {
            text.push_str("Results omitted; use recovery.\n");
        }
    }
    SelectedPage {
        record_range: start..end,
        diagnostic_range: 0..diagnostic_end,
        next_record: (end < record_count).then_some(end),
        omitted,
        text,
    }
}

fn group_search_records(records: &[SearchRecord]) -> Vec<SearchGroup> {
    let mut groups = Vec::<SearchGroup>::new();
    for record in records {
        if let Some(group) = groups
            .last_mut()
            .filter(|group| group.reference == record.reference)
        {
            group.lines.push(SearchLine {
                line: record.line,
                text: record.text.clone(),
            });
        } else {
            groups.push(SearchGroup {
                reference: record.reference.clone(),
                lines: vec![SearchLine {
                    line: record.line,
                    text: record.text.clone(),
                }],
            });
        }
    }
    groups
}

fn validate_result_limit(value: Option<usize>) -> Result<usize, ResourceError> {
    let value = value.unwrap_or(MAX_DISCOVERY_RESULTS);
    if value == 0 || value > MAX_DISCOVERY_RESULTS {
        return Err(ResourceError::new(
            ErrorCategory::LimitExceeded,
            format!("discovery result limit must be between 1 and {MAX_DISCOVERY_RESULTS}"),
        ));
    }
    Ok(value)
}

fn canonical_identity(reference: &PathReference) -> Result<String, ResourceError> {
    let canonical = match reference.address() {
        ResourceAddress::Workspace(WorkspaceAddress::Canonical { .. }) => {
            reference.projection().is_none()
        }
        ResourceAddress::Artifact(address) => {
            reference.projection().is_none()
                && PathReference::artifact(address.clone(), None)
                    .is_ok_and(|canonical| canonical.requested() == reference.requested())
        }
        ResourceAddress::Workspace(_) => false,
    };
    if !canonical {
        return Err(ResourceError::new(
            ErrorCategory::InvalidReference,
            "discovery record identity must be a canonical unprojected Resource reference",
        ));
    }
    Ok(reference.requested().to_owned())
}

fn ensure_discovery_live(
    session: &PathSession,
    operation: &OperationGuard,
) -> Result<(), ResourceError> {
    if !operation.is_active() {
        return Err(cancelled());
    }
    if !session.is_active() {
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
        "discovery continuation line is not representable",
    )
}

fn invalid_pattern(message: &'static str) -> ResourceError {
    ResourceError::new(ErrorCategory::InvalidPattern, message)
}

fn cancelled() -> ResourceError {
    ResourceError::new(
        ErrorCategory::Cancelled,
        "discovery operation was cancelled",
    )
}
