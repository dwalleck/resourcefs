//! Source-neutrality contract for the bounded HTTP substrate (rfs-g2z9 C18).
//!
//! The substrate exists so that the HTTPS, GitHub, and downstream-MCP sources
//! share one audited egress path instead of each building a client. That only
//! holds if its interface names no source-specific type — so this file is
//! written as a consumer that knows nothing about HTTPS, and its compilation
//! is the assertion.
#[path = "support/tls.rs"]
mod tls;

use std::{
    io,
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use resourcefs_core::{
    AllowedOrigin, ErrorCategory, HttpCeilings, MAX_ARTIFACT_BYTES, OperationGuard,
    OriginAllowlist, Secret,
};
use resourcefs_sources::{BoundedHttpResponse, HttpRequest, HttpSubstrate, OriginCredential};
use tls::{
    FIXTURE_HOST, FixtureResponse, MATCH_CERT, TlsListener, fixture_allowlist, settle,
    tls_substrate, tls_substrate_with_credentials,
};
use url::Url;

/// Drives the substrate using only neutral types.
///
/// The signature is the point: a caller reaches egress with a URL, a guard,
/// and nothing else. If `fetch` ever required a source-specific handle, this
/// function would stop compiling — which is exactly the regression the claim
/// forbids.
async fn neutral_consumer(
    substrate: &HttpSubstrate,
    url: Url,
) -> Result<BoundedHttpResponse, resourcefs_core::ResourceError> {
    substrate
        .fetch(HttpRequest::get(url), &OperationGuard::new())
        .await
}

#[tokio::test]
async fn substrate_is_source_neutral() {
    let allowlist = OriginAllowlist::new(vec![
        AllowedOrigin::new("https://neutral.invalid/", false).expect("origin is well formed"),
    ]);
    let substrate =
        HttpSubstrate::with_host_lookup(allowlist, HttpCeilings::default(), |_host| async {
            Ok::<_, io::Error>(vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))])
        })
        .expect("substrate builds");

    // A URL outside every declared origin is refused before egress, and the
    // neutral consumer observes that through the shared error vocabulary
    // rather than any HTTPS-specific type.
    let outside = Url::parse("https://elsewhere.invalid/doc").expect("url parses");
    let failure = neutral_consumer(&substrate, outside)
        .await
        .expect_err("an unallowlisted origin is refused");
    assert_eq!(failure.category(), ErrorCategory::PermissionDenied);

    // The ceilings the substrate enforces are core's neutral type.
    assert_eq!(
        substrate.ceilings().fetch_bytes(),
        HttpCeilings::default().fetch_bytes()
    );

    let failure = substrate
        .with_retry_control_for_test(SystemTime::now(), Duration::from_millis(251))
        .expect_err("test control cannot exceed the production jitter bound");
    assert_eq!(failure.category(), ErrorCategory::InvalidReference);
}

#[tokio::test]
async fn basic_credential_composition_is_exact_and_redacted() {
    const EMAIL: &str = "agent@example.com";
    const TOKEN: &str = "token-canary";
    const ENCODED: &str = "YWdlbnRAZXhhbXBsZS5jb206dG9rZW4tY2FuYXJ5";

    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "200 OK",
            headers: Vec::new(),
            body: b"{}".to_vec(),
        })
        .await;
    let port = listener.address.port();
    let origin = AllowedOrigin::new(&format!("https://{FIXTURE_HOST}:{port}/"), true)
        .expect("fixture origin");
    let token = Secret::new(TOKEN.to_owned()).expect("token");
    let credential =
        OriginCredential::basic(origin.clone(), EMAIL, &token).expect("Basic credential");
    let debug = format!("{credential:?}");
    for forbidden in [EMAIL, TOKEN, ENCODED] {
        assert!(!debug.contains(forbidden), "Debug leaked {forbidden}");
    }

    let substrate = tls_substrate_with_credentials(
        OriginAllowlist::new(vec![origin.clone()]),
        vec![loopback],
        vec![credential],
    );
    let url = Url::parse(&format!("https://{FIXTURE_HOST}:{port}/issue")).expect("URL");
    neutral_consumer(&substrate, url)
        .await
        .expect("authenticated fetch");
    let heads = listener.heads();
    assert_eq!(heads.len(), 1);
    let authorization = heads[0].lines().find_map(|line| {
        let (name, value) = line.split_once(": ")?;
        name.eq_ignore_ascii_case("Authorization").then_some(value)
    });
    let expected = format!("Basic {ENCODED}");
    assert_eq!(
        authorization,
        Some(expected.as_str()),
        "wire header differs from independent base64 oracle: {}",
        heads[0]
    );

    for invalid in ["", "agent:other@example.com", "agent\n@example.com"] {
        let error = OriginCredential::basic(origin.clone(), invalid, &token)
            .expect_err("invalid Basic username");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
        let message = error.message();
        assert!(invalid.is_empty() || !message.contains(invalid));
        assert!(!message.contains(TOKEN));
    }

    let maximum_username = "x".repeat(12_269);
    OriginCredential::basic(origin.clone(), &maximum_username, &token)
        .expect("largest encoded Basic value below the 16 KiB ceiling");
    let over_ceiling_username = "x".repeat(12_270);
    let error = OriginCredential::basic(origin, &over_ceiling_username, &token)
        .expect_err("encoded Basic value above the 16 KiB ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert!(!error.message().contains(&over_ceiling_username));
}

#[test]
fn basic_credential_maximum_stays_within_budget() {
    let origin = AllowedOrigin::new("https://budget.invalid/", false).expect("origin");
    let token = Secret::new("token-canary".to_owned()).expect("token");
    let username = "x".repeat(12_269);
    let started = Instant::now();
    for _ in 0..1_000 {
        OriginCredential::basic(origin.clone(), &username, &token)
            .expect("maximum bounded Basic credential");
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "1,000 maximum Basic credentials must average below the 5 ms budget"
    );
}

#[tokio::test]
async fn source_headers_and_response_metadata_are_typed_and_bounded() {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "200 OK",
            headers: vec![
                ("Content-Type".to_owned(), "application/json".to_owned()),
                ("ETag".to_owned(), "W/\"validator\"".to_owned()),
                (
                    "Link".to_owned(),
                    "<https://api.invalid/items?page=2>; rel=\"next\"".to_owned(),
                ),
                ("Retry-After".to_owned(), "7".to_owned()),
                ("X-RateLimit-Remaining".to_owned(), "0".to_owned()),
                ("Set-Cookie".to_owned(), "must-not-be-retained=1".to_owned()),
            ],
            body: b"{}".to_vec(),
        })
        .await;
    let port = listener.address.port();
    let substrate = tls_substrate(fixture_allowlist(port, true), vec![loopback]);
    let request = HttpRequest::get(
        Url::parse(&format!("https://{FIXTURE_HOST}:{port}/items")).expect("fixture URL"),
    )
    .with_header("Accept", "application/vnd.github+json")
    .expect("safe Accept")
    .with_header("User-Agent", "resourcefs-contract")
    .expect("safe User-Agent")
    .with_header("X-GitHub-Api-Version", "2026-03-10")
    .expect("safe API version")
    .with_header("If-None-Match", "W/\"previous\"")
    .expect("safe validator");

    let response = substrate
        .fetch(request, &OperationGuard::new())
        .await
        .expect("bounded response");
    assert_eq!(response.etag(), Some("W/\"validator\""));
    assert_eq!(
        response.link().expect("readable Link"),
        Some("<https://api.invalid/items?page=2>; rel=\"next\"")
    );
    assert_eq!(response.retry_after(), Some(Duration::from_secs(7)));
    assert_eq!(response.rate_limit_remaining(), Some(0));
    assert_eq!(response.body(), b"{}");

    let head = listener.heads().into_iter().next().expect("request head");
    let head = head.to_ascii_lowercase();
    for expected in [
        "accept: application/vnd.github+json",
        "user-agent: resourcefs-contract",
        "x-github-api-version: 2026-03-10",
        "if-none-match: w/\"previous\"",
    ] {
        assert!(head.contains(expected), "missing {expected} in {head}");
    }
}

#[tokio::test]
async fn mutation_request_is_bounded_and_non_redirecting() {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener = TlsListener::serve_router(loopback, 0, MATCH_CERT, |path| match path {
        "/mutate" => FixtureResponse::Redirect("/followed".to_owned()),
        "/create" => FixtureResponse::Response {
            status: "201 Created",
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: br#"{"id":7}"#.to_vec(),
        },
        "/followed" => panic!("[C15] mutation redirect was followed"),
        other => panic!("[C15] unexpected route {other}"),
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate(fixture_allowlist(port, true), vec![loopback]);
    let url = |path: &str| {
        Url::parse(&format!("https://{FIXTURE_HOST}:{port}{path}")).expect("[C15] fixture URL")
    };
    let patch_body = br#"{"title":"new"}"#.to_vec();
    let patch = substrate
        .fetch(
            HttpRequest::patch_json(url("/mutate"), patch_body.clone())
                .expect("[C15] PATCH request"),
            &OperationGuard::new(),
        )
        .await
        .expect("[C15] redirect response");
    assert_eq!(
        patch.status(),
        302,
        "[C15] redirect is returned, not followed"
    );

    let post_body = br#"{"title":"created"}"#.to_vec();
    let post = substrate
        .fetch(
            HttpRequest::post_json(url("/create"), post_body.clone()).expect("[C15] POST request"),
            &OperationGuard::new(),
        )
        .await
        .expect("[C15] create response");
    assert_eq!(post.status(), 201);

    let exact = HttpRequest::post_json(url("/create"), vec![b'x'; MAX_ARTIFACT_BYTES])
        .expect("[C15] exact request-body ceiling");
    std::hint::black_box(exact);
    let encoded_envelope =
        HttpRequest::post_json(url("/create"), vec![b'x'; MAX_ARTIFACT_BYTES + 64 * 1024])
            .expect("[C15] encoded envelope above content ceiling");
    std::hint::black_box(encoded_envelope);

    settle().await;
    assert_eq!(
        listener.requests(),
        vec![
            "PATCH /mutate HTTP/1.1".to_owned(),
            "POST /create HTTP/1.1".to_owned()
        ],
        "[C15] exact methods and zero redirect follow-up"
    );
    assert_eq!(
        listener.bodies(),
        vec![patch_body, post_body],
        "[C15] complete request bodies"
    );
    assert!(
        listener.heads().iter().all(|head| head
            .to_ascii_lowercase()
            .contains("content-type: application/json")),
        "[C15] JSON content type"
    );
}

#[tokio::test]
#[ignore = "checkpointed-build production-scale budget"]
async fn http_mutation_request_budget() {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "201 Created",
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: b"{}".to_vec(),
        })
        .await;
    let port = listener.address.port();
    let substrate = tls_substrate(fixture_allowlist(port, true), vec![loopback]);
    let url =
        Url::parse(&format!("https://{FIXTURE_HOST}:{port}/maximum")).expect("[C15] fixture URL");
    let request =
        HttpRequest::post_json(url, vec![b'x'; MAX_ARTIFACT_BYTES]).expect("[C15] maximum request");
    let started = Instant::now();
    let response = substrate
        .fetch(request, &OperationGuard::new())
        .await
        .expect("[C15] maximum request response");
    assert_eq!(response.status(), 201);
    let elapsed = started.elapsed();
    assert!(
        elapsed <= Duration::from_millis(100),
        "[C15] 64 MiB loopback mutation request took {elapsed:?}"
    );
    assert_eq!(
        listener.bodies().first().map(Vec::len),
        Some(MAX_ARTIFACT_BYTES),
        "[C15] complete maximum body observed"
    );
}

#[tokio::test]
async fn oversized_etag_degrades_but_link_metadata_ceiling_is_hard() {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let etag_listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "200 OK",
            headers: vec![("ETag".to_owned(), "x".repeat(16 * 1024 + 1))],
            body: Vec::new(),
        })
        .await;
    let etag_port = etag_listener.address.port();
    let substrate = tls_substrate(fixture_allowlist(etag_port, true), vec![loopback]);
    let request = HttpRequest::get(
        Url::parse(&format!("https://{FIXTURE_HOST}:{etag_port}/oversized")).expect("fixture URL"),
    );
    let response = substrate
        .fetch(request, &OperationGuard::new())
        .await
        .expect("oversized optional validator degrades to absence");
    assert_eq!(response.etag(), None);

    let link_listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "200 OK",
            headers: vec![("Link".to_owned(), "x".repeat(16 * 1024 + 1))],
            body: Vec::new(),
        })
        .await;
    let link_port = link_listener.address.port();
    let substrate = tls_substrate(fixture_allowlist(link_port, true), vec![loopback]);
    let request = HttpRequest::get(
        Url::parse(&format!("https://{FIXTURE_HOST}:{link_port}/oversized")).expect("fixture URL"),
    );
    let error = substrate
        .fetch(request, &OperationGuard::new())
        .await
        .expect_err("oversized authoritative continuation metadata");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
}

/// A validator or Link the substrate cannot read as text must not fail a
/// read that never uses it. The ETag degrades to absence at the point of use;
/// the Link surfaces as an error only when a caller asks to paginate.
#[tokio::test]
async fn unreadable_metadata_headers_fail_only_the_callers_that_need_them() {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "200 OK",
            headers: vec![
                // obs-text bytes (0xC3 0xA9): valid on the wire, not visible ASCII.
                ("ETag".to_owned(), "\"caf\u{e9}\"".to_owned()),
                (
                    "Link".to_owned(),
                    "<https://api.invalid/items?page=caf\u{e9}>; rel=\"next\"".to_owned(),
                ),
            ],
            body: b"body".to_vec(),
        })
        .await;
    let port = listener.address.port();
    let substrate = tls_substrate(fixture_allowlist(port, true), vec![loopback]);
    let request = HttpRequest::get(
        Url::parse(&format!("https://{FIXTURE_HOST}:{port}/odd-headers")).expect("fixture URL"),
    );
    let response = substrate
        .fetch(request, &OperationGuard::new())
        .await
        .expect("a plain read is not failed by metadata it never uses");
    assert_eq!(response.status(), 200);
    assert_eq!(response.body(), b"body");
    assert_eq!(response.etag(), None, "an unechoable validator is absent");
    let error = response
        .link()
        .expect_err("an unreadable continuation is corrupt, not the last page");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
}

#[test]
fn source_headers_cannot_override_authority_or_framing() {
    let url = Url::parse("https://neutral.invalid/").expect("URL");
    for forbidden in [
        "Authorization",
        "Proxy-Authorization",
        "Cookie",
        "Host",
        "Connection",
        "Transfer-Encoding",
        "Content-Length",
    ] {
        let error = HttpRequest::get(url.clone())
            .with_header(forbidden, "attacker-controlled")
            .expect_err("authority/framing header must be refused");
        assert_eq!(
            error.category(),
            ErrorCategory::InvalidReference,
            "{forbidden}"
        );
    }
}

#[test]
fn source_header_count_bytes_and_duplicates_are_bounded() {
    let url = Url::parse("https://neutral.invalid/").expect("URL");
    let mut request = HttpRequest::get(url.clone());
    for index in 0..16 {
        request = request
            .with_header(format!("X-Safe-{index}"), "value")
            .expect("within count ceiling");
    }
    assert_eq!(
        request
            .with_header("X-Safe-Overflow", "value")
            .expect_err("header count ceiling")
            .category(),
        ErrorCategory::LimitExceeded
    );

    assert_eq!(
        HttpRequest::get(url.clone())
            .with_header("X-Large", "x".repeat(16 * 1024 + 1))
            .expect_err("header byte ceiling")
            .category(),
        ErrorCategory::LimitExceeded
    );
    assert_eq!(
        HttpRequest::get(url)
            .with_header("Accept", "text/plain")
            .expect("first header")
            .with_header("accept", "application/json")
            .expect_err("duplicate header")
            .category(),
        ErrorCategory::InvalidReference
    );
}

#[tokio::test]
async fn idempotent_reads_honor_delta_and_http_date_once() {
    let fixed_now = UNIX_EPOCH + Duration::from_secs(784_111_776);
    for (status, guidance) in [
        ("429 Too Many Requests", "0"),
        ("503 Service Unavailable", "Sun, 06 Nov 1994 08:49:37 GMT"),
    ] {
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let attempts = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&attempts);
        let listener = TlsListener::serve_router(loopback, 0, MATCH_CERT, move |_path| {
            if counted.fetch_add(1, Ordering::AcqRel) == 0 {
                FixtureResponse::Response {
                    status,
                    headers: vec![("Retry-After".to_owned(), guidance.to_owned())],
                    body: Vec::new(),
                }
            } else {
                FixtureResponse::Response {
                    status: "200 OK",
                    headers: Vec::new(),
                    body: b"retried".to_vec(),
                }
            }
        })
        .await;
        let port = listener.address.port();
        let substrate = tls_substrate(fixture_allowlist(port, true), vec![loopback])
            .with_retry_control_for_test(fixed_now, Duration::ZERO)
            .expect("valid deterministic retry controls");
        let response = substrate
            .fetch(
                HttpRequest::get(
                    Url::parse(&format!("https://{FIXTURE_HOST}:{port}/retry"))
                        .expect("fixture URL"),
                ),
                &OperationGuard::new(),
            )
            .await
            .expect("valid guidance retries");

        assert_eq!(response.body(), b"retried", "{status}");
        assert_eq!(attempts.load(Ordering::Acquire), 2, "{status}");
        settle().await;
        assert_eq!(listener.requests().len(), 2, "{status}");
    }
}

#[tokio::test]
async fn retry_wait_is_promptly_cancelled() {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "429 Too Many Requests",
            headers: vec![("Retry-After".to_owned(), "10".to_owned())],
            body: Vec::new(),
        })
        .await;
    let port = listener.address.port();
    let substrate = Arc::new(
        tls_substrate(fixture_allowlist(port, true), vec![loopback])
            .with_retry_control_for_test(SystemTime::now(), Duration::ZERO)
            .expect("valid deterministic retry controls"),
    );
    let guard = OperationGuard::new();
    let worker_guard = guard.clone();
    let worker_substrate = Arc::clone(&substrate);
    let task = tokio::spawn(async move {
        worker_substrate
            .fetch(
                HttpRequest::get(
                    Url::parse(&format!("https://{FIXTURE_HOST}:{port}/cancel"))
                        .expect("fixture URL"),
                ),
                &worker_guard,
            )
            .await
    });

    for _ in 0..100 {
        if !listener.requests().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(listener.requests().len(), 1, "wait began after request one");
    assert!(guard.cancel(), "active retry wait accepts cancellation");
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("cancellation is prompt")
        .expect("retry task joins")
        .expect_err("cancelled wait fails");

    assert_eq!(error.category(), ErrorCategory::Cancelled);
    settle().await;
    assert_eq!(listener.requests().len(), 1, "no follow-up was sent");
}

#[tokio::test]
async fn retry_is_limited_to_idempotent_429_and_503_reads() {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "500 Internal Server Error",
            headers: vec![("Retry-After".to_owned(), "0".to_owned())],
            body: Vec::new(),
        })
        .await;
    let port = listener.address.port();
    let substrate = tls_substrate(fixture_allowlist(port, true), vec![loopback])
        .with_retry_control_for_test(SystemTime::now(), Duration::ZERO)
        .expect("valid deterministic retry controls");
    let response = substrate
        .fetch(
            HttpRequest::get(
                Url::parse(&format!("https://{FIXTURE_HOST}:{port}/status")).expect("fixture URL"),
            ),
            &OperationGuard::new(),
        )
        .await
        .expect("non-retryable status remains a response");

    assert_eq!(response.status(), 500);
    settle().await;
    assert_eq!(listener.requests().len(), 1);

    let mutation_listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "503 Service Unavailable",
            headers: vec![("Retry-After".to_owned(), "0".to_owned())],
            body: Vec::new(),
        })
        .await;
    let mutation_port = mutation_listener.address.port();
    let mutation_url = Url::parse(&format!("https://{FIXTURE_HOST}:{mutation_port}/mutation"))
        .expect("fixture URL");
    let mutation_substrate = tls_substrate(fixture_allowlist(mutation_port, true), vec![loopback])
        .with_retry_control_for_test(SystemTime::now(), Duration::ZERO)
        .expect("valid deterministic retry controls");
    let guard = OperationGuard::new();
    for request in [
        HttpRequest::post_json(mutation_url.clone(), b"{}".to_vec()).expect("POST"),
        HttpRequest::patch_json(mutation_url.clone(), b"{}".to_vec()).expect("PATCH"),
    ] {
        let response = mutation_substrate
            .fetch(request, &guard)
            .await
            .expect("mutation response remains observable");
        assert_eq!(response.status(), 503);
    }
    settle().await;
    assert_eq!(
        mutation_listener.requests().len(),
        2,
        "one request per mutation method"
    );
}

#[test]
#[ignore = "checkpointed-build production-scale budget"]
fn http_metadata_budget() {
    let url = Url::parse("https://neutral.invalid/").expect("URL");
    let value = "x".repeat(1_000);
    let iterations = 1_000_u32;
    let started = Instant::now();
    for _ in 0..iterations {
        let mut request = HttpRequest::get(url.clone());
        for index in 0..16 {
            request = request
                .with_header(format!("X-Safe-{index}"), &value)
                .expect("maximum safe headers");
        }
        std::hint::black_box(request);
    }
    let average = started.elapsed() / iterations;
    assert!(
        average <= Duration::from_millis(1),
        "maximum HTTP metadata construction took {average:?}"
    );
}
