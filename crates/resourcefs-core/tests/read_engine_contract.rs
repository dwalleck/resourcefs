use std::{
    collections::HashMap,
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use resourcefs_core::{
    ArtifactAddress, ArtifactId, ArtifactProjectionOrigin, DisplayedLineRange, ErrorCategory,
    LineSelector, MAX_ARTIFACT_BYTES, OperationGuard, PathReference, PathSession,
    ProjectionSelector, ReadEngine, ReadRequest, ResourceAddress, ResourceError, ServerLimits,
    ServerLimitsInput, SessionStorage, SessionToken, SourceAdapter, SourceResource, TextLimitInput,
    TextLimits, VersionTag, WorkspacePath, WorkspaceRootId, select_utf8,
};
use tokio::sync::Mutex;

#[derive(Default)]
struct MemoryStorage {
    objects: Mutex<HashMap<ArtifactId, Vec<u8>>>,
    write_calls: AtomicUsize,
}

#[async_trait]
impl SessionStorage for MemoryStorage {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError> {
        Ok(self
            .objects
            .lock()
            .await
            .get(&id)
            .is_some_and(|stored| stored == content))
    }

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError> {
        self.write_calls.fetch_add(1, Ordering::SeqCst);
        self.objects.lock().await.insert(id, content.to_vec());
        Ok(())
    }

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError> {
        let bytes = self
            .objects
            .lock()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(not_found)?;
        String::from_utf8(bytes).map_err(|_| not_found())
    }

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError> {
        self.objects.lock().await.remove(&id);
        Ok(())
    }

    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}

#[derive(Clone)]
struct SessionBackedSource {
    workspace_content: Arc<String>,
    session: PathSession,
}

#[async_trait]
impl SourceAdapter for SessionBackedSource {
    async fn read(
        &self,
        reference: &PathReference,
        _operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        match reference.address() {
            ResourceAddress::Catalog(_) => Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "test source does not implement catalog Resources",
            )),
            ResourceAddress::Workspace(_) => {
                let projection = reference.projection().or_else(|| {
                    reference
                        .selector_candidate()
                        .map(|candidate| candidate.selector())
                });
                match projection {
                    Some(projection) => {
                        let selected = select_utf8(
                            Cursor::new(self.workspace_content.as_bytes()),
                            Some(projection),
                        )?;
                        SourceResource::selected_text(workspace_reference(), selected)
                    }
                    None => SourceResource::text(
                        workspace_reference(),
                        self.workspace_content.as_str().to_owned(),
                    ),
                }
            }
            ResourceAddress::Artifact(address) => self.read_artifact(reference, address).await,
            ResourceAddress::Local(_) => Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "test source does not implement Session Scratch Resources",
            )),
            ResourceAddress::Https(_) => Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "test source does not implement HTTPS Resources",
            )),
            ResourceAddress::Jira(_) => Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "test source does not implement Jira Resources",
            )),
            ResourceAddress::Issue(_) | ResourceAddress::PullRequest(_) => Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "test source does not implement GitHub Resources",
            )),
        }
    }
}

impl SessionBackedSource {
    async fn read_artifact(
        &self,
        reference: &PathReference,
        address: &ArtifactAddress,
    ) -> Result<SourceResource, ResourceError> {
        let canonical = PathReference::artifact(address.clone(), None)?;
        let mut content = self.session.read_artifact(address).await?;
        let projection = reference.projection();
        let (content, version_tag, origin) =
            if let Some(offset) = projection.and_then(ProjectionSelector::page_offset) {
                let offset = usize::try_from(offset).map_err(|_| invalid_page())?;
                if offset >= content.len() || !content.is_char_boundary(offset) {
                    return Err(invalid_page());
                }
                let line = content.as_bytes()[..offset]
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count() as u64
                    + 1;
                let tag = VersionTag::from_content(content.as_bytes());
                content.drain(..offset);
                (
                    content,
                    tag,
                    Some(ArtifactProjectionOrigin::new(offset, line)),
                )
            } else if let Some(lines) = projection.and_then(ProjectionSelector::line_selection) {
                let origin = suffix_origin(&content, lines);
                let selected = select_utf8(Cursor::new(content), projection)?;
                let (content, version_tag, _) = selected.into_parts();
                (content, version_tag, origin)
            } else {
                let tag = VersionTag::from_content(content.as_bytes());
                (content, tag, Some(ArtifactProjectionOrigin::new(0, 1)))
            };
        let resource = SourceResource::text_projection(canonical, content, version_tag)?;
        match origin {
            Some(origin) => resource.with_artifact_origin(origin),
            None => Ok(resource),
        }
    }
}

struct Harness {
    engine: ReadEngine,
    session: PathSession,
    storage: Arc<MemoryStorage>,
}

impl Harness {
    fn new(content: String) -> Self {
        Self::with_limits(content, ServerLimits::default())
    }

    fn with_limits(content: String, limits: ServerLimits) -> Self {
        let storage = Arc::new(MemoryStorage::default());
        let session = PathSession::new(
            SessionToken::parse("00000000000000000000000000000042").expect("session token"),
            storage.clone(),
            limits,
        );
        let source: Arc<dyn SourceAdapter> = Arc::new(SessionBackedSource {
            workspace_content: Arc::new(content),
            session: session.clone(),
        });
        Self {
            engine: ReadEngine::new(source, session.clone(), limits),
            session,
            storage,
        }
    }

    async fn read(
        &self,
        reference: PathReference,

        limits: TextLimits,
    ) -> resourcefs_core::ReadResource {
        self.engine
            .read(
                ReadRequest {
                    reference,
                    limits,
                    numbered: false,
                },
                &OperationGuard::new(),
            )
            .await
            .expect("bounded read")
    }
}
/// A source that reports whether the guard the engine handed it ever became
/// cancelled.
struct GuardObservingSource {
    entered: Arc<tokio::sync::Notify>,
    observed_cancellation: Arc<AtomicBool>,
}

#[async_trait]
impl SourceAdapter for GuardObservingSource {
    async fn read(
        &self,
        _reference: &PathReference,
        operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        self.entered.notify_one();
        // A guard the engine forwarded resolves here the moment the caller
        // cancels. A fresh guard the engine minted for itself never resolves at
        // all, so the timeout is what tells the two apart.
        let observed = tokio::time::timeout(Duration::from_secs(5), operation.cancelled())
            .await
            .is_ok();
        self.observed_cancellation.store(observed, Ordering::SeqCst);
        SourceResource::text(workspace_reference(), "fixture\n".to_owned())
    }
}

/// The engine hands the caller's own guard to the Source Adapter (rfs-g2z9 C21).
///
/// The engine re-checks liveness *after* the source returns, so a `cancelled`
/// category alone cannot distinguish a guard that reached the source from one
/// the engine kept to itself — the wrong version still reports `cancelled`,
/// just after running the source to completion. A network source paid for that
/// difference with a full request timeout, which is the entire reason the guard
/// was threaded through the trait. So the load-bearing assertion is what the
/// *source* observed, not what the caller received.
#[tokio::test]
async fn read_forwards_the_callers_guard_to_the_source() {
    let limits = ServerLimits::default();
    let session = PathSession::new(
        SessionToken::parse("00000000000000000000000000000042").expect("session token"),
        Arc::new(MemoryStorage::default()),
        limits,
    );
    let entered = Arc::new(tokio::sync::Notify::new());
    let observed = Arc::new(AtomicBool::new(false));
    let source: Arc<dyn SourceAdapter> = Arc::new(GuardObservingSource {
        entered: Arc::clone(&entered),
        observed_cancellation: Arc::clone(&observed),
    });
    let engine = ReadEngine::new(source, session, limits);

    let guard = OperationGuard::new();
    let canceller = guard.clone();
    tokio::spawn(async move {
        entered.notified().await;
        canceller.cancel();
    });

    let refusal = engine
        .read(
            ReadRequest {
                reference: workspace_reference(),
                limits: TextLimits::default(),
                numbered: false,
            },
            &guard,
        )
        .await
        .expect_err("a cancelled read is refused");

    assert!(
        observed.load(Ordering::SeqCst),
        "the source never saw the caller's cancellation, so it had no way to \
         abandon its work early"
    );
    // This fake ignores its guard and returns successfully, so the refusal here
    // comes from the engine's post-read liveness check rather than from the
    // source. A source that honours the guard — the HTTPS one does — refuses
    // with `cancelled` from inside the read instead, which is what
    // `resourcefs-sources` fences end to end.
    assert_eq!(
        refusal.category(),
        ErrorCategory::SourceUnavailable,
        "the engine's own liveness check refuses a cancelled operation: {}",
        refusal.message()
    );
}

/// A source whose projection is complete on its side but names a further
/// upstream page, the way a GitHub repository collection does.
struct ContinuingSource {
    content: String,
}

#[async_trait]
impl SourceAdapter for ContinuingSource {
    async fn read(
        &self,
        _reference: &PathReference,
        _operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        let next = PathReference::parse("issue://owner/repo:page:2".to_owned())?;
        Ok(
            SourceResource::text(workspace_reference(), self.content.clone())?
                .with_continuation(&next),
        )
    }
}

/// A typed source continuation is the read's `continuationReference` when the
/// page fits inline. When the page itself overflows, the artifact continuation
/// wins: it is the only reference that reaches the bytes cut from the page,
/// and the source continuation stays reachable inside the retained content.
#[tokio::test]
async fn source_continuations_surface_unless_the_page_itself_overflows() {
    let limits = ServerLimits::default();
    let read = |content: String| async move {
        let session = PathSession::new(
            SessionToken::parse("00000000000000000000000000000042").expect("session token"),
            Arc::new(MemoryStorage::default()),
            limits,
        );
        let source: Arc<dyn SourceAdapter> = Arc::new(ContinuingSource { content });
        ReadEngine::new(source, session, limits)
            .read(
                ReadRequest {
                    reference: workspace_reference(),
                    limits: TextLimits::default(),
                    numbered: false,
                },
                &OperationGuard::new(),
            )
            .await
            .expect("read")
    };

    let fits = read("one page\n".to_owned()).await;
    assert_eq!(
        fits.continuation_reference(),
        Some("issue://owner/repo:page:2")
    );
    assert!(fits.is_bounded(), "a page with more upstream is bounded");
    assert_eq!(fits.recovery_reference(), None, "nothing was cut from it");
    assert_eq!(
        fits.source_continuation_reference(),
        None,
        "the continuation already names the source's next page, so it is not repeated"
    );

    let overflow = read(sized_lines(TextLimits::default().bytes() * 2)).await;
    let continuation = overflow
        .continuation_reference()
        .expect("an overflowing page progresses through its artifact");
    assert!(continuation.starts_with("artifact://"), "{continuation}");
    assert!(overflow.recovery_reference().is_some());
    assert_eq!(
        overflow.source_continuation_reference(),
        Some("issue://owner/repo:page:2"),
        "the source continuation the artifact chain would hide is named alongside it"
    );
}

#[test]
fn displayed_line_ranges_reject_zero_and_descending_bounds() {
    for (start, end) in [(0, 1), (2, 1)] {
        let error = DisplayedLineRange::new(start, end).expect_err("invalid displayed line range");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }
}

#[tokio::test]
async fn displayed_ranges_exclude_partial_lines() {
    let harness = Harness::new("first\nsecond\nthird".to_owned());

    let partial = harness
        .read(
            workspace_reference(),
            TextLimits::new(None, None, Some(3)).expect("partial-line limits"),
        )
        .await;
    assert_eq!(partial.content(), "fir");
    assert_eq!(partial.displayed_ranges(), &[]);
    assert!(!partial.displayed_eof());

    let one_line = harness
        .read(
            workspace_reference(),
            TextLimits::new(None, Some(1), None).expect("single-line limits"),
        )
        .await;
    assert_eq!(one_line.content(), "first\n");
    assert_eq!(
        one_line.displayed_ranges(),
        &[DisplayedLineRange::new(1, 1).expect("line range")]
    );
    assert!(!one_line.displayed_eof());

    let complete = harness
        .read(workspace_reference(), TextLimits::default())
        .await;
    assert_eq!(
        complete.displayed_ranges(),
        &[DisplayedLineRange::new(1, 3).expect("line range")]
    );
    assert!(complete.displayed_eof());
}

#[tokio::test]
async fn selected_workspace_ranges_keep_original_line_coordinates() {
    let harness = Harness::new("one\ntwo\nthree\nfour\n".to_owned());
    let reference =
        PathReference::parse("fixture.txt:2-2,4-4").expect("multi-range workspace reference");

    let selected = harness.read(reference, TextLimits::default()).await;

    assert_eq!(selected.content(), "two\nfour\n");
    assert_eq!(
        selected.displayed_ranges(),
        &[
            DisplayedLineRange::new(2, 2).expect("second line"),
            DisplayedLineRange::new(4, 4).expect("fourth line"),
        ]
    );
    assert!(selected.displayed_eof());
    assert_eq!(selected.display_line_numbers(), &[2, 4]);
}

fn workspace_reference() -> PathReference {
    PathReference::canonical(
        WorkspaceRootId::new("workspace").expect("root ID"),
        WorkspacePath::new("fixture.txt").expect("workspace path"),
    )
}

fn suffix_origin(content: &str, selector: &LineSelector) -> Option<ArtifactProjectionOrigin> {
    let [range] = selector.ranges() else {
        return None;
    };
    if range.inclusive_end().is_some() {
        return None;
    }
    let mut line = 1_u64;
    if range.start() == 1 && !content.is_empty() {
        return Some(ArtifactProjectionOrigin::new(0, 1));
    }
    for (offset, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            line += 1;
            if line == range.start() && offset + 1 < content.len() {
                return Some(ArtifactProjectionOrigin::new(offset + 1, line));
            }
        }
    }
    None
}

#[tokio::test]
async fn server_text_ceiling_and_per_call_ceiling_compose_by_minimum() {
    let limits = ServerLimits::new(ServerLimitsInput {
        text: TextLimitInput {
            bytes: Some(4),
            ..TextLimitInput::default()
        },
        ..ServerLimitsInput::default()
    })
    .expect("lower server text ceiling");
    let harness = Harness::with_limits("a\nb\nc\n".to_owned(), limits);

    let server_bounded = harness
        .read(workspace_reference(), TextLimits::default())
        .await;
    assert_eq!(server_bounded.content(), "a\nb\n");
    assert!(server_bounded.is_bounded());

    let call_bounded = harness
        .read(
            workspace_reference(),
            TextLimits::new(Some(2), None, None).expect("lower per-call ceiling"),
        )
        .await;
    assert_eq!(call_bounded.content(), "a\n");
    assert!(call_bounded.is_bounded());
}

fn invalid_page() -> ResourceError {
    ResourceError::new(ErrorCategory::InvalidReference, "invalid test page")
}

fn not_found() -> ResourceError {
    ResourceError::new(ErrorCategory::NotFound, "missing test artifact")
}

fn sized_lines(bytes: usize) -> String {
    let mut content = String::with_capacity(bytes);
    while bytes - content.len() > 17 {
        content.push_str("xxxxxxxxxxxxxxxx\n");
    }
    content.extend(std::iter::repeat_n('x', bytes - content.len()));
    content
}

fn displayed_shape(content: &str) -> (usize, usize) {
    if content.is_empty() {
        return (0, 0);
    }
    let mut lines = 0_usize;
    let mut maximum_columns = 0_usize;
    for segment in content.split_inclusive('\n') {
        lines += 1;
        let body = segment
            .strip_suffix('\n')
            .unwrap_or(segment)
            .strip_suffix('\r')
            .unwrap_or_else(|| segment.strip_suffix('\n').unwrap_or(segment));
        maximum_columns = maximum_columns.max(body.chars().count());
    }
    if !content.ends_with('\n') && content.rsplit_once('\n').is_none() {
        lines = 1;
    }
    (lines, maximum_columns)
}

async fn reconstruct(content: String, limits: TextLimits) -> (Harness, Vec<String>) {
    let expected = content.clone();
    let harness = Harness::new(content);
    let mut reference = workspace_reference();
    let mut reconstructed = String::new();
    let mut references = Vec::new();
    let mut recovery_root: Option<ArtifactAddress> = None;

    loop {
        let artifact_projection = matches!(reference.address(), ResourceAddress::Artifact(_))
            && reference.projection().is_some();
        let result = harness.read(reference, limits).await;
        let remaining = &expected[reconstructed.len()..];
        let expected_page_bytes = page_len_for_assertion(remaining, limits);
        assert_eq!(
            result.content().as_bytes(),
            &remaining.as_bytes()[..expected_page_bytes],
            "page was not the largest exact prefix under all limits"
        );
        assert!(result.content().len() <= limits.bytes());
        let (lines, columns) = displayed_shape(result.content());
        assert!(lines <= limits.lines(), "page has {lines} lines");
        assert!(columns <= limits.columns(), "page has {columns} columns");
        reconstructed.push_str(result.content());
        if artifact_projection {
            assert!(result.recovery_reference().is_some());
        }

        if let Some(recovery) = result.recovery_reference() {
            let parsed = PathReference::parse(recovery).expect("recovery reference");
            let ResourceAddress::Artifact(address) = parsed.address() else {
                panic!("recovery must be an artifact");
            };
            if let Some(root) = recovery_root.as_ref() {
                assert_eq!(address, root);
            } else {
                recovery_root = Some(address.clone());
            }
        }

        let Some(continuation) = result.continuation_reference() else {
            assert!(!result.is_bounded());
            break;
        };
        assert!(result.is_bounded());
        assert_ne!(references.last().map(String::as_str), Some(continuation));
        let next = PathReference::parse(continuation).expect("continuation reference");
        let ResourceAddress::Artifact(address) = next.address() else {
            panic!("continuation must be an artifact");
        };
        assert_eq!(Some(address), recovery_root.as_ref());
        references.push(continuation.to_owned());
        reference = next;
        assert!(references.len() < 10_000, "continuation did not terminate");
    }

    assert_eq!(reconstructed.as_bytes(), expected.as_bytes());
    assert_eq!(
        VersionTag::from_content(reconstructed.as_bytes()),
        VersionTag::from_content(expected.as_bytes())
    );
    (harness, references)
}

#[tokio::test]
async fn bounded_pages_reconstruct_every_boundary_fixture() {
    let cases = [
        (String::new(), TextLimits::default()),
        (sized_lines(49_151), TextLimits::default()),
        (sized_lines(49_152), TextLimits::default()),
        (sized_lines(49_153), TextLimits::default()),
        ("x\n".repeat(2_999), TextLimits::default()),
        ("x\n".repeat(3_000), TextLimits::default()),
        ("x\n".repeat(3_001), TextLimits::default()),
        ("x".repeat(511), TextLimits::default()),
        ("x".repeat(512), TextLimits::default()),
        ("x".repeat(513), TextLimits::default()),
        (
            "ééé".to_owned(),
            TextLimits::new(Some(5), None, None).expect("UTF-8 limits"),
        ),
        (
            "alpha\r\nbeta\r\ngamma\r\n".to_owned(),
            TextLimits::new(None, Some(1), None).expect("CRLF limits"),
        ),
        (
            "duplicate\nout-of-order\nduplicate\n".to_owned(),
            TextLimits::new(Some(11), Some(2), Some(9)).expect("intersection limits"),
        ),
    ];

    for (content, limits) in cases {
        let expected_spill = content.len() > page_len_for_assertion(&content, limits);
        let (harness, references) = reconstruct(content, limits).await;
        assert_eq!(
            harness.session.artifact_count().await,
            usize::from(expected_spill)
        );
        assert_eq!(references.is_empty(), !expected_spill);
    }
}

fn page_len_for_assertion(content: &str, limits: TextLimits) -> usize {
    let mut bytes = 0_usize;
    let mut lines = 0_usize;
    let mut columns = 0_usize;
    let mut at_start = true;
    for character in content.chars() {
        if at_start {
            if lines == limits.lines() {
                break;
            }
            lines += 1;
            at_start = false;
        }
        if bytes + character.len_utf8() > limits.bytes() {
            break;
        }
        if character != '\n' && columns == limits.columns() {
            break;
        }
        bytes += character.len_utf8();
        if character == '\n' {
            columns = 0;
            at_start = true;
        } else if character != '\r' {
            columns += 1;
        }
    }
    bytes
}

#[tokio::test]
async fn line_and_utf8_byte_continuations_are_stable_and_progressing() {
    let (_, line_references) = reconstruct(
        "alpha\nbeta\n".to_owned(),
        TextLimits::new(None, Some(1), None).expect("one-line limit"),
    )
    .await;
    assert_eq!(line_references.len(), 1);
    assert!(line_references[0].ends_with(":2-"));

    let (_, byte_references) = reconstruct(
        "ééé".to_owned(),
        TextLimits::new(Some(5), None, None).expect("five-byte limit"),
    )
    .await;
    assert_eq!(byte_references.len(), 1);
    assert!(byte_references[0].ends_with(":page:4"));
}

#[tokio::test]
async fn repeated_identical_spill_reuses_one_root_and_one_charge() {
    let content = "x\n".repeat(3_001);
    let harness = Harness::new(content);
    let limits = TextLimits::default();
    let first = harness.read(workspace_reference(), limits).await;
    let first_charge = harness.session.used_bytes().await;
    let second = harness.read(workspace_reference(), limits).await;

    assert_eq!(first.recovery_reference(), second.recovery_reference());
    assert_eq!(harness.session.artifact_count().await, 1);
    assert!(first_charge > 6_002, "snapshot metadata must be charged");
    assert_eq!(harness.session.used_bytes().await, first_charge);
    assert_eq!(harness.storage.write_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn snapshot_quota_failure_does_not_publish_recovery_artifact() {
    let limits = ServerLimits::new(ServerLimitsInput {
        storage: resourcefs_core::StorageLimitInput {
            object_bytes: Some(6_050),
            session_bytes: Some(6_050),
        },
        ..ServerLimitsInput::default()
    })
    .expect("snapshot-tight limits");
    let harness = Harness::with_limits("x\n".repeat(3_001), limits);

    let error = harness
        .engine
        .read(
            ReadRequest {
                reference: workspace_reference(),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect_err("snapshot plus artifact must exceed session quota");

    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(harness.session.artifact_count().await, 0);
    assert_eq!(harness.session.used_bytes().await, 0);
    assert_eq!(harness.storage.write_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn one_byte_over_object_ceiling_fails_without_artifact() {
    let harness = Harness::new("x".repeat(MAX_ARTIFACT_BYTES + 1));
    let error = harness
        .engine
        .read(
            ReadRequest {
                reference: workspace_reference(),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect_err("oversized selected projection");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert!(
        error
            .message()
            .starts_with("Source Adapter selected projection"),
        "{error}"
    );
    assert_eq!(harness.session.artifact_count().await, 0);
    assert_eq!(harness.storage.write_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn artifact_page_production_budget() {
    let harness = Harness::new("unused".to_owned());
    let content = "x\n".repeat(MAX_ARTIFACT_BYTES / 2);
    let address = harness
        .session
        .retain(&content, &OperationGuard::new())
        .await
        .expect("production artifact");
    drop(content);
    let reference = PathReference::artifact(
        address,
        Some(ProjectionSelector::parse("page:2").expect("page selector")),
    )
    .expect("artifact page");

    let started = Instant::now();
    let result = harness.read(reference, TextLimits::default()).await;
    let elapsed = started.elapsed();
    assert!(result.is_bounded());
    assert_eq!(result.content().len(), 6_000);
    #[cfg(feature = "test-support")]
    assert!(result.content_capacity_for_test() <= 70 * 1024 * 1024);
    assert_eq!(harness.session.artifact_count().await, 1);
    if !cfg!(debug_assertions) {
        assert!(
            elapsed <= Duration::from_secs(5),
            "64 MiB artifact page took {elapsed:?}"
        );
    }
}

#[tokio::test]
async fn workspace_snapshot_production_budget() {
    let content = "x\n".repeat(MAX_ARTIFACT_BYTES / 2);
    assert_eq!(content.len(), MAX_ARTIFACT_BYTES);
    let harness = Harness::new(content);

    let started = Instant::now();
    let page = harness
        .read(workspace_reference(), TextLimits::default())
        .await;
    let elapsed = started.elapsed();

    assert_eq!(
        page.displayed_ranges(),
        &[DisplayedLineRange::new(1, 3_000).expect("displayed page")]
    );
    assert!(!page.displayed_eof());
    assert_eq!(harness.session.artifact_count().await, 1);
    if !cfg!(debug_assertions) {
        assert!(
            elapsed <= Duration::from_secs(5),
            "64 MiB workspace snapshot page took {elapsed:?}"
        );
    }
}

#[tokio::test]
async fn inline_artifact_production_budget() {
    let harness = Harness::new("unused".to_owned());
    let content = sized_lines(resourcefs_core::MAX_TEXT_BYTES);
    let address = harness
        .session
        .retain(&content, &OperationGuard::new())
        .await
        .expect("inline artifact");
    let reference = PathReference::artifact(address, None).expect("artifact root");

    let started = Instant::now();
    let result = harness.read(reference, TextLimits::default()).await;
    let elapsed = started.elapsed();
    assert!(!result.is_bounded());
    assert_eq!(result.content(), content);
    if !cfg!(debug_assertions) {
        assert!(
            elapsed <= Duration::from_millis(25),
            "49 KiB artifact read took {elapsed:?}"
        );
    }
}
