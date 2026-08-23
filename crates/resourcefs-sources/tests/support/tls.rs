//! Shared loopback TLS fixture for the bounded HTTP substrate's contracts.
//!
//! rfs-g2z9 Slice 6 owns this helper; Slices 4, 5, and 7 consume it, because
//! every claim about redirects, body bounds, or extraction needs the substrate
//! to complete a real HTTP exchange, and origin scoping admits only `https`
//! (`AllowedOrigin::new` rejects any other scheme). A plain listener cannot
//! serve those slices: the request dies in the TLS handshake, so a fixture
//! built on one would observe an empty server log and pass while proving
//! nothing.
//!
//! The certificates are static DER committed under `tests/fixtures/tls/`, so
//! no certificate-generation crate enters the dependency tree. They are
//! test-only, self-issued, and carry no authority anywhere: the private keys
//! are deliberately public. They were minted for 100 years, so expiry is not a
//! maintenance trap; regenerating them is an openssl invocation recorded in
//! this slice's commit message.
//!
//! Two leaves chain to the **same** CA and differ only in their subject name:
//!
//! - [`MATCH_CERT`] covers [`FIXTURE_HOST`] — the name the client requests.
//! - [`WRONG_CERT`] covers `mismatch.invalid` — a name the client never asks
//!   for.
//!
//! Sharing one CA is the point. If the mismatched leaf were issued by an
//! untrusted issuer, a rejection would prove only that the client checks
//! chains, which no one doubts. Because both are trusted, the sole remaining
//! difference is the name, so a rejection is attributable to hostname binding
//! and nothing else.

#![allow(dead_code, reason = "each integration test binary uses a subset")]

use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use resourcefs_core::{HttpCeilings, OriginAllowlist};
use resourcefs_sources::HttpSubstrate;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    },
};

/// The CA both fixture leaves chain to; the client trusts exactly this.
pub const FIXTURE_CA: &[u8] = include_bytes!("../fixtures/tls/ca.der");
/// Leaf whose subject name matches [`FIXTURE_HOST`].
pub const MATCH_CERT: &[u8] = include_bytes!("../fixtures/tls/match.crt.der");
const MATCH_KEY: &[u8] = include_bytes!("../fixtures/tls/match.key.der");
/// Leaf for `mismatch.invalid`, trusted but wrong for [`FIXTURE_HOST`].
pub const WRONG_CERT: &[u8] = include_bytes!("../fixtures/tls/wrong.crt.der");
const WRONG_KEY: &[u8] = include_bytes!("../fixtures/tls/wrong.key.der");

/// The host every fixture request names. Never resolved by a system resolver:
/// the substrate is driven through an injected lookup, and `.invalid` is
/// reserved by RFC 2606 precisely so it cannot resolve anywhere.
pub const FIXTURE_HOST: &str = "tls.invalid";

/// What a listener observed for one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handshake {
    /// The TLS session completed and the request was served.
    Completed,
    /// The peer connected but the session never completed.
    Failed,
}

/// A loopback TLS listener recording accepts, handshake outcomes, and the
/// request lines it actually received.
///
/// The request log is the oracle Slices 4 and 8 need: direct evidence of what
/// was transmitted, rather than the client's account of what it did.
pub struct TlsListener {
    pub address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
}

impl TlsListener {
    /// Serves `response_body` over TLS using `cert`/`key`, on `ip`.
    ///
    /// `port` may be reused so two listeners differ only by address — the
    /// client substitutes the URL's port into whatever the resolver returns,
    /// so the address is the only discriminator available to a fixture.
    pub async fn serve(ip: Ipv4Addr, port: u16, cert: &'static [u8], response_body: &str) -> Self {
        let key: &[u8] = if cert == MATCH_CERT {
            MATCH_KEY
        } else {
            WRONG_KEY
        };
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert), CertificateDer::from(FIXTURE_CA)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
            )
            .expect("fixture certificate and key form a valid server config");
        let acceptor = TlsAcceptor::from(Arc::new(config));

        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(ip), port))
            .await
            .expect("loopback listener binds");
        let address = listener.local_addr().expect("listener reports its address");

        let accepts = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        );

        let (accept_counter, done_counter, log) = (
            Arc::clone(&accepts),
            Arc::clone(&completed),
            Arc::clone(&requests),
        );
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                accept_counter.fetch_add(1, Ordering::SeqCst);
                let acceptor = acceptor.clone();
                let done = Arc::clone(&done_counter);
                let log = Arc::clone(&log);
                let response = response.clone();
                tokio::spawn(async move {
                    // A handshake failure is an expected outcome here, not an
                    // error: the name-mismatch row exists to produce one.
                    let Ok(mut tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    done.fetch_add(1, Ordering::SeqCst);
                    let mut buffer = [0_u8; 1024];
                    if let Ok(read) = tls.read(&mut buffer).await
                        && read > 0
                        && let Some(line) = String::from_utf8_lossy(&buffer[..read]).lines().next()
                    {
                        log.lock()
                            .expect("request log is uncontended")
                            .push(line.to_owned());
                    }
                    let _ = tls.write_all(response.as_bytes()).await;
                    let _ = tls.shutdown().await;
                });
            }
        });

        Self {
            address,
            accepts,
            completed,
            requests,
        }
    }

    /// Connections accepted at the TCP layer, before any TLS work.
    pub fn accepts(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }

    /// Connections whose TLS session completed.
    pub fn completed(&self) -> usize {
        self.completed.load(Ordering::SeqCst)
    }

    /// Whether any session completed, and so whether the peer was accepted.
    pub fn handshake(&self) -> Handshake {
        if self.completed() > 0 {
            Handshake::Completed
        } else {
            Handshake::Failed
        }
    }

    /// Every request line this listener actually received.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("request log is uncontended")
            .clone()
    }
}

/// Gives an in-flight attempt time to reach the listener before counters read.
pub async fn settle() {
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

/// Declares one origin for [`FIXTURE_HOST`], including the fixture's port.
///
/// The port is not incidental: origin scoping compares `port_or_known_default`,
/// so an origin written without one defaults to 443 and refuses every request
/// in these fixtures — at the allowlist, before the resolver ever runs. A
/// fixture built that way passes while proving nothing about what it claims.
pub fn fixture_allowlist(port: u16, allow_private_network: bool) -> OriginAllowlist {
    OriginAllowlist::new(vec![
        resourcefs_core::AllowedOrigin::new(
            &format!("https://{FIXTURE_HOST}:{port}/"),
            allow_private_network,
        )
        .expect("origin is well formed"),
    ])
}

/// Builds a substrate that trusts the fixture CA and resolves
/// [`FIXTURE_HOST`] to `addresses`.
///
/// Loopback is restricted address space, so a fixture reaching a local
/// listener needs the private-network grant; withholding it is how the denial
/// rows are expressed.
pub fn tls_substrate(allowlist: OriginAllowlist, addresses: Vec<IpAddr>) -> HttpSubstrate {
    HttpSubstrate::with_host_lookup_and_roots(
        allowlist,
        HttpCeilings::default(),
        move |_host| {
            let addresses = addresses.clone();
            async move { Ok::<_, io::Error>(addresses) }
        },
        &[FIXTURE_CA],
    )
    .expect("substrate builds")
}
