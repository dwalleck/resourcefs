//! Shared loopback TLS fixture for the bounded HTTP substrate's contracts.
//!
//! Every claim about redirects, headers, body bounds, or extraction needs the
//! substrate to complete a real HTTP exchange, and origin scoping admits only
//! `https` (`AllowedOrigin::new` rejects any other scheme). A plain listener
//! cannot serve these contracts: the request dies in the TLS handshake, so a
//! fixture built on one would observe an empty server log and pass while
//! proving nothing.
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
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use resourcefs_core::{HttpCeilings, OriginAllowlist};
use resourcefs_sources::{HttpSubstrate, OriginCredential};
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

/// One response a fixture listener can serve.
///
/// Shaped per request path by [`TlsListener::serve_router`]. Fixed, typed,
/// header-bearing, redirect, and streamed responses share this one fixture
/// interface so every contract reaches the same TLS listener and request log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureResponse {
    /// `200 OK` carrying `body`.
    Body(String),
    /// `302 Found` pointing at `location`, absolute or origin-relative.
    Redirect(String),
    /// `200 OK` carrying `body` under an explicit `Content-Type`.
    ///
    /// [`Self::Body`] always declares `text/html`, so it cannot express the
    /// rows that prove reader mode refuses a non-HTML media type or a charset
    /// it does not decode. Those refusals exist because the alternative is
    /// returning replacement-character Markdown that looks like a faithful
    /// rendering, so they need a fixture that can actually declare the header.
    Typed { content_type: String, body: Vec<u8> },
    /// Arbitrary bounded status/headers/body for protocol-metadata contracts.
    Response {
        status: &'static str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// `200 OK` whose body is written incrementally.
    ///
    /// One variant covers every body shape the bound, timeout, and extraction
    /// rows need, because they differ only in these four numbers:
    ///
    /// - `declared` — the `Content-Length` to advertise, or `None` to send no
    ///   length at all and delimit the body by closing the connection.
    ///   Declaring more than `len` sends is how the short-count row is built.
    /// - `len` — how many body bytes to actually write.
    /// - `chunk` — bytes per write, so a reader can be observed stopping
    ///   part-way rather than only at the end.
    /// - `delay` — pause between writes, which is what makes a body outlast a
    ///   timeout without needing a large one.
    Stream {
        declared: Option<usize>,
        len: usize,
        chunk: usize,
        delay: Duration,
    },
}

impl FixtureResponse {
    /// A body of exactly `len` bytes, written promptly.
    pub const fn sized(len: usize) -> Self {
        Self::Stream {
            declared: Some(len),
            len,
            chunk: 64 * 1024,
            delay: Duration::ZERO,
        }
    }

    /// A body of `len` bytes dribbled out `chunk` at a time, pausing `delay`
    /// between writes.
    pub const fn trickle(len: usize, chunk: usize, delay: Duration) -> Self {
        Self::Stream {
            declared: Some(len),
            len,
            chunk,
            delay,
        }
    }

    /// A body advertising `declared` bytes but sending only `len`.
    pub const fn short_count(declared: usize, len: usize) -> Self {
        Self::Stream {
            declared: Some(declared),
            len,
            chunk: 64 * 1024,
            delay: Duration::ZERO,
        }
    }

    /// A body with no `Content-Length`, delimited by the connection closing.
    pub const fn undeclared(len: usize) -> Self {
        Self::Stream {
            declared: None,
            len,
            chunk: 64 * 1024,
            delay: Duration::ZERO,
        }
    }

    /// Renders the response head. `Stream` bodies are written separately by
    /// [`write_response`] so each write can be counted and paced.
    fn render_head(&self) -> String {
        match self {
            Self::Body(body) => format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
            Self::Redirect(location) => format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ),
            // Head only: the body may be arbitrary bytes (the invalid-UTF-8
            // row depends on that), so `write_response` writes it separately
            // rather than forcing it through a `String`.
            Self::Typed { content_type, body } => format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            ),
            Self::Response {
                status,
                headers,
                body,
            } => {
                let mut rendered = format!("HTTP/1.1 {status}\r\n");
                for (name, value) in headers {
                    rendered.push_str(name);
                    rendered.push_str(": ");
                    rendered.push_str(value);
                    rendered.push_str("\r\n");
                }
                rendered.push_str(&format!(
                    "Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                ));
                rendered
            }
            Self::Stream {
                declared: Some(declared),
                ..
            } => format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
            ),
            Self::Stream { declared: None, .. } => {
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n".to_owned()
            }
        }
    }
}

/// Writes a response, counting body bytes this listener actually flushed.
///
/// The count is the server-side oracle for every body bound: it is what the
/// peer was *sent*, measured here rather than inferred from what the client
/// says it accepted. A write error ends the loop, which is how the listener
/// observes the peer going away mid-body.
async fn write_response<W>(stream: &mut W, response: &FixtureResponse, flushed: &AtomicUsize)
where
    W: tokio::io::AsyncWrite + Unpin,
{
    if stream
        .write_all(response.render_head().as_bytes())
        .await
        .is_err()
    {
        return;
    }
    let body = match response {
        FixtureResponse::Typed { body, .. } | FixtureResponse::Response { body, .. } => body,
        _ => {
            let FixtureResponse::Stream {
                len, chunk, delay, ..
            } = response
            else {
                return;
            };
            let filler = vec![b'a'; *chunk];
            let mut sent = 0_usize;
            while sent < *len {
                let width = (*chunk).min(*len - sent);
                if stream.write_all(&filler[..width]).await.is_err()
                    || stream.flush().await.is_err()
                {
                    return;
                }
                flushed.fetch_add(width, Ordering::SeqCst);
                sent += width;
                if !delay.is_zero() {
                    tokio::time::sleep(*delay).await;
                }
            }
            return;
        }
    };
    if stream.write_all(body).await.is_ok() {
        flushed.fetch_add(body.len(), Ordering::SeqCst);
    }
}

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
    /// Full request heads (request line plus headers), so a fixture can assert
    /// what was transmitted on the wire — notably a credential header.
    heads: Arc<std::sync::Mutex<Vec<String>>>,
    flushed: Arc<AtomicUsize>,
}

impl TlsListener {
    /// Serves `response_body` over TLS using `cert`/`key`, on `ip`.
    ///
    /// `port` may be reused so two listeners differ only by address — the
    /// client substitutes the URL's port into whatever the resolver returns,
    /// so the address is the only discriminator available to a fixture.
    pub async fn serve(ip: IpAddr, port: u16, cert: &'static [u8], response_body: &str) -> Self {
        let body = response_body.to_owned();
        Self::serve_router(ip, port, cert, move |_path| {
            FixtureResponse::Body(body.clone())
        })
        .await
    }

    /// Serves a response chosen per request path.
    ///
    /// `router` receives the request target exactly as it arrived on the wire
    /// (the middle field of the request line) and returns the response to
    /// send. This is what lets one listener host a redirect chain, answering
    /// `/hop1` with a hop to `/hop2` and terminating with a body.
    pub async fn serve_router<R>(ip: IpAddr, port: u16, cert: &'static [u8], router: R) -> Self
    where
        R: Fn(&str) -> FixtureResponse + Send + Sync + 'static,
    {
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

        let listener = TcpListener::bind(SocketAddr::new(ip, port))
            .await
            .expect("loopback listener binds");
        let address = listener.local_addr().expect("listener reports its address");

        let accepts = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let heads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let flushed = Arc::new(AtomicUsize::new(0));
        let router: Arc<dyn Fn(&str) -> FixtureResponse + Send + Sync> = Arc::new(router);

        let (accept_counter, done_counter, log, head_source, flush_counter) = (
            Arc::clone(&accepts),
            Arc::clone(&completed),
            Arc::clone(&requests),
            Arc::clone(&heads),
            Arc::clone(&flushed),
        );
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                accept_counter.fetch_add(1, Ordering::SeqCst);
                let acceptor = acceptor.clone();
                let done = Arc::clone(&done_counter);
                let log = Arc::clone(&log);
                let head_log = Arc::clone(&head_source);
                let router = Arc::clone(&router);
                let flushed = Arc::clone(&flush_counter);
                tokio::spawn(async move {
                    // A handshake failure is an expected outcome here, not an
                    // error: the name-mismatch row exists to produce one.
                    let Ok(mut tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    done.fetch_add(1, Ordering::SeqCst);
                    let mut buffer = [0_u8; 1024];
                    let mut target = String::new();
                    if let Ok(read) = tls.read(&mut buffer).await
                        && read > 0
                        && let Some(line) = String::from_utf8_lossy(&buffer[..read]).lines().next()
                    {
                        // The request target is the middle field of the
                        // request line, recorded verbatim so a fixture can
                        // assert what was transmitted byte for byte.
                        target = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                        log.lock()
                            .expect("request log is uncontended")
                            .push(line.to_owned());
                        head_log
                            .lock()
                            .expect("request head log is uncontended")
                            .push(String::from_utf8_lossy(&buffer[..read]).into_owned());
                    }
                    write_response(&mut tls, &router(&target), &flushed).await;
                    let _ = tls.shutdown().await;
                });
            }
        });

        Self {
            address,
            accepts,
            completed,
            requests,
            heads,
            flushed,
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

    /// Body bytes this listener actually wrote and flushed.
    ///
    /// This is what the peer was *sent*, which is deliberately not the same as
    /// what the client accepted: socket and TLS buffers sit between them, so a
    /// server routinely flushes past a smaller client ceiling. Fixtures record
    /// this as an observation of transfer; the ceiling itself is asserted
    /// against retained bytes.
    pub fn flushed(&self) -> usize {
        self.flushed.load(Ordering::SeqCst)
    }

    /// Every request line this listener actually received.
    /// Returns the full request heads the listener received.
    ///
    /// Distinct from [`Self::requests`], which records only the request line:
    /// a credential is a header, so proving it reached the wire needs the head.
    pub fn heads(&self) -> Vec<String> {
        self.heads
            .lock()
            .expect("request head log is uncontended")
            .clone()
    }

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
    tls_substrate_with_ceilings(allowlist, addresses, HttpCeilings::default())
}

/// As [`tls_substrate`], with the ceilings under test.
///
/// The bound and timeout rows need ceilings far below the shipped ones so a
/// fixture can cross them in milliseconds instead of moving 8 MiB or waiting
/// 30 seconds. Lowering is the supported direction — `HttpCeilings::new`
/// refuses to raise any ceiling above its hard maximum — so exercising a small
/// value tests the same code path production uses.
pub fn tls_substrate_with_ceilings(
    allowlist: OriginAllowlist,
    addresses: Vec<IpAddr>,
    ceilings: HttpCeilings,
) -> HttpSubstrate {
    tls_substrate_full(allowlist, addresses, ceilings, Vec::new())
}

/// As [`tls_substrate`], carrying resolved per-origin credentials.
pub fn tls_substrate_with_credentials(
    allowlist: OriginAllowlist,
    addresses: Vec<IpAddr>,
    credentials: Vec<OriginCredential>,
) -> HttpSubstrate {
    tls_substrate_full(allowlist, addresses, HttpCeilings::default(), credentials)
}

fn tls_substrate_full(
    allowlist: OriginAllowlist,
    addresses: Vec<IpAddr>,
    ceilings: HttpCeilings,
    credentials: Vec<OriginCredential>,
) -> HttpSubstrate {
    HttpSubstrate::with_host_lookup_and_roots(
        allowlist,
        ceilings,
        move |_host| {
            let addresses = addresses.clone();
            async move { Ok::<_, io::Error>(addresses) }
        },
        &[FIXTURE_CA],
        credentials,
    )
    .expect("substrate builds")
}
