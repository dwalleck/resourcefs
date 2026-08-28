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
    time::{Duration, Instant},
};

use resourcefs_core::{
    AllowedOrigin, ErrorCategory, HttpCeilings, OperationGuard, OriginAllowlist,
};
use resourcefs_sources::{BoundedHttpResponse, HttpRequest, HttpSubstrate};
use tls::{
    FIXTURE_HOST, FixtureResponse, MATCH_CERT, TlsListener, fixture_allowlist, tls_substrate,
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
async fn retained_response_metadata_has_a_hard_byte_ceiling() {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener =
        TlsListener::serve_router(loopback, 0, MATCH_CERT, |_path| FixtureResponse::Response {
            status: "200 OK",
            headers: vec![("ETag".to_owned(), "x".repeat(16 * 1024 + 1))],
            body: Vec::new(),
        })
        .await;
    let port = listener.address.port();
    let substrate = tls_substrate(fixture_allowlist(port, true), vec![loopback]);
    let request = HttpRequest::get(
        Url::parse(&format!("https://{FIXTURE_HOST}:{port}/oversized")).expect("fixture URL"),
    );
    let error = substrate
        .fetch(request, &OperationGuard::new())
        .await
        .expect_err("oversized retained header");
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
