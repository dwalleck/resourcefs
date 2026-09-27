//! Loopback TLS fixture for profile-launched MCP contract tests.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};
use tokio_rustls::TlsAcceptor;

#[path = "../../../resourcefs-sources/tests/support/certificates.rs"]
mod certificates;
#[path = "../../../resourcefs-sources/tests/support/http_wire.rs"]
mod http_wire;

static CERTIFICATES: LazyLock<(Vec<u8>, [certificates::TestIdentity; 1])> =
    LazyLock::new(|| certificates::issue_test_certificates([&["localhost", "127.0.0.1", "::1"]]));

pub fn root_certificate() -> &'static [u8] {
    &CERTIFICATES.0
}

const MAX_REQUEST_HEAD: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub sequence: usize,
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

pub struct NativeResponse {
    /// Exact request target, including any query string.
    pub path: String,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

enum Responses {
    Html(Arc<str>),
    Native(Vec<NativeResponse>),
    BlockedNative {
        body: String,
        arrived: mpsc::Sender<()>,
        release: Arc<tokio::sync::Notify>,
    },
}

pub struct ProfileTlsServer {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    failures: Arc<Mutex<Vec<String>>>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ProfileTlsServer {
    pub fn start(body: &str) -> Self {
        Self::start_responses(Responses::Html(Arc::from(body)))
    }

    pub fn start_native(responses: Vec<NativeResponse>) -> Self {
        Self::start_responses(Responses::Native(responses))
    }

    pub fn start_blocked_native(
        body: String,
    ) -> (Self, mpsc::Receiver<()>, Arc<tokio::sync::Notify>) {
        let (arrived, receiver) = mpsc::channel();
        let release = Arc::new(tokio::sync::Notify::new());
        let server = Self::start_responses(Responses::BlockedNative {
            body,
            arrived,
            release: Arc::clone(&release),
        });
        (server, receiver, release)
    }

    fn start_responses(responses: Responses) -> Self {
        let responses = Arc::new(responses);
        let accepts = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let failures = Arc::new(Mutex::new(Vec::new()));
        let request_log = Arc::clone(&requests);
        let failure_log = Arc::clone(&failures);
        let accept_count = Arc::clone(&accepts);
        let (ready_sender, ready_receiver) = mpsc::channel();
        let (shutdown_sender, shutdown_receiver) = oneshot::channel();

        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("profile TLS runtime");
            runtime.block_on(async move {
                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                    .await
                    .expect("profile TLS listener");
                let address = listener.local_addr().expect("profile TLS address");
                ready_sender.send(address).expect("publish TLS address");
                let acceptor = TlsAcceptor::from(Arc::new(CERTIFICATES.1[0].server_config()));
                tokio::pin!(shutdown_receiver);

                loop {
                    tokio::select! {
                        _ = &mut shutdown_receiver => break,
                        accepted = listener.accept() => {
                            let (stream, _) = accepted.expect("accept profile TLS connection");
                            accept_count.fetch_add(1, Ordering::SeqCst);
                            let acceptor = acceptor.clone();
                            let responses = Arc::clone(&responses);
                            let requests = Arc::clone(&request_log);
                            let failures = Arc::clone(&failure_log);
                            tokio::spawn(async move {
                                if let Err(error) = serve(stream, acceptor, &responses, &requests).await {
                                    failures
                                        .lock()
                                        .expect("profile TLS failure log")
                                        .push(error.to_string());
                                }
                            });
                        }
                    }
                }
            });
        });
        let address = ready_receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("profile TLS fixture became ready");

        Self {
            address,
            accepts,
            requests,
            failures,
            shutdown: Some(shutdown_sender),
            thread: Some(thread),
        }
    }

    pub fn base_url(&self) -> String {
        format!("https://localhost:{}/", self.address.port())
    }

    pub fn accepts(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }

    pub fn requests(&self) -> Vec<String> {
        self.recorded_requests()
            .into_iter()
            .map(|request| request.path)
            .collect()
    }

    pub fn recorded_requests(&self) -> Vec<RecordedRequest> {
        let failures = self.failures.lock().expect("profile TLS failure log");
        assert!(
            failures.is_empty(),
            "profile TLS fixture failures: {failures:?}"
        );
        self.requests
            .lock()
            .expect("profile TLS request log")
            .clone()
    }
}

impl Drop for ProfileTlsServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take()
            && shutdown.send(()).is_err()
        {
            // The runtime already stopped; joining below remains the
            // authoritative check that its thread did not panic.
        }
        if let Some(thread) = self.thread.take() {
            thread.join().expect("profile TLS fixture thread");
        }
    }
}

async fn serve<S>(
    stream: S,
    acceptor: TlsAcceptor,
    responses: &Responses,
    requests: &Mutex<Vec<RecordedRequest>>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // The startup probe is intentionally a bare TCP connect. Its TLS handshake
    // fails cleanly; the next accepted connection is the real HTTPS read.
    let Ok(tls) = acceptor.accept(stream).await else {
        return Ok(());
    };
    serve_request(tls, responses, requests).await
}

async fn serve_request<S>(
    mut tls: S,
    responses: &Responses,
    requests: &Mutex<Vec<RecordedRequest>>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut head = Vec::new();
    let mut chunk = [0_u8; 1024];
    let head_end = loop {
        let read = match tls.read(&mut chunk).await {
            Ok(read) => read,
            Err(error)
                if head.is_empty()
                    && matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                    ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if read == 0 {
            return Ok(());
        }
        let scanned = head.len();
        head.extend_from_slice(&chunk[..read]);
        if let Some(end) = http_wire::head_end(&head, scanned) {
            break end;
        }
        if head.len() > MAX_REQUEST_HEAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "profile TLS request head exceeded fixture limit",
            ));
        }
    };
    // Only the head: bytes a client sent after the blank line are not headers
    // and need not be UTF-8.
    let request = std::str::from_utf8(&head[..head_end])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request head was not UTF-8"))?;
    let mut request_line = request.lines().next().unwrap_or("").split_whitespace();
    let method = request_line.next().unwrap_or("");
    let target = request_line
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request target"))?;
    // Rejected before the request is recorded, as `tls.rs` rejects it before
    // routing. Native identity operands name this actual listener.
    let host = http_wire::header(request, "host")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "request has no Host header"))?;
    let headers = http_wire::headers(request)
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();
    let sequence = {
        let mut log = requests.lock().expect("profile TLS request log");
        let sequence = log.len() + 1;
        log.push(RecordedRequest {
            sequence,
            method: method.to_owned(),
            path: target.to_owned(),
            headers,
        });
        sequence
    };
    let (status, content_type, body) = match responses {
        Responses::Html(body) => ("200 OK", "text/html; charset=utf-8", body.as_ref()),
        Responses::Native(responses) => match responses.get(sequence - 1) {
            Some(response) if response.path == target && method == "GET" => {
                ("200 OK", "application/json", response.body.as_str())
            }
            _ => (
                "404 Not Found",
                "application/json",
                "{\"message\":\"unexpected fixture request\"}",
            ),
        },
        Responses::BlockedNative {
            body,
            arrived,
            release,
        } => {
            if sequence == 1 {
                arrived
                    .send(())
                    .map_err(|_| io::Error::other("barrier observer closed"))?;
                release.notified().await;
            }
            ("200 OK", "application/json", body.as_str())
        }
    };
    let response_headers = match responses {
        Responses::Native(responses) => responses
            .get(sequence - 1)
            .map(|response| response.headers.clone())
            .unwrap_or_default(),
        Responses::Html(_) | Responses::BlockedNative { .. } => Vec::new(),
    };
    let api = format!("https://{host}/");
    let body = body.replace("@API@", &api);
    let response_headers: Vec<(String, String)> = response_headers
        .into_iter()
        .map(|(name, value)| (name, value.replace("@API@", &api)))
        .collect();
    let mut fields = vec![("Content-Type", content_type)];
    fields.extend(
        response_headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    );
    let response = http_wire::response_head(status, &fields, Some(body.len()));
    let sent = async {
        tls.write_all(response.as_bytes()).await?;
        tls.write_all(body.as_bytes()).await?;
        tls.shutdown().await
    }
    .await;
    match sent {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
            ) =>
        {
            Ok(())
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::io::{DuplexStream, ReadBuf, duplex};
    use tokio_rustls::{
        TlsConnector,
        rustls::{
            ClientConfig, RootCertStore,
            pki_types::{CertificateDer, ServerName},
        },
    };

    enum Fault {
        Read { after: usize, kind: io::ErrorKind },
        Write { after: usize, kind: io::ErrorKind },
        Shutdown(io::ErrorKind),
    }

    // Wrap established TLS, not its raw transport: counts now name handler
    // reads/writes rather than rustls handshake records or session tickets.
    struct FaultStream {
        inner: tokio_rustls::server::TlsStream<DuplexStream>,
        fault: Fault,
        fired: bool,
    }

    impl AsyncRead for FaultStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if let Fault::Read { after: 0, kind } = self.fault {
                self.fired = true;
                return Poll::Ready(Err(io::Error::new(kind, "injected request read fault")));
            }
            let before = buffer.filled().len();
            let result = Pin::new(&mut self.inner).poll_read(cx, buffer);
            if matches!(result, Poll::Ready(Ok(())))
                && buffer.filled().len() > before
                && let Fault::Read { after, .. } = &mut self.fault
            {
                *after -= 1;
            }
            result
        }
    }

    impl AsyncWrite for FaultStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            if let Fault::Write { after: 0, kind } = self.fault {
                self.fired = true;
                return Poll::Ready(Err(io::Error::new(kind, "injected response write fault")));
            }
            let result = Pin::new(&mut self.inner).poll_write(cx, buffer);
            if let Poll::Ready(Ok(written)) = result
                && written > 0
                && let Fault::Write { after, .. } = &mut self.fault
            {
                *after -= 1;
            }
            result
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.inner).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            if let Fault::Shutdown(kind) = self.fault {
                self.fired = true;
                return Poll::Ready(Err(io::Error::new(
                    kind,
                    "injected response shutdown fault",
                )));
            }
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }

    #[derive(Clone, Copy)]
    enum ResponseKind {
        Html,
        Native,
        BlockedNative,
    }

    async fn exercise(
        kind: ResponseKind,
        request: &[u8],
        fault: Fault,
    ) -> (io::Result<()>, Vec<RecordedRequest>) {
        tokio::time::timeout(Duration::from_secs(10), async {
            let (client_io, server_io) = duplex(64 * 1024);
            let mut roots = RootCertStore::empty();
            roots
                .add(CertificateDer::from(root_certificate().to_vec()))
                .expect("test root certificate");
            let connector = TlsConnector::from(Arc::new(
                ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            ));
            let acceptor = TlsAcceptor::from(Arc::new(CERTIFICATES.1[0].server_config()));
            // Both sides must finish successfully before any fault can fire.
            let (client, server) = tokio::join!(
                connector.connect(
                    ServerName::try_from("localhost").expect("server name"),
                    client_io
                ),
                acceptor.accept(server_io),
            );
            let mut client = client.expect("client TLS handshake");
            let mut server = FaultStream {
                inner: server.expect("server TLS handshake"),
                fault,
                fired: false,
            };
            client.write_all(request).await.expect("send request bytes");
            client.flush().await.expect("flush request bytes");
            let (arrived, _receiver) = mpsc::channel();
            let responses = match kind {
                ResponseKind::Html => Responses::Html(Arc::from("body")),
                ResponseKind::Native => Responses::Native(vec![NativeResponse {
                    path: "/".to_owned(),
                    body: "{}".to_owned(),
                    headers: Vec::new(),
                }]),
                ResponseKind::BlockedNative => {
                    let release = Arc::new(tokio::sync::Notify::new());
                    release.notify_one();
                    Responses::BlockedNative {
                        body: "{}".to_owned(),
                        arrived,
                        release,
                    }
                }
            };
            let requests = Mutex::new(Vec::new());
            let result = serve_request(&mut server, &responses, &requests).await;
            assert!(server.fired, "the configured I/O fault must execute");
            (result, requests.into_inner().expect("request log"))
        })
        .await
        .expect("TLS fault scenario exceeded ten seconds")
    }

    const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";

    #[tokio::test]
    async fn serve_treats_empty_head_disconnects_as_eof() {
        for kind in [
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
        ] {
            let (result, requests) =
                exercise(ResponseKind::Html, b"", Fault::Read { after: 0, kind }).await;
            assert!(result.is_ok(), "empty-head {kind:?}: {result:?}");
            assert!(requests.is_empty());
        }
    }

    #[tokio::test]
    async fn serve_propagates_partial_head_disconnects() {
        for kind in [
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
        ] {
            let (result, requests) = exercise(
                ResponseKind::Html,
                b"GET / HTTP/1.1\r\n",
                Fault::Read { after: 1, kind },
            )
            .await;
            assert_eq!(
                result
                    .expect_err("partial-head disconnect must fail")
                    .kind(),
                kind
            );
            assert!(requests.is_empty());
        }
    }

    #[tokio::test]
    async fn serve_propagates_unrelated_request_read_errors() {
        let (result, requests) = exercise(
            ResponseKind::Html,
            b"",
            Fault::Read {
                after: 0,
                kind: io::ErrorKind::Other,
            },
        )
        .await;
        assert_eq!(
            result.expect_err("unrelated read error must fail").kind(),
            io::ErrorKind::Other
        );
        assert!(requests.is_empty());
    }

    #[tokio::test]
    async fn serve_tolerates_response_disconnects_for_each_variant() {
        for kind in [
            ResponseKind::Html,
            ResponseKind::Native,
            ResponseKind::BlockedNative,
        ] {
            let (result, requests) = exercise(
                kind,
                REQUEST,
                Fault::Write {
                    after: 0,
                    kind: io::ErrorKind::BrokenPipe,
                },
            )
            .await;
            assert!(result.is_ok(), "response disconnect: {result:?}");
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].path, "/");
        }
    }

    #[tokio::test]
    async fn serve_tolerates_response_body_disconnect() {
        let (result, requests) = exercise(
            ResponseKind::Html,
            REQUEST,
            Fault::Write {
                after: 1,
                kind: io::ErrorKind::ConnectionReset,
            },
        )
        .await;
        assert!(result.is_ok(), "body disconnect: {result:?}");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/");
    }

    #[tokio::test]
    async fn serve_tolerates_response_shutdown_disconnect() {
        let (result, requests) = exercise(
            ResponseKind::Html,
            REQUEST,
            Fault::Shutdown(io::ErrorKind::ConnectionAborted),
        )
        .await;
        assert!(result.is_ok(), "shutdown disconnect: {result:?}");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/");
    }

    #[tokio::test]
    async fn serve_propagates_unrelated_response_write_errors() {
        let (result, requests) = exercise(
            ResponseKind::Html,
            REQUEST,
            Fault::Write {
                after: 0,
                kind: io::ErrorKind::Other,
            },
        )
        .await;
        assert_eq!(
            result.expect_err("unrelated write error must fail").kind(),
            io::ErrorKind::Other
        );
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/");
    }

    // `http_wire.rs` is included by both TLS fixtures; its own rows live here,
    // in the one binary that includes this file, so they run once.
    #[test]
    fn head_end_finds_a_terminator_split_across_reads() {
        let request = b"GET / HTTP/1.1\r\nHost: a\r\n\r\nbody";
        let end = request.len() - 4;
        assert_eq!(super::http_wire::head_end(request, 0), Some(end));
        // The previous read ended two bytes into the terminator.
        assert_eq!(super::http_wire::head_end(request, end - 2), Some(end));
        assert_eq!(super::http_wire::head_end(b"GET / HTTP/1.1\r\n", 0), None);
    }

    #[test]
    fn header_lookup_and_header_list_agree() {
        let head = "GET http://x:1/ HTTP/1.1\r\nHost:  tls.invalid \r\nX-Y: 1\r\n\r\n";
        let listed: Vec<_> = super::http_wire::headers(head).collect();
        assert_eq!(listed, [("Host", "tls.invalid"), ("X-Y", "1")]);
        assert_eq!(super::http_wire::header(head, "host"), Some("tls.invalid"));
        // The request line's colon is not a header.
        assert_eq!(super::http_wire::header(head, "get http"), None);
    }

    #[test]
    fn response_head_omits_an_absent_length() {
        assert_eq!(
            super::http_wire::response_head("200 OK", &[("A", "b")], None),
            "HTTP/1.1 200 OK\r\nA: b\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(
            super::http_wire::response_head("302 Found", &[], Some(0)),
            "HTTP/1.1 302 Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
    }
}
