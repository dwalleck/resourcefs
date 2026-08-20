use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use resourcefs_core::{
    DiscoveryAdapter, DiscoveryDiagnostic, DiscoveryEngine, DiscoveryRetainGate, ErrorCategory,
    GlobEntry, GlobKind, GlobLimits, GlobOptions, GlobRequest, GlobTarget, MAX_ARTIFACT_BYTES,
    MAX_DISCOVERY_PATTERN_BYTES, MAX_DISCOVERY_RESULTS, MAX_TEXT_BYTES, MAX_TEXT_COLUMNS,
    MAX_TEXT_LINES, OperationGuard, PathReference, PathSession, ResourceAddress, ResourceError,
    SearchEngine, SearchLimits, SearchOptions, SearchRecord, SearchRequest, SearchSourceResult,
    SearchTarget, SessionStorage, SessionToken, SourceGlobResult, WorkspacePath, WorkspaceRootId,
};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};

struct CountingAllocator;

static CURRENT_ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: this allocator delegates the exact layout to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer and layout came from this allocator's delegated allocation.
        unsafe { System.dealloc(pointer, layout) };
        CURRENT_ALLOCATED.fetch_sub(layout.size(), Ordering::AcqRel);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the pointer and old layout came from this allocator; the new size is forwarded.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() {
            if new_size >= layout.size() {
                record_allocation(new_size - layout.size());
            } else {
                CURRENT_ALLOCATED.fetch_sub(layout.size() - new_size, Ordering::AcqRel);
            }
        }
        replacement
    }
}

fn record_allocation(bytes: usize) {
    let current = CURRENT_ALLOCATED.fetch_add(bytes, Ordering::AcqRel) + bytes;
    PEAK_ALLOCATED.fetch_max(current, Ordering::AcqRel);
}

fn begin_allocation_measurement() -> usize {
    let baseline = CURRENT_ALLOCATED.load(Ordering::Acquire);
    PEAK_ALLOCATED.store(baseline, Ordering::Release);
    baseline
}

fn measured_peak_bytes(baseline: usize) -> usize {
    PEAK_ALLOCATED
        .load(Ordering::Acquire)
        .saturating_sub(baseline)
}

#[derive(Default)]
struct MemoryStorage {
    content: Mutex<HashMap<u64, Vec<u8>>>,
}

impl MemoryStorage {
    async fn count(&self) -> usize {
        self.content.lock().await.len()
    }
}

#[async_trait]
impl SessionStorage for MemoryStorage {
    async fn content_equals(
        &self,
        id: resourcefs_core::ArtifactId,
        content: &[u8],
    ) -> Result<bool, ResourceError> {
        Ok(self
            .content
            .lock()
            .await
            .get(&id.get())
            .is_some_and(|stored| stored == content))
    }

    async fn write_atomic(
        &self,
        id: resourcefs_core::ArtifactId,
        content: &[u8],
    ) -> Result<(), ResourceError> {
        self.content.lock().await.insert(id.get(), content.to_vec());
        Ok(())
    }

    async fn read(&self, id: resourcefs_core::ArtifactId) -> Result<String, ResourceError> {
        let bytes = self
            .content
            .lock()
            .await
            .get(&id.get())
            .cloned()
            .ok_or_else(|| {
                ResourceError::new(ErrorCategory::NotFound, "missing fixture artifact")
            })?;
        String::from_utf8(bytes).map_err(|_| {
            ResourceError::new(ErrorCategory::SourceUnavailable, "invalid fixture UTF-8")
        })
    }

    async fn remove(&self, id: resourcefs_core::ArtifactId) -> Result<(), ResourceError> {
        self.content.lock().await.remove(&id.get());
        Ok(())
    }

    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}

#[derive(Default)]
struct CallGate {
    entered: Notify,
    release: Notify,
}

type SearchFactory = dyn Fn() -> SearchSourceResult + Send + Sync;
type GlobFactory = dyn Fn() -> SourceGlobResult + Send + Sync;

struct FakeAdapter {
    search_factory: Arc<SearchFactory>,
    glob_factory: Arc<GlobFactory>,
    search_calls: AtomicUsize,
    glob_calls: AtomicUsize,
    search_gate: Mutex<Option<Arc<CallGate>>>,
}

impl FakeAdapter {
    fn new(
        search_factory: impl Fn() -> SearchSourceResult + Send + Sync + 'static,
        glob_factory: impl Fn() -> SourceGlobResult + Send + Sync + 'static,
    ) -> Self {
        Self {
            search_factory: Arc::new(search_factory),
            glob_factory: Arc::new(glob_factory),
            search_calls: AtomicUsize::new(0),
            glob_calls: AtomicUsize::new(0),
            search_gate: Mutex::new(None),
        }
    }

    fn search_calls(&self) -> usize {
        self.search_calls.load(Ordering::Acquire)
    }

    fn glob_calls(&self) -> usize {
        self.glob_calls.load(Ordering::Acquire)
    }

    async fn arm_search_gate(&self) -> Arc<CallGate> {
        let gate = Arc::new(CallGate::default());
        *self.search_gate.lock().await = Some(Arc::clone(&gate));
        gate
    }
}

#[async_trait]
impl DiscoveryAdapter for FakeAdapter {
    async fn search(
        &self,
        _target: &SearchTarget,
        _pattern: &str,
        _options: SearchOptions,
        _operation: &OperationGuard,
    ) -> Result<SearchSourceResult, ResourceError> {
        self.search_calls.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = self.search_gate.lock().await.take() {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
        Ok((self.search_factory)())
    }

    async fn glob(
        &self,
        _target: &GlobTarget,
        _options: GlobOptions,
        _operation: &OperationGuard,
    ) -> Result<SourceGlobResult, ResourceError> {
        self.glob_calls.fetch_add(1, Ordering::AcqRel);
        Ok((self.glob_factory)())
    }
}

fn token(value: u8) -> SessionToken {
    SessionToken::parse(format!("{value:032x}")).expect("fixture session token")
}

fn session(value: u8) -> (PathSession, Arc<MemoryStorage>) {
    let storage = Arc::new(MemoryStorage::default());
    let trait_storage: Arc<dyn SessionStorage> = storage.clone();
    (PathSession::new(token(value), trait_storage), storage)
}

fn workspace_reference(path: &str) -> PathReference {
    PathReference::canonical(
        WorkspaceRootId::new("main").expect("fixture root"),
        WorkspacePath::new(path).expect("fixture path"),
    )
}

fn search_record(path: &str, line: u64, text: impl Into<String>) -> SearchRecord {
    SearchRecord::new(workspace_reference(path), line, text).expect("fixture search record")
}

fn glob_entry(path: &str, kind: GlobKind) -> GlobEntry {
    GlobEntry::new(workspace_reference(path), kind).expect("fixture glob entry")
}

fn diagnostic(path: Option<&str>, category: ErrorCategory, message: &str) -> DiscoveryDiagnostic {
    DiscoveryDiagnostic::new(path.map(workspace_reference), category, message)
}

fn empty_glob() -> SourceGlobResult {
    SourceGlobResult::new(Vec::new(), Vec::new())
}

fn engine(adapter: Arc<FakeAdapter>, path_session: PathSession) -> DiscoveryEngine {
    let trait_adapter: Arc<dyn DiscoveryAdapter> = adapter;
    DiscoveryEngine::new(trait_adapter, path_session)
}

fn search_request(
    pattern: impl Into<String>,
    skip: usize,
    limits: SearchLimits,
) -> Result<SearchRequest, ResourceError> {
    SearchRequest::new(
        SearchTarget::primary(),
        pattern,
        SearchOptions::default(),
        skip,
        limits,
    )
}

fn artifact_address(reference: &str) -> resourcefs_core::ArtifactAddress {
    let parsed = PathReference::parse(reference).expect("recovery Path Reference");
    match parsed.address() {
        ResourceAddress::Artifact(address) => address.clone(),
        ResourceAddress::Workspace(_) => panic!("recovery must name an Artifact"),
    }
}

fn escape_field(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn canonical_search_line(reference: &str, line: u64, text: &str) -> String {
    format!("{reference}\t{line}\t{}\n", escape_field(text))
}

fn canonical_glob_line(reference: &str, kind: GlobKind) -> String {
    let suffix = if matches!(kind, GlobKind::Directory) {
        "/"
    } else {
        ""
    };
    format!("{reference}{suffix}\n")
}

fn canonical_diagnostic_line(
    reference: Option<&str>,
    category: ErrorCategory,
    message: &str,
) -> String {
    format!(
        "!{}\t{}\t{}\n",
        reference.unwrap_or("-"),
        category.as_str(),
        escape_field(message)
    )
}

fn flatten_search(result: &resourcefs_core::SearchResult) -> Vec<(String, u64, String)> {
    result
        .groups()
        .iter()
        .flat_map(|group| {
            group.lines().iter().map(|line| {
                (
                    group.reference().to_owned(),
                    line.line(),
                    line.text().to_owned(),
                )
            })
        })
        .collect()
}

#[tokio::test]
async fn request_validation_precedes_adapter() {
    let (path_session, _) = session(20);
    let adapter = Arc::new(FakeAdapter::new(
        || {
            SearchSourceResult::new(
                SearchEngine::RustRegex,
                vec![search_record("valid.txt", 1, "match")],
                Vec::new(),
            )
        },
        || SourceGlobResult::new(vec![glob_entry("valid.txt", GlobKind::File)], Vec::new()),
    ));
    let discovery = engine(Arc::clone(&adapter), path_session);
    let search_defaults = SearchOptions::default();
    assert!(search_defaults.case_sensitive(), "C2 search case default");
    assert!(search_defaults.gitignore(), "C2 search gitignore default");
    assert!(!search_defaults.hidden(), "C2 search hidden default");
    let custom_search = SearchOptions::new(false, false, true);
    assert!(!custom_search.case_sensitive(), "C2 custom search case");
    assert!(!custom_search.gitignore(), "C2 custom search gitignore");
    assert!(custom_search.hidden(), "C2 custom search hidden");
    assert!(
        SearchTarget::primary().reference().is_none(),
        "C2 primary target representation"
    );
    let explicit_target = SearchTarget::resource(workspace_reference("valid.txt"));
    assert_eq!(
        explicit_target.reference().map(PathReference::requested),
        Some("rfs://workspace/main/valid.txt"),
        "C2 explicit target representation"
    );
    let glob_defaults = GlobOptions::default();
    assert!(glob_defaults.case_sensitive(), "C2 glob case default");
    assert!(glob_defaults.gitignore(), "C2 glob gitignore default");
    assert!(!glob_defaults.hidden(), "C2 glob hidden default");
    let custom_glob = GlobOptions::new(false, false, true);
    assert!(!custom_glob.case_sensitive(), "C2 custom glob case");
    assert!(!custom_glob.gitignore(), "C2 custom glob gitignore");
    assert!(custom_glob.hidden(), "C2 custom glob hidden");
    let diagnostic_contract =
        diagnostic(Some("valid.txt"), ErrorCategory::PermissionDenied, "denied");
    assert_eq!(
        diagnostic_contract.reference(),
        Some("rfs://workspace/main/valid.txt"),
        "C2 diagnostic reference"
    );
    assert_eq!(
        diagnostic_contract.category(),
        ErrorCategory::PermissionDenied,
        "C2 diagnostic category"
    );
    assert_eq!(
        diagnostic_contract.message(),
        "denied",
        "C2 diagnostic message"
    );
    let artifact_reference = PathReference::artifact(
        resourcefs_core::ArtifactAddress::new(token(99).as_str(), 1).expect("C2 artifact identity"),
        None,
    )
    .expect("C2 artifact reference");
    GlobEntry::new(artifact_reference.clone(), GlobKind::Artifact)
        .expect("C2 Artifact kind matches Artifact identity");
    assert_eq!(
        GlobEntry::new(artifact_reference, GlobKind::File)
            .expect_err("C2 file kind cannot name Artifact")
            .category(),
        ErrorCategory::InvalidReference,
        "C2 Artifact kind mismatch"
    );
    assert_eq!(
        GlobEntry::new(workspace_reference("valid.txt"), GlobKind::Artifact)
            .expect_err("C2 Artifact kind cannot name Workspace file")
            .category(),
        ErrorCategory::InvalidReference,
        "C2 Workspace kind mismatch"
    );

    let exact_pattern = "a".repeat(MAX_DISCOVERY_PATTERN_BYTES);
    let exact_request = search_request(exact_pattern, 0, SearchLimits::default())
        .expect("C2 exact pattern boundary");
    discovery
        .search(exact_request, &OperationGuard::new())
        .await
        .expect("C2 valid search reaches adapter");
    assert_eq!(adapter.search_calls(), 1, "C2 exact pattern adapter entry");

    let over_pattern = "a".repeat(MAX_DISCOVERY_PATTERN_BYTES + 1);
    let error = match search_request(over_pattern, 0, SearchLimits::default()) {
        Err(error) => error,
        Ok(request) => {
            let _ = discovery.search(request, &OperationGuard::new()).await;
            assert_eq!(
                adapter.search_calls(),
                1,
                "C2 invalid pattern reached adapter"
            );
            panic!("C2 one-over pattern was accepted");
        }
    };
    assert_eq!(
        error.category(),
        ErrorCategory::LimitExceeded,
        "C2 pattern category"
    );
    assert_eq!(
        adapter.search_calls(),
        1,
        "C2 invalid pattern reached adapter"
    );

    let empty = search_request("", 0, SearchLimits::default()).expect_err("C2 empty pattern");
    assert_eq!(
        empty.category(),
        ErrorCategory::InvalidPattern,
        "C2 empty category"
    );
    assert_eq!(
        adapter.search_calls(),
        1,
        "C2 empty pattern reached adapter"
    );

    let exact_limits = SearchLimits::new(
        Some(MAX_DISCOVERY_RESULTS),
        Some(MAX_TEXT_BYTES),
        Some(MAX_TEXT_LINES),
        Some(MAX_TEXT_COLUMNS),
    )
    .expect("C2 exact search limits");
    discovery
        .search(
            search_request("match", 0, exact_limits).expect("C2 exact request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C2 exact limits reach adapter");
    assert_eq!(adapter.search_calls(), 2, "C2 exact limit adapter entry");

    for (name, invalid) in [
        (
            "maxResults zero",
            SearchLimits::new(Some(0), None, None, None),
        ),
        (
            "maxResults one-over",
            SearchLimits::new(Some(MAX_DISCOVERY_RESULTS + 1), None, None, None),
        ),
        (
            "maxBytes zero",
            SearchLimits::new(None, Some(0), None, None),
        ),
        (
            "maxBytes one-over",
            SearchLimits::new(None, Some(MAX_TEXT_BYTES + 1), None, None),
        ),
        (
            "maxLines zero",
            SearchLimits::new(None, None, Some(0), None),
        ),
        (
            "maxLines one-over",
            SearchLimits::new(None, None, Some(MAX_TEXT_LINES + 1), None),
        ),
        (
            "maxColumns zero",
            SearchLimits::new(None, None, None, Some(0)),
        ),
        (
            "maxColumns one-over",
            SearchLimits::new(None, None, None, Some(MAX_TEXT_COLUMNS + 1)),
        ),
    ] {
        let error = invalid.expect_err(name);
        assert_eq!(error.category(), ErrorCategory::LimitExceeded, "C2 {name}");
        assert_eq!(adapter.search_calls(), 2, "C2 {name} reached adapter");
    }

    let glob_target = GlobTarget::new("**/*.txt").expect("C2 valid glob target");
    assert_eq!(glob_target.pattern(), "**/*.txt", "C2 glob pattern");
    let exact_glob_limits =
        GlobLimits::new(Some(MAX_DISCOVERY_RESULTS)).expect("C2 exact glob limit");
    discovery
        .glob(
            GlobRequest::new(glob_target, GlobOptions::default(), 0, exact_glob_limits),
            &OperationGuard::new(),
        )
        .await
        .expect("C2 exact glob reaches adapter");
    assert_eq!(adapter.glob_calls(), 1, "C2 exact glob adapter entry");

    let empty_glob = GlobTarget::new("").expect_err("C2 empty glob");
    assert_eq!(
        empty_glob.category(),
        ErrorCategory::InvalidPattern,
        "C2 empty glob category"
    );
    assert_eq!(
        GlobLimits::new(Some(MAX_DISCOVERY_RESULTS + 1))
            .expect_err("C2 one-over glob limit")
            .category(),
        ErrorCategory::LimitExceeded,
        "C2 glob limit category"
    );
    assert_eq!(adapter.glob_calls(), 1, "C2 invalid glob reached adapter");
}

#[tokio::test]
async fn lower_only_limits_and_skip() {
    let search_factory = || {
        SearchSourceResult::new(
            SearchEngine::RustRegex,
            vec![
                search_record("b.txt", 2, "bravo"),
                search_record("a.txt", 2, "alpha two"),
                search_record("a.txt", 1, "alpha one"),
                search_record("a.txt", 1, "alpha one"),
            ],
            Vec::new(),
        )
    };
    let glob_factory = || {
        SourceGlobResult::new(
            vec![
                glob_entry("z", GlobKind::Directory),
                glob_entry("a.txt", GlobKind::File),
                glob_entry("m.bin", GlobKind::File),
                glob_entry("a.txt", GlobKind::File),
            ],
            Vec::new(),
        )
    };
    let (path_session, storage) = session(21);
    let adapter = Arc::new(FakeAdapter::new(search_factory, glob_factory));
    let discovery = engine(adapter, path_session.clone());

    let expected_records = vec![
        (
            "rfs://workspace/main/a.txt".to_owned(),
            1,
            "alpha one".to_owned(),
        ),
        (
            "rfs://workspace/main/a.txt".to_owned(),
            2,
            "alpha two".to_owned(),
        ),
        (
            "rfs://workspace/main/b.txt".to_owned(),
            2,
            "bravo".to_owned(),
        ),
    ];
    let expected_complete = expected_records
        .iter()
        .map(|(reference, line, text)| canonical_search_line(reference, *line, text))
        .collect::<String>();

    let full = discovery
        .search(
            search_request("alpha", 0, SearchLimits::default()).expect("C9 full request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C9 full result");
    assert_eq!(
        flatten_search(&full),
        expected_records,
        "C9 canonical record order"
    );
    assert_eq!(full.text(), expected_complete, "C9 complete inline text");
    assert_eq!(full.returned_records(), 3, "C9 full returned count");
    assert_eq!(full.total_records(), 3, "C9 full total count");
    assert!(
        full.recovery_reference().is_none(),
        "C9 false full recovery"
    );

    let one = discovery
        .search(
            search_request(
                "alpha",
                1,
                SearchLimits::new(Some(1), None, None, None).expect("C9 one-result limit"),
            )
            .expect("C9 skipped request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C9 skipped page");
    assert_eq!(
        flatten_search(&one),
        vec![expected_records[1].clone()],
        "C9 skip page"
    );
    assert_eq!(one.returned_records(), 1, "C9 skipped returned count");
    assert_eq!(one.total_records(), 3, "C9 skipped total count");
    let recovery = one.recovery_reference().expect("C9 skipped recovery");
    let recovered = path_session
        .read_artifact(&artifact_address(recovery))
        .await
        .expect("C9 recovery read");
    assert_eq!(
        recovered, expected_complete,
        "C9 complete recovery document"
    );
    assert!(
        one.continuation_reference()
            .is_some_and(|reference| reference.ends_with(":3-")),
        "C9 continuation must begin at the third canonical record"
    );

    let skipped_all = discovery
        .search(
            search_request("alpha", 3, SearchLimits::default()).expect("C9 skip-total request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C9 skip-total result");
    assert_eq!(
        skipped_all.returned_records(),
        0,
        "C9 skip-total returned count"
    );
    assert!(
        skipped_all.recovery_reference().is_some(),
        "C9 skip-total recovery"
    );
    assert!(
        skipped_all.continuation_reference().is_none(),
        "C9 skip-total continuation"
    );

    let first_line = canonical_search_line(
        &expected_records[0].0,
        expected_records[0].1,
        &expected_records[0].2,
    );
    for (name, limits, expected_returned) in [
        (
            "exact bytes",
            SearchLimits::new(None, Some(first_line.len()), None, None).expect("C9 exact bytes"),
            1,
        ),
        (
            "one-under bytes",
            SearchLimits::new(None, Some(first_line.len() - 1), None, None)
                .expect("C9 one-under bytes"),
            0,
        ),
        (
            "one line",
            SearchLimits::new(None, None, Some(1), None).expect("C9 one line"),
            1,
        ),
        (
            "exact columns",
            SearchLimits::new(
                None,
                None,
                None,
                Some(first_line.trim_end().chars().count()),
            )
            .expect("C9 exact columns"),
            3,
        ),
        (
            "one-under columns",
            SearchLimits::new(
                None,
                None,
                None,
                Some(first_line.trim_end().chars().count() - 1),
            )
            .expect("C9 one-under columns"),
            0,
        ),
    ] {
        let result = discovery
            .search(
                search_request("alpha", 0, limits).expect("C9 bounded request"),
                &OperationGuard::new(),
            )
            .await
            .expect("C9 bounded result");
        assert_eq!(result.returned_records(), expected_returned, "C9 {name}");
        assert_eq!(
            result.recovery_reference().is_some(),
            expected_returned < expected_records.len(),
            "C9 {name} recovery"
        );
    }

    let glob = discovery
        .glob(
            GlobRequest::new(
                GlobTarget::new("**").expect("C9 glob target"),
                GlobOptions::default(),
                1,
                GlobLimits::new(Some(1)).expect("C9 glob limit"),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C9 glob page");
    assert_eq!(glob.returned_records(), 1, "C9 glob returned count");
    assert_eq!(glob.total_records(), 3, "C9 glob total count");
    assert_eq!(glob.entries()[0].reference(), "rfs://workspace/main/m.bin");
    assert_eq!(glob.entries()[0].kind(), GlobKind::File);
    assert_eq!(
        glob.text(),
        canonical_glob_line("rfs://workspace/main/m.bin", GlobKind::File),
        "C9 bounded glob text"
    );
    assert!(glob.diagnostics().is_empty(), "C9 bounded glob diagnostics");
    assert!(
        glob.continuation_reference()
            .is_some_and(|reference| reference.ends_with(":3-")),
        "C9 bounded glob continuation"
    );
    let glob_recovered = path_session
        .read_artifact(&artifact_address(
            glob.recovery_reference().expect("C9 glob recovery"),
        ))
        .await
        .expect("C9 glob recovery read");
    let expected_glob = [
        canonical_glob_line("rfs://workspace/main/a.txt", GlobKind::File),
        canonical_glob_line("rfs://workspace/main/m.bin", GlobKind::File),
        canonical_glob_line("rfs://workspace/main/z", GlobKind::Directory),
    ]
    .concat();
    assert_eq!(glob_recovered, expected_glob, "C9 complete glob recovery");
    assert_eq!(
        storage.count().await,
        2,
        "C9 one deduplicated spill per complete document"
    );
}

#[tokio::test]
async fn bounded_pages_recover_the_complete_document() {
    let (path_session, storage) = session(22);
    let mut records = (1..=1_001)
        .rev()
        .map(|line| search_record("many.txt", line, format!("row {line}")))
        .collect::<Vec<_>>();
    records.push(search_record("many.txt", 500, "row 500"));
    let diagnostics = vec![
        diagnostic(Some("z.bin"), ErrorCategory::SourceUnavailable, "gone\nnow"),
        diagnostic(Some("a.bin"), ErrorCategory::PermissionDenied, "denied"),
        diagnostic(Some("a.bin"), ErrorCategory::PermissionDenied, "denied"),
    ];
    let source = SearchSourceResult::new(SearchEngine::Pcre2, records, diagnostics);
    let adapter = Arc::new(FakeAdapter::new(move || source.clone(), empty_glob));
    let discovery = engine(adapter, path_session.clone());
    let result = discovery
        .search(
            search_request(
                "(?<=row )\\d+",
                0,
                SearchLimits::new(Some(1_000), None, None, None).expect("C9 page limit"),
            )
            .expect("C9 page request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C9 bounded page");
    assert_eq!(result.engine(), SearchEngine::Pcre2, "C9 engine identity");
    assert_eq!(result.returned_records(), 1_000, "C9 inline page count");
    assert_eq!(result.total_records(), 1_001, "C9 complete record count");
    assert_eq!(
        result.diagnostics().len(),
        0,
        "C9 omitted diagnostics stay in recovery"
    );

    let expected_records = (1..=1_001)
        .map(|line| {
            canonical_search_line(
                "rfs://workspace/main/many.txt",
                line,
                &format!("row {line}"),
            )
        })
        .collect::<String>();
    let expected_diagnostics = [
        canonical_diagnostic_line(
            Some("rfs://workspace/main/a.bin"),
            ErrorCategory::PermissionDenied,
            "denied",
        ),
        canonical_diagnostic_line(
            Some("rfs://workspace/main/z.bin"),
            ErrorCategory::SourceUnavailable,
            "gone\nnow",
        ),
    ]
    .concat();
    let expected = expected_records + &expected_diagnostics;
    let recovered = path_session
        .read_artifact(&artifact_address(
            result.recovery_reference().expect("C9 page recovery"),
        ))
        .await
        .expect("C9 page recovery read");
    assert_eq!(recovered, expected, "C9 recovery is complete and canonical");
    assert_eq!(
        Sha256::digest(recovered.as_bytes()),
        Sha256::digest(expected.as_bytes()),
        "C9 independent recovery digest"
    );

    let exact_reference = workspace_reference("ceiling.txt");
    let empty_line = canonical_search_line(exact_reference.requested(), 1, "");
    let exact_text = "x".repeat(MAX_ARTIFACT_BYTES - empty_line.len());
    let exact_source = SearchSourceResult::new(
        SearchEngine::RustRegex,
        vec![
            SearchRecord::new(exact_reference.clone(), 1, exact_text)
                .expect("C9 exact ceiling record"),
        ],
        Vec::new(),
    );
    let exact_adapter = Arc::new(FakeAdapter::new(move || exact_source.clone(), empty_glob));
    let exact_engine = engine(exact_adapter, path_session.clone());
    let exact = exact_engine
        .search(
            search_request("x", 0, SearchLimits::default()).expect("C9 exact ceiling request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C9 exact 64 MiB result");
    let exact_recovered = path_session
        .read_artifact(&artifact_address(
            exact
                .recovery_reference()
                .expect("C9 exact ceiling recovery"),
        ))
        .await
        .expect("C9 exact ceiling read");
    assert_eq!(
        exact_recovered.len(),
        MAX_ARTIFACT_BYTES,
        "C9 exact 64 MiB recovery"
    );

    let before_oversize = storage.count().await;
    let oversize_text = "x".repeat(MAX_ARTIFACT_BYTES + 1 - empty_line.len());
    let oversize_source = SearchSourceResult::new(
        SearchEngine::RustRegex,
        vec![SearchRecord::new(exact_reference, 1, oversize_text).expect("C9 one-over record")],
        Vec::new(),
    );
    let oversize_adapter = Arc::new(FakeAdapter::new(
        move || oversize_source.clone(),
        empty_glob,
    ));
    let oversize_engine = engine(oversize_adapter, path_session);
    let error = oversize_engine
        .search(
            search_request("x", 0, SearchLimits::default()).expect("C9 one-over request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("C9 one byte over 64 MiB");
    assert_eq!(
        error.category(),
        ErrorCategory::LimitExceeded,
        "C9 one-over category"
    );
    assert_eq!(
        storage.count().await,
        before_oversize,
        "C9 one-over published artifact"
    );
}

#[tokio::test]
async fn cancelled_operations_never_retain() {
    let source = || {
        SearchSourceResult::new(
            SearchEngine::RustRegex,
            vec![
                search_record("cancel.txt", 1, "first"),
                search_record("cancel.txt", 2, "second"),
            ],
            Vec::new(),
        )
    };

    let (cancel_session, cancel_storage) = session(23);
    let cancel_adapter = Arc::new(FakeAdapter::new(source, empty_glob));
    let cancel_gate = cancel_adapter.arm_search_gate().await;
    let cancel_engine = engine(cancel_adapter, cancel_session);
    let cancel_operation = OperationGuard::new();
    let pending_operation = cancel_operation.clone();
    let pending = tokio::spawn(async move {
        cancel_engine
            .search(
                search_request(
                    "first",
                    0,
                    SearchLimits::new(Some(1), None, None, None).expect("C10 cancel limit"),
                )
                .expect("C10 cancel request"),
                &pending_operation,
            )
            .await
    });
    cancel_gate.entered.notified().await;
    cancel_operation.cancel();
    cancel_gate.release.notify_one();
    let error = pending
        .await
        .expect("C10 cancel task")
        .expect_err("C10 cancelled discovery");
    assert_eq!(
        error.category(),
        ErrorCategory::Cancelled,
        "C10 cancel category"
    );
    assert_eq!(
        cancel_storage.count().await,
        0,
        "C10 cancelled artifact publication"
    );

    let (retain_session, retain_storage) = session(26);
    let retain_adapter = Arc::new(FakeAdapter::new(source, empty_glob));
    let retain_gate = Arc::new(DiscoveryRetainGate::new());
    let retain_engine =
        engine(retain_adapter, retain_session).with_retain_gate_for_test(Arc::clone(&retain_gate));
    let retain_operation = OperationGuard::new();
    let pending_operation = retain_operation.clone();
    let pending = tokio::spawn(async move {
        retain_engine
            .search(
                search_request(
                    "first",
                    0,
                    SearchLimits::new(Some(1), None, None, None).expect("C10 pre-retain limit"),
                )
                .expect("C10 pre-retain request"),
                &pending_operation,
            )
            .await
    });
    retain_gate.wait_until_entered().await;
    retain_operation.cancel();
    retain_gate.release();
    let error = pending
        .await
        .expect("C10 pre-retain task")
        .expect_err("C10 pre-retain cancellation");
    assert_eq!(
        error.category(),
        ErrorCategory::Cancelled,
        "C10 pre-retain cancellation category"
    );
    assert_eq!(
        retain_storage.count().await,
        0,
        "C10 pre-retain artifact publication"
    );

    let (disconnect_session, disconnect_storage) = session(24);
    let disconnect_adapter = Arc::new(FakeAdapter::new(source, empty_glob));
    let disconnect_gate = disconnect_adapter.arm_search_gate().await;
    let disconnect_engine = engine(disconnect_adapter, disconnect_session.clone());
    let pending = tokio::spawn(async move {
        disconnect_engine
            .search(
                search_request(
                    "first",
                    0,
                    SearchLimits::new(Some(1), None, None, None).expect("C10 disconnect limit"),
                )
                .expect("C10 disconnect request"),
                &OperationGuard::new(),
            )
            .await
    });
    disconnect_gate.entered.notified().await;
    disconnect_session.invalidate();
    disconnect_gate.release.notify_one();
    let error = pending
        .await
        .expect("C10 disconnect task")
        .expect_err("C10 disconnected discovery");
    assert_eq!(
        error.category(),
        ErrorCategory::SourceUnavailable,
        "C10 disconnect category"
    );
    assert_eq!(
        disconnect_storage.count().await,
        0,
        "C10 disconnected artifact publication"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hundred_thousand_resources_remain_bounded_and_recoverable() {
    let scale_factory = || {
        let mut records = Vec::with_capacity(20_000);
        for resource in (0..100_000).rev() {
            if resource % 10 == 0 {
                let path = format!("resource-{resource:06}.txt");
                let text = format!("match {resource:06}");
                records.push(search_record(&path, 1, &text));
                records.push(search_record(&path, 1, text));
            }
        }
        SearchSourceResult::new(
            SearchEngine::RustRegex,
            records,
            vec![
                diagnostic(Some("z.bin"), ErrorCategory::SourceUnavailable, "gone"),
                diagnostic(Some("a.bin"), ErrorCategory::PermissionDenied, "denied"),
                diagnostic(Some("a.bin"), ErrorCategory::PermissionDenied, "denied"),
            ],
        )
    };
    let (path_session, _) = session(25);
    let adapter = Arc::new(FakeAdapter::new(scale_factory, empty_glob));
    let discovery = engine(adapter, path_session.clone());
    let request = search_request(
        "match",
        0,
        SearchLimits::new(Some(100), None, None, None).expect("C13 result limit"),
    )
    .expect("C13 stress request");

    let baseline = begin_allocation_measurement();
    let started = Instant::now();
    let result = discovery
        .search(request, &OperationGuard::new())
        .await
        .expect("C13 stress result");
    let elapsed = started.elapsed();
    let peak_bytes = measured_peak_bytes(baseline);

    assert_eq!(result.returned_records(), 100, "C13 inline record count");
    assert_eq!(result.total_records(), 10_000, "C13 total record count");
    assert!(
        elapsed <= Duration::from_secs(2),
        "C13 engine wall budget: {elapsed:?}"
    );
    assert!(
        peak_bytes <= 256 * 1024 * 1024,
        "C13 counted peak allocation exceeded 256 MiB: {peak_bytes}"
    );

    let recovered = path_session
        .read_artifact(&artifact_address(
            result.recovery_reference().expect("C13 recovery reference"),
        ))
        .await
        .expect("C13 recovery read");
    let mut oracle = Sha256::new();
    let mut expected_records = 0_usize;
    for resource in (0..100_000).step_by(10) {
        let line =
            format!("rfs://workspace/main/resource-{resource:06}.txt\t1\tmatch {resource:06}\n");
        oracle.update(line.as_bytes());
        expected_records += 1;
    }
    oracle.update(
        canonical_diagnostic_line(
            Some("rfs://workspace/main/a.bin"),
            ErrorCategory::PermissionDenied,
            "denied",
        )
        .as_bytes(),
    );
    oracle.update(
        canonical_diagnostic_line(
            Some("rfs://workspace/main/z.bin"),
            ErrorCategory::SourceUnavailable,
            "gone",
        )
        .as_bytes(),
    );
    assert_eq!(expected_records, 10_000, "C13 oracle record count");
    assert_eq!(
        Sha256::digest(recovered.as_bytes()),
        oracle.finalize(),
        "C13 independent canonical recovery digest"
    );
}
