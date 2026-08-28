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
};

use resourcefs_core::{
    DiscoveryAdapter, ErrorCategory, OperationGuard, PathReference, SearchOptions, SearchTarget,
    SourceAdapter,
};
use resourcefs_sources::HttpsSource;
use tls::{
    FIXTURE_HOST, FixtureResponse, MATCH_CERT, TlsListener, fixture_allowlist, tls_substrate,
};

const ERROR_PAGE: &[u8] =
    b"<html><body><h1>Sorry, this page could not be found</h1><p>Try the index.</p></body></html>";

async fn source_serving(status: &'static str) -> (u16, HttpsSource) {
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let listener = TlsListener::serve_router(loopback, 0, MATCH_CERT, move |_path| {
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
