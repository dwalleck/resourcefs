//! Upstream status classification for the HTTPS Source Adapter (rfs-0ox5).
//!
//! Found live, fenced here: a real `404` page was rendered and returned as a
//! successful Resource because nothing consulted the status. The permanent
//! fixtures had only ever served `200` documents to the HTTPS source, so the
//! deterministic suite could not see it — which is exactly the class of gap
//! the live smokes exist for, and this file is the permanent fence that keeps
//! it closed.
#[path = "support/tls.rs"]
mod tls;

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use resourcefs_core::{
    DiscoveryAdapter, ErrorCategory, HttpCeilings, HttpCeilingsInput, OperationGuard,
    PathReference, SearchOptions, SearchTarget, SourceAdapter,
};
use resourcefs_sources::HttpsSource;
use tls::{
    FIXTURE_HOST, FixtureResponse, TlsListener, fixture_allowlist, match_cert, settle,
    tls_substrate, tls_substrate_with_ceilings,
};

const ERROR_PAGE: &[u8] =
    b"<html><body><h1>Sorry, this page could not be found</h1><p>Try the index.</p></body></html>";

async fn source_serving(status: &'static str) -> (u16, HttpsSource) {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener = TlsListener::serve_router(loopback, 0, match_cert(), move |_path| {
        FixtureResponse::Response {
            status,
            headers: vec![(
                "Content-Type".to_owned(),
                "text/html; charset=utf-8".to_owned(),
            )],
            body: ERROR_PAGE.to_vec(),
        }
    })
    .await;
    let port = listener.address.port();
    let source = HttpsSource::new(Arc::new(tls_substrate(
        fixture_allowlist(port, true),
        vec![loopback],
    )));
    // The listener lives as long as the source does.
    std::mem::forget(listener);
    (port, source)
}

/// Every non-2xx status maps to one stable category — on the reader-mode
/// read, the `:raw` read, a selected read, and search alike — and the error
/// never carries the page's prose.
#[tokio::test]
async fn upstream_statuses_map_to_stable_categories_before_any_body_is_content() {
    for (status, category) in [
        ("401 Unauthorized", ErrorCategory::PermissionDenied),
        ("403 Forbidden", ErrorCategory::PermissionDenied),
        ("404 Not Found", ErrorCategory::NotFound),
        ("410 Gone", ErrorCategory::NotFound),
        ("429 Too Many Requests", ErrorCategory::SourceUnavailable),
        (
            "500 Internal Server Error",
            ErrorCategory::SourceUnavailable,
        ),
        ("503 Service Unavailable", ErrorCategory::SourceUnavailable),
    ] {
        let (port, source) = source_serving(status).await;
        let reference = format!("https://{FIXTURE_HOST}:{port}/missing");
        for spelling in [
            reference.clone(),
            format!("{reference}:raw"),
            format!("{reference}:1-3"),
        ] {
            let error = source
                .read(
                    &PathReference::parse(spelling.clone()).expect("reference"),
                    &OperationGuard::new(),
                )
                .await
                .expect_err("an error page is not a Resource");
            assert_eq!(error.category(), category, "{status} {spelling}");
            assert!(
                !error.message().contains("Sorry") && !error.message().contains("index"),
                "{status}: body prose reached the error: {}",
                error.message()
            );
        }
        let search = DiscoveryAdapter::search(
            &source,
            &SearchTarget::resource(PathReference::parse(reference.clone()).expect("reference")),
            "Sorry",
            SearchOptions::default(),
            &OperationGuard::new(),
        )
        .await
        .expect_err("an error page is not searchable content");
        assert_eq!(search.category(), category, "{status} search");
    }

    // Positive control: the same markup under 200 is a Resource, so the
    // refusals above come from the status and not from the page shape.
    let (port, source) = source_serving("200 OK").await;
    let rendered = source
        .read(
            &PathReference::parse(format!("https://{FIXTURE_HOST}:{port}/present"))
                .expect("reference"),
            &OperationGuard::new(),
        )
        .await
        .expect("a 200 page renders");
    assert!(
        rendered
            .content()
            .contains("Sorry, this page could not be found")
    );
}

#[tokio::test]
async fn unusable_retry_guidance_is_terminal_without_wait() {
    let ceilings = HttpCeilings::new(HttpCeilingsInput {
        timeout_millis: Some(1_000),
        ..HttpCeilingsInput::default()
    })
    .expect("lowered logical deadline");
    let fixed_now = UNIX_EPOCH + Duration::from_secs(784_111_777);
    for guidance in [
        None,
        Some(""),
        Some("-1"),
        Some("+1"),
        Some("18446744073709551616"),
        Some("Sun, 06 Nov 1994 08:49:36 GMT"),
        Some("0, 1"),
        Some("60"),
    ] {
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let listener = TlsListener::serve_router(loopback, 0, match_cert(), move |_path| {
            FixtureResponse::Response {
                status: "429 Too Many Requests",
                headers: guidance
                    .map(|value| vec![("Retry-After".to_owned(), value.to_owned())])
                    .unwrap_or_default(),
                body: Vec::new(),
            }
        })
        .await;
        let port = listener.address.port();
        let substrate =
            tls_substrate_with_ceilings(fixture_allowlist(port, true), vec![loopback], ceilings)
                .with_retry_control_for_test(fixed_now, Duration::ZERO)
                .expect("valid deterministic retry controls");
        let source = HttpsSource::new(Arc::new(substrate));
        let error = source
            .read(
                &PathReference::parse(format!("https://{FIXTURE_HOST}:{port}/retry:raw"))
                    .expect("fixture reference"),
                &OperationGuard::new(),
            )
            .await
            .expect_err("unusable guidance is terminal");

        assert_eq!(
            error.category(),
            ErrorCategory::SourceUnavailable,
            "{guidance:?}"
        );
        settle().await;
        assert_eq!(listener.requests().len(), 1, "{guidance:?}");
    }
}
