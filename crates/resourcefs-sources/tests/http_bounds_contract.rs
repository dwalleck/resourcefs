//! Body-bound, cancellation, and timeout contracts for the substrate
//! (rfs-g2z9 C16).
//!
//! The ceiling here bounds what ResourceFS **accepts and retains**, not what
//! the peer transmits. `prove-it-prototype` measured a server flushing roughly
//! three megabytes against a one-megabyte client ceiling: socket and TLS
//! buffers sit between the two, so transfer is not the thing a client-side
//! ceiling can bound. Every assertion below is therefore made against retained
//! bytes, and the server's flushed count is recorded as an *observation* of
//! transfer rather than asserted to be bounded. Writing the stronger claim
//! would be writing a falsehood.
//!
//! The oracle is server-side wherever one exists — bytes the listener actually
//! wrote and flushed, and whether it stopped early because the peer went away.
//!
//! Each abandonment row carries a **control**: the same fixture, uncancelled
//! and untimed-out, must succeed. Without it, a row asserting "the request
//! failed with `cancelled`" could pass because the request failed for some
//! unrelated reason, which is how four fixtures in this project have already
//! passed while proving nothing.

#[path = "support/tls.rs"]
mod tls;

use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use resourcefs_core::{ErrorCategory, HttpCeilings, HttpCeilingsInput, OperationGuard};
use resourcefs_sources::HttpRequest;
use tls::{
    FIXTURE_HOST, FixtureResponse, MATCH_CERT, TlsListener, fixture_allowlist, settle,
    tls_substrate_with_ceilings,
};
use url::Url;

const LOOPBACK: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);

fn fixture_url(port: u16) -> Url {
    Url::parse(&format!("https://{FIXTURE_HOST}:{port}/doc")).expect("url parses")
}

/// Ceilings lowered so a fixture crosses them in milliseconds.
///
/// Lowering is the supported direction, so these exercise the same code path
/// production uses at its shipped 8 MiB / 30 s values.
fn ceilings(fetch_bytes: usize, timeout_millis: usize) -> HttpCeilings {
    HttpCeilings::new(HttpCeilingsInput {
        fetch_bytes: Some(fetch_bytes),
        redirect_depth: None,
        timeout_millis: Some(timeout_millis),
    })
    .expect("lowered ceilings validate")
}

/// A body slow enough to still be arriving when a fixture acts on it.
const TRICKLE_LEN: usize = 256 * 1024;
const TRICKLE_CHUNK: usize = 32 * 1024;
const TRICKLE_DELAY: Duration = Duration::from_millis(50);

/// C16 — a cancelled read returns `cancelled` and abandons the connection.
///
/// The control row runs first and deliberately: it proves the trickling body
/// *does* complete when nothing cancels it. Only against that baseline does the
/// cancelled row's failure mean "cancellation stopped it" rather than "this
/// fixture never worked".
#[tokio::test]
async fn cancelled_read_abandons_connection() {
    // Control — same body, no cancellation: succeeds, whole body retained.
    let control = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let control_port = control.address.port();
    let control_substrate = tls_substrate_with_ceilings(
        fixture_allowlist(control_port, true),
        vec![IpAddr::V4(LOOPBACK)],
        ceilings(TRICKLE_LEN * 4, 30_000),
    );
    let served = control_substrate
        .fetch(
            HttpRequest::get(fixture_url(control_port)),
            &OperationGuard::new(),
        )
        .await
        .expect("an uncancelled trickle completes");
    assert_eq!(
        served.body().len(),
        TRICKLE_LEN,
        "the control must retain the whole body, otherwise the cancelled row \
         proves nothing about cancellation"
    );

    // Cancelled — the same fixture, cancelled while the body is still arriving.
    let listener = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![IpAddr::V4(LOOPBACK)],
        ceilings(TRICKLE_LEN * 4, 30_000),
    );

    let guard = OperationGuard::new();
    let canceller = guard.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        canceller.cancel();
    });

    let error = substrate
        .fetch(HttpRequest::get(fixture_url(port)), &guard)
        .await
        .expect_err("a cancelled read does not return a response");

    // Category alone is not enough: a timeout also fails this request, and a
    // transport error would too. The message pins which control fired.
    assert_eq!(
        error.category(),
        ErrorCategory::Cancelled,
        "cancellation must not be reported as a transport failure: {error}"
    );
    assert!(
        error.message().contains("cancel"),
        "the refusal must name cancellation, not merely share its category: {error}"
    );

    settle().await;
    assert!(
        listener.flushed() < TRICKLE_LEN,
        "the server must stop well short of the body once the peer goes away; \
         flushed {} of {TRICKLE_LEN}",
        listener.flushed()
    );
}

/// C16 — a request outliving the timeout returns `source_unavailable`.
///
/// The control uses the same trickling body under a generous timeout, so the
/// timed-out row cannot pass merely because the fixture is broken.
#[tokio::test]
async fn timeout_returns_source_unavailable() {
    let control = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let control_port = control.address.port();
    let control_substrate = tls_substrate_with_ceilings(
        fixture_allowlist(control_port, true),
        vec![IpAddr::V4(LOOPBACK)],
        ceilings(TRICKLE_LEN * 4, 30_000),
    );
    assert_eq!(
        control_substrate
            .fetch(
                HttpRequest::get(fixture_url(control_port)),
                &OperationGuard::new(),
            )
            .await
            .expect("a trickle inside the timeout completes")
            .body()
            .len(),
        TRICKLE_LEN,
        "the control must complete, otherwise the timeout row proves nothing"
    );

    // The same body under a timeout it cannot possibly meet.
    let listener = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![IpAddr::V4(LOOPBACK)],
        ceilings(TRICKLE_LEN * 4, 250),
    );

    let error = substrate
        .fetch(HttpRequest::get(fixture_url(port)), &OperationGuard::new())
        .await
        .expect_err("a body outliving the timeout does not return a response");

    assert_eq!(
        error.category(),
        ErrorCategory::SourceUnavailable,
        "a timeout is an availability failure, not a policy refusal: {error}"
    );
    assert!(
        error.message().contains("timeout"),
        "the refusal must name the timeout rather than surfacing an opaque \
         transport error: {error}"
    );
}

/// C16 — retained bytes never exceed the ceiling, whatever the peer sends.
///
/// The flushed count is recorded, not asserted: the ceiling is a bound on what
/// ResourceFS accepts, and the probe measured transfer running well past it.
#[tokio::test]
async fn retained_body_never_exceeds_the_ceiling() {
    const CEILING: usize = 64 * 1024;
    const BODY: usize = 1024 * 1024;

    let listener = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::sized(BODY)
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![IpAddr::V4(LOOPBACK)],
        ceilings(CEILING, 30_000),
    );

    let response = substrate
        .fetch(HttpRequest::get(fixture_url(port)), &OperationGuard::new())
        .await
        .expect("an over-ceiling body is truncated, not an error at this layer");

    assert_eq!(
        response.body().len(),
        CEILING,
        "retained bytes must stop exactly at the ceiling"
    );
    assert!(
        response.truncated(),
        "a body cut at the ceiling must report itself truncated, so the caller \
         never mistakes a partial document for a whole one"
    );

    settle().await;
    // Observation, deliberately not an assertion. Transfer is not bounded by a
    // client-side ceiling and asserting otherwise would encode a falsehood.
    println!(
        "observed transfer: server flushed {} bytes against a {CEILING}-byte accept ceiling",
        listener.flushed()
    );
}

/// C16 — cancelling before the call opens no socket at all.
#[tokio::test]
async fn cancellation_before_egress_opens_no_socket() {
    let listener = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::sized(1024)
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![IpAddr::V4(LOOPBACK)],
        ceilings(64 * 1024, 30_000),
    );

    let guard = OperationGuard::new();
    guard.cancel();

    let error = substrate
        .fetch(HttpRequest::get(fixture_url(port)), &guard)
        .await
        .expect_err("a cancelled operation performs no egress");
    assert_eq!(error.category(), ErrorCategory::Cancelled);

    settle().await;
    assert_eq!(
        listener.accepts(),
        0,
        "cancellation before egress must not reach the network at all"
    );
}

/// C16 — odd body framings stay bounded and are still reported truncated.
///
/// A `Content-Length` larger than the bytes actually sent, and a body with no
/// declared length at all, are both shapes a hostile or broken origin can
/// produce. Neither may read unbounded, and neither may be mistaken for a
/// complete document when it is cut at the ceiling.
#[tokio::test]
async fn odd_body_framings_stay_bounded() {
    const CEILING: usize = 32 * 1024;

    // Declares far more than it sends; the connection close ends the body.
    let short = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::short_count(4 * 1024 * 1024, 8 * 1024)
    })
    .await;
    let short_port = short.address.port();
    let short_substrate = tls_substrate_with_ceilings(
        fixture_allowlist(short_port, true),
        vec![IpAddr::V4(LOOPBACK)],
        ceilings(CEILING, 2_000),
    );
    let short_result = short_substrate
        .fetch(
            HttpRequest::get(fixture_url(short_port)),
            &OperationGuard::new(),
        )
        .await;
    match short_result {
        Ok(response) => assert!(
            response.body().len() <= CEILING,
            "a short-counted body must never exceed the accept ceiling"
        ),
        Err(error) => assert_eq!(
            error.category(),
            ErrorCategory::SourceUnavailable,
            "a truncated-by-the-peer body is an availability failure, never a \
             silent success: {error}"
        ),
    }

    // No declared length: the body is delimited by the connection closing.
    let undeclared = TlsListener::serve_router(LOOPBACK, 0, MATCH_CERT, |_path| {
        FixtureResponse::undeclared(256 * 1024)
    })
    .await;
    let undeclared_port = undeclared.address.port();
    let undeclared_substrate = tls_substrate_with_ceilings(
        fixture_allowlist(undeclared_port, true),
        vec![IpAddr::V4(LOOPBACK)],
        ceilings(CEILING, 30_000),
    );
    let response = undeclared_substrate
        .fetch(
            HttpRequest::get(fixture_url(undeclared_port)),
            &OperationGuard::new(),
        )
        .await
        .expect("an undeclared-length body is bounded, not refused");
    assert_eq!(
        response.body().len(),
        CEILING,
        "a body with no declared length must still stop at the ceiling"
    );
    assert!(response.truncated(), "and must report itself truncated");
}
