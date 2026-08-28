//! Live read-only smoke over real public HTTPS origins.
//!
//! The permanent HTTPS contracts run against fixture HTML served by a local
//! TLS listener. Reader-mode extraction, redirect following, and validators
//! were tuned on those fixtures; the shapes a real origin serves — CDN
//! redirects, real document markup, real 404 pages — are exactly what a
//! fixture cannot be trusted to imitate, so this suite reads them for real.
//!
//! Ignored by default and skipped without `RFS_LIVE=1`; `scripts/live-smoke.sh`
//! runs it with every other `live_*` row.
#[path = "support/mod.rs"]
mod session_support;

use std::sync::Arc;

use resourcefs_core::{
    AllowedOrigin, DiscoveryEngine, ErrorCategory, HttpCeilings, OperationGuard, OriginAllowlist,
    PathReference, SearchLimits, SearchOptions, SearchRequest, SearchTarget, ServerLimits,
    SourceAdapter, SourceResource,
};
use resourcefs_sources::{HttpSubstrate, HttpsSource};

const BOOK_PAGE: &str = "https://doc.rust-lang.org/stable/book/ch01-01-installation.html";
const MISSING_PAGE: &str = "https://doc.rust-lang.org/stable/book/no-such-page-for-resourcefs.html";
/// docs.rs answers this with a `302` to `/serde/latest/serde/` — a real
/// same-origin redirect chain rather than a fixture's.
const REDIRECTED_CRATE: &str = "https://docs.rs/serde";

fn live_enabled() -> bool {
    std::env::var_os("RFS_LIVE").is_some_and(|value| value == "1")
}

fn live_source() -> HttpsSource {
    let origins = ["https://doc.rust-lang.org/", "https://docs.rs/"]
        .into_iter()
        .map(|origin| AllowedOrigin::new(origin, false).expect("origin"))
        .collect();
    let substrate = HttpSubstrate::new(
        OriginAllowlist::new(origins),
        HttpCeilings::default(),
        Vec::new(),
    )
    .expect("substrate");
    HttpsSource::new(Arc::new(substrate))
}

async fn read(source: &HttpsSource, reference: &str) -> SourceResource {
    source
        .read(
            &PathReference::parse(reference).expect("reference"),
            &OperationGuard::new(),
        )
        .await
        .unwrap_or_else(|error| panic!("{reference}: {:?} {}", error.category(), error.message()))
}

async fn read_err(source: &HttpsSource, reference: &str) -> ErrorCategory {
    source
        .read(
            &PathReference::parse(reference).expect("reference"),
            &OperationGuard::new(),
        )
        .await
        .map(|resource| {
            panic!(
                "{reference}: expected an error, read {} bytes",
                resource.content().len()
            )
        })
        .unwrap_err()
        .category()
}

#[tokio::test]
#[ignore = "live HTTPS smoke; needs RFS_LIVE=1"]
async fn live_https_reads_hold_up() {
    if !live_enabled() {
        eprintln!("RFS_LIVE is not 1; skipping the live HTTPS smoke");
        return;
    }
    let source = live_source();

    // H1 — reader mode over a real documentation page yields the article,
    // deterministically: two reads produce one Version Tag.
    let page = read(&source, BOOK_PAGE).await;
    assert_eq!(page.canonical_reference(), BOOK_PAGE);
    for expected in ["Installation", "rustup"] {
        assert!(
            page.content().contains(expected),
            "H1 reader mode lacks {expected}:\n{}",
            &page.content()[..page.content().len().min(600)]
        );
    }
    assert!(
        !page.content().contains("<html"),
        "H1 reader mode must not leak markup"
    );
    let again = read(&source, BOOK_PAGE).await;
    assert_eq!(
        page.version_tag(),
        again.version_tag(),
        "H1 extraction is stable"
    );
    eprintln!(
        "H1 reader mode: {} bytes, {} lines",
        page.content().len(),
        page.content().lines().count()
    );

    // H2 — `:raw` returns the document as served, and a line selection on it
    // is a bounded slice of that same Resource.
    let raw = read(&source, &format!("{BOOK_PAGE}:raw")).await;
    assert!(
        raw.content().to_ascii_lowercase().contains("<html"),
        "H2 raw read is the served markup"
    );
    assert_eq!(raw.canonical_reference(), BOOK_PAGE);
    let slice = read(&source, &format!("{BOOK_PAGE}:1-5")).await;
    assert!(slice.content().lines().count() <= 5);
    eprintln!("H2 raw: {} bytes; :1-5 slice ok", raw.content().len());

    // H3 — a real same-origin redirect is followed and the Resource keeps the
    // requested identity.
    let redirected = read(&source, REDIRECTED_CRATE).await;
    assert_eq!(redirected.canonical_reference(), REDIRECTED_CRATE);
    assert!(
        redirected.content().to_ascii_lowercase().contains("serde"),
        "H3 redirect target was rendered"
    );
    eprintln!("H3 redirect: {} bytes", redirected.content().len());

    // H4 — a real 404 is `not_found`; an origin outside the allowlist is
    // refused before egress.
    assert_eq!(
        read_err(&source, MISSING_PAGE).await,
        ErrorCategory::NotFound,
        "H4 a real 404"
    );
    assert_eq!(
        read_err(&source, "https://www.rust-lang.org/").await,
        ErrorCategory::PermissionDenied,
        "H4 an unallowlisted origin"
    );
    eprintln!("H4 refusals: not_found, permission_denied");

    // H5 — search runs over the reader-mode rendering and attributes hits to
    // the canonical URL at the lines a read displays.
    let session = session_support::scratch_fixture().await;
    let engine = DiscoveryEngine::new(
        Arc::new(source),
        session.path_session().clone(),
        ServerLimits::default(),
    );
    let search = engine
        .search(
            SearchRequest::new(
                SearchTarget::resource(PathReference::parse(BOOK_PAGE).expect("target")),
                "rustup",
                SearchOptions::default(),
                0,
                SearchLimits::default(),
            )
            .expect("search request"),
            &OperationGuard::new(),
        )
        .await
        .expect("live search");
    assert!(search.total_records() > 0, "H5 search found nothing");
    let group = &search.groups()[0];
    assert_eq!(group.reference(), BOOK_PAGE);
    let line = group.lines()[0].line();
    let displayed = page
        .content()
        .lines()
        .nth(usize::try_from(line).expect("line index") - 1)
        .expect("hit line exists in the read");
    assert_eq!(
        displayed,
        group.lines()[0].text(),
        "H5 hit matches the read"
    );
    eprintln!(
        "H5 search: {} records, first at line {line}",
        search.total_records()
    );
}
