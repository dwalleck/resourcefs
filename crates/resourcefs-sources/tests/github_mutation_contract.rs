#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use resourcefs_core::{
    ErrorCategory, MAX_ARTIFACT_BYTES, MAX_HTTP_FETCH_BYTES, MutationAccess, MutationAdapter,
    MutationEngine, MutationOperation, OperationGuard, OperationId, PathReference,
    SessionCacheEntry, SessionCacheKey, SourceAdapter, SourceMutation, VersionSelector, VersionTag,
    WriteRequest,
};
use resourcefs_sources::{
    GithubConfig, GithubRepository, GithubSource, GithubSourceMount, HttpSubstrate, MutationGrants,
    SecretReference,
};
use serde_json::{Value, json};
use tls::{
    FIXTURE_HOST, FixtureRequest, FixtureResponse, TlsListener, fixture_allowlist, match_cert,
    settle,
};

#[derive(Debug)]
struct GithubState {
    issue_title: String,
    issue_body: String,
    pull_title: String,
    pull_body: String,
    comment_body: String,
    pull_comment_body: String,
    patches: usize,
    patch_status: Option<&'static str>,
    rate_limited: bool,
    force_not_modified: bool,
    posts: usize,
    post_status: Option<&'static str>,
    creation_response: &'static str,
}

impl Default for GithubState {
    fn default() -> Self {
        Self {
            issue_title: "original".to_owned(),
            issue_body: "issue body".to_owned(),
            pull_title: "pull original".to_owned(),
            pull_body: "pull body".to_owned(),
            comment_body: "comment original".to_owned(),
            pull_comment_body: "pull comment original".to_owned(),
            patches: 0,
            patch_status: None,
            rate_limited: false,
            force_not_modified: false,
            posts: 0,
            post_status: None,
            creation_response: "valid",
        }
    }
}

fn response(status: &'static str, body: Value) -> FixtureResponse {
    FixtureResponse::Response {
        status,
        headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body: serde_json::to_vec(&body).expect("[C8] fixture JSON"),
    }
}

fn issue_json(state: &GithubState) -> Value {
    json!({
        "id": 9001,
        "number": 42,
        "state": "open",
        "title": state.issue_title,
        "body": state.issue_body,
        "user": {"login": "alice", "id": 1},
        "html_url": "https://github.example/owner/repo/issues/42",
        "created_at": "2026-08-20T01:02:03Z",
        "updated_at": "2026-08-21T02:03:04Z"
    })
}

fn get_response(request: &FixtureRequest, state: &GithubState, body: Value) -> FixtureResponse {
    if state.force_not_modified
        && request
            .head()
            .to_ascii_lowercase()
            .contains("if-none-match:")
    {
        return FixtureResponse::Response {
            status: "304 Not Modified",
            headers: Vec::new(),
            body: Vec::new(),
        };
    }
    FixtureResponse::Response {
        status: "200 OK",
        headers: vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("ETag".to_owned(), "\"fixture-etag\"".to_owned()),
        ],
        body: serde_json::to_vec(&body).expect("[C8] fixture JSON"),
    }
}

fn pull_json(state: &GithubState) -> Value {
    json!({
        "id": 8001,
        "number": 7,
        "state": "open",
        "title": state.pull_title,
        "body": state.pull_body,
        "user": {"login": "bob", "id": 2},
        "html_url": "https://github.example/owner/repo/pull/7",
        "created_at": "2026-08-20T01:02:03Z",
        "updated_at": "2026-08-21T02:03:04Z",
        "draft": false,
        "merged": false,
        "merged_at": null,
        "head": {"ref": "feature"},
        "base": {"ref": "main"}
    })
}

fn comment_json(id: u64, number: u64, body: &str) -> Value {
    json!({
        "id": id,
        "body": body,
        "user": {"login": "carol", "id": 3},
        "created_at": "2026-08-20T03:00:00Z",
        "updated_at": "2026-08-20T03:00:01Z",
        "issue_url": format!("https://api.github.example/repos/owner/repo/issues/{number}")
    })
}

fn route(request: &FixtureRequest, state: &Arc<Mutex<GithubState>>) -> FixtureResponse {
    let mut state = state.lock().expect("[C8] state lock");
    match (request.method(), request.target()) {
        ("GET", "/repos/owner/repo/issues/42") => get_response(request, &state, issue_json(&state)),
        ("GET", "/repos/owner/repo/pulls/7") => get_response(request, &state, pull_json(&state)),
        ("GET", "/repos/owner/repo/issues/comments/100") => {
            get_response(request, &state, comment_json(100, 42, &state.comment_body))
        }
        ("GET", "/repos/owner/repo/issues/comments/200") => get_response(
            request,
            &state,
            comment_json(200, 7, &state.pull_comment_body),
        ),
        ("PATCH", target) => {
            state.patches += 1;
            if let Some(status) = state.patch_status {
                let mut headers = vec![("Content-Type".to_owned(), "application/json".to_owned())];
                if state.rate_limited {
                    headers.push(("X-RateLimit-Remaining".to_owned(), "0".to_owned()));
                }
                return FixtureResponse::Response {
                    status,
                    headers,
                    body: br#"{"message":"UPSTREAM_SECRET"}"#.to_vec(),
                };
            }
            let body: Value = serde_json::from_slice(request.body()).expect("[C20] request JSON");
            match target {
                "/repos/owner/repo/issues/42" => {
                    if let Some(title) = body.get("title").and_then(Value::as_str) {
                        state.issue_title = format!("{title}-normalized");
                    }
                    if let Some(value) = body.get("body").and_then(Value::as_str) {
                        state.issue_body = value.to_owned();
                    }
                    let mut response_body = issue_json(&state);
                    if state.creation_response == "wrong-object" {
                        response_body["id"] = json!(999_999);
                    }
                    response("200 OK", response_body)
                }
                "/repos/owner/repo/pulls/7" => {
                    if let Some(title) = body.get("title").and_then(Value::as_str) {
                        state.pull_title = format!("{title}-normalized");
                    }
                    if let Some(value) = body.get("body").and_then(Value::as_str) {
                        state.pull_body = value.to_owned();
                    }
                    response("200 OK", pull_json(&state))
                }
                "/repos/owner/repo/issues/comments/100" => {
                    state.comment_body = body["body"]
                        .as_str()
                        .expect("[C20] comment body")
                        .to_owned();
                    response("200 OK", comment_json(100, 42, &state.comment_body))
                }
                "/repos/owner/repo/issues/comments/200" => {
                    state.pull_comment_body = body["body"]
                        .as_str()
                        .expect("[C20] pull comment body")
                        .to_owned();
                    response("200 OK", comment_json(200, 7, &state.pull_comment_body))
                }
                other => panic!("[C8] unexpected PATCH {other}"),
            }
        }
        ("POST", target) => {
            state.posts += 1;
            if let Some(status) = state.post_status {
                let mut headers = vec![
                    ("Content-Type".to_owned(), "application/json".to_owned()),
                    ("Retry-After".to_owned(), "0".to_owned()),
                ];
                if status.starts_with("301") {
                    headers.push(("Location".to_owned(), "/redirected".to_owned()));
                }
                return FixtureResponse::Response {
                    status,
                    headers,
                    body: br#"{"message":"UPSTREAM_SECRET"}"#.to_vec(),
                };
            }
            if state.creation_response == "malformed" {
                return FixtureResponse::Response {
                    status: "201 Created",
                    headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
                    body: b"{".to_vec(),
                };
            }
            if state.creation_response == "truncated" {
                return FixtureResponse::Response {
                    status: "201 Created",
                    headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
                    body: vec![b'x'; MAX_HTTP_FETCH_BYTES + 1],
                };
            }
            let request_body: Value =
                serde_json::from_slice(request.body()).expect("[C20] creation request JSON");
            let status = if state.creation_response == "wrong-status" {
                "202 Accepted"
            } else {
                "201 Created"
            };
            let mut created = match target {
                "/repos/owner/repo/issues" => json!({
                    "id": 9077,
                    "number": 77,
                    "state": "open",
                    "title": request_body["title"],
                    "body": request_body["body"],
                    "user": {"login": "alice", "id": 1},
                    "html_url": "https://github.example/owner/repo/issues/77",
                    "created_at": "2026-08-20T01:02:03Z",
                    "updated_at": "2026-08-21T02:03:04Z"
                }),
                "/repos/owner/repo/pulls" => json!({
                    "id": 8088,
                    "number": 88,
                    "state": "open",
                    "title": request_body["title"],
                    "body": request_body["body"],
                    "user": {"login": "bob", "id": 2},
                    "html_url": "https://github.example/owner/repo/pull/88",
                    "created_at": "2026-08-20T01:02:03Z",
                    "updated_at": "2026-08-21T02:03:04Z",
                    "draft": request_body.get("draft").and_then(Value::as_bool).unwrap_or(false),
                    "merged": false,
                    "merged_at": null,
                    "head": {"ref": request_body["head"]},
                    "base": {"ref": request_body["base"]}
                }),
                "/repos/owner/repo/issues/42/comments" => {
                    comment_json(101, 42, request_body["body"].as_str().expect("[C20] body"))
                }
                "/repos/owner/repo/issues/7/comments" => {
                    comment_json(201, 7, request_body["body"].as_str().expect("[C20] body"))
                }
                other => panic!("[C10] unexpected POST {other}"),
            };
            if state.creation_response == "missing-identity" {
                if target.ends_with("/comments") {
                    created.as_object_mut().expect("[C19] object").remove("id");
                } else {
                    created
                        .as_object_mut()
                        .expect("[C19] object")
                        .remove("number");
                }
            }
            if state.creation_response == "blank-title" && !target.ends_with("/comments") {
                created["title"] = json!("");
            }
            if state.creation_response == "wrong-repository" && !target.ends_with("/comments") {
                created["html_url"] = json!("https://github.example/other/repository/issues/77");
            }
            if state.creation_response == "wrong-parent" && target.ends_with("/comments") {
                created["issue_url"] =
                    json!("https://api.github.example/repos/owner/repo/issues/999");
            }
            response(status, created)
        }
        other => panic!("[C8] unexpected request {other:?}"),
    }
}

async fn fixture(
    update_grant: bool,
) -> (
    TlsListener,
    GithubSource,
    resourcefs_core::PathSession,
    Arc<Mutex<GithubState>>,
) {
    fixture_with_grants(false, update_grant).await
}

async fn fixture_with_grants(
    create_grant: bool,
    update_grant: bool,
) -> (
    TlsListener,
    GithubSource,
    resourcefs_core::PathSession,
    Arc<Mutex<GithubState>>,
) {
    let state = Arc::new(Mutex::new(GithubState::default()));
    let router_state = Arc::clone(&state);
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener = TlsListener::serve_request_router(loopback, 0, match_cert(), move |request| {
        route(request, &router_state)
    })
    .await;
    let port = listener.address.port();
    let session_fixture = session_support::scratch_fixture().await;
    let session = session_fixture.path_session().clone();
    let grants = MutationGrants::new(create_grant, update_grant, false);
    let config = GithubConfig::new(
        "github",
        false,
        grants,
        resourcefs_sources::GithubDeployment::new(
            Some(format!("https://{FIXTURE_HOST}:{port}/")),
            None,
        )
        .expect("fixture deployment"),
        true,
        SecretReference::environment("GITHUB_TOKEN").expect("[C8] secret reference"),
        vec![GithubRepository::new("owner/repo", grants).expect("[C8] repository")],
        resourcefs_core::ReadAcquisitionLimits::default(),
    )
    .expect("[C8] config");
    let substrate = HttpSubstrate::with_host_lookup_and_roots(
        fixture_allowlist(port, true),
        resourcefs_core::HttpCeilings::default(),
        move |_host| async move { Ok::<_, std::io::Error>(vec![loopback]) },
        &[tls::fixture_ca()],
        Vec::new(),
    )
    .expect("[C8] substrate");
    let source = GithubSourceMount::new(config, Arc::new(substrate))
        .bind(session.clone())
        .expect("[C8] source");
    (listener, source, session, state)
}

async fn read_tag(source: &GithubSource, path: &str) -> VersionTag {
    let resource = source
        .read(
            &PathReference::parse(path).expect("[C8] read reference"),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("[C8] Field read");
    assert!(
        resource.is_mutable(),
        "[C8] update grant marks {path} mutable"
    );
    resource.version_tag().clone()
}

#[tokio::test]
async fn field_replace_compares_uncached_authority() {
    let (listener, source, session, state) = fixture(true).await;
    let path = "issue://owner/repo/42/title";
    let tag = read_tag(&source, path).await;
    let before = listener.requests().len();
    let receipt = MutationEngine::new(Arc::new(source.clone()), session.clone())
        .write(
            WriteRequest::new(
                PathReference::parse(path).expect("[C8] reference"),
                "changed".to_owned(),
                Some(tag),
                None,
            )
            .expect("[C8] write request"),
            &OperationGuard::new(),
        )
        .await
        .expect("[C8] replacement");
    assert_eq!(receipt.operation(), MutationOperation::Replaced);
    assert_eq!(
        state.lock().expect("[C8] state").issue_title,
        "changed-normalized"
    );
    settle().await;
    let requests = listener.requests();
    assert_eq!(
        &requests[before..],
        [
            "GET /repos/owner/repo/issues/42 HTTP/1.1",
            "PATCH /repos/owner/repo/issues/42 HTTP/1.1"
        ],
        "[C8] uncached GET then one PATCH"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(listener.bodies().last().expect("[C20] PATCH body"))
            .expect("[C20] JSON"),
        json!({"title": "changed"}),
        "[C20] exact one-field request"
    );

    let stale_tag = read_tag(&source, path).await;
    {
        let mut state = state.lock().expect("[C8] state");
        state.issue_title = "external".to_owned();
        state.force_not_modified = true;
    }
    let before = listener.requests().len();
    let error = MutationEngine::new(Arc::new(source.clone()), session.clone())
        .write(
            WriteRequest::new(
                PathReference::parse(path).expect("[C8] reference"),
                "stale".to_owned(),
                Some(stale_tag),
                None,
            )
            .expect("[C8] stale request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("[C8] stale replacement");
    assert_eq!(error.category(), ErrorCategory::VersionConflict);
    settle().await;
    assert_eq!(
        &listener.requests()[before..],
        ["GET /repos/owner/repo/issues/42 HTTP/1.1"],
        "[C8] stale comparison sends no PATCH"
    );
    state.lock().expect("[C8] state").force_not_modified = false;

    for (field, content, expected_requests, expected_body) in [
        (
            "issue://owner/repo/42/body",
            "",
            vec![
                "GET /repos/owner/repo/issues/42 HTTP/1.1",
                "PATCH /repos/owner/repo/issues/42 HTTP/1.1",
            ],
            json!({"body": ""}),
        ),
        (
            "pr://owner/repo/7/title",
            "pull changed",
            vec![
                "GET /repos/owner/repo/pulls/7 HTTP/1.1",
                "PATCH /repos/owner/repo/pulls/7 HTTP/1.1",
            ],
            json!({"title": "pull changed"}),
        ),
        (
            "pr://owner/repo/7/body",
            "",
            vec![
                "GET /repos/owner/repo/pulls/7 HTTP/1.1",
                "PATCH /repos/owner/repo/pulls/7 HTTP/1.1",
            ],
            json!({"body": ""}),
        ),
        (
            "issue://owner/repo/42/comments/100",
            "issue comment changed",
            vec![
                "GET /repos/owner/repo/issues/42 HTTP/1.1",
                "GET /repos/owner/repo/issues/comments/100 HTTP/1.1",
                "PATCH /repos/owner/repo/issues/comments/100 HTTP/1.1",
            ],
            json!({"body": "issue comment changed"}),
        ),
        (
            "pr://owner/repo/7/comments/200",
            "pull comment changed",
            vec![
                "GET /repos/owner/repo/pulls/7 HTTP/1.1",
                "GET /repos/owner/repo/issues/comments/200 HTTP/1.1",
                "PATCH /repos/owner/repo/issues/comments/200 HTTP/1.1",
            ],
            json!({"body": "pull comment changed"}),
        ),
    ] {
        let tag = read_tag(&source, field).await;
        let before = listener.requests().len();
        MutationEngine::new(Arc::new(source.clone()), session.clone())
            .write(
                WriteRequest::new(
                    PathReference::parse(field).expect("[C8] Field reference"),
                    content.to_owned(),
                    Some(tag),
                    None,
                )
                .expect("[C8] Field request"),
                &OperationGuard::new(),
            )
            .await
            .expect("[C8] Field replacement");
        settle().await;
        assert_eq!(
            &listener.requests()[before..],
            expected_requests,
            "[C8] exact preflight/PATCH order for {field}"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(listener.bodies().last().expect("[C20] body"))
                .expect("[C20] JSON"),
            expected_body,
            "[C20] exact payload for {field}"
        );
    }
}

#[tokio::test]
async fn concurrent_field_replacements_serialize_and_reject_stale() {
    let (listener, source, session, state) = fixture(true).await;
    let path = "issue://owner/repo/42/title";
    let tag = read_tag(&source, path).await;
    let engine = MutationEngine::new(Arc::new(source), session);
    let first_guard = OperationGuard::new();
    let second_guard = OperationGuard::new();
    let first = engine.write(
        WriteRequest::new(
            PathReference::parse(path).expect("[C8] reference"),
            "first".to_owned(),
            Some(tag.clone()),
            None,
        )
        .expect("[C8] first"),
        &first_guard,
    );
    let second = engine.write(
        WriteRequest::new(
            PathReference::parse(path).expect("[C8] reference"),
            "second".to_owned(),
            Some(tag),
            None,
        )
        .expect("[C8] second"),
        &second_guard,
    );
    let (first, second) = tokio::join!(first, second);
    assert_eq!(
        usize::from(first.is_ok()) + usize::from(second.is_ok()),
        1,
        "[C8]"
    );
    let loser = if let Err(error) = first {
        error
    } else {
        second.expect_err("[C8] loser")
    };
    assert_eq!(loser.category(), ErrorCategory::VersionConflict);
    assert_eq!(
        state.lock().expect("[C8] state").patches,
        1,
        "[C8] one PATCH"
    );
    settle().await;
    assert_eq!(
        listener
            .requests()
            .iter()
            .filter(|line| line.starts_with("PATCH "))
            .count(),
        1,
        "[C8] canonical lock serializes contenders"
    );
}

#[tokio::test]
async fn field_receipt_tags_response_without_seen_coverage() {
    let (_listener, source, session, _state) = fixture(true).await;
    let path = "issue://owner/repo/42/title";
    let tag = read_tag(&source, path).await;
    let receipt = MutationEngine::new(Arc::new(source), session.clone())
        .write(
            WriteRequest::new(
                PathReference::parse(path).expect("[C9] reference"),
                "changed".to_owned(),
                Some(tag),
                None,
            )
            .expect("[C9] request"),
            &OperationGuard::new(),
        )
        .await
        .expect("[C9] receipt");
    assert_eq!(
        receipt.version_tag(),
        Some(
            &VersionTag::parse(
                "sha256:3f884ab7d9ea96b933085805f4e1f59c2e5d6531d80e7b989fe04543af94ab96",
            )
            .expect("[C9] Python hashlib vector")
        ),
        "[C9] response-derived tag"
    );
    assert_eq!(receipt.displayed_ranges(), None, "[C9] no receipt coverage");
    assert_eq!(
        session
            .resolve_seen_for_test(
                path,
                &VersionSelector::full(receipt.version_tag().expect("[C9] tag").clone()),
            )
            .await
            .expect_err("[C9] no seen entry")
            .category(),
        ErrorCategory::InvalidReference,
        "[C9] GitHub Field never enters seen snapshots"
    );
}

#[tokio::test]
async fn invalid_field_content_precedes_egress() {
    for (path, content) in [
        ("issue://owner/repo/42/title", " \t"),
        ("issue://owner/repo/42/comments/100", "\n"),
    ] {
        let (listener, source, session, _state) = fixture(true).await;
        let tag = read_tag(&source, path).await;
        let before = listener.requests().len();
        let error = MutationEngine::new(Arc::new(source), session)
            .write(
                WriteRequest::new(
                    PathReference::parse(path).expect("[C5] Field reference"),
                    content.to_owned(),
                    Some(tag),
                    None,
                )
                .expect("[C5] request"),
                &OperationGuard::new(),
            )
            .await
            .expect_err("[C5] blank Field content");
        assert_eq!(
            error.category(),
            ErrorCategory::InvalidReference,
            "[C5] {path}"
        );
        settle().await;
        assert_eq!(
            listener.requests().len(),
            before,
            "[C5] validation precedes upstream access for {path}"
        );
    }
}

#[tokio::test]
async fn field_response_object_identity_is_stable() {
    let (_listener, source, session, state) = fixture(true).await;
    let path = "issue://owner/repo/42/title";
    let tag = read_tag(&source, path).await;
    state.lock().expect("[C19] state").creation_response = "wrong-object";
    let error = MutationEngine::new(Arc::new(source), session)
        .write(
            WriteRequest::new(
                PathReference::parse(path).expect("[C19] Field"),
                "changed".to_owned(),
                Some(tag),
                None,
            )
            .expect("[C19] request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("[C19] wrong stable object ID");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
}

#[tokio::test]
async fn mutation_status_matrix_is_stable_and_redacted() {
    for (status, rate_limited, category) in [
        (
            "301 Moved Permanently",
            false,
            ErrorCategory::SourceUnavailable,
        ),
        ("400 Bad Request", false, ErrorCategory::InvalidReference),
        ("401 Unauthorized", false, ErrorCategory::PermissionDenied),
        ("403 Forbidden", false, ErrorCategory::PermissionDenied),
        ("403 Forbidden", true, ErrorCategory::SourceUnavailable),
        ("404 Not Found", false, ErrorCategory::NotFound),
        ("409 Conflict", false, ErrorCategory::VersionConflict),
        ("410 Gone", false, ErrorCategory::NotFound),
        (
            "422 Unprocessable Entity",
            false,
            ErrorCategory::InvalidReference,
        ),
        (
            "429 Too Many Requests",
            false,
            ErrorCategory::SourceUnavailable,
        ),
        (
            "500 Internal Server Error",
            false,
            ErrorCategory::SourceUnavailable,
        ),
    ] {
        let (_listener, source, session, state) = fixture(true).await;
        let path = "issue://owner/repo/42/title";
        let tag = read_tag(&source, path).await;
        {
            let mut state = state.lock().expect("[C12] state");
            state.patch_status = Some(status);
            state.rate_limited = rate_limited;
        }
        let error = MutationEngine::new(Arc::new(source), session)
            .write(
                WriteRequest::new(
                    PathReference::parse(path).expect("[C12] reference"),
                    "changed".to_owned(),
                    Some(tag),
                    None,
                )
                .expect("[C12] request"),
                &OperationGuard::new(),
            )
            .await
            .expect_err("[C12] status failure");
        assert_eq!(error.category(), category, "[C12] {status}");
        assert!(
            !error.message().contains("UPSTREAM_SECRET"),
            "[C12] {status}"
        );
    }
}

#[tokio::test]
async fn success_invalidates_github_cache_before_commit_returns() {
    let (_listener, source, session, state) = fixture(true).await;
    let github_key = SessionCacheKey::new("github-http", "field").expect("[C13] key");
    let other_key = SessionCacheKey::new("github-http-extra", "field").expect("[C13] key");
    let entry = SessionCacheEntry::new(Vec::new(), b"cached".to_vec()).expect("[C13] entry");
    session
        .cache_put(github_key.clone(), entry.clone())
        .await
        .expect("[C13] put");
    session
        .cache_put(other_key.clone(), entry)
        .await
        .expect("[C13] put");
    let fetch_generation = session
        .cache_generation("github-http")
        .await
        .expect("[C13] generation");
    let reference = PathReference::parse("issue://owner/repo/42/title").expect("[C13] ref");
    let target = MutationAdapter::resolve(&source, &reference, MutationAccess::Update)
        .await
        .expect("[C13] target");
    let guard = OperationGuard::new();
    assert!(guard.begin_commit().is_ok(), "[C13] committing");
    let outcome = MutationAdapter::commit(
        &source,
        SourceMutation::Replace {
            target,
            expected: VersionTag::from_content(b"original"),
            content: Arc::from("changed"),
        },
        &guard,
    )
    .await
    .expect("[C13] commit");
    assert!(matches!(
        outcome,
        resourcefs_core::MutationCommitOutcome::AuthoritativeText { .. }
    ));
    assert!(
        session
            .cache_get(&github_key)
            .await
            .expect("[C13] get")
            .is_none()
    );
    assert!(
        session
            .cache_get(&other_key)
            .await
            .expect("[C13] get")
            .is_some()
    );
    assert!(
        !session
            .cache_put_if_generation(
                github_key.clone(),
                SessionCacheEntry::new(Vec::new(), b"stale-race".to_vec())
                    .expect("[C13] stale entry"),
                fetch_generation,
            )
            .await
            .expect("[C13] stale conditional put"),
        "[C13] in-flight pre-mutation fetch cannot repopulate cache"
    );
    assert!(
        session
            .cache_get(&github_key)
            .await
            .expect("[C13] stale get")
            .is_none()
    );

    session
        .cache_put(
            github_key.clone(),
            SessionCacheEntry::new(Vec::new(), b"cached-again".to_vec())
                .expect("[C13] failure entry"),
        )
        .await
        .expect("[C13] failure put");
    state.lock().expect("[C13] state").patch_status = Some("422 Unprocessable Entity");
    let target = MutationAdapter::resolve(&source, &reference, MutationAccess::Update)
        .await
        .expect("[C13] failure target");
    let guard = OperationGuard::new();
    assert!(guard.begin_commit().is_ok(), "[C13] failure committing");
    let error = MutationAdapter::commit(
        &source,
        SourceMutation::Replace {
            target,
            expected: VersionTag::from_content(b"changed-normalized"),
            content: Arc::from("rejected"),
        },
        &guard,
    )
    .await
    .expect_err("[C13] rejected commit");
    assert_eq!(error.error().category(), ErrorCategory::InvalidReference);
    assert!(
        session
            .cache_get(&github_key)
            .await
            .expect("[C13] failure get")
            .is_some(),
        "[C13] failed mutation preserves cache"
    );
}

#[tokio::test]
#[ignore = "checkpointed-build production-scale budget"]
async fn github_cache_namespace_removal_budget() {
    let (_listener, _source, session, _state) = fixture(true).await;
    let entry = SessionCacheEntry::new(Vec::new(), Vec::new()).expect("[C13] entry");
    for index in 0..500 {
        session
            .cache_put(
                SessionCacheKey::new("github-http", format!("github-{index}"))
                    .expect("[C13] GitHub key"),
                entry.clone(),
            )
            .await
            .expect("[C13] GitHub put");
        session
            .cache_put(
                SessionCacheKey::new("other", format!("other-{index}")).expect("[C13] other key"),
                entry.clone(),
            )
            .await
            .expect("[C13] other put");
    }
    let started = Instant::now();
    assert_eq!(
        session
            .cache_remove_namespace("github-http")
            .await
            .expect("[C13] namespace removal"),
        500
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed <= Duration::from_millis(50),
        "[C13] 500-of-1,000 cache removal took {elapsed:?}"
    );
    assert!(
        session
            .cache_get(
                &SessionCacheKey::new("other", "other-499").expect("[C13] retained other key")
            )
            .await
            .expect("[C13] other get")
            .is_some(),
        "[C13] non-GitHub namespace remains"
    );
}

#[tokio::test]
async fn unsupported_mutation_matrix_has_zero_egress() {
    let (listener, source, session, _state) = fixture(true).await;
    for path in [
        "issue://owner/repo",
        "issue://owner/repo/42",
        "issue://owner/repo/42/comments",
        "pr://owner/repo/7/reviews/1",
        "pr://owner/repo/7/review-comments/1",
        "pr://owner/repo/7/diff",
    ] {
        let error = MutationAdapter::resolve(
            &source,
            &PathReference::parse(path).expect("[C14] reference"),
            MutationAccess::Update,
        )
        .await
        .expect_err("[C14] unsupported mutation");
        assert_eq!(
            error.category(),
            ErrorCategory::UnsupportedMutation,
            "[C14] {path}"
        );
    }
    let tag = VersionTag::from_content(b"unseen");
    let patch = format!("[issue://owner/repo/42/title#{tag}]\nPUT 1.=1:\n+changed");
    let error = MutationEngine::new(Arc::new(source), session)
        .edit(&patch, &OperationGuard::new())
        .await
        .expect_err("[C14] GitHub hashline edit");
    assert_eq!(error.category(), ErrorCategory::UnsupportedMutation);
    settle().await;
    assert!(listener.requests().is_empty(), "[C14] zero mutation egress");

    let (denied_listener, denied, _session, _state) = fixture(false).await;
    let error = MutationAdapter::resolve(
        &denied,
        &PathReference::parse("issue://owner/repo/42/title").expect("[C14] title"),
        MutationAccess::Update,
    )
    .await
    .expect_err("[C14] grant denial");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    settle().await;
    assert!(
        denied_listener.requests().is_empty(),
        "[C14] grant precedes egress"
    );
}

#[tokio::test]
async fn metadata_grant_target_matrix_precedes_egress() {
    let (listener, denied, denied_session, _state) = fixture_with_grants(false, true).await;
    let target = PathReference::parse("issue://owner/repo/new").expect("[C5] target");
    let error = MutationAdapter::resolve(&denied, &target, MutationAccess::Create)
        .await
        .expect_err("[C5] create grant denied");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    let error = MutationEngine::new(Arc::new(denied), denied_session)
        .write(
            WriteRequest::new(target.clone(), valid_issue_document(), None, None)
                .expect("[C5] missing-ID request"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("[C5] operation ID required");
    assert_eq!(error.category(), ErrorCategory::InvalidReference);
    settle().await;
    assert!(
        listener.requests().is_empty(),
        "[C5] denials precede egress"
    );

    let (listener, source, session, _state) = fixture_with_grants(true, true).await;
    let resolved = MutationAdapter::resolve(&source, &target, MutationAccess::Create)
        .await
        .expect("[C5] granted Creation Target");
    assert_eq!(
        resolved.mode(),
        resourcefs_core::MutationTargetMode::CreationTarget
    );
    assert!(listener.requests().is_empty(), "[C5] resolution has no I/O");
    let engine = MutationEngine::new(Arc::new(source), session);
    for request in [
        WriteRequest::new(
            target.clone(),
            valid_issue_document(),
            Some(VersionTag::from_content(b"not applicable")),
            Some(OperationId::parse("with-version").expect("[C5] ID")),
        )
        .expect("[C5] ifVersion request"),
        WriteRequest::new(
            target.clone(),
            "---\ntitle: malformed".to_owned(),
            None,
            Some(OperationId::parse("malformed").expect("[C5] ID")),
        )
        .expect("[C5] malformed request"),
    ] {
        let error = engine
            .write(request, &OperationGuard::new())
            .await
            .expect_err("[C5] metadata/content rejection");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }
    settle().await;
    assert!(
        listener.requests().is_empty(),
        "[C5] metadata and parser validation precede egress"
    );
}

fn valid_issue_document() -> String {
    "---\ntitle: Created issue\n---\nIssue body\n".to_owned()
}

fn valid_pull_document() -> String {
    "---\ntitle: \"Pull: created\"\nhead: feature\nbase: main\ndraft: true\n---\nPull body\n"
        .to_owned()
}

#[tokio::test]
async fn creation_document_strict_matrix() {
    let (listener, source, _session, _state) = fixture_with_grants(true, false).await;
    let issue = MutationAdapter::resolve(
        &source,
        &PathReference::parse("issue://owner/repo/new").expect("[C6] issue target"),
        MutationAccess::Create,
    )
    .await
    .expect("[C6] issue target");
    let pull = MutationAdapter::resolve(
        &source,
        &PathReference::parse("pr://owner/repo/new").expect("[C6] pull target"),
        MutationAccess::Create,
    )
    .await
    .expect("[C6] pull target");
    let comment = MutationAdapter::resolve(
        &source,
        &PathReference::parse("issue://owner/repo/42/comments/new").expect("[C6] comment target"),
        MutationAccess::Create,
    )
    .await
    .expect("[C6] comment target");

    for content in [
        valid_issue_document(),
        "---\r\ntitle: 'It''s fixed'\r\n---\r\n".to_owned(),
        "---\ntitle: Empty body\n---".to_owned(),
    ] {
        MutationAdapter::validate_write(&source, &issue, &content)
            .expect("[C6] valid issue document");
    }
    MutationAdapter::validate_write(&source, &pull, &valid_pull_document())
        .expect("[C6] valid pull document");
    MutationAdapter::validate_write(&source, &comment, "Markdown **comment**\n")
        .expect("[C6] valid comment Markdown");

    for content in [
        "title: no delimiters",
        "---\ntitle: missing close",
        "---\n---\n",
        "---\ntitle: duplicate\ntitle: twice\n---\n",
        "---\ntitle: ok\nlabels: bug\n---\n",
        "---\ntitle: |\n  block\n---\n",
        "---\ntitle: - sequence\n---\n",
        "---\ntitle: %YAML\n---\n",
        "---\ntitle: unquoted: colon\n---\n",
        "---\ntitle: ok # comment\n---\n",
        "---\ntitle: 42\n---\n",
        "---\ntitle: #comment\n---\n",
        "---\ntitle: ok\t# comment\n---\n",
        "---\ntitle: -\n---\n",
        "---\ntitle: ?\n---\n",
        "---\ntitle: :\n---\n",
        "---\ntitle: ...\n---\n",
        "---\ntitle: \"line\\nbreak\"\n---\n",
    ] {
        assert_eq!(
            MutationAdapter::validate_write(&source, &issue, content)
                .expect_err("[C6] invalid issue document")
                .category(),
            ErrorCategory::InvalidReference,
            "[C6] {content:?}"
        );
    }
    for content in [
        "---\ntitle: PR\nhead: feature\nbase: main\ndraft: \"true\"\n---\n",
        "---\ntitle: PR\nhead: feature\n---\n",
        "---\ntitle: PR\nhead: \nbase: main\n---\n",
    ] {
        assert_eq!(
            MutationAdapter::validate_write(&source, &pull, content)
                .expect_err("[C6] invalid pull document")
                .category(),
            ErrorCategory::InvalidReference,
            "[C6] {content:?}"
        );
    }
    assert_eq!(
        MutationAdapter::validate_write(&source, &comment, " \n")
            .expect_err("[C6] blank comment")
            .category(),
        ErrorCategory::InvalidReference
    );
    let huge_key = format!("{}: value", "a".repeat(1024 * 1024));
    let huge_document = format!("---\n{huge_key}\n---\n");
    let error = MutationAdapter::validate_write(&source, &issue, &huge_document)
        .expect_err("[C6] huge unknown key");
    assert!(
        error.message().len() <= 512,
        "[C6] invalid document diagnostic remains bounded"
    );
    settle().await;
    assert!(
        listener.requests().is_empty(),
        "[C6] parsing performs no I/O"
    );
}

#[tokio::test]
#[ignore = "checkpointed-build production-scale budget"]
async fn creation_document_budget() {
    let (_listener, source, _session, _state) = fixture_with_grants(true, false).await;
    let issue = MutationAdapter::resolve(
        &source,
        &PathReference::parse("issue://owner/repo/new").expect("[C6] issue target"),
        MutationAccess::Create,
    )
    .await
    .expect("[C6] issue target");
    let prefix = "---\ntitle: Maximum\n---\n";
    let document = format!("{prefix}{}", "x".repeat(MAX_ARTIFACT_BYTES - prefix.len()));
    let started = Instant::now();
    MutationAdapter::validate_write(&source, &issue, &document).expect("[C6] maximum document");
    let elapsed = started.elapsed();
    assert!(
        elapsed <= Duration::from_millis(100),
        "[C6] 64 MiB frontmatter/body validation took {elapsed:?}"
    );
}

async fn create(
    engine: &MutationEngine,
    path: &str,
    content: String,
    operation_id: &str,
) -> Result<resourcefs_core::MutationReceipt, resourcefs_core::ResourceError> {
    engine
        .write(
            WriteRequest::new(
                PathReference::parse(path).expect("[C10] target"),
                content,
                None,
                Some(OperationId::parse(operation_id).expect("[C10] ID")),
            )
            .expect("[C10] request"),
            &OperationGuard::new(),
        )
        .await
}

#[tokio::test]
async fn creation_receipts_use_returned_identity_without_seen_coverage() {
    let (listener, source, session, _state) = fixture_with_grants(true, true).await;
    let engine = MutationEngine::new(Arc::new(source), session.clone());
    let rows = [
        (
            "issue://owner/repo/new",
            valid_issue_document(),
            "issue-create",
            "issue://owner/repo/77",
            vec!["POST /repos/owner/repo/issues HTTP/1.1"],
            json!({"title": "Created issue", "body": "Issue body\n"}),
        ),
        (
            "pr://owner/repo/new",
            valid_pull_document(),
            "pull-create",
            "pr://owner/repo/88",
            vec!["POST /repos/owner/repo/pulls HTTP/1.1"],
            json!({
                "title": "Pull: created",
                "head": "feature",
                "base": "main",
                "draft": true,
                "body": "Pull body\n"
            }),
        ),
        (
            "issue://owner/repo/42/comments/new",
            "Issue comment\n".to_owned(),
            "issue-comment-create",
            "issue://owner/repo/42/comments/101",
            vec![
                "GET /repos/owner/repo/issues/42 HTTP/1.1",
                "POST /repos/owner/repo/issues/42/comments HTTP/1.1",
            ],
            json!({"body": "Issue comment\n"}),
        ),
        (
            "pr://owner/repo/7/comments/new",
            "Pull comment\n".to_owned(),
            "pull-comment-create",
            "pr://owner/repo/7/comments/201",
            vec![
                "GET /repos/owner/repo/pulls/7 HTTP/1.1",
                "POST /repos/owner/repo/issues/7/comments HTTP/1.1",
            ],
            json!({"body": "Pull comment\n"}),
        ),
    ];
    for (path, content, id, canonical, expected_requests, expected_body) in rows {
        let before = listener.requests().len();
        let receipt = create(&engine, path, content.clone(), id)
            .await
            .expect("[C10] creation");
        assert_eq!(receipt.operation(), MutationOperation::Created);
        assert_eq!(receipt.canonical_reference().requested(), canonical);
        assert_eq!(receipt.version_tag(), None);
        assert_eq!(receipt.displayed_ranges(), None);
        settle().await;
        assert_eq!(
            &listener.requests()[before..],
            expected_requests,
            "[C10] {path}"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(listener.bodies().last().expect("[C20] POST body"))
                .expect("[C20] JSON"),
            expected_body,
            "[C20] {path}"
        );
        assert_eq!(
            session
                .resolve_seen_for_test(
                    canonical,
                    &VersionSelector::full(VersionTag::from_content(content.as_bytes())),
                )
                .await
                .expect_err("[C10] no seen coverage")
                .category(),
            ErrorCategory::InvalidReference,
            "[C10] {path}"
        );
    }

    let before = listener.requests().len();
    let replay = create(
        &engine,
        "issue://owner/repo/new",
        valid_issue_document(),
        "issue-create",
    )
    .await
    .expect("[C10] replay");
    assert_eq!(
        replay.canonical_reference().requested(),
        "issue://owner/repo/77"
    );
    settle().await;
    assert_eq!(
        listener.requests().len(),
        before,
        "[C10] replay has no egress"
    );
    let conflict = create(
        &engine,
        "issue://owner/repo/new",
        "---\ntitle: Different\n---\n".to_owned(),
        "issue-create",
    )
    .await
    .expect_err("[C10] conflicting ID");
    assert_eq!(conflict.category(), ErrorCategory::VersionConflict);
}

#[tokio::test]
async fn mutation_never_retries_or_redirects() {
    for status in ["301 Moved Permanently", "503 Service Unavailable"] {
        let (listener, source, session, state) = fixture_with_grants(true, false).await;
        state.lock().expect("[C11] state").post_status = Some(status);
        let engine = MutationEngine::new(Arc::new(source), session);
        let error = create(
            &engine,
            "issue://owner/repo/new",
            valid_issue_document(),
            "once",
        )
        .await
        .expect_err("[C11] creation error");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
        settle().await;
        assert_eq!(
            listener
                .requests()
                .iter()
                .filter(|line| line.starts_with("POST "))
                .count(),
            1,
            "[C11] {status} gets one POST"
        );
        assert_eq!(
            listener.requests().len(),
            1,
            "[C11] {status} has zero redirect/retry follow-up"
        );
        if status.starts_with("503") {
            let before = listener.requests().len();
            let repeat = create(
                &engine,
                "issue://owner/repo/new",
                valid_issue_document(),
                "once",
            )
            .await
            .expect_err("[C11] unknown repeat");
            assert_eq!(repeat.category(), ErrorCategory::SourceUnavailable);
            settle().await;
            assert_eq!(
                listener.requests().len(),
                before,
                "[C11] unknown blocks retry"
            );
        }
    }
}

#[tokio::test]
async fn operation_recovery_state_matrix() {
    let (listener, source, session, state) = fixture_with_grants(true, false).await;
    let engine = MutationEngine::new(Arc::new(source), session);
    let first_guard = OperationGuard::new();
    let second_guard = OperationGuard::new();
    let first = engine.write(
        WriteRequest::new(
            PathReference::parse("issue://owner/repo/new").expect("[C18] target"),
            valid_issue_document(),
            None,
            Some(OperationId::parse("concurrent-create").expect("[C18] ID")),
        )
        .expect("[C18] request"),
        &first_guard,
    );
    let second = engine.write(
        WriteRequest::new(
            PathReference::parse("issue://owner/repo/new").expect("[C18] target"),
            valid_issue_document(),
            None,
            Some(OperationId::parse("concurrent-create").expect("[C18] ID")),
        )
        .expect("[C18] request"),
        &second_guard,
    );
    let (first, second) = tokio::join!(first, second);
    assert_eq!(
        first.expect("[C18] first").canonical_reference(),
        second.expect("[C18] second").canonical_reference()
    );
    settle().await;
    assert_eq!(
        state.lock().expect("[C18] state").posts,
        1,
        "[C18] one POST"
    );
    assert_eq!(
        listener
            .requests()
            .iter()
            .filter(|line| line.starts_with("POST "))
            .count(),
        1,
        "[C18] concurrent repeat coalesces"
    );

    let (listener, source, session, state) = fixture_with_grants(true, false).await;
    state.lock().expect("[C18] state").post_status = Some("422 Unprocessable Entity");
    let engine = MutationEngine::new(Arc::new(source), session);
    for _ in 0..2 {
        let error = create(
            &engine,
            "issue://owner/repo/new",
            valid_issue_document(),
            "conclusive",
        )
        .await
        .expect_err("[C18] conclusive rejection");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }
    settle().await;
    assert_eq!(
        listener
            .requests()
            .iter()
            .filter(|line| line.starts_with("POST "))
            .count(),
        2,
        "[C18] conclusive identical retry transmits"
    );

    let (listener, source, session, state) = fixture_with_grants(true, false).await;
    state.lock().expect("[C18] state").post_status = Some("500 Internal Server Error");
    let engine = MutationEngine::new(Arc::new(source), session);
    for _ in 0..2 {
        let error = create(
            &engine,
            "issue://owner/repo/new",
            valid_issue_document(),
            "unknown",
        )
        .await
        .expect_err("[C18] unknown");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    }
    settle().await;
    assert_eq!(
        listener
            .requests()
            .iter()
            .filter(|line| line.starts_with("POST "))
            .count(),
        1,
        "[C18] unknown blocks repeat"
    );
}

#[tokio::test]
async fn mutation_response_validation_matrix() {
    for (mode, path) in [
        ("missing-identity", "issue://owner/repo/new"),
        ("wrong-repository", "issue://owner/repo/new"),
        ("blank-title", "issue://owner/repo/new"),
        ("malformed", "issue://owner/repo/new"),
        ("truncated", "issue://owner/repo/new"),
        ("wrong-status", "issue://owner/repo/new"),
        ("wrong-parent", "issue://owner/repo/42/comments/new"),
    ] {
        let (listener, source, session, state) = fixture_with_grants(true, false).await;
        state.lock().expect("[C19] state").creation_response = mode;
        let engine = MutationEngine::new(Arc::new(source), session);
        let content = if path.ends_with("comments/new") {
            "comment".to_owned()
        } else {
            valid_issue_document()
        };
        let error = create(&engine, path, content.clone(), "invalid-success")
            .await
            .expect_err("[C19] invalid response");
        assert_eq!(
            error.category(),
            ErrorCategory::SourceUnavailable,
            "[C19] {mode}"
        );
        let before = listener.requests().len();
        let repeat = create(&engine, path, content, "invalid-success")
            .await
            .expect_err("[C19] blocked repeat");
        assert_eq!(repeat.category(), ErrorCategory::SourceUnavailable);
        settle().await;
        assert_eq!(listener.requests().len(), before, "[C19] {mode} is unknown");
    }
}

#[tokio::test]
async fn mutation_payloads_are_minimal_and_exact() {
    let (listener, source, session, _state) = fixture_with_grants(true, true).await;
    let engine = MutationEngine::new(Arc::new(source), session);
    create(
        &engine,
        "issue://owner/repo/new",
        valid_issue_document(),
        "payload-issue",
    )
    .await
    .expect("[C20] issue");
    create(
        &engine,
        "pr://owner/repo/new",
        valid_pull_document(),
        "payload-pull",
    )
    .await
    .expect("[C20] pull");
    create(
        &engine,
        "issue://owner/repo/42/comments/new",
        "comment bytes".to_owned(),
        "payload-comment",
    )
    .await
    .expect("[C20] comment");
    settle().await;
    let bodies = listener.bodies();
    let post_bodies = listener
        .requests()
        .iter()
        .zip(bodies.iter())
        .filter(|(line, _)| line.starts_with("POST "))
        .map(|(_, body)| serde_json::from_slice::<Value>(body).expect("[C20] JSON"))
        .collect::<Vec<_>>();
    assert_eq!(
        post_bodies,
        vec![
            json!({"title": "Created issue", "body": "Issue body\n"}),
            json!({
                "title": "Pull: created",
                "head": "feature",
                "base": "main",
                "draft": true,
                "body": "Pull body\n"
            }),
            json!({"body": "comment bytes"})
        ],
        "[C20] only approved keys"
    );
}
