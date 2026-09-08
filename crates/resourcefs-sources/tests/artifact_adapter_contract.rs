use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use resourcefs_core::{
    DiscoveryEngine, ErrorCategory, GlobKind, GlobLimits, GlobOptions, GlobRequest, GlobTarget,
    MAX_ARTIFACT_BYTES, OperationGuard, PathReference, ProjectionSelector, ResourceAddress,
    SearchEngine, SearchLimits, SearchOptions, SearchRequest, SearchTarget, SourceAdapter,
    VersionTag, WorkspacePath, WorkspaceRootId,
};
use resourcefs_sources::{ArtifactSource, SessionStore};
use tempfile::TempDir;

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

struct Fixture {
    session: resourcefs_sources::StoredSession,
    source: ArtifactSource,
    root: String,
    content: String,
    _temporary: TempDir,
}

async fn fixture() -> Fixture {
    let temporary = TempDir::new().expect("temporary session cache");
    let store = SessionStore::open_with(
        resourcefs_sources::SessionStorageConfig::new(
            temporary.path(),
            resourcefs_sources::SESSION_CLEANUP_TTL.as_secs() as i64,
        )
        .expect("default session storage config"),
    )
    .await
    .expect("session store");
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("stored session");
    let content = "αlpha\r\nbeta\ngamma\r\ndelta".to_owned();
    let address = session
        .path_session()
        .retain(&content, &OperationGuard::new())
        .await
        .expect("artifact");
    let root = PathReference::artifact(address, None)
        .expect("artifact root")
        .requested()
        .to_owned();
    let source = ArtifactSource::new(session.path_session().clone());
    Fixture {
        _temporary: temporary,
        session,
        source,
        root,
        content,
    }
}

fn selected(root: &str, selector: &str) -> PathReference {
    let root = PathReference::parse(root).expect("artifact root");
    let resourcefs_core::ResourceAddress::Artifact(address) = root.address() else {
        panic!("artifact root");
    };
    PathReference::artifact(
        address.clone(),
        Some(ProjectionSelector::parse(selector).expect("selector")),
    )
    .expect("selected artifact")
}

#[tokio::test]
async fn artifact_root_raw_ranges_and_pages_match_immutable_bytes() {
    let fixture = fixture().await;
    let expected_tag = VersionTag::from_content(fixture.content.as_bytes());
    let controls = resourcefs_core::ReadAcquisitionLimits::default();
    let error = fixture
        .source
        .read(
            &PathReference::parse(&fixture.root).expect("root reference"),
            &OperationGuard::new(),
            Some(&controls),
        )
        .await
        .expect_err("direct artifact controls must be refused");
    assert_eq!(error.category(), ErrorCategory::UnsupportedProjection);
    assert_eq!(
        error.details().expect("typed refusal").reason(),
        resourcefs_core::ErrorReason::AcquisitionControlsUnsupported
    );

    let root = fixture
        .source
        .read(
            &PathReference::parse(&fixture.root).expect("root reference"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("root read");
    assert_eq!(root.canonical_reference(), fixture.root);
    assert_eq!(root.content(), fixture.content);
    assert_eq!(root.version_tag(), &expected_tag);
    assert_eq!(root.backing_file_uri(), None);
    assert!(!root.is_mutable());
    assert_eq!(
        root.artifact_origin(),
        Some(resourcefs_core::ArtifactProjectionOrigin::new(0, 1))
    );

    let raw = fixture
        .source
        .read(
            &selected(&fixture.root, "raw"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("raw read");
    assert_eq!(raw.content(), fixture.content);
    assert_eq!(raw.version_tag(), &expected_tag);

    let ranges = fixture
        .source
        .read(
            &selected(&fixture.root, "3-3,1-2,3-3"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("range read");
    assert_eq!(ranges.content(), "gamma\r\nαlpha\r\nbeta\ngamma\r\n");
    assert_eq!(ranges.version_tag(), &expected_tag);
    assert_eq!(ranges.artifact_origin(), None);

    let suffix = fixture
        .source
        .read(&selected(&fixture.root, "2-"), &OperationGuard::new(), None)
        .await
        .expect("suffix read");
    assert_eq!(suffix.content(), "beta\ngamma\r\ndelta");
    assert_eq!(
        suffix.artifact_origin(),
        Some(resourcefs_core::ArtifactProjectionOrigin::new(
            "αlpha\r\n".len(),
            2,
        ))
    );

    let page = fixture
        .source
        .read(
            &selected(&fixture.root, "page:2"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("page read");
    assert_eq!(page.content(), &fixture.content[2..]);
    assert_eq!(page.version_tag(), &expected_tag);
    assert_eq!(
        page.artifact_origin(),
        Some(resourcefs_core::ArtifactProjectionOrigin::new(2, 1))
    );
    assert!(!format!("{page:?}").contains(fixture._temporary.path().to_string_lossy().as_ref()));
}

#[tokio::test]
async fn invalid_page_and_non_artifact_authority_fail_without_disclosure() {
    let fixture = fixture().await;
    let invalid_boundary = fixture
        .source
        .read(
            &selected(&fixture.root, "page:1"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect_err("UTF-8 midpoint");
    assert_eq!(invalid_boundary.category(), ErrorCategory::InvalidReference);

    let end = fixture.content.len();
    let at_end = fixture
        .source
        .read(
            &selected(&fixture.root, &format!("page:{end}")),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect_err("end is not a progressing page");
    assert_eq!(at_end.category(), ErrorCategory::InvalidReference);

    let workspace = PathReference::canonical(
        WorkspaceRootId::new("workspace").expect("root ID"),
        WorkspacePath::new("file.txt").expect("workspace path"),
    );
    let wrong_source = fixture
        .source
        .read(&workspace, &OperationGuard::new(), None)
        .await
        .expect_err("workspace authority");
    assert_eq!(
        wrong_source.category(),
        ErrorCategory::UnsupportedProjection
    );
    assert!(
        !wrong_source
            .message()
            .contains(fixture._temporary.path().to_string_lossy().as_ref())
    );
}

#[tokio::test]
async fn disconnect_invalidates_all_artifact_projections() {
    let fixture = fixture().await;
    fixture.session.path_session().invalidate();
    for reference in [
        PathReference::parse(&fixture.root).expect("root"),
        selected(&fixture.root, "raw"),
        selected(&fixture.root, "2-"),
        selected(&fixture.root, "page:2"),
    ] {
        let error = fixture
            .source
            .read(&reference, &OperationGuard::new(), None)
            .await
            .expect_err("inactive artifact");
        assert_eq!(error.category(), ErrorCategory::NotFound);
    }
}

async fn retain(
    session: &resourcefs_sources::StoredSession,
    content: &str,
) -> resourcefs_core::ArtifactAddress {
    session
        .path_session()
        .retain(content, &OperationGuard::new())
        .await
        .expect("C8 retain fixture artifact")
}

fn canonical_artifact(address: resourcefs_core::ArtifactAddress) -> PathReference {
    PathReference::artifact(address, None).expect("C8 canonical Artifact")
}

#[tokio::test]
async fn search_and_glob_are_session_isolated() {
    let temporary = TempDir::new().expect("C8 temporary cache");
    let store = SessionStore::open_with(
        resourcefs_sources::SessionStorageConfig::new(
            temporary.path(),
            resourcefs_sources::SESSION_CLEANUP_TTL.as_secs() as i64,
        )
        .expect("default session storage config"),
    )
    .await
    .expect("C8 session store");
    let first = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("C8 first session");
    let foreign = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("C8 foreign session");

    let selected_address = retain(&first, "top\nneedle needle\nend\n").await;
    let second_address = retain(&first, "other\n").await;
    let foreign_address = retain(&foreign, "foreign sentinel needle\n").await;
    let selected = PathReference::artifact(
        selected_address.clone(),
        Some(ProjectionSelector::parse("2-").expect("C8 suffix selector")),
    )
    .expect("C8 selected Artifact");

    let source = Arc::new(ArtifactSource::new(first.path_session().clone()));
    let discovery = DiscoveryEngine::new(
        source,
        first.path_session().clone(),
        resourcefs_core::ServerLimits::default(),
    );
    let search_started = Instant::now();
    let search = discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(selected),
                "needle",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C8 search request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C8 selected search");
    assert!(
        search_started.elapsed() <= Duration::from_secs(10),
        "C8 search exceeded ten seconds: {:?}",
        search_started.elapsed()
    );
    assert_eq!(search.engine(), SearchEngine::RustRegex, "C8 search engine");
    assert_eq!(search.total_records(), 1, "C8 one matching line");
    assert_eq!(search.groups().len(), 1, "C8 one matching Artifact");
    assert_eq!(
        search.groups()[0].reference(),
        canonical_artifact(selected_address.clone()).requested(),
        "C8 canonical selected identity"
    );
    assert_eq!(search.groups()[0].lines()[0].line(), 2, "C8 root line");
    assert_eq!(
        search.groups()[0].lines()[0].text(),
        "needle needle",
        "C8 one row per line"
    );

    let mut expected = vec![
        canonical_artifact(selected_address).requested().to_owned(),
        canonical_artifact(second_address).requested().to_owned(),
    ];
    expected.sort_unstable();
    let glob_started = Instant::now();
    let glob = discovery
        .glob(
            GlobRequest::new(
                GlobTarget::new("artifact://*").expect("C8 Artifact glob"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C8 session glob");
    assert!(
        glob_started.elapsed() <= Duration::from_millis(250),
        "C8 glob exceeded 250 ms: {:?}",
        glob_started.elapsed()
    );
    let actual: Vec<_> = glob
        .entries()
        .iter()
        .map(|entry| entry.reference().to_owned())
        .collect();
    assert_eq!(actual, expected, "C8 current-session catalog only");
    assert!(
        !actual.contains(
            &canonical_artifact(foreign_address.clone())
                .requested()
                .to_owned()
        ),
        "C8 foreign sentinel absent"
    );

    let foreign_error = discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(canonical_artifact(foreign_address)),
                "needle",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C8 foreign request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("C8 foreign Artifact must be unavailable");
    assert_eq!(
        foreign_error.category(),
        ErrorCategory::NotFound,
        "C8 foreign category"
    );

    let maximum_content = "x".repeat(MAX_ARTIFACT_BYTES);
    let maximum_address = retain(&first, &maximum_content).await;
    drop(maximum_content);
    let allocation_baseline = begin_allocation_measurement();
    let maximum_started = Instant::now();
    let maximum_search = discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(canonical_artifact(maximum_address)),
                "absent-pattern",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C8 maximum search request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C8 maximum selected-Artifact search");
    let maximum_peak = measured_peak_bytes(allocation_baseline);
    assert_eq!(maximum_search.total_records(), 0, "C8 maximum no-match");
    assert!(
        maximum_started.elapsed() <= Duration::from_secs(10),
        "C8 maximum search exceeded ten seconds: {:?}",
        maximum_started.elapsed()
    );
    assert!(
        maximum_peak <= 96 * 1024 * 1024,
        "C8 maximum search transient allocation exceeded 96 MiB: {maximum_peak}"
    );
}

#[tokio::test]
async fn searches_selected_artifact_text() {
    let fixture = fixture().await;
    let content = "outside needle\r\nneedle alpha\r\ngap\r\nbeta needle\r\nneedle needle\r\n";
    let address = retain(&fixture.session, content).await;
    let sibling = retain(&fixture.session, "sibling needle\r\n").await;
    let selected = PathReference::artifact(
        address.clone(),
        Some(ProjectionSelector::parse("2-").expect("C6 suffix selector")),
    )
    .expect("C6 selected Artifact");

    let source = Arc::new(ArtifactSource::new(fixture.session.path_session().clone()));
    let discovery = DiscoveryEngine::new(
        source,
        fixture.session.path_session().clone(),
        resourcefs_core::ServerLimits::default(),
    );
    let search = discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(selected),
                "needle",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("C6 search request"),
            &OperationGuard::new(),
        )
        .await
        .expect("C6 selected search");

    assert_eq!(search.engine(), SearchEngine::RustRegex, "C6 search engine");
    assert_eq!(
        search.total_records(),
        3,
        "C6 one terminator-free row per matching selected line"
    );
    assert_eq!(search.groups().len(), 1, "C6 one selected Artifact group");
    assert_eq!(
        search.groups()[0].reference(),
        canonical_artifact(address.clone()).requested(),
        "C6 canonical selected identity"
    );
    assert!(
        !search
            .groups()
            .iter()
            .any(|group| group.reference() == canonical_artifact(sibling.clone()).requested()),
        "C6 sibling Artifact stays outside the search"
    );
    let expected_lines = [
        (2_u64, "needle alpha"),
        (4, "beta needle"),
        (5, "needle needle"),
    ];
    let rows = search.groups()[0].lines();
    assert_eq!(rows.len(), expected_lines.len(), "C6 per-line rows");
    for (index, (row, (line, text))) in rows.iter().zip(expected_lines).enumerate() {
        assert_eq!(row.line(), line, "C6 original line number for row {index}");
        assert_eq!(row.text(), text, "C6 row text for row {index}");
        assert!(
            !row.text().contains(['\r', '\n']),
            "C6 terminator-free row {index}"
        );
    }
}

#[tokio::test]
async fn glob_lists_session_artifacts() {
    let temporary = TempDir::new().expect("C7 temporary cache");
    let store = SessionStore::open_with(
        resourcefs_sources::SessionStorageConfig::new(
            temporary.path(),
            resourcefs_sources::SESSION_CLEANUP_TTL.as_secs() as i64,
        )
        .expect("default session storage config"),
    )
    .await
    .expect("C7 session store");
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("C7 session");
    let foreign = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("C7 foreign session");

    let contents = [
        "C7 ninth",
        "C7 second",
        "C7 eighth",
        "C7 first",
        "C7 seventh",
        "C7 third",
        "C7 sixth",
        "C7 fourth",
        "C7 tenth",
        "C7 fifth",
        "C7 eleventh",
    ];
    let mut expected = Vec::with_capacity(contents.len());
    for content in contents {
        expected.push(
            canonical_artifact(retain(&session, content).await)
                .requested()
                .to_owned(),
        );
    }
    expected.sort_unstable();
    let foreign_sentinel = retain(&foreign, "C7 foreign sentinel").await;

    let source = Arc::new(ArtifactSource::new(session.path_session().clone()));
    let discovery = DiscoveryEngine::new(
        source,
        session.path_session().clone(),
        resourcefs_core::ServerLimits::default(),
    );
    let glob = discovery
        .glob(
            GlobRequest::new(
                GlobTarget::new("artifact://*").expect("C7 Artifact glob"),
                GlobOptions::default(),
                0,
                GlobLimits::default(),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C7 session glob");

    let actual: Vec<_> = glob
        .entries()
        .iter()
        .map(|entry| entry.reference().to_owned())
        .collect();
    assert_eq!(actual, expected, "C7 canonical bytewise-order identities");
    assert_eq!(
        glob.total_records(),
        contents.len(),
        "C7 exact session catalog"
    );
    assert!(
        glob.entries()
            .iter()
            .all(|entry| entry.kind() == GlobKind::Artifact),
        "C7 every entry is an Artifact, no directories"
    );
    assert!(
        !actual.contains(&canonical_artifact(foreign_sentinel).requested().to_owned()),
        "C7 foreign-session sentinel absent"
    );
}

#[tokio::test]
async fn glob_snapshot_excludes_its_recovery_artifact() {
    let temporary = TempDir::new().expect("C8 temporary cache");
    let store = SessionStore::open_with(
        resourcefs_sources::SessionStorageConfig::new(
            temporary.path(),
            resourcefs_sources::SESSION_CLEANUP_TTL.as_secs() as i64,
        )
        .expect("default session storage config"),
    )
    .await
    .expect("C8 session store");
    let session = store
        .create_session(resourcefs_core::ServerLimits::default())
        .await
        .expect("C8 session");
    let mut expected = Vec::with_capacity(999);
    for index in 0..999 {
        expected.push(
            canonical_artifact(retain(&session, &format!("C8 object {index:04}")).await)
                .requested()
                .to_owned(),
        );
    }
    expected.sort_unstable();
    assert_eq!(
        session.path_session().artifact_count().await,
        999,
        "C8 pre-call catalog"
    );

    let source = Arc::new(ArtifactSource::new(session.path_session().clone()));
    let discovery = DiscoveryEngine::new(
        source,
        session.path_session().clone(),
        resourcefs_core::ServerLimits::default(),
    );
    let allocation_baseline = begin_allocation_measurement();
    let started = Instant::now();
    let result = discovery
        .glob(
            GlobRequest::new(
                GlobTarget::new("artifact://*").expect("C8 Artifact glob"),
                GlobOptions::default(),
                0,
                GlobLimits::new(Some(1)).expect("C8 one-entry page"),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("C8 spilling glob");
    let peak_bytes = measured_peak_bytes(allocation_baseline);
    assert!(
        started.elapsed() <= Duration::from_millis(250),
        "C8 999-entry glob exceeded 250 ms: {:?}",
        started.elapsed()
    );
    assert!(
        peak_bytes <= 1024 * 1024,
        "C8 glob transient allocation exceeded 1 MiB: {peak_bytes}"
    );
    assert_eq!(result.total_records(), 999, "C8 exact snapshot count");
    assert_eq!(result.returned_records(), 1, "C8 forced one-entry page");
    let recovery = result
        .recovery_reference()
        .expect("C8 spill Recovery Reference");
    let recovery = PathReference::parse(recovery).expect("C8 recovery parse");
    let ResourceAddress::Artifact(recovery_address) = recovery.address() else {
        panic!("C8 recovery Artifact");
    };
    let recovery_root = canonical_artifact(recovery_address.clone())
        .requested()
        .to_owned();
    assert!(
        !expected.contains(&recovery_root),
        "C8 recovery ID was not in the pre-call oracle"
    );
    assert!(
        result
            .entries()
            .iter()
            .all(|entry| entry.reference() != recovery_root),
        "C8 recovery Artifact excluded from its own result"
    );

    let recovered = session
        .path_session()
        .read_artifact(recovery_address)
        .await
        .expect("C8 recovered complete glob");
    let recovered_references: Vec<_> = recovered.lines().map(str::to_owned).collect();
    assert_eq!(
        recovered_references, expected,
        "C8 recovered pre-call catalog"
    );
    let post_catalog = session
        .path_session()
        .artifact_catalog()
        .await
        .expect("C8 post-call catalog");
    assert_eq!(post_catalog.len(), 1_000, "C8 one recovery object added");
    assert_eq!(
        post_catalog
            .last()
            .map(resourcefs_core::ArtifactAddress::object_id),
        Some(recovery_address.object_id()),
        "C8 recovery is the sole post-snapshot ID"
    );
}
