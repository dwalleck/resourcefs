//! Egress-policy contracts for the bounded HTTP substrate and the HTTPS Source
//! Adapter (rfs-g2z9 C2, C3, C4, C5, C7, C12, C13, C15, C19).
//!
//! The oracle throughout is **server-side observation**: each listener counts
//! the TCP connections it actually accepted. That is deliberately not the
//! client's own report of where it connected — a client that believed it had
//! been denied while still opening a socket would satisfy a client-side check
//! and fail this one.
//!
//! Every fixture is loopback-only; no name reaches a system resolver, because
//! the substrate is driven through an injected host lookup.

use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use resourcefs_core::{
    AddressPolicy, AllowedOrigin, ErrorCategory, HttpCeilings, OperationGuard, OriginAllowlist,
};
use resourcefs_sources::{HttpRequest, HttpSubstrate, NetworkProbe};
use tokio::net::TcpListener;
use url::Url;

/// A loopback listener that counts every connection it accepts.
struct CountingListener {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
}

impl CountingListener {
    /// Binds on `ip`, optionally reusing `port` so two listeners can differ
    /// only by address — the client substitutes the URL's port into whatever
    /// the resolver returns, so the address is the only usable discriminator.
    async fn bind(ip: IpAddr, port: u16) -> Self {
        let listener = TcpListener::bind(SocketAddr::new(ip, port))
            .await
            .expect("loopback listener binds");
        let address = listener.local_addr().expect("listener reports its address");
        let accepts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&accepts);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        });
        Self { address, accepts }
    }

    fn accepts(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }
}

/// Gives an in-flight connection attempt time to reach the listener before the
/// accept counter is read.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(150)).await;
}

/// Declares one origin, including the fixture's ephemeral port.
///
/// The port is not incidental: origin scoping compares `port_or_known_default`,
/// so an origin written without one would default to 443 and refuse every
/// request in these fixtures — at the allowlist, before the resolver ever ran.
/// A denial fixture built that way would pass while proving nothing about the
/// address policy it claims to test.
fn allowlist(host: &str, port: u16, allow_private_network: bool) -> OriginAllowlist {
    OriginAllowlist::new(vec![
        AllowedOrigin::new(&format!("https://{host}:{port}/"), allow_private_network)
            .expect("origin is well formed"),
    ])
}

/// Asserts a refusal came from the address policy inside the resolver.
///
/// Category alone is too weak: origin scoping also refuses with
/// `permission_denied`, so a fixture asserting only the category cannot tell a
/// resolver denial from a misconfigured allowlist.
fn assert_denied_by_address_policy(failure: &resourcefs_core::ResourceError) {
    assert_eq!(failure.category(), ErrorCategory::PermissionDenied);
    assert!(
        failure.message().contains("private-network"),
        "refusal must come from the address policy, not origin scoping; got: {}",
        failure.message()
    );
}

/// Builds a substrate whose resolution is fixed to `addresses`.
fn substrate(allowlist: OriginAllowlist, addresses: Vec<IpAddr>) -> HttpSubstrate {
    HttpSubstrate::with_host_lookup(allowlist, HttpCeilings::default(), move |_host| {
        let addresses = addresses.clone();
        async move { Ok::<_, io::Error>(addresses) }
    })
    .expect("substrate builds")
}

/// C2 — a denied address opens no socket at all.
///
/// The listener is on loopback, which is restricted space; the origin withholds
/// the private-network grant, so authorization must fail inside the resolver
/// before any connection is attempted.
#[tokio::test]
async fn denied_address_opens_no_socket() {
    let listener = CountingListener::bind(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).await;
    let port = listener.address.port();
    let substrate = substrate(
        allowlist("denied.invalid", port, false),
        vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
    );

    let url = Url::parse(&format!("https://denied.invalid:{port}/doc")).expect("url parses");
    let failure = substrate
        .fetch(HttpRequest::get(url), &OperationGuard::new())
        .await
        .expect_err("a denied address must not be fetched");

    assert_denied_by_address_policy(&failure);
    settle().await;
    assert_eq!(
        listener.accepts(),
        0,
        "policy denied the address, so no socket may be opened to it"
    );
}

/// C3 — each connection lands on the address its own resolution authorized.
///
/// A resolver that returned one address and then another must not have its
/// first answer substituted for its second: that substitution is precisely the
/// rebinding window this control closes.
#[tokio::test]
async fn rebinding_never_reaches_denied_address() {
    let first = CountingListener::bind(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).await;
    let port = first.address.port();
    // IPv6 loopback, not 127.0.0.2: macOS assigns only 127.0.0.1 to lo0 by
    // default, so binding 127.0.0.2 fails there and this row would panic on the
    // macOS CI leg. `::1` exists on all three targets and keeps the property
    // intact — two distinct addresses at one port, the address being the only
    // thing that differs between the two resolutions.
    let second = CountingListener::bind(IpAddr::V6(Ipv6Addr::LOCALHOST), port).await;

    // The grant is present, so both loopback addresses are authorized: what is
    // under test is *which* address each connection reached, not whether it
    // was allowed.
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let substrate = HttpSubstrate::with_host_lookup(
        allowlist("rebind.invalid", port, true),
        HttpCeilings::default(),
        move |_host| {
            let call = counter.fetch_add(1, Ordering::SeqCst);
            async move {
                let address = if call == 0 {
                    IpAddr::V4(Ipv4Addr::LOCALHOST)
                } else {
                    IpAddr::V6(Ipv6Addr::LOCALHOST)
                };
                Ok::<_, io::Error>(vec![address])
            }
        },
    )
    .expect("substrate builds");

    let url = Url::parse(&format!("https://rebind.invalid:{port}/doc")).expect("url parses");
    // Both requests fail at the TLS handshake against these plain listeners —
    // irrelevant here, because the oracle is the TCP accept that precedes it.
    let _ = substrate
        .fetch(HttpRequest::get(url.clone()), &OperationGuard::new())
        .await;
    settle().await;
    let _ = substrate
        .fetch(HttpRequest::get(url), &OperationGuard::new())
        .await;
    settle().await;

    assert_eq!(
        (first.accepts(), second.accepts()),
        (1, 1),
        "each request must land on the address its own resolution returned"
    );
}

/// C7 — a connection is never pooled or reconnected into an unauthorized address.
///
/// One substrate holds the grant and reaches the listener; a second substrate
/// against the same host and listener withholds it and must reach nothing,
/// proving no connection state crosses a policy boundary.
#[tokio::test]
async fn pooled_reuse_stays_authorized() {
    let listener = CountingListener::bind(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).await;
    let port = listener.address.port();
    let address = vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))];

    let granted = substrate(allowlist("pool.invalid", port, true), address.clone());
    let url = Url::parse(&format!("https://pool.invalid:{port}/doc")).expect("url parses");
    let _ = granted
        .fetch(HttpRequest::get(url.clone()), &OperationGuard::new())
        .await;
    settle().await;
    let reached = listener.accepts();
    assert!(
        reached >= 1,
        "the granted origin must actually reach the listener for this to prove anything"
    );

    // Same host, same listener, grant withheld: every further connection must
    // be refused in the resolver rather than served from any retained state.
    let denied = substrate(allowlist("pool.invalid", port, false), address);
    for _ in 0..2 {
        let failure = denied
            .fetch(HttpRequest::get(url.clone()), &OperationGuard::new())
            .await
            .expect_err("a withheld grant must refuse the connection");
        assert_denied_by_address_policy(&failure);
    }
    settle().await;
    assert_eq!(
        listener.accepts(),
        reached,
        "no additional connection may reach an address the policy denies"
    );
}

/// C15 — the reachability probe is policed by the same address policy.
///
/// Before this slice the probe dialled an unvalidated system resolution
/// directly, so an origin that withheld the private-network grant still had
/// its startup probe reach private space.
#[tokio::test]
async fn probe_egress_is_policed() {
    use resourcefs_core::{ProbeState, SourceProbe};

    let listener = CountingListener::bind(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).await;
    let port = listener.address.port();

    let withheld = NetworkProbe::new(
        vec![("127.0.0.1".to_owned(), port, AddressPolicy::new(false))],
        false,
    )
    .expect("probe is well formed");
    let outcome = withheld.probe(&OperationGuard::new()).await;
    settle().await;
    assert_eq!(
        outcome.state(),
        ProbeState::Degraded,
        "a probe refused by policy is not Available"
    );
    assert_eq!(
        listener.accepts(),
        0,
        "the probe must not connect to an address the origin's grant withholds"
    );

    // The same endpoint with the grant present proves the fixture can reach
    // the listener at all, so the zero above is policy and not a dead port.
    let granted = NetworkProbe::new(
        vec![("127.0.0.1".to_owned(), port, AddressPolicy::new(true))],
        false,
    )
    .expect("probe is well formed");
    let outcome = granted.probe(&OperationGuard::new()).await;
    settle().await;
    assert_eq!(outcome.state(), ProbeState::Available);
    assert!(
        listener.accepts() >= 1,
        "the granted probe must reach the listener"
    );
}

#[path = "support/tls.rs"]
mod tls;

/// C4 — a redirect leaving the allowlist is refused and never transmitted.
///
/// The off-allowlist target is deliberately a path on the **same** host, not a
/// foreign host. A foreign host would be refused a second time by the resolver,
/// which has no policy for it — so its request log would stay empty even with
/// the redirect control deleted, and the fence would pass while proving
/// nothing. Scoping by path keeps the host resolvable and the address
/// authorized, leaving the redirect check as the only thing that can stop the
/// request. The empty log then means exactly what it claims.
#[tokio::test]
async fn offsite_redirect_never_requested() {
    let listener = tls::TlsListener::serve_router(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        tls::MATCH_CERT,
        |path| {
            if path == "/docs/start" {
                tls::FixtureResponse::Redirect("/secret".to_owned())
            } else {
                tls::FixtureResponse::Body("secret".to_owned())
            }
        },
    )
    .await;
    let port = listener.address.port();

    // The grant is present and the origin is scoped to `/docs/`, so `/secret`
    // is off-allowlist by path alone.
    let scoped = OriginAllowlist::new(vec![
        AllowedOrigin::new(&format!("https://{}:{port}/docs/", tls::FIXTURE_HOST), true)
            .expect("origin is well formed"),
    ]);
    let substrate = tls::tls_substrate(scoped, vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))]);

    let url = Url::parse(&format!("https://{}:{port}/docs/start", tls::FIXTURE_HOST))
        .expect("fixture URL is well formed");
    let failure = substrate
        .fetch(HttpRequest::get(url), &OperationGuard::new())
        .await
        .expect_err("a redirect off the allowlist must be refused");
    tls::settle().await;

    assert_eq!(failure.category(), ErrorCategory::PermissionDenied);
    // Origin scoping refuses the initial URL with the same category, so the
    // category alone cannot show which control fired.
    assert!(
        failure.message().contains("redirect was not followed"),
        "the refusal must identify the redirect control, got: {}",
        failure.message()
    );
    let received = listener.requests();
    // The decisive evidence: the refused target was never put on the wire,
    // even though the host was resolvable and the address authorized.
    assert!(
        !received.iter().any(|line| line.contains("/secret")),
        "the off-allowlist target must never be requested, got: {received:?}"
    );
    // The guard against passing for the wrong reason: the exchange did work.
    assert!(
        received.iter().any(|line| line.contains("/docs/start")),
        "the allowlisted hop must have been served, got: {received:?}"
    );
}

/// C5 — the redirect chain is bounded, and the bound is observed server-side.
///
/// The oracle is the redirecting listener's own request log, independent of
/// the client's redirect counter: the hop past the ceiling must appear nowhere
/// in it.
#[tokio::test]
async fn redirect_depth_boundary() {
    let listener = tls::TlsListener::serve_router(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        tls::MATCH_CERT,
        |path| {
            let step = |prefix: &str, last: usize| -> Option<tls::FixtureResponse> {
                let index: usize = path.strip_prefix(prefix)?.parse().ok()?;
                Some(if index < last {
                    tls::FixtureResponse::Redirect(format!("{prefix}{}", index + 1))
                } else {
                    tls::FixtureResponse::Body("arrived".to_owned())
                })
            };
            if path == "/loop" {
                return tls::FixtureResponse::Redirect("/loop".to_owned());
            }
            // `/hop/1` costs five redirects to reach `/hop/6`; `/deep/1` would
            // cost six to reach `/deep/7`, one past the ceiling.
            step("/hop/", 6)
                .or_else(|| step("/deep/", 7))
                .unwrap_or_else(|| tls::FixtureResponse::Body("arrived".to_owned()))
        },
    )
    .await;
    let port = listener.address.port();
    let substrate = tls::tls_substrate(
        tls::fixture_allowlist(port, true),
        vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))],
    );
    let fetch = async |path: &str| {
        let url = Url::parse(&format!("https://{}:{port}{path}", tls::FIXTURE_HOST))
            .expect("fixture URL is well formed");
        substrate
            .fetch(HttpRequest::get(url), &OperationGuard::new())
            .await
    };

    // Depth zero: a request that never redirects is served directly.
    let plain = fetch("/plain").await.expect("a direct request succeeds");
    assert_eq!(plain.status(), 200);
    assert_eq!(
        listener.requests().len(),
        1,
        "a request with no redirect must cost exactly one hop"
    );

    // Exactly at the ceiling: five redirects are followed and the body lands.
    let arrived = fetch("/hop/1")
        .await
        .expect("a chain at the ceiling succeeds");
    assert_eq!(arrived.status(), 200);
    assert_eq!(arrived.body(), b"arrived");

    // One past the ceiling: refused, and the extra hop never leaves the client.
    let failure = fetch("/deep/1")
        .await
        .expect_err("a chain past the ceiling must be refused");
    tls::settle().await;
    assert_eq!(failure.category(), ErrorCategory::LimitExceeded);
    let received = listener.requests();
    assert!(
        received.iter().any(|line| line.contains("/deep/6")),
        "the chain must run up to the ceiling, got: {received:?}"
    );
    assert!(
        !received.iter().any(|line| line.contains("/deep/7")),
        "the hop past the ceiling must never be requested, got: {received:?}"
    );

    // A loop terminates at the bound rather than hanging.
    let looped = fetch("/loop")
        .await
        .expect_err("a redirect loop must terminate at the ceiling");
    assert_eq!(looped.category(), ErrorCategory::LimitExceeded);
}

/// C4 — a hop the allowlist permits is still subject to the address policy.
///
/// Both controls apply to every hop and neither substitutes for the other: the
/// redirect target here is inside the allowlist, so only the resolver can stop
/// it. Without this row a fixture could pass with address authorization applied
/// to the first connection alone.
#[tokio::test]
async fn redirect_to_denied_address_is_refused() {
    // Bind first so the redirect Location can name the listener's real port.
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("port probe binds");
    let redirect_port = probe.local_addr().expect("probe reports its port").port();
    drop(probe);
    let listener = tls::TlsListener::serve_router(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        redirect_port,
        tls::MATCH_CERT,
        move |path: &str| {
            if path == "/start" {
                tls::FixtureResponse::Redirect(format!(
                    "https://denied.invalid:{redirect_port}/next"
                ))
            } else {
                tls::FixtureResponse::Body("arrived".to_owned())
            }
        },
    )
    .await;
    let port = listener.address.port();

    // Both hosts are allowlisted, so the redirect check passes; they differ
    // only in the private-network grant, leaving the address policy as the one
    // control that can refuse the hop.
    let origins = OriginAllowlist::new(vec![
        AllowedOrigin::new(&format!("https://{}:{port}/", tls::FIXTURE_HOST), true)
            .expect("origin is well formed"),
        AllowedOrigin::new(&format!("https://denied.invalid:{port}/"), false)
            .expect("origin is well formed"),
    ]);
    let substrate = resourcefs_sources::HttpSubstrate::with_host_lookup_and_roots(
        origins,
        HttpCeilings::default(),
        move |_host: String| async move {
            Ok::<_, io::Error>(vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))])
        },
        &[tls::FIXTURE_CA],
        Vec::new(),
    )
    .expect("substrate builds");

    let url = Url::parse(&format!("https://{}:{port}/start", tls::FIXTURE_HOST))
        .expect("fixture URL is well formed");
    let failure = substrate
        .fetch(HttpRequest::get(url), &OperationGuard::new())
        .await
        .expect_err("a hop into restricted space must be refused");
    tls::settle().await;

    assert_denied_by_address_policy(&failure);
    assert!(
        !listener
            .requests()
            .iter()
            .any(|line| line.contains("/next")),
        "the denied hop must never be requested, got: {:?}",
        listener.requests()
    );
}

/// C2, IP-literal route — an origin declared as an IP literal is authorized
/// before any connect, so a withheld private-network grant opens no socket.
///
/// This route never reaches [`PolicyResolver`]: the connector short-circuits
/// DNS when the host parses as an address. Relying on the resolver alone left
/// `allow_private_network` unenforced for every IP-literal origin, and the
/// server's accept counter — not the client's error — is what proves it.
///
/// The granted row runs first and is not decoration. Without it a zero accept
/// count would be satisfied by a fixture that never worked at all, which is
/// how four earlier fixtures in this change passed while proving nothing.
#[tokio::test]
async fn ip_literal_origin_is_authorized_before_connect() {
    let granted_listener =
        tls::TlsListener::serve(IpAddr::V4(Ipv4Addr::LOCALHOST), 0, tls::MATCH_CERT, "ok").await;
    let granted_port = granted_listener.address.port();
    let granted = literal_substrate(granted_port, true);
    let granted_url = Url::parse(&format!("https://127.0.0.1:{granted_port}/doc"))
        .expect("literal URL is well formed");
    let _ = granted
        .fetch(HttpRequest::get(granted_url), &OperationGuard::new())
        .await;
    tls::settle().await;
    assert!(
        granted_listener.accepts() > 0,
        "control: a granted IP-literal origin must reach the listener, else the \
         denial row below proves nothing"
    );

    let denied_listener =
        tls::TlsListener::serve(IpAddr::V4(Ipv4Addr::LOCALHOST), 0, tls::MATCH_CERT, "ok").await;
    let denied_port = denied_listener.address.port();
    let denied = literal_substrate(denied_port, false);
    let denied_url = Url::parse(&format!("https://127.0.0.1:{denied_port}/doc"))
        .expect("literal URL is well formed");
    let failure = denied
        .fetch(HttpRequest::get(denied_url), &OperationGuard::new())
        .await
        .expect_err("a withheld grant must refuse an IP-literal origin");
    tls::settle().await;

    assert_denied_by_address_policy(&failure);
    assert_eq!(
        denied_listener.accepts(),
        0,
        "no socket may be opened to a restricted address when the grant is withheld"
    );
}

/// Builds a substrate whose single origin is the loopback IP literal itself.
///
/// The injected lookup would resolve the name route, and is deliberately left
/// in place: if the implementation ever regressed to routing literals through
/// the resolver, this fixture would still pass, so the accept counter above is
/// what carries the proof rather than the lookup being unreachable.
fn literal_substrate(port: u16, allow_private_network: bool) -> HttpSubstrate {
    let origins = OriginAllowlist::new(vec![
        resourcefs_core::AllowedOrigin::new(
            &format!("https://127.0.0.1:{port}/"),
            allow_private_network,
        )
        .expect("IP-literal origin is well formed"),
    ]);
    HttpSubstrate::with_host_lookup_and_roots(
        origins,
        HttpCeilings::default(),
        move |_host: String| async move {
            Ok::<_, io::Error>(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)])
        },
        &[tls::FIXTURE_CA],
        Vec::new(),
    )
    .expect("substrate builds")
}

/// C12 — no resolved IP address reaches an observable channel.
///
/// Scoped to addresses by `design.md` Revision 2: credential transmission does
/// not exist yet (tracked as C20), so a credential sentinel would have no path
/// to any channel and scanning for one would pass unconditionally.
///
/// The sentinel is the **resolved address itself**, which is genuinely present
/// in the system — the resolver authorizes it and the denial is caused by it —
/// so its absence from the message is a real property rather than a vacuous
/// one. Every scanned channel is asserted non-empty first: a scan over a
/// channel that produced no output proves nothing, which is the failure mode
/// four fixtures in this change already hit.
#[tokio::test]
async fn no_resolved_address_leak() {
    // A distinctive address so a substring hit cannot be coincidental.
    let sentinel_ip = Ipv4Addr::new(10, 213, 47, 91);
    let sentinel = sentinel_ip.to_string();

    let allowlist = tls::fixture_allowlist(8443, false);
    let substrate = tls::tls_substrate(allowlist, vec![IpAddr::V4(sentinel_ip)]);
    let url = Url::parse(&format!("https://{}:8443/doc", tls::FIXTURE_HOST)).expect("url");

    let error = substrate
        .fetch(HttpRequest::get(url), &OperationGuard::new())
        .await
        .expect_err("a private address without the grant must be refused");

    // The channel genuinely carries content, so the absence below is meaningful.
    let message = error.message();
    assert!(
        !message.is_empty(),
        "the refusal must explain itself; an empty message makes the scan vacuous"
    );
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    // Message, not category: an allowlist refusal shares this category.
    assert!(
        message.contains("private-network"),
        "the refusal must name the control that fired, got {message:?}"
    );

    for channel in [message, &format!("{error}"), &format!("{error:?}")] {
        assert!(
            !channel.is_empty(),
            "every scanned channel must carry output"
        );
        assert!(
            !channel.contains(&sentinel),
            "a resolved address must never reach an observable channel; found {sentinel} in {channel:?}"
        );
    }
}

/// C19 (wire half) — a percent-encoded separator reaches the origin unchanged.
///
/// The oracle is the **server's received request line**, not the client's URL:
/// a client that re-encoded or decoded the path would still report whatever it
/// believed it sent.
#[tokio::test]
async fn encoded_url_reaches_wire_unchanged() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("port");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);

    let server = tls::TlsListener::serve(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
        tls::MATCH_CERT,
        "<html><body><p>ok</p></body></html>",
    )
    .await;

    let allowlist = tls::fixture_allowlist(port, true);
    let substrate = tls::tls_substrate(allowlist, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
    let url = Url::parse(&format!(
        "https://{}:{port}/search?q=a%2Fb",
        tls::FIXTURE_HOST
    ))
    .expect("url");

    substrate
        .fetch(HttpRequest::get(url), &OperationGuard::new())
        .await
        .expect("an allowlisted encoded URL must be fetched");
    tls::settle().await;

    let requests = server.requests();
    assert!(
        !requests.is_empty(),
        "the server must have received a request; an empty log makes the assertion vacuous"
    );
    assert!(
        requests.iter().any(|line| line.contains("/search?q=a%2Fb")),
        "the encoding must reach the wire verbatim, got {requests:?}"
    );
    assert!(
        !requests.iter().any(|line| line.contains("/search?q=a/b")),
        "the separator must not be decoded on the way out, got {requests:?}"
    );
}

/// C13 (read half) — an HTTPS read uses the common Resource shape.
///
/// The read-only refusal half lives in `stdio_mcp_contract.rs`, where the
/// public tools are reachable; this asserts the shape a mounted source
/// produces, which needs a real fetch.
#[tokio::test]
async fn https_read_uses_common_resource_shape() {
    use resourcefs_core::{PathReference, SourceAdapter};
    use resourcefs_sources::HttpsSource;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("port");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);

    let _server = tls::TlsListener::serve(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
        tls::MATCH_CERT,
        "<html><body><h1>Title</h1><p>Body text.</p></body></html>",
    )
    .await;

    let allowlist = tls::fixture_allowlist(port, true);
    let substrate = Arc::new(tls::tls_substrate(
        allowlist,
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
    ));
    let source = HttpsSource::new(substrate);
    let reference =
        PathReference::parse(format!("https://{}:{port}/doc", tls::FIXTURE_HOST)).expect("parse");

    let resource = source
        .read(&reference, &OperationGuard::new())
        .await
        .expect("read succeeds");

    assert!(
        !resource.content().is_empty(),
        "a successful read must carry content"
    );
    assert!(
        resource.content().contains("Title"),
        "reader mode must render the document, got {:?}",
        resource.content()
    );
    // HTTPS is read-only: the family never reports itself mutable.
    assert!(
        !resource.is_mutable(),
        "an https:// Resource must never report itself mutable"
    );
    // Identity is the canonical URL, so a later read under a selector is the
    // same Resource rather than a second one.
    assert_eq!(
        resource.canonical_reference(),
        format!("https://{}:{port}/doc", tls::FIXTURE_HOST)
    );
}
