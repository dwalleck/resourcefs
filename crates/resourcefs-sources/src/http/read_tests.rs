//! Owning-module contracts use the existing TLS server, not a replay mock.
#[path = "../../tests/support/tls.rs"]
mod tls;

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};

use resourcefs_core::{ErrorCategory, HttpCeilings, OperationGuard};
use url::Url;

use super::{HttpReadBudget, HttpRequest, HttpSubstrate, LogicalDeadline};
use tls::{FIXTURE_HOST, FixtureResponse, TlsListener, fixture_allowlist, fixture_ca, match_cert};

fn loopback() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

fn substrate(listener: &TlsListener) -> HttpSubstrate {
    // The shared fixture's substrate helper refers to the self dev-dependency;
    // construct this unit-test crate's concrete substrate with the same root.
    HttpSubstrate::with_host_lookup_and_roots(
        fixture_allowlist(listener.address.port(), true),
        HttpCeilings::default(),
        |_| async { Ok(vec![loopback()]) },
        &[fixture_ca()],
        Vec::new(),
    )
    .expect("fixture substrate")
    .with_retry_control_for_test(UNIX_EPOCH, Duration::ZERO)
    .expect("controlled retry clock")
}

fn url(listener: &TlsListener, path: &str) -> Url {
    Url::parse(&format!(
        "https://{FIXTURE_HOST}:{}{path}",
        listener.address.port()
    ))
    .expect("fixture URL")
}

fn unavailable(delay: &str) -> FixtureResponse {
    FixtureResponse::Response {
        status: "503 Service Unavailable",
        headers: vec![("Retry-After".to_owned(), delay.to_owned())],
        body: b"busy".to_vec(),
    }
}

#[tokio::test]
async fn get_replay_preserves_headers_and_shared_retry_is_spent_across_fetches() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&attempts);
    let listener = TlsListener::serve_router(loopback(), 0, match_cert(), move |_| {
        if counted.fetch_add(1, Ordering::SeqCst) == 1 {
            FixtureResponse::Body("retried".to_owned())
        } else {
            unavailable("0")
        }
    })
    .await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let read = substrate.begin_read(&operation).expect("logical read");
    let mut budget = HttpReadBudget::new(10).expect("budget");
    let request = HttpRequest::get(url(&listener, "/first"))
        .with_header("X-Read-Identity", "same-read")
        .expect("header");
    assert_eq!(
        read.fetch_with_budget(request, &mut budget)
            .await
            .expect("retry")
            .body(),
        b"retried"
    );
    assert_eq!(budget.remaining_attempts(), 8);
    assert_eq!(listener.requests(), vec!["GET /first HTTP/1.1"; 2]);
    assert_eq!(listener.bodies(), vec![Vec::<u8>::new(); 2]);
    for head in listener.heads() {
        assert!(
            head.to_ascii_lowercase()
                .contains("x-read-identity: same-read\r\n")
        );
    }
    let response = read
        .fetch_with_budget(HttpRequest::get(url(&listener, "/later")), &mut budget)
        .await
        .expect("terminal 503 response");
    assert_eq!(response.status(), 503);
    assert_eq!(budget.remaining_attempts(), 7);
    assert_eq!(
        listener.requests().len(),
        3,
        "later fetch cannot regain the retry permit"
    );
}

#[tokio::test]
async fn ordinary_post_and_patch_never_replay_and_preserve_encoded_body() {
    let listener =
        TlsListener::serve_router(loopback(), 0, match_cert(), |_| unavailable("0")).await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let read = substrate.begin_read(&operation).expect("logical read");
    let mut budget = HttpReadBudget::new(10).expect("budget");
    let body = br#"{"text":"encoded \\ and \u263a","n":7}"#.to_vec();
    for request in [
        HttpRequest::post_json(url(&listener, "/mutation"), body.clone()).expect("POST"),
        HttpRequest::patch_json(url(&listener, "/mutation"), body.clone()).expect("PATCH"),
    ] {
        assert_eq!(
            read.fetch_with_budget(request, &mut budget)
                .await
                .expect("response")
                .status(),
            503
        );
    }
    assert_eq!(
        listener.requests(),
        ["POST /mutation HTTP/1.1", "PATCH /mutation HTTP/1.1"]
    );
    assert_eq!(listener.bodies(), vec![body.clone(), body]);
    assert_eq!(budget.remaining_attempts(), 8);
    // Mutations must not spend the read retry permit either.
    assert_eq!(
        read.fetch_with_budget(HttpRequest::get(url(&listener, "/read")), &mut budget)
            .await
            .expect("GET retry")
            .status(),
        503
    );
    assert_eq!(listener.requests().len(), 4);
}

#[tokio::test]
async fn physical_attempt_ceiling_counts_retry_and_refuses_eleventh_request() {
    let listener =
        TlsListener::serve_router(loopback(), 0, match_cert(), |_| unavailable("0")).await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let read = substrate.begin_read(&operation).expect("logical read");
    let mut budget = HttpReadBudget::new(10).expect("budget");
    for _ in 0..9 {
        assert_eq!(
            read.fetch_with_budget(HttpRequest::get(url(&listener, "/page")), &mut budget)
                .await
                .expect("bounded response")
                .status(),
            503
        );
    }
    assert_eq!(budget.remaining_attempts(), 0);
    let error = read
        .fetch_with_budget(HttpRequest::get(url(&listener, "/eleventh")), &mut budget)
        .await
        .expect_err("attempt ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(listener.requests(), vec!["GET /page HTTP/1.1"; 10]);
    let mut last = HttpReadBudget::new(1).expect("one attempt");
    assert_eq!(
        read.fetch_with_budget(HttpRequest::get(url(&listener, "/last")), &mut last)
            .await
            .expect("no room for retry")
            .status(),
        503
    );
    assert_eq!(listener.requests().len(), 11);
}

#[tokio::test]
async fn retry_wait_cancellation_and_over_deadline_delay_send_no_followup() {
    let listener =
        TlsListener::serve_router(loopback(), 0, match_cert(), |_| unavailable("10")).await;
    let mut substrate = substrate(&listener);
    let waiting = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&waiting);
    substrate.retry_runtime.jitter = Arc::new(move || {
        signal.notify_one();
        Ok(Duration::ZERO)
    });
    let operation = OperationGuard::new();
    let read = substrate.begin_read(&operation).expect("logical read");
    let fetch = read.fetch(HttpRequest::get(url(&listener, "/cancel")));
    let cancel = async {
        tokio::time::timeout(Duration::from_secs(5), waiting.notified())
            .await
            .expect("retry wait entered");
        assert!(operation.cancel());
    };
    let (result, ()) = tokio::join!(fetch, cancel);
    assert_eq!(
        result.expect_err("cancelled wait").category(),
        ErrorCategory::Cancelled
    );
    let operation = OperationGuard::new();
    let mut read = substrate.begin_read(&operation).expect("logical read");
    read.deadline = LogicalDeadline(tokio::time::Instant::now() + Duration::from_secs(2));
    assert_eq!(
        read.fetch(HttpRequest::get(url(&listener, "/too-long")))
            .await
            .expect_err("delay cannot fit")
            .category(),
        ErrorCategory::SourceUnavailable
    );
    assert_eq!(
        listener.requests(),
        ["GET /cancel HTTP/1.1", "GET /too-long HTTP/1.1"]
    );
}
