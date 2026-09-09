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
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    },
};

#[path = "../../../resourcefs-sources/tests/support/certificates.rs"]
mod certificates;

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
    pub path: String,
    pub body: String,
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
                let acceptor = TlsAcceptor::from(Arc::new(server_config()));
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

fn server_config() -> ServerConfig {
    let identity = &CERTIFICATES.1[0];
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(identity.certificate.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.private_key.clone())),
        )
        .expect("profile TLS certificate and key")
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
    let Ok(mut tls) = acceptor.accept(stream).await else {
        return Ok(());
    };
    let mut head = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = tls.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        head.extend_from_slice(&chunk[..read]);
        if head.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if head.len() > MAX_REQUEST_HEAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "profile TLS request head exceeded fixture limit",
            ));
        }
    }
    let request = std::str::from_utf8(&head)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request head was not UTF-8"))?;
    let mut lines = request.lines();
    let mut request_line = lines.next().unwrap_or("").split_whitespace();
    let method = request_line.next().unwrap_or("");
    let target = request_line
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request target"))?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
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
            // Collection reads carry a `per_page`/`page` query; the fixture
            // names the endpoint, so compare the path only.
            Some(response)
                if response.path == target.split('?').next().unwrap_or(target)
                    && method == "GET" =>
            {
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
    // Native identity operands name this actual listener, not a guessed API.
    let host = request
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
                .map(|(_, value)| value.trim())
        })
        .unwrap_or("");
    let body = body.replace("@API@", &format!("https://{host}/"));

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let sent = async {
        tls.write_all(response.as_bytes()).await?;
        tls.write_all(body.as_bytes()).await?;
        tls.shutdown().await
    }
    .await;
    match sent {
        Err(error)
            if matches!(responses, Responses::BlockedNative { .. })
                && matches!(
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
