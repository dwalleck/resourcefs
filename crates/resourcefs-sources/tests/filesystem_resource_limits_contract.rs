use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs::{self, File},
    io::{BufWriter, Write as _},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use resourcefs_core::{
    DiscoveryAdapter, DiscoveryEngine, ErrorCategory, MAX_ARTIFACT_BYTES, OperationGuard,
    PathReference, SearchLimits, SearchOptions, SearchRequest, SearchTarget, WorkspaceRootId,
};
use resourcefs_sources::{
    ArtifactSource, BackingPathVisibility, FilesystemSource, LaunchRoot, LaunchRootSource,
    SessionStore,
};
use tempfile::TempDir;

struct CountingAllocator;

static CURRENT_ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK_ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static MEASUREMENT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the exact allocation request is delegated to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: this pointer and layout came from the delegated system allocation.
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

async fn filesystem(root: &std::path::Path) -> FilesystemSource {
    FilesystemSource::new(
        LaunchRootSource::Cli(vec![LaunchRoot {
            id: WorkspaceRootId::new("workspace").expect("C4 root ID"),
            path: root.to_owned(),
        }]),
        Some("workspace".to_owned()),
        BackingPathVisibility::Hidden,
    )
    .await
    .expect("C4 filesystem source")
}

fn reference(path: &str) -> PathReference {
    PathReference::parse(path).expect("C6 Workspace reference")
}

#[tokio::test]
async fn newline_free_files_are_rejected_before_unbounded_allocation() {
    let _measurement = MEASUREMENT_LOCK.lock().await;
    let temporary = TempDir::new().expect("C6 temporary Workspace");
    let root = temporary.path();
    File::create(root.join("oversized.txt"))
        .expect("C6 oversized file")
        .set_len((MAX_ARTIFACT_BYTES + 1) as u64)
        .expect("C6 sparse oversized file");
    let source = filesystem(root).await;

    let baseline = begin_allocation_measurement();
    let error = source
        .search(
            &SearchTarget::resource(reference("oversized.txt")),
            "needle",
            SearchOptions::default(),
            &OperationGuard::new(),
        )
        .await
        .expect_err("C6 oversized newline-free file must fail");
    let peak = measured_peak_bytes(baseline);
    assert_eq!(
        error.category(),
        ErrorCategory::LimitExceeded,
        "C6 oversized source category"
    );
    assert!(
        peak <= MAX_ARTIFACT_BYTES + 16 * 1024 * 1024,
        "C6 oversized line allocated beyond the bounded reader: {peak}"
    );
    fs::remove_file(root.join("oversized.txt")).expect("C6 remove first limit fixture");

    File::create(root.join(".gitignore"))
        .expect("C5 oversized gitignore")
        .set_len((MAX_ARTIFACT_BYTES + 1) as u64)
        .expect("C5 sparse oversized gitignore");
    fs::write(root.join("visible.txt"), "needle\n").expect("C5 visible fixture");
    let cache = TempDir::new().expect("C5 session cache");
    let store = SessionStore::open_with(
        resourcefs_sources::SessionStorageConfig::new(
            cache.path(),
            resourcefs_sources::SESSION_CLEANUP_TTL.as_secs() as i64,
        )
        .expect("default session storage config"),
    )
    .await
    .expect("C5 session store");
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("C5 session");
    let compiled = Arc::new(
        resourcefs_sources::CompiledSources::new(
            source,
            ArtifactSource::new(session.path_session().clone()),
        )
        .await
        .expect("C5 compiled sources"),
    );
    let engine = DiscoveryEngine::new(
        compiled,
        session.path_session().clone(),
        resourcefs_core::ServerLimits::default(),
    );

    let baseline = begin_allocation_measurement();
    let result = engine
        .search(
            SearchRequest::new(
                SearchTarget::primary(),
                "needle",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C5 gitignore request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C5 oversized gitignore is a partial failure");
    let peak = measured_peak_bytes(baseline);
    assert_eq!(
        result.total_records(),
        1,
        "C5 visible file remains searchable"
    );
    assert_eq!(result.diagnostics().len(), 1, "C5 one gitignore diagnostic");
    assert_eq!(
        result.diagnostics()[0].category(),
        ErrorCategory::LimitExceeded,
        "C5 oversized gitignore category"
    );
    assert!(
        peak <= MAX_ARTIFACT_BYTES + 16 * 1024 * 1024,
        "C5 oversized gitignore allocated beyond the bounded reader: {peak}"
    );
}

#[tokio::test]
async fn source_result_budget_precedes_owned_record_exhaustion() {
    let _measurement = MEASUREMENT_LOCK.lock().await;
    let temporary = TempDir::new().expect("C6 temporary Workspace");
    let root = temporary.path();
    let file = File::create(root.join("dense.txt")).expect("C6 dense file");
    let mut writer = BufWriter::new(file);
    let line = format!("{}\n", "x".repeat(2_047));
    for _ in 0..32_768 {
        writer.write_all(line.as_bytes()).expect("C6 dense line");
    }
    writer.flush().expect("C6 dense flush");
    drop(writer);
    assert_eq!(
        fs::metadata(root.join("dense.txt"))
            .expect("C6 dense metadata")
            .len(),
        MAX_ARTIFACT_BYTES as u64,
        "C6 exact source-byte fixture"
    );
    let source = filesystem(root).await;

    let baseline = begin_allocation_measurement();
    let error = source
        .search(
            &SearchTarget::resource(reference("dense.txt")),
            "x",
            SearchOptions::default(),
            &OperationGuard::new(),
        )
        .await
        .expect_err("C6 over-ceiling complete result must fail in the Source Adapter");
    let peak = measured_peak_bytes(baseline);
    assert_eq!(
        error.category(),
        ErrorCategory::LimitExceeded,
        "C6 complete source result category"
    );
    assert!(
        peak <= 128 * 1024 * 1024,
        "C6 Source Adapter exceeded its 128 MiB transient budget: {peak}"
    );
}

#[tokio::test]
async fn cumulative_pcre2_work_has_an_exact_subject_ceiling() {
    let _measurement = MEASUREMENT_LOCK.lock().await;
    let temporary = TempDir::new().expect("C3 temporary Workspace");
    let root = temporary.path();
    fs::write(root.join("subjects.txt"), "xy\n".repeat(10_000)).expect("C3 exact PCRE2 subjects");
    let source = filesystem(root).await;
    let target = SearchTarget::resource(reference("subjects.txt"));

    source
        .search(
            &target,
            r"(?<=x)y",
            SearchOptions::default(),
            &OperationGuard::new(),
        )
        .await
        .expect("C3 exact cumulative PCRE2 subject ceiling");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(root.join("subjects.txt"))
        .expect("C3 append subject");
    file.write_all(b"xy\n").expect("C3 one-over PCRE2 subject");
    drop(file);

    let error = source
        .search(
            &target,
            r"(?<=x)y",
            SearchOptions::default(),
            &OperationGuard::new(),
        )
        .await
        .expect_err("C3 one-over cumulative PCRE2 work must fail");
    assert_eq!(
        error.category(),
        ErrorCategory::LimitExceeded,
        "C3 cumulative PCRE2 category"
    );
}
