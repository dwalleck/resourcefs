#[path = "support/tls.rs"]
mod tls;

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::{Duration, Instant},
};

use resourcefs_core::{ErrorCategory, OperationGuard, PathReference, SourceAdapter};
use resourcefs_sources::{
    GithubConfig, GithubRepository, GithubSource, HttpSubstrate, MutationGrants, SecretReference,
    render_issue_for_test,
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

async fn fixture_source<R>(router: R) -> (TlsListener, GithubSource)
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
        resourcefs_core::HttpCeilings::default(),
        move |_host| async move { Ok::<_, std::io::Error>(vec![loopback]) },
        &[tls::FIXTURE_CA],
        Vec::new(),
    )
    .expect("substrate");
    let source = GithubSource::new(config, Arc::new(substrate)).expect("GitHub source");
    (listener, source)
}

#[tokio::test]
async fn issue_aggregate_and_fields_are_stable_and_read_only() {
    let (listener, source) = fixture_source(|path| match path {
        "/repos/owner/repo/issues/42" => response(ISSUE),
        "/repos/owner/repo/issues/42/comments" => response(ISSUE_COMMENTS),
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
        "/repos/owner/repo/issues/7/comments" => response("[]"),
        "/repos/owner/repo/pulls/7/reviews" => response(REVIEWS),
        "/repos/owner/repo/pulls/7/reviews/202" => response(
            r#"{"id":202,"body":"looks good","user":{"login":"reviewer","id":3},"state":"APPROVED","submitted_at":null,"commit_id":null}"#,
        ),
        "/repos/owner/repo/pulls/7/comments" => response(REVIEW_COMMENTS),
        "/repos/owner/repo/pulls/comments/303" => response(
            r#"{"id":303,"body":"inline","user":{"login":"reviewer","id":3},"path":"src/lib.rs","diff_hunk":"@@ -1 +1 @@","created_at":"2026-08-20T04:00:00Z","updated_at":"2026-08-20T04:00:00Z","pull_request_review_id":null}"#,
        ),
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
