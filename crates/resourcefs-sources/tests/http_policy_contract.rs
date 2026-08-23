//! Egress-policy contracts for the bounded HTTP substrate (rfs-g2z9 C2, C3,
//! C7, C15).
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
    net::{IpAddr, Ipv4Addr, SocketAddr},
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
    async fn bind(ip: Ipv4Addr, port: u16) -> Self {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(ip), port))
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
    let listener = CountingListener::bind(Ipv4Addr::new(127, 0, 0, 1), 0).await;
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
    let first = CountingListener::bind(Ipv4Addr::new(127, 0, 0, 1), 0).await;
    let port = first.address.port();
    let second = CountingListener::bind(Ipv4Addr::new(127, 0, 0, 2), port).await;

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
                    Ipv4Addr::new(127, 0, 0, 1)
                } else {
                    Ipv4Addr::new(127, 0, 0, 2)
                };
                Ok::<_, io::Error>(vec![IpAddr::V4(address)])
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
    let listener = CountingListener::bind(Ipv4Addr::new(127, 0, 0, 1), 0).await;
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

    let listener = CountingListener::bind(Ipv4Addr::new(127, 0, 0, 1), 0).await;
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
