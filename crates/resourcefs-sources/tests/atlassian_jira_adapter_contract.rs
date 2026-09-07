#[path = "support/jira.rs"]
mod jira;
#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;
use jira::{fixture_source, fixture_source_with_ceilings, protocol_response, response};

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use resourcefs_core::{
    AllowedOrigin, AtlassianSiteId, DiscoveryEngine, ErrorCategory, HttpCeilings,
    HttpCeilingsInput, OperationGuard, OriginAllowlist, PathReference, SearchLimits, SearchOptions,
    SearchRequest, SearchTarget, ServerLimits, SourceAdapter, VersionTag,
};
use resourcefs_sources::{
    ArtifactSource, AtlassianSite, AtlassianSourceMount, CompiledSources, HttpSubstrate,
    MAX_CONFIGURATION_ENTRIES,
};
use tls::{FixtureResponse, settle};

const ISSUE: &str = r#"{
  "id":"10001","key":"NEW-2",
  "self":"https://tls.invalid/rest/api/3/issue/10001",
  "fields":{
    "summary":"Hello",
    "description":{"version":1,"type":"doc","content":[
      {"type":"paragraph","content":[{"type":"text","text":"Rich text"}]}
    ]}
  },
  "names":{"summary":"Summary","description":"Description"},
  "schema":{
    "summary":{"type":"string","system":"summary"},
    "description":{"type":"string","system":"description"}
  }
}"#;

#[tokio::test]
async fn stable_and_alias_reads_share_canonical_identity() {
    let (listener, source) = fixture_source(|path| {
        assert!(
            path.starts_with("/rest/api/3/issue/10001?")
                || path.starts_with("/rest/api/3/issue/OLD-1?")
        );
        assert!(path.contains("fields=%2Aall") || path.contains("fields=*all"));
        assert!(path.contains("fieldsByKeys=false"));
        assert!(path.contains("expand=names%2Cschema") || path.contains("expand=names%2Cschema"));
        protocol_response("200 OK", [("ETag", "\"alias-v1\"".to_owned())], ISSUE)
    })
    .await;

    let stable = source
        .read(
            &PathReference::parse("jira://acme/issues/10001").expect("stable reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("stable read");
    let alias = source
        .read(
            &PathReference::parse("jira://acme/issue-keys/OLD-1").expect("alias reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("alias read");
    let alias_repeat = source
        .read(
            &PathReference::parse("jira://acme/issue-keys/OLD-1").expect("alias reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("repeated alias read");
    assert_eq!(stable.canonical_reference(), "jira://acme/issues/10001");
    assert_eq!(alias.canonical_reference(), stable.canonical_reference());
    assert_eq!(alias.content(), stable.content());
    assert_eq!(alias_repeat.content(), stable.content());
    let heads = listener.heads();
    assert_eq!(heads.len(), 3);
    for head in &heads {
        assert!(head.to_ascii_lowercase().contains("authorization: basic "));
        assert!(!head.contains("agent@example.com"));
        assert!(!head.contains("token-canary"));
    }
    assert!(
        heads[1..]
            .iter()
            .all(|head| !head.to_ascii_lowercase().contains("if-none-match")),
        "issue-key aliases must never identify cache entries"
    );
}

#[tokio::test]
async fn aggregate_index_and_fields_preserve_media_and_tags() {
    let (_listener, source) = fixture_source(|_path| response(ISSUE)).await;
    let operation = OperationGuard::new();
    let aggregate = source
        .read(
            &PathReference::parse("jira://acme/issues/10001").expect("Aggregate"),
            &operation,
        )
        .await
        .expect("Aggregate read");
    assert_eq!(aggregate.content_type(), "text/markdown; charset=utf-8");
    assert_eq!(
        aggregate.version_tag(),
        &VersionTag::from_content(aggregate.content().as_bytes())
    );
    assert!(aggregate.content().contains("### description"));
    assert!(aggregate.content().contains("### summary"));

    let index = source
        .read(
            &PathReference::parse("jira://acme/issues/10001/fields").expect("index"),
            &operation,
        )
        .await
        .expect("index read");
    assert_eq!(index.content_type(), "text/markdown; charset=utf-8");
    assert!(index.content().contains("Field ID: description"));

    let summary = source
        .read(
            &PathReference::parse("jira://acme/issues/10001/fields/summary")
                .expect("summary Field"),
            &operation,
        )
        .await
        .expect("summary read");
    assert_eq!(summary.content_type(), "application/json; charset=utf-8");
    assert_eq!(summary.content(), r#""Hello""#);
    assert_eq!(
        summary.version_tag(),
        &VersionTag::from_content(summary.content().as_bytes())
    );

    let description = source
        .read(
            &PathReference::parse("jira://acme/issues/10001/fields/description")
                .expect("description Field"),
            &operation,
        )
        .await
        .expect("description read");
    assert_eq!(
        description.content_type(),
        "application/vnd.atlassian.adf+json; charset=utf-8"
    );
    assert!(description.content().contains(r#""type":"doc""#));
}

#[tokio::test]
async fn ordinary_field_read_ignores_unrelated_malformed_adf_projection() {
    let malformed = ISSUE.replace(
        r#"{"version":1,"type":"doc","content":[
      {"type":"paragraph","content":[{"type":"text","text":"Rich text"}]}
    ]}"#,
        r#"{"type":"doc","content":[]}"#,
    );
    let (_listener, source) = fixture_source(move |_path| response(&malformed)).await;
    let operation = OperationGuard::new();

    let summary = source
        .read(
            &PathReference::parse("jira://acme/issues/10001/fields/summary")
                .expect("summary Field"),
            &operation,
        )
        .await
        .expect("ordinary Field authority is independent");
    assert_eq!(summary.content(), r#""Hello""#);
    assert_eq!(summary.content_type(), "application/json; charset=utf-8");

    let index = source
        .read(
            &PathReference::parse("jira://acme/issues/10001/fields").expect("Field index"),
            &operation,
        )
        .await
        .expect("Field index depends on metadata, not unrelated value projections");
    assert!(index.content().contains("Field ID: summary"));
    assert!(index.content().contains("Field ID: description"));

    let aggregate = source
        .read(
            &PathReference::parse("jira://acme/issues/10001").expect("Aggregate"),
            &operation,
        )
        .await
        .expect_err("Aggregate projection validates every ADF value atomically");
    assert_eq!(aggregate.category(), ErrorCategory::SourceUnavailable);
}

#[tokio::test]
async fn selector_media_contract_and_zero_egress_refusals() {
    let (listener, source) = fixture_source(|_path| response(ISSUE)).await;

    let selected = source
        .read(
            &PathReference::parse("jira://acme/issues/10001:8-8").expect("selection"),
            &OperationGuard::new(),
        )
        .await
        .expect("selected Aggregate");
    assert_eq!(selected.content_type(), "text/markdown; charset=utf-8");

    let requests_after_selection = listener.requests().len();
    for reference in [
        "jira://acme/issues/10001/fields/summary:raw",
        "jira://missing/issues/10001",
    ] {
        let error = source
            .read(
                &PathReference::parse(reference).expect("syntactically valid refusal"),
                &OperationGuard::new(),
            )
            .await
            .expect_err("local refusal");
        assert!(matches!(
            error.category(),
            ErrorCategory::UnsupportedProjection | ErrorCategory::PermissionDenied
        ));
    }
    settle().await;
    assert_eq!(listener.requests().len(), requests_after_selection);
}
#[tokio::test]
async fn compiled_registry_routes_and_advertises_mounted_jira() {
    let (_listener, source) = fixture_source(|_path| response(ISSUE)).await;
    let scratch = session_support::scratch_fixture().await;
    let compiled = Arc::new(
        CompiledSources::new(
            scratch.filesystem.clone(),
            ArtifactSource::new(scratch.path_session().clone()),
            scratch.local.clone(),
            None,
            None,
            Some(source),
        )
        .await
        .expect("compiled Atlassian source"),
    );
    let operation = OperationGuard::new();

    let catalog = compiled
        .read(
            &PathReference::parse("rfs://").expect("catalog"),
            &operation,
        )
        .await
        .expect("catalog read");
    assert!(catalog.content().contains("jira://"));

    let issue_reference = PathReference::parse("jira://acme/issues/10001").expect("Jira reference");
    let issue = compiled
        .read(&issue_reference, &operation)
        .await
        .expect("compiled Jira read");
    assert_eq!(issue.canonical_reference(), "jira://acme/issues/10001");

    let discovery = DiscoveryEngine::new(
        Arc::clone(&compiled) as Arc<dyn resourcefs_core::DiscoveryAdapter>,
        scratch.path_session().clone(),
        ServerLimits::default(),
    );
    let searched = discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(issue_reference),
                "Hello",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("search request"),
            &operation,
        )
        .await
        .expect("compiled Jira search");
    assert!(searched.total_records() >= 1);

    let alias_search = discovery
        .search(
            SearchRequest::new(
                SearchTarget::resource(
                    PathReference::parse("jira://acme/issue-keys/OLD-1").expect("alias reference"),
                ),
                "Hello",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("alias search request"),
            &operation,
        )
        .await
        .expect("compiled Jira alias search");
    assert!(alias_search.total_records() >= 1);
    assert!(
        alias_search
            .groups()
            .iter()
            .all(|group| group.reference() == "jira://acme/issues/10001"),
        "mutable aliases must never identify SearchRecords"
    );
}

#[tokio::test]
async fn etag_revalidation_matrix() {
    let count = Arc::new(AtomicUsize::new(0));
    let router_count = Arc::clone(&count);
    let (listener, source) = fixture_source(move |_path| {
        if router_count.fetch_add(1, Ordering::SeqCst) == 0 {
            protocol_response("200 OK", [("ETag", "\"v1\"".to_owned())], ISSUE)
        } else {
            protocol_response("304 Not Modified", [], Vec::new())
        }
    })
    .await;
    let reference = PathReference::parse("jira://acme/issues/10001").expect("reference");
    let first = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("first read");
    let second = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("304 revalidation");
    assert_eq!(first, second);
    let heads = listener.heads();
    assert_eq!(heads.len(), 2);
    assert!(!heads[0].to_ascii_lowercase().contains("if-none-match"));
    assert!(
        heads[1]
            .to_ascii_lowercase()
            .contains("if-none-match: \"v1\"")
    );

    let unconditional_count = Arc::new(AtomicUsize::new(0));
    let observed_unconditional = Arc::clone(&unconditional_count);
    let (unconditional_listener, unconditional_source) = fixture_source(move |_path| {
        observed_unconditional.fetch_add(1, Ordering::SeqCst);
        response(ISSUE)
    })
    .await;
    unconditional_source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("first unconditional read");
    unconditional_source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("second unconditional read");
    assert_eq!(unconditional_count.load(Ordering::SeqCst), 2);
    assert!(
        unconditional_listener
            .heads()
            .iter()
            .all(|head| !head.to_ascii_lowercase().contains("if-none-match"))
    );

    let oversized_etag = "x".repeat(20 * 1_024);
    let first_etag = oversized_etag.clone();
    let oversized_attempts = Arc::new(AtomicUsize::new(0));
    let observed_oversized = Arc::clone(&oversized_attempts);
    let (oversized_listener, oversized_source) = fixture_source(move |_path| {
        if observed_oversized.fetch_add(1, Ordering::SeqCst) == 0 {
            protocol_response("200 OK", [("ETag", first_etag.clone())], ISSUE)
        } else {
            response(ISSUE)
        }
    })
    .await;
    oversized_source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("oversized validator first read");
    oversized_source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("oversized validator degrades to unconditional fetch");
    assert_eq!(oversized_attempts.load(Ordering::SeqCst), 2);
    assert!(
        oversized_listener
            .heads()
            .iter()
            .all(|head| !head.to_ascii_lowercase().contains("if-none-match"))
    );
    let (orphan_listener, orphan_source) =
        fixture_source(|_path| protocol_response("304 Not Modified", [], ISSUE)).await;
    let error = orphan_source
        .read(&reference, &OperationGuard::new())
        .await
        .expect_err("orphan 304");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert!(
        error
            .message()
            .contains("without a matching session cache entry"),
        "304 without authority must report the missing cache invariant"
    );
    assert_eq!(orphan_listener.requests().len(), 1);
}

#[tokio::test]
async fn empty_etag_reads_refetch_without_poisoning_the_session() {
    let count = Arc::new(AtomicUsize::new(0));
    let router_count = Arc::clone(&count);
    let (listener, source) = fixture_source(move |_path| {
        let attempt = router_count.fetch_add(1, Ordering::SeqCst);
        let body = ISSUE.replace("Hello", &format!("Revision {attempt}"));
        protocol_response(
            "200 OK",
            [
                ("ETag", String::new()),
                ("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT".to_owned()),
            ],
            body,
        )
    })
    .await;
    let reference = PathReference::parse("jira://acme/issues/10001").expect("reference");
    for attempt in 0..3 {
        let resource = source
            .read(&reference, &OperationGuard::new())
            .await
            .expect("an empty upstream ETag must not poison later reads");
        assert!(resource.content().contains(&format!("Revision {attempt}")));
    }
    let heads = listener.heads();
    assert_eq!(heads.len(), 3);
    assert!(heads.iter().all(|head| {
        let head = head.to_ascii_lowercase();
        !head.contains("if-none-match") && !head.contains("if-modified-since")
    }));
}

#[tokio::test]
async fn quoted_empty_etag_remains_a_usable_validator() {
    let count = Arc::new(AtomicUsize::new(0));
    let router_count = Arc::clone(&count);
    let (listener, source) = fixture_source(move |_path| {
        if router_count.fetch_add(1, Ordering::SeqCst) == 0 {
            protocol_response("200 OK", [("ETag", "\"\"".to_owned())], ISSUE)
        } else {
            protocol_response("304 Not Modified", [], Vec::new())
        }
    })
    .await;
    let reference = PathReference::parse("jira://acme/issues/10001").expect("reference");
    let first = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("first read");
    let second = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("quoted empty ETag revalidation");
    assert_eq!(first, second);
    let heads = listener.heads();
    assert_eq!(heads.len(), 2);
    assert!(!heads[0].to_ascii_lowercase().contains("if-none-match"));
    assert!(
        heads[1]
            .to_ascii_lowercase()
            .contains("if-none-match: \"\"")
    );
}

#[tokio::test]
async fn unexpected_redirects_fail_without_url_disclosure() {
    let (_listener, source) = fixture_source(|path| {
        if path.starts_with("/rest/api/3/issue/10001?") {
            FixtureResponse::Redirect("/rest/api/3/issue/redirected".to_owned())
        } else {
            response(ISSUE)
        }
    })
    .await;
    let reference = PathReference::parse("jira://acme/issues/10001").expect("reference");
    let error = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect_err("documented direct issue endpoint must not redirect");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);

    let (_listener, source) = fixture_source(|_path| {
        FixtureResponse::Redirect("https://outside.invalid/CANARY-SECRET".to_owned())
    })
    .await;
    let error = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect_err("out-of-origin redirect");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    assert!(!error.message().contains("CANARY-SECRET"));
    assert!(!error.message().contains("https://"));
}

#[tokio::test]
async fn status_retry_bound_cancel_redaction_matrix() {
    for (status, expected) in [
        ("401 Unauthorized", ErrorCategory::PermissionDenied),
        ("403 Forbidden", ErrorCategory::PermissionDenied),
        ("404 Not Found", ErrorCategory::NotFound),
        ("410 Gone", ErrorCategory::NotFound),
        ("413 Payload Too Large", ErrorCategory::LimitExceeded),
        (
            "500 Internal Server Error",
            ErrorCategory::SourceUnavailable,
        ),
    ] {
        let (listener, source) = fixture_source(move |_path| {
            protocol_response(
                status,
                [],
                b"token-canary malicious upstream prose".to_vec(),
            )
        })
        .await;
        let error = source
            .read(
                &PathReference::parse("jira://acme/issues/10001").expect("reference"),
                &OperationGuard::new(),
            )
            .await
            .expect_err("status failure");
        assert_eq!(error.category(), expected, "{status}");
        assert!(!error.message().contains("token-canary"));
        assert!(!error.message().contains("malicious"));
        assert_eq!(listener.requests().len(), 1);
    }

    for (status, headers) in [
        ("403 Forbidden", vec![("Retry-After", "10".to_owned())]),
        ("429 Too Many Requests", Vec::new()),
    ] {
        let (listener, source) =
            fixture_source(move |_path| protocol_response(status, headers.clone(), Vec::new()))
                .await;
        let error = source
            .read(
                &PathReference::parse("jira://acme/issues/10001").expect("reference"),
                &OperationGuard::new(),
            )
            .await
            .expect_err("rate refusal");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
        assert_eq!(listener.requests().len(), 1);
    }

    let attempts = Arc::new(AtomicUsize::new(0));
    let router_attempts = Arc::clone(&attempts);
    let (retry_listener, retry_source) = fixture_source(move |_path| {
        if router_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            protocol_response(
                "503 Service Unavailable",
                [("Retry-After", "0".to_owned())],
                Vec::new(),
            )
        } else {
            response(ISSUE)
        }
    })
    .await;
    retry_source
        .read(
            &PathReference::parse("jira://acme/issues/10001").expect("reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("bounded retry");
    assert_eq!(retry_listener.requests().len(), 2);

    let ceilings = HttpCeilings::new(HttpCeilingsInput {
        fetch_bytes: Some(64),
        ..HttpCeilingsInput::default()
    })
    .expect("ceilings");
    let (bounded_listener, bounded_source) =
        fixture_source_with_ceilings(|_path| response(ISSUE), ceilings).await;
    let error = bounded_source
        .read(
            &PathReference::parse("jira://acme/issues/10001").expect("reference"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("body ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(bounded_listener.requests().len(), 1);

    let (cancel_listener, cancel_source) = fixture_source(|_path| response(ISSUE)).await;
    let operation = OperationGuard::new();
    assert!(operation.cancel());
    let error = cancel_source
        .read(
            &PathReference::parse("jira://acme/issues/10001").expect("reference"),
            &operation,
        )
        .await
        .expect_err("cancelled before send");
    assert_eq!(error.category(), ErrorCategory::Cancelled);
    settle().await;
    assert!(cancel_listener.requests().is_empty());

    let (wait_listener, wait_source) = fixture_source(|_path| {
        protocol_response(
            "503 Service Unavailable",
            [("Retry-After", "1".to_owned())],
            Vec::new(),
        )
    })
    .await;
    let wait_operation = OperationGuard::new();
    let task_source = wait_source.clone();
    let task_operation = wait_operation.clone();
    let waiting = tokio::spawn(async move {
        task_source
            .read(
                &PathReference::parse("jira://acme/issues/10001").expect("reference"),
                &task_operation,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(wait_operation.cancel());
    let error = waiting
        .await
        .expect("read task")
        .expect_err("cancellation during retry wait");
    assert_eq!(error.category(), ErrorCategory::Cancelled);
    assert_eq!(wait_listener.requests().len(), 1);

    let (_abort_listener, abort_source) = fixture_source(|_path| FixtureResponse::Abort).await;
    let error = abort_source
        .read(
            &PathReference::parse("jira://acme/issue-keys/CANARY-SECRET").expect("canary alias"),
            &OperationGuard::new(),
        )
        .await
        .expect_err("aborted transport");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert!(!error.message().contains("CANARY-SECRET"));
    assert!(!error.message().contains("https://"));
}

#[test]
fn mount_validation_and_site_routing() {
    let substrate = Arc::new(
        HttpSubstrate::new(
            OriginAllowlist::new(Vec::new()),
            HttpCeilings::default(),
            Vec::new(),
        )
        .expect("substrate"),
    );
    let site = |id: &str, url: &str| {
        AtlassianSite::new(
            AtlassianSiteId::new(id).expect("Site ID"),
            AllowedOrigin::new(url, false).expect("origin"),
        )
    };
    let positive = site("alpha", "https://same.invalid/").expect("positive site");
    assert_eq!(positive.id().as_str(), "alpha");
    assert_eq!(
        positive.origin().base_url().host_str(),
        Some("same.invalid")
    );
    let distinct_port = site("beta", "https://same.invalid:8443/").expect("distinct port");
    AtlassianSourceMount::new(
        vec![positive.clone(), distinct_port],
        Arc::clone(&substrate),
    )
    .expect("distinct Site IDs and authorities");
    let alpha = site("alpha", "https://same.invalid/").expect("alpha");
    let duplicate_id = site("alpha", "https://other.invalid/").expect("duplicate ID");
    let duplicate_origin = site("beta", "https://same.invalid:443/").expect("duplicate origin");
    for sites in [
        Vec::new(),
        vec![alpha.clone(), duplicate_id],
        vec![alpha, duplicate_origin],
    ] {
        let error =
            AtlassianSourceMount::new(sites, Arc::clone(&substrate)).expect_err("invalid mount");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }
    for url in [
        "https://user@site.invalid/",
        "https://site.invalid/path",
        "https://site.invalid/?query=1",
        "https://site.invalid/#fragment",
    ] {
        let error = site("site", url).expect_err("non-origin Site URL");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }
}

#[test]
fn mount_validation_maximum_stays_within_budget() {
    let substrate = Arc::new(
        HttpSubstrate::new(
            OriginAllowlist::new(Vec::new()),
            HttpCeilings::default(),
            Vec::new(),
        )
        .expect("substrate"),
    );
    let sites = (0..MAX_CONFIGURATION_ENTRIES)
        .map(|index| {
            AtlassianSite::new(
                AtlassianSiteId::new(format!("site-{index}")).expect("Site ID"),
                AllowedOrigin::new(&format!("https://site-{index}.invalid/"), false)
                    .expect("origin"),
            )
            .expect("site")
        })
        .collect();
    let started = Instant::now();
    let mount = AtlassianSourceMount::new(sites, substrate).expect("maximum mount");
    let elapsed = started.elapsed();
    drop(mount);
    let budget = if cfg!(debug_assertions) {
        Duration::from_millis(100)
    } else {
        Duration::from_millis(50)
    };
    eprintln!("maximum Site Mount validation: {elapsed:?}; CI budget: {budget:?}");
    assert!(
        elapsed <= budget,
        "maximum Site Mount validation took {elapsed:?}, exceeding CI budget {budget:?}"
    );
}
