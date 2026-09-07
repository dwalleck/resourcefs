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
            .read(&reference, &operation)
            .await
            .expect_err("cancelled")
            .category(),
        ErrorCategory::Cancelled
    );
    for bad in [
        "jira://acme/projects:offset:0",
        "jira://acme/projects/2:offset:1",
        "jira://acme/projects:offset:1:raw",
        "jira://acme/issues",
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
