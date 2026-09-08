#[path = "support/jira.rs"]
mod jira;
#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;
use jira::{fixture_with_session, offset, page, project, protocol_response, read, response};
use resourcefs_core::{
    ErrorCategory, HttpCeilings, HttpCeilingsInput, OperationGuard, PathReference, SourceAdapter,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[tokio::test]
async fn logical_budget_counts_attempts_and_one_retry() {
    let (listener, source, _session) = fixture_with_session(
        |path| {
            let start = offset(path);
            response(&page(
                vec![project(&(start + 1).to_string(), "P", "Bounded row")],
                start,
                7,
                Some(start + 7),
            ))
        },
        HttpCeilings::default(),
    )
    .await;
    let result = read(&source, "jira://acme/projects")
        .await
        .expect("attempt-limited page");
    assert_eq!(listener.requests().len(), 10);
    assert_eq!(result.content().matches("Bounded row").count(), 10);
    assert_eq!(
        result.continuation(),
        Some("jira://acme/projects:offset:70")
    );

    for second_trigger in [false, true] {
        let count = Arc::new(AtomicUsize::new(0));
        let attempts = count.clone();
        let (listener, source, _session) = fixture_with_session(
            move |path| {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 || (second_trigger && attempt == 2) {
                    protocol_response(
                        "503 Service Unavailable",
                        [("Retry-After", "0".into())],
                        Vec::new(),
                    )
                } else {
                    let start = offset(path);
                    response(&page(
                        vec![project(&(start + 1).to_string(), "P", "Retry row")],
                        start,
                        1,
                        Some(start + 1),
                    ))
                }
            },
            HttpCeilings::default(),
        )
        .await;
        let result = read(&source, "jira://acme/projects").await;
        if second_trigger {
            assert_eq!(
                result
                    .expect_err("second fetch cannot obtain another retry")
                    .category(),
                ErrorCategory::SourceUnavailable
            );
            assert_eq!(listener.requests().len(), 3);
        } else {
            let result = result.expect("one retry consumes attempt budget");
            assert_eq!(listener.requests().len(), 10);
            assert_eq!(result.content().matches("Retry row").count(), 9);
            assert_eq!(result.continuation(), Some("jira://acme/projects:offset:9"));
        }
    }
}

#[tokio::test]
async fn logical_page_failure_never_becomes_partial_success() {
    let first = project("2", "TWO", "Must not escape on later failure");
    let mut foreign = project("10", "TEN", "Foreign");
    foreign["self"] = serde_json::json!("https://foreign.invalid/rest/api/3/project/10");
    let corruptions = vec![
        "{".to_owned(),
        page(vec![first.clone()], 1, 1, None),
        page(vec![foreign], 1, 1, None),
        page(
            vec![
                project("10", "TEN", "one"),
                project("11", "ELEVEN", "over max"),
            ],
            1,
            1,
            None,
        ),
    ];
    for corrupt in corruptions {
        let first = first.clone();
        let (listener, source, _session) = fixture_with_session(
            move |path| {
                if offset(path) == 0 {
                    response(&page(vec![first.clone()], 0, 1, Some(1)))
                } else {
                    response(&corrupt)
                }
            },
            HttpCeilings::default(),
        )
        .await;
        let error = read(&source, "jira://acme/projects")
            .await
            .expect_err("later-page failure is atomic");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
        assert_eq!(listener.requests().len(), 2);
    }
    let ceilings = HttpCeilings::new(HttpCeilingsInput {
        fetch_bytes: Some(1024),
        ..Default::default()
    })
    .expect("body bound");
    let (listener, source, _session) = fixture_with_session(
        |path| {
            let start = offset(path);
            let name = if start == 0 {
                "valid".into()
            } else {
                "x".repeat(2048)
            };
            response(&page(
                vec![project(&(start + 1).to_string(), "P", &name)],
                start,
                1,
                if start == 0 { Some(1) } else { None },
            ))
        },
        ceilings,
    )
    .await;
    assert_eq!(
        read(&source, "jira://acme/projects")
            .await
            .expect_err("later oversized body")
            .category(),
        ErrorCategory::LimitExceeded
    );
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browse_errors_are_typed_redacted_and_read_only() {
    for (status, category) in [
        ("401 Unauthorized", ErrorCategory::PermissionDenied),
        ("403 Forbidden", ErrorCategory::PermissionDenied),
        ("404 Not Found", ErrorCategory::NotFound),
        ("410 Gone", ErrorCategory::NotFound),
        ("413 Payload Too Large", ErrorCategory::LimitExceeded),
        ("429 Too Many Requests", ErrorCategory::SourceUnavailable),
        (
            "500 Internal Server Error",
            ErrorCategory::SourceUnavailable,
        ),
    ] {
        let (listener, source, _session) = fixture_with_session(
            move |_| {
                protocol_response(
                    status,
                    [],
                    b"token-canary response-canary agent@example.com".to_vec(),
                )
            },
            HttpCeilings::default(),
        )
        .await;
        for reference in [
            "jira://acme/projects",
            "jira://acme/projects/2",
            "jira://acme/project-keys/SECRET-KEY",
        ] {
            let error = read(&source, reference)
                .await
                .expect_err("typed native refusal");
            assert_eq!(error.category(), category);
            for secret in [
                "token-canary",
                "response-canary",
                "agent@example.com",
                "SECRET-KEY",
                "https://",
            ] {
                assert!(!error.message().contains(secret));
            }
        }
        assert_eq!(listener.requests().len(), 3);
        assert!(listener.heads().iter().all(|head| head.starts_with("GET ")));
    }
    let (listener, source, _session) =
        fixture_with_session(|_| response("{}"), HttpCeilings::default()).await;
    let operation = OperationGuard::new();
    operation.cancel();
    let reference = PathReference::parse("jira://acme/projects").expect("collection");
    assert_eq!(
        source
            .read(&reference, &operation, None)
            .await
            .expect_err("cancelled")
            .category(),
        ErrorCategory::Cancelled
    );
    for bad in [
        "jira://acme/projects:offset:0",
        "jira://acme/projects/2:offset:1",
        "jira://acme/projects:offset:1:raw",
        "jira://acme/issues:offset:1",
    ] {
        assert!(PathReference::parse(bad).is_err());
    }
    assert_eq!(
        read(&source, "jira://missing/projects")
            .await
            .expect_err("unmounted")
            .category(),
        ErrorCategory::PermissionDenied
    );
    assert!(listener.requests().is_empty());
    let registry = resourcefs_sources::CompiledSources::new(
        _session.filesystem.clone(),
        resourcefs_sources::ArtifactSource::new(_session.path_session().clone()),
        _session.local.clone(),
        None,
        None,
        Some(source),
    )
    .await
    .expect("compiled read-only source");
    for access in [
        resourcefs_core::MutationAccess::Create,
        resourcefs_core::MutationAccess::Update,
        resourcefs_core::MutationAccess::Delete,
    ] {
        let error = resourcefs_core::MutationAdapter::resolve(&registry, &reference, access)
            .await
            .expect_err("Jira remains read-only");
        assert_eq!(error.category(), ErrorCategory::UnsupportedMutation);
    }
    let glob = resourcefs_core::GlobTarget::new("jira://acme/projects/*").expect("glob input");
    assert!(
        resourcefs_core::DiscoveryAdapter::glob(
            &registry,
            &glob,
            resourcefs_core::GlobOptions::default(),
            &OperationGuard::new()
        )
        .await
        .is_err()
    );
    assert!(listener.requests().is_empty());

    let (listener, source, _session) = fixture_with_session(
        |_| tls::FixtureResponse::Redirect("/redirect-canary".into()),
        HttpCeilings::default(),
    )
    .await;
    let error = read(&source, "jira://acme/projects")
        .await
        .expect_err("budgeted read never follows redirects");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert!(!error.message().contains("redirect-canary"));
    assert_eq!(listener.requests().len(), 1);

    let ceilings = HttpCeilings::new(HttpCeilingsInput {
        timeout_millis: Some(100),
        ..Default::default()
    })
    .expect("deadline");
    let (listener, source, _session) = fixture_with_session(
        |_| tls::FixtureResponse::trickle(100, 1, Duration::from_millis(40)),
        ceilings,
    )
    .await;
    assert_eq!(
        read(&source, "jira://acme/projects")
            .await
            .expect_err("original deadline")
            .category(),
        ErrorCategory::SourceUnavailable
    );
    assert_eq!(listener.requests().len(), 1);

    let ceilings = HttpCeilings::new(HttpCeilingsInput {
        timeout_millis: Some(1800),
        ..Default::default()
    })
    .expect("shared deadline");
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let (listener, source, _session) = fixture_with_session(
        move |_| match count.fetch_add(1, Ordering::SeqCst) {
            0 => {
                std::thread::sleep(Duration::from_millis(1000));
                response(&page(vec![project("2", "P", "First page")], 0, 1, Some(1)))
            }
            1 => protocol_response(
                "503 Service Unavailable",
                [("Retry-After", "1".into())],
                Vec::new(),
            ),
            _ => response(&page(
                vec![project("10", "Q", "Incorrect fresh deadline")],
                1,
                1,
                None,
            )),
        },
        ceilings,
    )
    .await;
    assert_eq!(
        read(&source, "jira://acme/projects")
            .await
            .expect_err("later fetch cannot restart deadline")
            .category(),
        ErrorCategory::SourceUnavailable
    );
    assert_eq!(listener.requests().len(), 2);

    let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
    let (listener, source, _session) = fixture_with_session(
        move |_| {
            sent.send(()).expect("notify cancellation");
            protocol_response(
                "503 Service Unavailable",
                [("Retry-After", "1".into())],
                Vec::new(),
            )
        },
        HttpCeilings::default(),
    )
    .await;
    let operation = OperationGuard::new();
    let pending_guard = operation.clone();
    let pending = tokio::spawn(async move {
        source
            .read(
                &PathReference::parse("jira://acme/projects").expect("collection"),
                &pending_guard,
                None,
            )
            .await
    });
    received.recv().await.expect("first attempt observed");
    operation.cancel();
    assert_eq!(
        pending
            .await
            .expect("cancel task")
            .expect_err("cancel before retry")
            .category(),
        ErrorCategory::Cancelled
    );
    assert_eq!(listener.requests().len(), 1);
}

fn issue_page(id: usize, token: Option<&str>, summary: &str) -> String {
    let mut page = serde_json::json!({"issues":[{
        "id":id.to_string(),"key":format!("P-{id}"),
        "self":format!("https://tls.invalid/rest/api/3/issue/{id}"),
        "fields":{"summary":summary,"project":{"id":"2","self":"https://tls.invalid/rest/api/3/project/2"}}
    }]});
    if let Some(token) = token {
        page["nextPageToken"] = serde_json::json!(token);
    }
    page.to_string()
}

#[tokio::test]
async fn issue_logical_budget_counts_attempts_and_one_retry() {
    for scoped in [false, true] {
        for retry in [false, true] {
            let attempts = Arc::new(AtomicUsize::new(0));
            let count = attempts.clone();
            let (listener, source, _session) = fixture_with_session(
                move |path| {
                    let attempt = count.fetch_add(1, Ordering::SeqCst);
                    if retry && attempt == 0 {
                        return protocol_response(
                            "503 Service Unavailable",
                            [("Retry-After", "0".into())],
                            Vec::new(),
                        );
                    }
                    if path == "/rest/api/3/project/2" {
                        return response(&project("2", "P", "Parent").to_string());
                    }
                    response(&issue_page(
                        attempt + 1,
                        Some(&format!("opaque-{attempt}")),
                        "Budget row",
                    ))
                },
                HttpCeilings::default(),
            )
            .await;
            let owner = if scoped {
                "jira://acme/projects/2/issues"
            } else {
                "jira://acme/issues"
            };
            let result = read(&source, owner).await.expect("bounded issues");
            assert_eq!(listener.requests().len(), 10);
            assert_eq!(
                result.content().matches("Budget row").count(),
                10 - usize::from(scoped) - usize::from(retry)
            );
            assert!(
                result
                    .continuation()
                    .expect("budget continuation")
                    .starts_with(&format!("{owner}:cursor:"))
            );
        }
    }
    // A retry consumed by parent verification cannot be recreated for search.
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let (listener, source, _session) = fixture_with_session(
        move |path| match count.fetch_add(1, Ordering::SeqCst) {
            0 | 2 => protocol_response(
                "503 Service Unavailable",
                [("Retry-After", "0".into())],
                Vec::new(),
            ),
            1 => {
                assert_eq!(path, "/rest/api/3/project/2");
                response(&project("2", "P", "Parent").to_string())
            }
            _ => panic!("second retry escaped the logical budget"),
        },
        HttpCeilings::default(),
    )
    .await;
    assert_eq!(
        read(&source, "jira://acme/projects/2/issues")
            .await
            .expect_err("one retry for parent and pages together")
            .category(),
        ErrorCategory::SourceUnavailable
    );
    assert_eq!(listener.requests().len(), 3);
}

#[tokio::test]
async fn issue_logical_page_failure_never_becomes_partial_success() {
    let mut foreign: serde_json::Value =
        serde_json::from_str(&issue_page(10, None, "Foreign")).expect("fixture");
    foreign["issues"][0]["fields"]["project"]["self"] =
        serde_json::json!("https://foreign.invalid/rest/api/3/project/2");
    let mut over_rows: serde_json::Value =
        serde_json::from_str(&issue_page(10, None, "Over native cap")).expect("fixture");
    over_rows["issues"] = serde_json::json!(
        (10..111)
            .map(|id| {
                let page: serde_json::Value =
                    serde_json::from_str(&issue_page(id, None, "Over native cap"))
                        .expect("fixture");
                page["issues"][0].clone()
            })
            .collect::<Vec<_>>()
    );
    let duplicate = {
        let mut page: serde_json::Value =
            serde_json::from_str(&issue_page(10, None, "Duplicate within page")).expect("fixture");
        let row = page["issues"][0].clone();
        page["issues"].as_array_mut().expect("rows").push(row);
        page.to_string()
    };
    for (corrupt, category) in [
        ("{".to_owned(), ErrorCategory::SourceUnavailable),
        (
            issue_page(2, None, "Duplicate across pages"),
            ErrorCategory::SourceUnavailable,
        ),
        (duplicate, ErrorCategory::SourceUnavailable),
        (foreign.to_string(), ErrorCategory::SourceUnavailable),
        (over_rows.to_string(), ErrorCategory::LimitExceeded),
    ] {
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = attempts.clone();
        let (listener, source, _session) = fixture_with_session(
            move |_| {
                if count.fetch_add(1, Ordering::SeqCst) == 0 {
                    response(&issue_page(2, Some("later"), "Must not escape"))
                } else {
                    response(&corrupt)
                }
            },
            HttpCeilings::default(),
        )
        .await;
        assert_eq!(
            read(&source, "jira://acme/issues")
                .await
                .expect_err("later corruption cannot become partial success")
                .category(),
            category
        );
        assert_eq!(listener.requests().len(), 2);
    }
    let oversized_token = "x".repeat(65536);
    let (listener, source, _session) = fixture_with_session(
        move |_| {
            response(&issue_page(
                2,
                Some(&oversized_token),
                "Unrepresentable cursor",
            ))
        },
        HttpCeilings::default(),
    )
    .await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 1)
        .expect("force continuation");
    assert_eq!(
        read(&source, "jira://acme/issues")
            .await
            .expect_err("oversized continuation cannot be discarded")
            .category(),
        ErrorCategory::LimitExceeded
    );
    assert_eq!(listener.requests().len(), 1);
    let ceilings = HttpCeilings::new(HttpCeilingsInput {
        fetch_bytes: Some(1024),
        ..Default::default()
    })
    .expect("body ceiling");
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let (listener, source, _session) = fixture_with_session(
        move |_| {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                response(&issue_page(2, Some("later"), "First"))
            } else {
                response(&issue_page(10, None, &"x".repeat(2048)))
            }
        },
        ceilings,
    )
    .await;
    assert_eq!(
        read(&source, "jira://acme/issues")
            .await
            .expect_err("later body ceiling")
            .category(),
        ErrorCategory::LimitExceeded
    );
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn issue_browse_errors_are_typed_redacted_and_read_only() {
    for owner in ["jira://acme/issues", "jira://acme/projects/2/issues"] {
        for (status, category) in [
            ("401 Unauthorized", ErrorCategory::PermissionDenied),
            ("403 Forbidden", ErrorCategory::PermissionDenied),
            ("404 Not Found", ErrorCategory::NotFound),
            ("410 Gone", ErrorCategory::NotFound),
            ("413 Payload Too Large", ErrorCategory::LimitExceeded),
            ("429 Too Many Requests", ErrorCategory::SourceUnavailable),
            (
                "500 Internal Server Error",
                ErrorCategory::SourceUnavailable,
            ),
        ] {
            let (listener, source, _session) = fixture_with_session(
                move |path| {
                    if path == "/rest/api/3/project/2" {
                        return response(&project("2", "P", "Parent").to_string());
                    }
                    protocol_response(
                        status,
                        [],
                        b"token-canary response-canary agent@example.com".to_vec(),
                    )
                },
                HttpCeilings::default(),
            )
            .await;
            let error = read(&source, owner).await.expect_err("typed issue refusal");
            assert_eq!(error.category(), category);
            for secret in [
                "token-canary",
                "response-canary",
                "agent@example.com",
                "https://",
            ] {
                assert!(!error.message().contains(secret));
            }
            assert_eq!(
                listener.requests().len(),
                if owner.contains("/projects/") { 2 } else { 1 }
            );
            assert!(listener.heads().iter().all(|head| head.starts_with("GET ")));
        }
        let (listener, source, session) = fixture_with_session(
            |_| panic!("read-only rejection must precede egress"),
            HttpCeilings::default(),
        )
        .await;
        let reference = PathReference::parse(owner).expect("collection");
        let cancelled = OperationGuard::new();
        cancelled.cancel();
        assert_eq!(
            source
                .read(&reference, &cancelled, None)
                .await
                .expect_err("cancelled")
                .category(),
            ErrorCategory::Cancelled
        );
        assert_eq!(
            read(&source, &owner.replace("acme", "missing"))
                .await
                .expect_err("unmounted issue site")
                .category(),
            ErrorCategory::PermissionDenied
        );
        let registry = resourcefs_sources::CompiledSources::new(
            session.filesystem.clone(),
            resourcefs_sources::ArtifactSource::new(session.path_session().clone()),
            session.local.clone(),
            None,
            None,
            Some(source),
        )
        .await
        .expect("registry");
        for access in [
            resourcefs_core::MutationAccess::Create,
            resourcefs_core::MutationAccess::Update,
            resourcefs_core::MutationAccess::Delete,
        ] {
            assert_eq!(
                resourcefs_core::MutationAdapter::resolve(&registry, &reference, access)
                    .await
                    .expect_err("read only")
                    .category(),
                ErrorCategory::UnsupportedMutation
            );
        }
        let glob = resourcefs_core::GlobTarget::new(format!("{owner}/*")).expect("glob");
        assert!(
            resourcefs_core::DiscoveryAdapter::glob(
                &registry,
                &glob,
                resourcefs_core::GlobOptions::default(),
                &OperationGuard::new()
            )
            .await
            .is_err()
        );
        assert!(listener.requests().is_empty());
    }
    let ceilings = HttpCeilings::new(HttpCeilingsInput {
        timeout_millis: Some(1800),
        ..Default::default()
    })
    .expect("shared deadline");
    let (listener, source, _session) = fixture_with_session(
        |path| {
            if path == "/rest/api/3/project/2" {
                std::thread::sleep(Duration::from_millis(1000));
                response(&project("2", "P", "Parent").to_string())
            } else {
                protocol_response(
                    "503 Service Unavailable",
                    [("Retry-After", "1".into())],
                    Vec::new(),
                )
            }
        },
        ceilings,
    )
    .await;
    assert_eq!(
        read(&source, "jira://acme/projects/2/issues")
            .await
            .expect_err("parent consumes original deadline")
            .category(),
        ErrorCategory::SourceUnavailable
    );
    assert_eq!(listener.requests().len(), 2);
    let (listener, source, _session) = fixture_with_session(
        |_| tls::FixtureResponse::Redirect("/redirect-canary".into()),
        HttpCeilings::default(),
    )
    .await;
    let error = read(&source, "jira://acme/issues")
        .await
        .expect_err("no redirect");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert!(!error.message().contains("redirect-canary"));
    assert_eq!(listener.requests().len(), 1);
    let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
    let (listener, source, _session) = fixture_with_session(
        move |path| {
            if path == "/rest/api/3/project/2" {
                return response(&project("2", "P", "Parent").to_string());
            }
            sent.send(()).expect("notify cancellation");
            protocol_response(
                "503 Service Unavailable",
                [("Retry-After", "1".into())],
                Vec::new(),
            )
        },
        HttpCeilings::default(),
    )
    .await;
    let operation = OperationGuard::new();
    let pending_guard = operation.clone();
    let pending = tokio::spawn(async move {
        source
            .read(
                &PathReference::parse("jira://acme/projects/2/issues").expect("issues"),
                &pending_guard,
                None,
            )
            .await
    });
    received.recv().await.expect("issue attempt observed");
    operation.cancel();
    assert_eq!(
        pending
            .await
            .expect("read task")
            .expect_err("cancel during issue retry")
            .category(),
        ErrorCategory::Cancelled
    );
    assert_eq!(
        listener.requests().len(),
        2,
        "cancelled retry cannot issue a third request"
    );
}
