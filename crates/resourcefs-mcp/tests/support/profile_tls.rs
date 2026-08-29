//! Loopback TLS fixture for profile-launched MCP contract tests.

use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
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

const CERTIFICATE: &[u8] = include_bytes!("../fixtures/profile_https/server.crt.der");
const PRIVATE_KEY: &[u8] = include_bytes!("../fixtures/profile_https/server.key.der");
const MAX_REQUEST_HEAD: usize = 16 * 1024;

pub struct ProfileTlsServer {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    failures: Arc<Mutex<Vec<String>>>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ProfileTlsServer {
    pub fn start(body: &str) -> Self {
        let body: Arc<str> = Arc::from(body);
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
                            let body = Arc::clone(&body);
                            let requests = Arc::clone(&request_log);
                            let failures = Arc::clone(&failure_log);
                            tokio::spawn(async move {
                                if let Err(error) = serve(stream, acceptor, &body, &requests).await {
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
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(CERTIFICATE.to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(PRIVATE_KEY.to_vec())),
        )
        .expect("profile TLS certificate and key")
}

async fn serve<S>(
    stream: S,
    acceptor: TlsAcceptor,
    body: &str,
    requests: &Mutex<Vec<String>>,
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
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request target"))?;
    requests
        .lock()
        .expect("profile TLS request log")
        .push(target.to_owned());

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tls.write_all(response.as_bytes()).await?;
    tls.write_all(body.as_bytes()).await?;
    tls.shutdown().await
}
