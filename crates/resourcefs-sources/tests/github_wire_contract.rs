use std::time::{Duration, Instant};

use resourcefs_core::ErrorCategory;
use resourcefs_sources::{
    GithubWireKindForTest, inspect_github_mutation_route_for_test, inspect_github_wire_for_test,
};

#[test]
fn official_nullable_and_distinct_shapes_decode() {
    let issue = br#"{
        "id": 9000000000001,
        "number": 42,
        "state": "open",
        "title": "Issue title",
        "body": null,
        "user": null,
        "html_url": "https://github.example/owner/repo/issues/42",
        "created_at": "2026-08-20T01:02:03Z",
        "updated_at": "2026-08-21T02:03:04Z",
        "unknown_future_field": {"ignored": true}
    }"#;
    let observed =
        inspect_github_wire_for_test(GithubWireKindForTest::Issue, issue).expect("valid issue");
    assert_eq!(observed.ids(), &[9_000_000_000_001]);
    assert_eq!(observed.numbers(), &[42]);
    assert_eq!(observed.absent_fields(), &["body", "user"]);

    let pull = br#"{
        "id": 7000000000001,
        "number": 42,
        "state": "closed",
        "title": "Pull title",
        "body": "body",
        "user": {"login": "alice", "id": 12},
        "html_url": "https://github.example/owner/repo/pull/42",
        "created_at": "2026-08-20T01:02:03Z",
        "updated_at": "2026-08-21T02:03:04Z",
        "draft": false,
        "merged": true,
        "merged_at": "2026-08-21T02:03:04Z",
        "head": {"ref": "feature"},
        "base": {"ref": "main"}
    }"#;
    let observed =
        inspect_github_wire_for_test(GithubWireKindForTest::PullRequest, pull).expect("valid pull");
    assert_eq!(observed.ids(), &[7_000_000_000_001]);
    assert_eq!(observed.numbers(), &[42]);
    assert!(observed.absent_fields().is_empty());

    let comments = br#"[{
        "id": 101,
        "body": "conversation",
        "user": null,
        "created_at": "2026-08-20T01:02:03Z",
        "updated_at": "2026-08-20T01:02:03Z",
        "issue_url": "https://api.example/repos/owner/repo/issues/42"
    }]"#;
    let observed =
        inspect_github_wire_for_test(GithubWireKindForTest::ConversationComments, comments)
            .expect("valid comments");
    assert_eq!(observed.ids(), &[101]);
    assert_eq!(observed.absent_fields(), &["user"]);

    let reviews = br#"[{
        "id": 202,
        "body": "pending review",
        "user": {"login": "reviewer", "id": 13},
        "state": "PENDING",
        "submitted_at": null,
        "commit_id": null
    }]"#;
    let observed = inspect_github_wire_for_test(GithubWireKindForTest::Reviews, reviews)
        .expect("valid reviews");
    assert_eq!(observed.ids(), &[202]);
    assert_eq!(observed.absent_fields(), &["commit_id", "submitted_at"]);

    let inline = br#"[{
        "id": 303,
        "body": "inline",
        "user": {"login": "reviewer", "id": 13},
        "path": "src/lib.rs",
        "diff_hunk": "@@ -1 +1 @@",
        "created_at": "2026-08-20T01:02:03Z",
        "updated_at": "2026-08-20T01:02:03Z",
        "pull_request_review_id": null,
        "pull_request_url": "https://api.example/repos/owner/repo/pulls/42"
    }]"#;
    let observed = inspect_github_wire_for_test(GithubWireKindForTest::ReviewComments, inline)
        .expect("valid inline comments");
    assert_eq!(observed.ids(), &[303]);
    assert_eq!(observed.absent_fields(), &["pull_request_review_id"]);

    let files = br#"[
        {"filename":"src/lib.rs","status":"modified","patch":"@@ -1 +1 @@"},
        {"filename":"image.png","status":"added"}
    ]"#;
    let observed =
        inspect_github_wire_for_test(GithubWireKindForTest::DiffFiles, files).expect("valid files");
    assert_eq!(observed.absent_fields(), &["patch"]);
}

#[test]
fn mutation_routes_match_empirical_contract() {
    for (reference, method, suffix) in [
        ("issue://owner/repo/42/title", "PATCH", "issues/42"),
        ("pr://owner/repo/7/body", "PATCH", "pulls/7"),
        (
            "issue://owner/repo/42/comments/100",
            "PATCH",
            "issues/comments/100",
        ),
        ("issue://owner/repo/new", "POST", "issues"),
        ("pr://owner/repo/new", "POST", "pulls"),
        (
            "pr://owner/repo/7/comments/new",
            "POST",
            "issues/7/comments",
        ),
    ] {
        assert_eq!(
            inspect_github_mutation_route_for_test(reference).expect("[C1] route"),
            (method.to_owned(), suffix.to_owned()),
            "[C1] {reference}"
        );
    }
}

#[test]
fn required_identity_and_types_fail_as_source_unavailable() {
    for (kind, input) in [
        (GithubWireKindForTest::Issue, br#"{"id":1,"state":"open","title":"missing number","body":"","user":null,"html_url":"https://example/1","created_at":"x","updated_at":"x"}"#.as_slice()),
        (GithubWireKindForTest::Issue, br#"{"id":1,"number":0,"state":"open","title":"zero","body":"","user":null,"html_url":"https://example/1","created_at":"x","updated_at":"x"}"#.as_slice()),
        (GithubWireKindForTest::Reviews, br#"[{"id":"not-an-integer","body":"","user":null,"state":"PENDING","submitted_at":null,"commit_id":null}]"#.as_slice()),
        (GithubWireKindForTest::DiffFiles, br#"[{"status":"added"}]"#.as_slice()),
    ] {
        let error = inspect_github_wire_for_test(kind, input).expect_err("malformed upstream");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
        assert!(error.message().len() <= 512, "diagnostic must stay bounded");
    }
}

#[test]
#[ignore = "checkpointed-build production-scale budget"]
fn github_wire_decode_budget() {
    let body = "x".repeat(8 * 1024 * 1024 - 512);
    let document = format!(
        "{{\"id\":1,\"number\":1,\"state\":\"open\",\"title\":\"large\",\"body\":{},\"user\":null,\"html_url\":\"https://example/1\",\"created_at\":\"2026-08-20T00:00:00Z\",\"updated_at\":\"2026-08-20T00:00:00Z\"}}",
        serde_json::to_string(&body).expect("encode body")
    );
    assert!(document.len() <= 8 * 1024 * 1024);
    let started = Instant::now();
    let observed = inspect_github_wire_for_test(GithubWireKindForTest::Issue, document.as_bytes())
        .expect("maximum issue");
    let elapsed = started.elapsed();
    assert!(
        elapsed <= Duration::from_millis(250),
        "decode took {elapsed:?}"
    );
    assert!(observed.retained_bytes() <= 24 * 1024 * 1024);
}
