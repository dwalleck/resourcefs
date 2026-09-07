//! Body-bound, cancellation, and timeout contracts for the substrate
//! (rfs-g2z9 C16), plus the reader-mode fetch ceiling and its guarantee that
//! extraction never runs over a truncated document (C8).
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
    FIXTURE_HOST, FixtureResponse, TlsListener, fixture_allowlist, match_cert, settle,
    tls_substrate_with_ceilings,
};
use url::Url;

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

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
    let control = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let control_port = control.address.port();
    let control_substrate = tls_substrate_with_ceilings(
        fixture_allowlist(control_port, true),
        vec![LOOPBACK],
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
    let listener = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![LOOPBACK],
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
    let control = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let control_port = control.address.port();
    let control_substrate = tls_substrate_with_ceilings(
        fixture_allowlist(control_port, true),
        vec![LOOPBACK],
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
    let listener = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::trickle(TRICKLE_LEN, TRICKLE_CHUNK, TRICKLE_DELAY)
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![LOOPBACK],
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
}

/// C16 — retained bytes never exceed the ceiling, whatever the peer sends.
///
/// The flushed count is recorded, not asserted: the ceiling is a bound on what
/// ResourceFS accepts, and the probe measured transfer running well past it.
#[tokio::test]
async fn retained_body_never_exceeds_the_ceiling() {
    const CEILING: usize = 64 * 1024;
    const BODY: usize = 1024 * 1024;

    let listener = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::sized(BODY)
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![LOOPBACK],
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
    let listener = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::sized(1024)
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![LOOPBACK],
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
    let short = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::short_count(4 * 1024 * 1024, 8 * 1024)
    })
    .await;
    let short_port = short.address.port();
    let short_substrate = tls_substrate_with_ceilings(
        fixture_allowlist(short_port, true),
        vec![LOOPBACK],
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
    let undeclared = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::undeclared(256 * 1024)
    })
    .await;
    let undeclared_port = undeclared.address.port();
    let undeclared_substrate = tls_substrate_with_ceilings(
        fixture_allowlist(undeclared_port, true),
        vec![LOOPBACK],
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

/// C8 — a body at exactly the ceiling succeeds; one byte over is refused.
///
/// The boundary is the whole point: the signed spec says a document at the
/// ceiling must be readable, so an off-by-one that treats "filled the buffer"
/// as "there was more" would silently refuse a legal document.
#[tokio::test]
async fn fetch_ceiling_boundary() {
    const CEILING: usize = 8 * 1024;

    for (size, expect_ok) in [(CEILING, true), (CEILING + 1, false)] {
        let listener = TlsListener::serve_router(LOOPBACK, 0, match_cert(), move |_path| {
            FixtureResponse::sized(size)
        })
        .await;
        let port = listener.address.port();
        let substrate = tls_substrate_with_ceilings(
            fixture_allowlist(port, true),
            vec![LOOPBACK],
            ceilings(CEILING, 30_000),
        );

        let outcome = substrate
            .fetch_reader_mode(HttpRequest::get(fixture_url(port)), &OperationGuard::new())
            .await;

        if expect_ok {
            assert!(
                outcome.is_ok(),
                "a document of exactly {CEILING} bytes must be accepted, got {:?}",
                outcome.err()
            );
        } else {
            let error = outcome.expect_err("a document past the ceiling must be refused");
            assert_eq!(error.category(), ErrorCategory::LimitExceeded);
            // Message, not just category: a cancellation, a redirect bound and
            // a body bound can all surface as a limit. Only the text says
            // which ceiling fired, and only it names the documented escape.
            assert!(
                error.message().contains("fetch ceiling") && error.message().contains(":raw"),
                "the refusal must name the fetch ceiling and the :raw escape: {}",
                error.message()
            );
        }
        settle().await;
    }
}

/// C8 — extraction never runs over a document that exceeded the ceiling.
///
/// Asserting only that the call failed would be satisfied by any refusal, so
/// it cannot tell "refused before extracting" from "extracted, then failed".
/// The invocation counter is the positive evidence, and the control row proves
/// the counter moves at all — without it, a permanently broken extractor would
/// pass this fence.
#[tokio::test]
async fn over_ceiling_never_extracts() {
    const CEILING: usize = 4 * 1024;

    // Control: an in-ceiling document must move the counter.
    let listener = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::Body("<html><body><p>ok</p></body></html>".to_owned())
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![LOOPBACK],
        ceilings(CEILING, 30_000),
    );
    let before_control = substrate.extraction_count();
    substrate
        .fetch_reader_mode(HttpRequest::get(fixture_url(port)), &OperationGuard::new())
        .await
        .expect("an in-ceiling document extracts");
    assert!(
        substrate.extraction_count() > before_control,
        "the control row must move the extraction counter, otherwise this \
         fence would pass with the extractor never wired up at all"
    );
    settle().await;

    // The claim: an over-ceiling document must not reach the extractor.
    let listener = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |_path| {
        FixtureResponse::sized(CEILING * 4)
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![LOOPBACK],
        ceilings(CEILING, 30_000),
    );
    let before = substrate.extraction_count();
    let error = substrate
        .fetch_reader_mode(HttpRequest::get(fixture_url(port)), &OperationGuard::new())
        .await
        .expect_err("an over-ceiling document must be refused");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(
        substrate.extraction_count(),
        before,
        "extraction must not run over a truncated document: a partial parse \
         can drop or mangle structure and the caller cannot tell"
    );
    settle().await;
}

/// C8 — reader mode refuses an input that is not an HTML document.
///
/// Every one of these previously went through `String::from_utf8_lossy` into
/// the tokenizer and returned `Ok`: a PDF became replacement-character
/// Markdown, an ISO-8859-1 page became mojibake, and the caller had no way to
/// tell either from a faithful rendering. That output is what a
/// content-derived Version Tag hashes, so the damage outlives the request.
///
/// Refusing matches the choice the signed spec already made for an over-ceiling
/// document: fail loudly rather than return something plausible and wrong. The
/// control row runs first — an ordinary HTML response through the same fixture
/// must succeed, so a refusal below cannot be a fixture that never worked.
#[tokio::test]
async fn reader_mode_refuses_non_html_inputs() {
    let listener = TlsListener::serve_router(LOOPBACK, 0, match_cert(), |path| match path {
        "/pdf" => FixtureResponse::Typed {
            content_type: "application/pdf".to_owned(),
            body: b"%PDF-1.7 binary".to_vec(),
        },
        "/latin1" => FixtureResponse::Typed {
            content_type: "text/html; charset=ISO-8859-1".to_owned(),
            body: b"<p>caf\xe9</p>".to_vec(),
        },
        // Declares UTF-8 and is not: the bytes themselves are the last check.
        "/invalid" => FixtureResponse::Typed {
            content_type: "text/html; charset=utf-8".to_owned(),
            body: b"<p>\xff\xfe</p>".to_vec(),
        },
        _ => FixtureResponse::Typed {
            content_type: "text/html; charset=UTF-8".to_owned(),
            body: b"<p>Fine</p>".to_vec(),
        },
    })
    .await;
    let port = listener.address.port();
    let substrate = tls_substrate_with_ceilings(
        fixture_allowlist(port, true),
        vec![LOOPBACK],
        ceilings(1024 * 1024, 30_000),
    );

    let control = substrate
        .fetch_reader_mode(
            HttpRequest::get(path_url(port, "/ok")),
            &OperationGuard::new(),
        )
        .await
        .expect("control: an ordinary HTML response must render");
    assert!(
        control.markdown().contains("Fine"),
        "control must produce real Markdown, else the refusals below prove nothing"
    );

    for (path, expected) in [
        ("/pdf", "application/pdf"),
        ("/latin1", "iso-8859-1"),
        ("/invalid", "not valid UTF-8"),
    ] {
        let failure = substrate
            .fetch_reader_mode(
                HttpRequest::get(path_url(port, path)),
                &OperationGuard::new(),
            )
            .await
            .expect_err("a non-HTML input must be refused, not rendered");
        assert_eq!(
            failure.category(),
            ErrorCategory::UnsupportedProjection,
            "{path} must be refused as an unsupported projection"
        );
        // The message, not just the category: it must name *why* this input is
        // not renderable and point at the `:raw` recovery, or an operator
        // cannot tell a media-type refusal from a charset one.
        let message = failure.message().to_ascii_lowercase();
        assert!(
            message.contains(&expected.to_ascii_lowercase()),
            "{path} refusal must name the cause; got: {}",
            failure.message()
        );
        assert!(
            failure.message().contains(":raw"),
            "{path} refusal must name the :raw recovery; got: {}",
            failure.message()
        );
    }
    settle().await;
}

/// A fixture URL for one specific path on the listener.
fn path_url(port: u16, path: &str) -> Url {
    Url::parse(&format!("https://{FIXTURE_HOST}:{port}{path}")).expect("fixture URL is well formed")
}
