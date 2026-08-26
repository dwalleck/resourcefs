#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;

use resourcefs_core::{
    DiscoveryEngine, ErrorCategory, MutationAccess, MutationAdapter, OperationGuard, PathReference,
    SearchLimits, SearchOptions, SearchRequest, SearchTarget, ServerLimits, SourceAdapter,
};
use resourcefs_sources::{
    ArtifactSource, CompiledSources, GithubConfig, GithubRepository, GithubSource,
    GithubSourceMount, HttpSubstrate, MutationGrants, SecretReference, render_issue_for_test,
};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc,
        atomic::{AtomicU16, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tls::{FIXTURE_HOST, FixtureResponse, MATCH_CERT, TlsListener, fixture_allowlist, settle};

const ISSUE: &str = r#"{
  "id":9001,"number":42,"state":"open","title":"Parser bug","body":null,
  "user":{"login":"alice","id":1},"html_url":"https://github.example/owner/repo/issues/42",
  "created_at":"2026-08-20T01:02:03Z","updated_at":"2026-08-21T02:03:04Z"
}"#;
const ISSUE_COMMENTS: &str = r#"[
{
  "id":101,"body":"conversation","user":null,
  "created_at":"2026-08-20T03:00:00Z","updated_at":"2026-08-20T03:00:00Z",
  "issue_url":"https://api.github.example/repos/owner/repo/issues/42"
},
{
  "id":100,"body":"lower stable identity","user":{"login":"carol","id":4},
  "created_at":"2026-08-20T03:00:00Z","updated_at":"2026-08-20T03:00:00Z",
  "issue_url":"https://api.github.example/repos/owner/repo/issues/42"
}
]"#;
const PULL: &str = r#"{
  "id":8001,"number":7,"state":"closed","title":"Fix parser","body":"PR body",
  "user":{"login":"bob","id":2},"html_url":"https://github.example/owner/repo/pull/7",
  "created_at":"2026-08-20T01:02:03Z","updated_at":"2026-08-21T02:03:04Z",
  "draft":false,"merged":true,"merged_at":"2026-08-21T02:03:04Z",
  "head":{"ref":"feature"},"base":{"ref":"main"}
}"#;
const REVIEWS: &str = r#"[{
  "id":202,"body":"looks good","user":{"login":"reviewer","id":3},
  "state":"APPROVED","submitted_at":null,"commit_id":null
}]"#;
const REVIEW_COMMENTS: &str = r#"[{
  "id":303,"body":"inline","user":{"login":"reviewer","id":3},
  "path":"src/lib.rs","diff_hunk":"@@ -1 +1 @@",
  "created_at":"2026-08-20T04:00:00Z","updated_at":"2026-08-20T04:00:00Z",
  "pull_request_review_id":null
}]"#;
const FILES: &str = r#"[
  {"filename":"src/lib.rs","status":"modified","patch":"@@ -1 +1 @@"},
  {"filename":"image.png","status":"added"}
]"#;

fn response(body: &str) -> FixtureResponse {
    FixtureResponse::Response {
        status: "200 OK",
        headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body: body.as_bytes().to_vec(),
    }
}

fn protocol_response(
    status: &'static str,
    headers: impl IntoIterator<Item = (&'static str, String)>,
    body: impl Into<Vec<u8>>,
) -> FixtureResponse {
    FixtureResponse::Response {
        status,
        headers: headers
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
        body: body.into(),
    }
}

async fn fixture_source<R>(router: R) -> (TlsListener, GithubSource)
where
    R: Fn(&str) -> FixtureResponse + Send + Sync + 'static,
{
    fixture_source_with_ceilings(router, resourcefs_core::HttpCeilings::default()).await
}

async fn fixture_source_with_ceilings<R>(
    router: R,
    ceilings: resourcefs_core::HttpCeilings,
) -> (TlsListener, GithubSource)
where
    R: Fn(&str) -> FixtureResponse + Send + Sync + 'static,
{
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener = TlsListener::serve_router(loopback, 0, MATCH_CERT, router).await;
    let port = listener.address.port();
    let config = GithubConfig::new(
        "github",
        false,
        MutationGrants::default(),
        Some(format!("https://{FIXTURE_HOST}:{port}/")),
        true,
        SecretReference::environment("GITHUB_TOKEN").expect("secret reference"),
        vec![GithubRepository::new("owner/repo", MutationGrants::default()).expect("repository")],
    )
    .expect("GitHub config");
    let substrate = HttpSubstrate::with_host_lookup_and_roots(
        fixture_allowlist(port, true),
        ceilings,
        move |_host| async move { Ok::<_, std::io::Error>(vec![loopback]) },
        &[tls::FIXTURE_CA],
        Vec::new(),
    )
    .expect("substrate");
    let session_fixture = session_support::scratch_fixture().await;
    let source = GithubSourceMount::new(config, Arc::new(substrate))
        .bind(session_fixture.path_session().clone())
        .expect("GitHub source");
    (listener, source)
}

#[tokio::test]
async fn issue_aggregate_and_fields_are_stable_and_read_only() {
    let (listener, source) = fixture_source(|path| match path {
        "/repos/owner/repo/issues/42" => response(ISSUE),
        "/repos/owner/repo/issues/42/comments?per_page=100&page=1" => response(ISSUE_COMMENTS),
        other => panic!("unexpected route {other}"),
    })
    .await;

    let aggregate = source
        .read(
            &PathReference::parse("issue://owner/repo/42").expect("reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("issue aggregate");
    assert!(!aggregate.is_mutable());
    assert_eq!(
        aggregate.content(),
        "# Issue #42: Parser bug\n\nKind: issue\nNumber: 42\nState: open\nAuthor: alice\nCreated: 2026-08-20T01:02:03Z\nUpdated: 2026-08-21T02:03:04Z\nURL: https://github.example/owner/repo/issues/42\nTitle: issue://owner/repo/42/title\nBody: issue://owner/repo/42/body\n\n## Body\n\n(no body)\n\n## Conversation comments\n\n### Comment 100\nAuthor: carol\nCreated: 2026-08-20T03:00:00Z\nUpdated: 2026-08-20T03:00:00Z\nReference: issue://owner/repo/42/comments/100\n\nlower stable identity\n\n### Comment 101\nAuthor: [deleted]\nCreated: 2026-08-20T03:00:00Z\nUpdated: 2026-08-20T03:00:00Z\nReference: issue://owner/repo/42/comments/101\n\nconversation\n"
    );

    let title = source
        .read(
            &PathReference::parse("issue://owner/repo/42/title").expect("title"),
            &OperationGuard::new(),
        )
        .await
        .expect("title field");
    assert_eq!(title.content(), "Parser bug");
    let body = source
        .read(
            &PathReference::parse("issue://owner/repo/42/body").expect("body"),
            &OperationGuard::new(),
        )
        .await
        .expect("body field");
    assert_eq!(body.content(), "");

    settle().await;
    assert_eq!(listener.requests().len(), 4);
}

#[tokio::test]
async fn pr_projection_kinds_remain_distinct_and_patch_absence_is_explicit() {
    let (listener, source) = fixture_source(|path| match path {
        "/repos/owner/repo/pulls/7" => response(PULL),
        "/repos/owner/repo/issues/7/comments?per_page=100&page=1" => response("[]"),
        "/repos/owner/repo/pulls/7/reviews?per_page=100&page=1" => response(REVIEWS),
        "/repos/owner/repo/pulls/7/reviews/202" => response(
            r#"{"id":202,"body":"looks good","user":{"login":"reviewer","id":3},"state":"APPROVED","submitted_at":null,"commit_id":null}"#,
        ),
        "/repos/owner/repo/pulls/7/comments?per_page=100&page=1" => response(REVIEW_COMMENTS),
        "/repos/owner/repo/pulls/comments/303" => response(
            r#"{"id":303,"body":"inline","user":{"login":"reviewer","id":3},"path":"src/lib.rs","diff_hunk":"@@ -1 +1 @@","created_at":"2026-08-20T04:00:00Z","updated_at":"2026-08-20T04:00:00Z","pull_request_review_id":null}"#,
        ),
        "/repos/owner/repo/pulls/7/files?per_page=100&page=1" => response(FILES),
        "/repos/owner/repo/pulls/7/files" => response(FILES),
        other => panic!("unexpected route {other}"),
    })
    .await;

    let aggregate = source
        .read(
            &PathReference::parse("pr://owner/repo/7").expect("reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("PR aggregate");
    let content = aggregate.content();
    for expected in [
        "Kind: pull request",
        "Draft: false",
        "Merged: true",
        "Head: feature",
        "Base: main",
        "Reference: pr://owner/repo/7/reviews/202",
        "Reference: pr://owner/repo/7/review-comments/303",
        "- pr://owner/repo/7/diff/1 — src/lib.rs (modified)",
        "- pr://owner/repo/7/diff/2 — image.png (added; patch unavailable)",
    ] {
        assert!(content.contains(expected), "missing {expected}\n{content}");
    }

    let review = source
        .read(
            &PathReference::parse("pr://owner/repo/7/reviews/202").expect("review"),
            &OperationGuard::new(),
        )
        .await
        .expect("review field");
    assert!(review.content().contains("State: APPROVED"));
    assert!(review.content().contains("Submitted: pending"));

    let inline = source
        .read(
            &PathReference::parse("pr://owner/repo/7/review-comments/303").expect("inline"),
            &OperationGuard::new(),
        )
        .await
        .expect("inline field");
    assert!(inline.content().contains("Path: src/lib.rs"));
    assert!(!inline.content().contains("Review: pr://"));

    let binary = source
        .read(
            &PathReference::parse("pr://owner/repo/7/diff/2").expect("diff file"),
            &OperationGuard::new(),
        )
        .await
        .expect("binary diff metadata");
    assert!(binary.content().contains("Patch: unavailable"));

    settle().await;
    let requests = listener.requests().join("\n");
    assert!(requests.contains("/issues/7/comments"));
    assert!(requests.contains("/pulls/7/reviews"));
    assert!(requests.contains("/pulls/7/comments"));
    assert!(requests.contains("/pulls/7/files"));
}

#[tokio::test]
async fn repository_policy_precedes_egress() {
    let (listener, source) = fixture_source(|path| panic!("must not request {path}")).await;
    let error = source
        .read(
            &PathReference::parse("issue://other/repo/1").expect("reference"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("unallowlisted repository");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    settle().await;
    assert_eq!(listener.accepts(), 0);
}

#[tokio::test]
async fn errors_match_typed_status_header_matrix() {
    for (status, headers, category) in [
        ("401 Unauthorized", vec![], ErrorCategory::PermissionDenied),
        ("403 Forbidden", vec![], ErrorCategory::PermissionDenied),
        (
            "403 Forbidden",
            vec![("X-RateLimit-Remaining", "0")],
            ErrorCategory::SourceUnavailable,
        ),
        (
            "403 Forbidden",
            vec![("Retry-After", "1")],
            ErrorCategory::SourceUnavailable,
        ),
        ("404 Not Found", vec![], ErrorCategory::NotFound),
        (
            "429 Too Many Requests",
            vec![],
            ErrorCategory::SourceUnavailable,
        ),
        (
            "500 Internal Server Error",
            vec![],
            ErrorCategory::SourceUnavailable,
        ),
    ] {
        let headers = headers
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect::<Vec<_>>();
        let (_listener, source) = fixture_source(move |_path| FixtureResponse::Response {
            status,
            headers: headers.clone(),
            body: br#"{"message":"wording must not drive classification"}"#.to_vec(),
        })
        .await;
        let error = source
            .read(
                &PathReference::parse("issue://owner/repo/42/title").expect("reference"),
                &OperationGuard::new(),
            )
            .await
            .expect_err(status);
        assert_eq!(error.category(), category, "{status}");
    }
}

#[tokio::test]
async fn search_and_line_selectors_use_the_rendered_resource() {
    let (_listener, source) = fixture_source(|path| match path {
        "/repos/owner/repo/issues/42" => response(ISSUE),
        "/repos/owner/repo/issues/42/comments?per_page=100&page=1" => response(ISSUE_COMMENTS),
        other => panic!("unexpected route {other}"),
    })
    .await;
    let selected = source
        .read(
            &PathReference::parse("issue://owner/repo/42:1-3").expect("line selector"),
            &OperationGuard::new(),
        )
        .await
        .expect("selected aggregate");
    assert_eq!(
        selected.content(),
        "# Issue #42: Parser bug\n\nKind: issue\n"
    );

    let engine_session = session_support::scratch_fixture().await;
    let engine = DiscoveryEngine::new(
        Arc::new(source),
        engine_session.path_session().clone(),
        ServerLimits::default(),
    );
    let result = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(
                    PathReference::parse("issue://owner/repo/42").expect("aggregate"),
                ),
                "^CONVERSATION$",
                SearchOptions::new(false, true, false),
                0,
                SearchLimits::default(),
            )
            .expect("search request"),
            &OperationGuard::new(),
        )
        .await
        .expect("GitHub search");
    assert_eq!(result.total_records(), 1);
    assert_eq!(result.groups()[0].reference(), "issue://owner/repo/42");
    assert_eq!(result.groups()[0].lines()[0].text(), "conversation");
}
#[tokio::test]
async fn compiled_registry_mounts_dispatches_and_refuses_github_mutation() {
    let (_listener, github) = fixture_source(|path| match path {
        "/repos/owner/repo/issues/42" => response(ISSUE),
        other => panic!("unexpected route {other}"),
    })
    .await;
    let scratch = session_support::scratch_fixture().await;
    let session = scratch.path_session().clone();
    let compiled = CompiledSources::new(
        scratch.filesystem.clone(),
        ArtifactSource::new(session.clone()),
        scratch.local.clone(),
        None,
        Some(github),
    )
    .await
    .expect("compiled GitHub source");
    let catalog = compiled
        .read(
            &PathReference::parse("rfs://").expect("catalog"),
            &OperationGuard::new(),
        )
        .await
        .expect("catalog read");
    assert!(catalog.content().contains("issue://"));
    assert!(catalog.content().contains("pr://"));
    let title_reference = PathReference::parse("issue://owner/repo/42/title").expect("title");
    assert_eq!(
        compiled
            .read(&title_reference, &OperationGuard::new())
            .await
            .expect("compiled read")
            .content(),
        "Parser bug"
    );
    let mutation = MutationAdapter::resolve(&compiled, &title_reference, MutationAccess::Update)
        .await
        .expect_err("GitHub mutation unsupported");
    assert_eq!(mutation.category(), ErrorCategory::UnsupportedMutation);
}

#[tokio::test]
async fn repository_collections_are_bounded_filtered_and_continuable() {
    let port = Arc::new(AtomicU16::new(0));
    let observed_port = Arc::clone(&port);
    let (listener, source) = fixture_source(move |path| {
        let page = path
            .split('?')
            .nth(1)
            .into_iter()
            .flat_map(|query| query.split('&'))
            .find_map(|part| part.strip_prefix("page="))
            .and_then(|value| value.parse::<u64>().ok())
            .expect("numeric page");
        let issue = format!(
            "{{\"id\":{id},\"number\":{number},\"state\":\"open\",\"title\":\"issue {number}\",\"body\":\"\",\"user\":null,\"html_url\":\"https://example/{number}\",\"created_at\":\"2026-08-20T00:00:00Z\",\"updated_at\":\"2026-08-20T00:00:00Z\"}}",
            id = 10_000 + page,
            number = page
        );
        let body = if page == 1 {
            format!(
                "[{issue},{{\"id\":99999,\"number\":999,\"state\":\"open\",\"title\":\"PR row\",\"body\":\"\",\"user\":null,\"html_url\":\"https://example/999\",\"created_at\":\"2026-08-20T00:00:00Z\",\"updated_at\":\"2026-08-20T00:00:00Z\",\"pull_request\":{{\"url\":\"https://api.example/pulls/999\"}}}}]"
            )
        } else {
            format!("[{issue}]")
        };
        let next_page = page + 1;
        let headers = (page <= 10).then(|| {
            (
                "Link",
                format!(
                    "<https://{FIXTURE_HOST}:{}/repos/owner/repo/issues?state=all&sort=updated&direction=desc&per_page=100&page={next_page}&after=opaque>; rel=\"next\"",
                    observed_port.load(Ordering::Acquire)
                ),
            )
        });
        protocol_response("200 OK", headers, body.into_bytes())
    })
    .await;
    port.store(listener.address.port(), Ordering::Release);

    let listed = source
        .read(
            &PathReference::parse("issue://owner/repo").expect("collection"),
            &OperationGuard::new(),
        )
        .await
        .expect("bounded listing");
    assert!(listed.content().starts_with("# Issues: owner/repo\n"));
    assert!(listed.content().contains("issue://owner/repo/1"));
    assert!(!listed.content().contains("issue://owner/repo/999"));
    assert!(
        listed
            .content()
            .contains("\nContinuation: issue://owner/repo:page:11\n"),
        "{}",
        listed.content()
    );
    let engine_session = session_support::scratch_fixture().await;
    let engine = DiscoveryEngine::new(
        Arc::new(source),
        engine_session.path_session().clone(),
        ServerLimits::default(),
    );
    let search = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(
                    PathReference::parse("issue://owner/repo").expect("collection"),
                ),
                "^Continuation:",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("search"),
            &OperationGuard::new(),
        )
        .await
        .expect("continuation search");
    assert_eq!(search.total_records(), 0);
    assert_eq!(
        search.continuation_reference(),
        Some("issue://owner/repo:page:11")
    );
    settle().await;
    assert_eq!(listener.requests().len(), 20);
}

#[tokio::test]
async fn etag_cache_revalidates_replaces_and_never_hides_requests() {
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&requests);
    let (listener, source) =
        fixture_source(move |_path| match counter.fetch_add(1, Ordering::AcqRel) {
            0 => protocol_response(
                "200 OK",
                [("ETag", "W/\"one\"".to_owned())],
                ISSUE.as_bytes().to_vec(),
            ),
            1 => protocol_response(
                "304 Not Modified",
                [("ETag", "W/\"one\"".to_owned())],
                Vec::new(),
            ),
            2 => protocol_response(
                "200 OK",
                [("ETag", "W/\"two\"".to_owned())],
                ISSUE.replace("Parser bug", "Parser fixed").into_bytes(),
            ),
            _ => protocol_response("500 Internal Server Error", [], Vec::new()),
        })
        .await;
    let reference = PathReference::parse("issue://owner/repo/42/title").expect("title");
    let first = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("first");
    let unchanged = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("304");
    let changed = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("changed");
    assert_eq!(first.content(), "Parser bug");
    assert_eq!(unchanged.content(), "Parser bug");
    assert_eq!(changed.content(), "Parser fixed");
    let failure = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect_err("failed revalidation must not serve stale content");
    assert_eq!(failure.category(), ErrorCategory::SourceUnavailable);
    settle().await;
    let heads = listener.heads();
    assert_eq!(heads.len(), 4);
    assert!(!heads[0].to_ascii_lowercase().contains("if-none-match"));
    assert!(
        heads[1]
            .to_ascii_lowercase()
            .contains("if-none-match: w/\"one\"")
    );
    assert!(
        heads[2]
            .to_ascii_lowercase()
            .contains("if-none-match: w/\"one\"")
    );
}

#[tokio::test]
async fn retry_is_single_and_machine_signaled() {
    let rate_attempts = Arc::new(AtomicUsize::new(0));
    let rate_counter = Arc::clone(&rate_attempts);
    let (_listener, source) = fixture_source(move |_path| {
        if rate_counter.fetch_add(1, Ordering::AcqRel) == 0 {
            protocol_response(
                "429 Too Many Requests",
                [("Retry-After", "0".to_owned())],
                Vec::new(),
            )
        } else {
            response(ISSUE)
        }
    })
    .await;
    let title = source
        .read(
            &PathReference::parse("issue://owner/repo/42/title").expect("title"),
            &OperationGuard::new(),
        )
        .await
        .expect("rate retry");
    assert_eq!(title.content(), "Parser bug");
    assert_eq!(rate_attempts.load(Ordering::Acquire), 2);

    let transport_attempts = Arc::new(AtomicUsize::new(0));
    let transport_counter = Arc::clone(&transport_attempts);
    let (_listener, source) = fixture_source(move |_path| {
        if transport_counter.fetch_add(1, Ordering::AcqRel) == 0 {
            FixtureResponse::Abort
        } else {
            response(ISSUE)
        }
    })
    .await;
    source
        .read(
            &PathReference::parse("issue://owner/repo/42/title").expect("title"),
            &OperationGuard::new(),
        )
        .await
        .expect("transport retry");
    assert_eq!(transport_attempts.load(Ordering::Acquire), 2);
}

#[tokio::test]
async fn pagination_links_are_confined_and_page_failures_are_atomic() {
    let port = Arc::new(AtomicU16::new(0));
    let observed_port = Arc::clone(&port);
    let (listener, source) = fixture_source(move |_path| {
        protocol_response(
            "200 OK",
            [(
                "Link",
                format!(
                    "<https://{FIXTURE_HOST}:{}/repos/other/repo/issues?per_page=100&page=2>; rel=\"next\"",
                    observed_port.load(Ordering::Acquire)
                ),
            )],
            b"[]".to_vec(),
        )
    })
    .await;
    port.store(listener.address.port(), Ordering::Release);
    let error = source
        .read(
            &PathReference::parse("issue://owner/repo").expect("collection"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("cross-repository Link");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    settle().await;
    assert_eq!(listener.requests().len(), 1);

    let port = Arc::new(AtomicU16::new(0));
    let observed_port = Arc::clone(&port);
    let (listener, source) = fixture_source(move |path| {
        if path.contains("page=2") {
            protocol_response("500 Internal Server Error", [], Vec::new())
        } else {
            protocol_response(
                "200 OK",
                [(
                    "Link",
                    format!(
                        "<https://{FIXTURE_HOST}:{}/repos/owner/repo/issues?state=all&sort=updated&direction=desc&per_page=100&page=2>; rel=\"next\"",
                        observed_port.load(Ordering::Acquire)
                    ),
                )],
                b"[]".to_vec(),
            )
        }
    })
    .await;
    port.store(listener.address.port(), Ordering::Release);
    let error = source
        .read(
            &PathReference::parse("issue://owner/repo").expect("collection"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("page two failure cannot return partial success");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    settle().await;
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test]
async fn retry_after_shares_the_configured_logical_deadline() {
    let ceilings = resourcefs_core::HttpCeilings::new(resourcefs_core::HttpCeilingsInput {
        timeout_millis: Some(50),
        ..resourcefs_core::HttpCeilingsInput::default()
    })
    .expect("lowered timeout");
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&attempts);
    let (_listener, source) = fixture_source_with_ceilings(
        move |_path| {
            counted.fetch_add(1, Ordering::AcqRel);
            protocol_response(
                "429 Too Many Requests",
                [("Retry-After", "1".to_owned())],
                Vec::new(),
            )
        },
        ceilings,
    )
    .await;
    let started = Instant::now();
    let error = source
        .read(
            &PathReference::parse("issue://owner/repo/42/title").expect("title"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("Retry-After exceeds logical deadline");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(attempts.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn typed_page_continuations_are_directly_readable() {
    let (listener, source) = fixture_source(|path| {
        assert!(path.ends_with("per_page=100&page=11"), "{path}");
        protocol_response("200 OK", [], b"[]".to_vec())
    })
    .await;
    let page = source
        .read(
            &PathReference::parse("issue://owner/repo:page:11").expect("page reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("page read");
    assert_eq!(page.content(), "# Issues: owner/repo\n\n(no issues)\n");
    settle().await;
    assert_eq!(listener.requests().len(), 1);
}

#[tokio::test]
#[ignore = "checkpointed-build production-scale budget"]
async fn github_pagination_cache_budget() {
    let port = Arc::new(AtomicU16::new(0));
    let observed_port = Arc::clone(&port);
    let (listener, source) = fixture_source(move |path| {
        let page = path
            .split('?')
            .nth(1)
            .into_iter()
            .flat_map(|query| query.split('&'))
            .find_map(|part| part.strip_prefix("page="))
            .and_then(|value| value.parse::<u64>().ok())
            .expect("numeric page");
        let first = (page - 1) * 100 + 1;
        let body = (first..first + 100)
            .map(|number| {
                format!(
                    "{{\"id\":{},\"number\":{number},\"state\":\"open\",\"title\":\"issue {number}\",\"body\":\"\",\"user\":null,\"html_url\":\"https://example/{number}\",\"created_at\":\"2026-08-20T00:00:00Z\",\"updated_at\":\"2026-08-20T00:00:00Z\"}}",
                    10_000 + number
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let headers = (page < 10).then(|| {
            (
                "Link",
                format!(
                    "<https://{FIXTURE_HOST}:{}/repos/owner/repo/issues?state=all&sort=updated&direction=desc&per_page=100&page={}>; rel=\"next\"",
                    observed_port.load(Ordering::Acquire),
                    page + 1
                ),
            )
        });
        protocol_response("200 OK", headers, format!("[{body}]").into_bytes())
    })
    .await;
    port.store(listener.address.port(), Ordering::Release);
    let started = Instant::now();
    let listed = source
        .read(
            &PathReference::parse("issue://owner/repo").expect("collection"),
            &OperationGuard::new(),
        )
        .await
        .expect("1,000-object listing");
    assert_eq!(listed.content().matches("- issue://").count(), 1_000);
    assert!(started.elapsed() <= Duration::from_secs(30));
    settle().await;
    assert_eq!(listener.requests().len(), 10);
}

#[tokio::test]
#[ignore = "checkpointed-build production-scale budget"]
async fn github_search_budget() {
    let mut body = "x".repeat(7 * 1024 * 1024);
    body.push_str("\nneedle\n");
    let issue = format!(
        "{{\"id\":1,\"number\":42,\"state\":\"open\",\"title\":\"large\",\"body\":{},\"user\":null,\"html_url\":\"https://example/42\",\"created_at\":\"2026-08-20T00:00:00Z\",\"updated_at\":\"2026-08-20T00:00:00Z\"}}",
        serde_json::to_string(&body).expect("encode body")
    );
    let (_listener, source) = fixture_source(move |path| match path {
        "/repos/owner/repo/issues/42" => response(&issue),
        "/repos/owner/repo/issues/42/comments?per_page=100&page=1" => response("[]"),
        other => panic!("unexpected route {other}"),
    })
    .await;
    let engine_session = session_support::scratch_fixture().await;
    let engine = DiscoveryEngine::new(
        Arc::new(source),
        engine_session.path_session().clone(),
        ServerLimits::default(),
    );
    let started = Instant::now();
    let result = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(
                    PathReference::parse("issue://owner/repo/42").expect("aggregate"),
                ),
                "^needle$",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("request"),
            &OperationGuard::new(),
        )
        .await
        .expect("search");
    assert_eq!(result.total_records(), 1);
    assert!(
        started.elapsed() <= Duration::from_secs(1),
        "maximum GitHub search exceeded one second"
    );
}
#[test]
#[ignore = "checkpointed-build production-scale budget"]
fn github_single_resource_render_budget() {
    let body = "x".repeat(8 * 1024 * 1024 - 512);
    let issue = format!(
        "{{\"id\":1,\"number\":1,\"state\":\"open\",\"title\":\"large\",\"body\":{},\"user\":null,\"html_url\":\"https://example/1\",\"created_at\":\"2026-08-20T00:00:00Z\",\"updated_at\":\"2026-08-20T00:00:00Z\"}}",
        serde_json::to_string(&body).expect("encode body")
    );
    let started = Instant::now();
    let rendered = render_issue_for_test(issue.as_bytes()).expect("render maximum issue");
    let elapsed = started.elapsed();
    assert!(rendered.len() >= body.len());
    assert!(rendered.starts_with("# Issue #1: large"));
    assert!(
        elapsed <= Duration::from_millis(250),
        "maximum issue decode/render took {elapsed:?}"
    );
}
