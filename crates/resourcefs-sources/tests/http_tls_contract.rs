//! TLS contracts for the bounded HTTP substrate (rfs-g2z9 C6).
//!
//! `prove-it-prototype` exercised the substrate's address authorization over
//! plain HTTP only. It confirmed *structurally* that the client's TLS variants
//! wrap the same connector, and recorded the TLS path as an open residual
//! rather than claiming it. This file executes that residual: the C2/C3
//! scenarios are re-run over real TLS and must produce identical accept-counter
//! outcomes.
//!
//! The second half is the property that makes address pinning safe. The
//! connection is pinned to an address the policy validated, but certificate
//! verification stays bound to the **hostname the caller asked for**. Were it
//! bound to the address instead, pinning would quietly dismantle TLS identity:
//! any certificate reachable at an authorized address would be accepted.
//!
//! The oracle throughout is **server-side**: accept counts and handshake
//! outcomes recorded by the listener, never the client's own error text, which
//! could report a rejection for a different reason than the one under test.

#[path = "support/tls.rs"]
mod tls;

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use resourcefs_core::{ErrorCategory, OperationGuard};
use resourcefs_sources::HttpRequest;
use tls::{
    FIXTURE_HOST, Handshake, MATCH_CERT, TlsListener, WRONG_CERT, fixture_allowlist, settle,
    tls_substrate,
};
use url::Url;

fn fixture_url(port: u16) -> Url {
    Url::parse(&format!("https://{FIXTURE_HOST}:{port}/doc")).expect("url parses")
}

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// The second address the rebinding row resolves to.
///
/// IPv6 loopback rather than 127.0.0.2: only 127.0.0.1 is assigned to lo0 on
/// macOS by default, so binding 127.0.0.2 fails there with EADDRNOTAVAIL and
/// the row would panic on the macOS CI leg. `::1` is present on all three
/// target platforms and preserves exactly what this row tests — two distinct
/// addresses reachable at the same port, with the address as the only
/// discriminator between them.
const LOOPBACK_ALT: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);

/// C6 — over TLS, the address policy behaves exactly as it does over plain HTTP.
///
/// Three rows mirror the plain-HTTP contracts: an authorized address is
/// reached and its session completes; a denied address is refused inside the
/// resolver with no socket opened; and a rebinding resolver never has one
/// request's answer substituted for another's.
#[tokio::test]
async fn tls_address_policy_matches_plain() {
    // Row 1 — authorized: the request reaches the listener and TLS completes,
    // proving the fixture is genuinely serving and the trust anchor is real.
    let granted = TlsListener::serve(LOOPBACK, 0, MATCH_CERT, "<html><body>ok</body></html>").await;
    let port = granted.address.port();
    let substrate = tls_substrate(fixture_allowlist(port, true), vec![LOOPBACK]);
    let response = substrate
        .fetch(HttpRequest::get(fixture_url(port)), &OperationGuard::new())
        .await
        .expect("an authorized address with a matching certificate is fetched");
    assert_eq!(response.status(), 200);
    settle().await;
    assert_eq!(granted.accepts(), 1, "the authorized request must connect");
    assert_eq!(
        granted.handshake(),
        Handshake::Completed,
        "the fixture certificate must actually validate, or the denial rows below prove nothing"
    );

    // Row 2 — denied: same listener, grant withheld. The refusal happens in the
    // resolver, so nothing reaches the socket. Asserting the message and not
    // merely the category matters: origin scoping also refuses with
    // `permission_denied`, and a category-only assertion could not tell a
    // policy denial from a misconfigured allowlist.
    let denied_listener =
        TlsListener::serve(LOOPBACK, 0, MATCH_CERT, "<html><body>ok</body></html>").await;
    let denied_port = denied_listener.address.port();
    let denied = tls_substrate(fixture_allowlist(denied_port, false), vec![LOOPBACK]);
    let failure = denied
        .fetch(
            HttpRequest::get(fixture_url(denied_port)),
            &OperationGuard::new(),
        )
        .await
        .expect_err("a denied address must not be fetched over TLS either");
    assert_eq!(failure.category(), ErrorCategory::PermissionDenied);
    assert!(
        failure.message().contains("private-network"),
        "the refusal must come from the address policy, not origin scoping; got: {}",
        failure.message()
    );
    settle().await;
    assert_eq!(
        denied_listener.accepts(),
        0,
        "policy denied the address, so TLS must never be attempted against it"
    );

    // Row 3 — rebinding: two listeners differing only by address. Each request
    // must land on the address its own resolution returned.
    let first = TlsListener::serve(LOOPBACK, 0, MATCH_CERT, "<html><body>a</body></html>").await;
    let shared_port = first.address.port();
    let second = TlsListener::serve(
        LOOPBACK_ALT,
        shared_port,
        MATCH_CERT,
        "<html><body>b</body></html>",
    )
    .await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&calls);
    let rebinding = resourcefs_sources::HttpSubstrate::with_host_lookup_and_roots(
        fixture_allowlist(shared_port, true),
        resourcefs_core::HttpCeilings::default(),
        move |_host| {
            let call = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                let address = if call == 0 { LOOPBACK } else { LOOPBACK_ALT };
                Ok::<_, std::io::Error>(vec![address])
            }
        },
        &[tls::FIXTURE_CA],
        Vec::new(),
    )
    .expect("substrate builds");

    let _ = rebinding
        .fetch(
            HttpRequest::get(fixture_url(shared_port)),
            &OperationGuard::new(),
        )
        .await;
    settle().await;
    let _ = rebinding
        .fetch(
            HttpRequest::get(fixture_url(shared_port)),
            &OperationGuard::new(),
        )
        .await;
    settle().await;
    assert_eq!(
        (first.accepts(), second.accepts()),
        (1, 1),
        "over TLS as over plain HTTP, each request lands on its own resolution's address"
    );
}

/// C6 — certificate verification stays bound to the requested hostname.
///
/// Both certificates chain to the same trusted CA and are served from the same
/// authorized loopback address. The only difference is the subject name, so a
/// rejection can be attributed to hostname binding and to nothing else — not
/// to an untrusted issuer, and not to a denied address.
#[tokio::test]
async fn tls_certificate_binds_hostname() {
    // The matching certificate establishes the baseline: this host, this
    // address, and this trust anchor do produce a completed session.
    let matching =
        TlsListener::serve(LOOPBACK, 0, MATCH_CERT, "<html><body>ok</body></html>").await;
    let matching_port = matching.address.port();
    let trusted = tls_substrate(fixture_allowlist(matching_port, true), vec![LOOPBACK]);
    let response = trusted
        .fetch(
            HttpRequest::get(fixture_url(matching_port)),
            &OperationGuard::new(),
        )
        .await
        .expect("a certificate naming the requested host is accepted");
    assert_eq!(response.status(), 200);
    settle().await;
    assert_eq!(matching.handshake(), Handshake::Completed);

    // The mismatched certificate is trusted and served from an address the
    // policy authorized — every other control says yes. Only the name is
    // wrong, and that alone must stop it.
    let mismatched =
        TlsListener::serve(LOOPBACK, 0, WRONG_CERT, "<html><body>secret</body></html>").await;
    let mismatched_port = mismatched.address.port();
    let substrate = tls_substrate(fixture_allowlist(mismatched_port, true), vec![LOOPBACK]);
    let failure = substrate
        .fetch(
            HttpRequest::get(fixture_url(mismatched_port)),
            &OperationGuard::new(),
        )
        .await
        .expect_err("a certificate naming a different host must be rejected");
    assert_eq!(
        failure.category(),
        ErrorCategory::SourceUnavailable,
        "a rejected certificate is a transport failure, not a policy denial: {}",
        failure.message()
    );
    settle().await;

    // Server-side confirmation. The peer connected — so the refusal was not the
    // address policy declining to open a socket — and the session never
    // completed, so the client rejected the certificate rather than reading a
    // body it should never have seen.
    assert!(
        mismatched.accepts() >= 1,
        "the address was authorized, so the connection must reach the listener"
    );
    assert_eq!(
        mismatched.handshake(),
        Handshake::Failed,
        "the name-mismatched certificate must be rejected at the TLS layer"
    );
    assert!(
        mismatched.requests().is_empty(),
        "no request may be transmitted over a session whose certificate was rejected"
    );
}

#[test]
fn system_lookup_substrate_accepts_valid_additional_root() {
    use resourcefs_core::HttpCeilings;
    use resourcefs_sources::{HttpSubstrate, TestRootCertificate};

    let root = TestRootCertificate::from_der(tls::FIXTURE_CA).expect("fixture CA is valid DER");
    HttpSubstrate::with_system_lookup_and_root(
        fixture_allowlist(443, true),
        HttpCeilings::default(),
        root,
        Vec::new(),
    )
    .expect("valid root builds the policy-preserving substrate");
}

#[test]
fn malformed_additional_root_is_rejected_before_egress() {
    use resourcefs_sources::TestRootCertificate;

    let listener =
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("no-egress oracle listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking no-egress listener");

    let error = match TestRootCertificate::from_der(b"not a DER certificate") {
        Ok(_) => panic!("malformed DER must not produce a trusted root"),
        Err(error) => error,
    };
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert_eq!(
        listener
            .accept()
            .expect_err("root parsing must open no socket")
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
}
