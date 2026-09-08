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
    let error = result.expect_err("cancelled wait");
    assert_eq!(error.category(), ErrorCategory::Cancelled);
    assert_eq!(
        error.details().expect("cancellation facts").reason(),
        resourcefs_core::ErrorReason::Cancelled
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

#[tokio::test]
async fn read_only_post_replays_exact_bytes_and_headers_once_across_pages() {
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
    let body = br#"{"text":"encoded \\ and \u263a","n":7}"#.to_vec();
    let request = HttpRequest::post_json_read_only(url(&listener, "/query"), body.clone())
        .expect("read-only POST")
        .with_header("X-Read-Identity", "same-read")
        .expect("header");
    assert_eq!(
        read.fetch_with_budget(request, &mut budget)
            .await
            .expect("retry")
            .body(),
        b"retried"
    );
    assert_eq!(listener.requests(), ["POST /query HTTP/1.1"; 2]);
    assert_eq!(listener.bodies(), vec![body.clone(); 2]);
    let heads = listener.heads();
    assert_eq!(
        heads[0], heads[1],
        "C7 replay preserves every transmitted header byte"
    );
    assert!(
        heads[0]
            .to_ascii_lowercase()
            .contains("content-type: application/json\r\n")
    );
    assert!(
        heads[0]
            .to_ascii_lowercase()
            .contains("x-read-identity: same-read\r\n")
    );
    assert_eq!(budget.remaining_attempts(), 8);
    let later =
        HttpRequest::post_json_read_only(url(&listener, "/later"), body).expect("next POST");
    assert_eq!(
        read.fetch_with_budget(later, &mut budget)
            .await
            .expect("spent retry")
            .status(),
        503
    );
    assert_eq!(
        listener.requests().len(),
        3,
        "C7 no fresh retry on later pages"
    );
    assert_eq!(budget.remaining_attempts(), 7);
}

#[tokio::test]
async fn read_only_post_physical_ceiling_counts_retry() {
    let listener =
        TlsListener::serve_router(loopback(), 0, match_cert(), |_| unavailable("0")).await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let read = substrate.begin_read(&operation).expect("logical read");
    let mut budget = HttpReadBudget::new(10).expect("budget");
    for _ in 0..9 {
        let request = HttpRequest::post_json_read_only(url(&listener, "/page"), b"{}".to_vec())
            .expect("POST");
        assert_eq!(
            read.fetch_with_budget(request, &mut budget)
                .await
                .expect("response")
                .status(),
            503
        );
    }
    let request = HttpRequest::post_json_read_only(url(&listener, "/eleventh"), b"{}".to_vec())
        .expect("POST");
    assert_eq!(
        read.fetch_with_budget(request, &mut budget)
            .await
            .expect_err("ceiling")
            .category(),
        ErrorCategory::LimitExceeded
    );
    assert_eq!(budget.remaining_attempts(), 0);
    assert_eq!(listener.requests(), vec!["POST /page HTTP/1.1"; 10]);
}

#[tokio::test]
async fn original_deadline_bounds_all_post_and_patch_before_and_during_egress() {
    let listener = TlsListener::serve_router(loopback(), 0, match_cert(), |path| {
        if path == "/control" {
            FixtureResponse::Body("reachable".to_owned())
        } else {
            FixtureResponse::Stream {
                declared: Some(2),
                len: 2,
                chunk: 1,
                delay: Duration::from_secs(10),
            }
        }
    })
    .await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    for constructor in [
        HttpRequest::post_json,
        HttpRequest::patch_json,
        HttpRequest::post_json_read_only,
    ] {
        let mut read = substrate.begin_read(&operation).expect("logical read");
        assert_eq!(
            read.fetch(HttpRequest::get(url(&listener, "/control")))
                .await
                .expect("control")
                .body(),
            b"reachable"
        );
        read.deadline = LogicalDeadline(tokio::time::Instant::now() + Duration::from_millis(500));
        let request = constructor(url(&listener, "/slow"), b"{}".to_vec()).expect("JSON");
        let error = tokio::time::timeout(Duration::from_secs(2), read.fetch(request))
            .await
            .expect("original deadline interrupts response")
            .expect_err("deadline");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
        read.deadline = LogicalDeadline(tokio::time::Instant::now());
        let request = constructor(url(&listener, "/expired"), b"{}".to_vec()).expect("JSON");
        assert_eq!(
            read.fetch(request).await.expect_err("expired").category(),
            ErrorCategory::SourceUnavailable
        );
    }
    assert_eq!(
        listener.requests(),
        [
            "GET /control HTTP/1.1",
            "POST /slow HTTP/1.1",
            "GET /control HTTP/1.1",
            "PATCH /slow HTTP/1.1",
            "GET /control HTTP/1.1",
            "POST /slow HTTP/1.1",
        ]
    );
}

#[tokio::test]
async fn read_only_post_wait_obeys_cancellation_and_remaining_deadline() {
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
    let mut budget = HttpReadBudget::new(10).expect("budget");
    let request =
        HttpRequest::post_json_read_only(url(&listener, "/cancel"), b"{}".to_vec()).expect("POST");
    let cancel = async {
        tokio::time::timeout(Duration::from_secs(5), waiting.notified())
            .await
            .expect("retry wait entered");
        assert!(operation.cancel());
    };
    let (result, ()) = tokio::join!(read.fetch_with_budget(request, &mut budget), cancel);
    assert_eq!(
        result.expect_err("cancelled").category(),
        ErrorCategory::Cancelled
    );
    assert_eq!(budget.remaining_attempts(), 9);
    let operation = OperationGuard::new();
    let mut read = substrate.begin_read(&operation).expect("logical read");
    read.deadline = LogicalDeadline(tokio::time::Instant::now() + Duration::from_secs(2));
    let request = HttpRequest::post_json_read_only(url(&listener, "/too-long"), b"{}".to_vec())
        .expect("POST");
    assert_eq!(
        read.fetch_with_budget(request, &mut budget)
            .await
            .expect("spent retry returns status")
            .status(),
        503
    );
    let mut budget = HttpReadBudget::new(10).expect("fresh operation budget");
    let request = HttpRequest::post_json_read_only(url(&listener, "/too-long"), b"{}".to_vec())
        .expect("POST");
    assert_eq!(
        read.fetch_with_budget(request, &mut budget)
            .await
            .expect_err("wait exceeds remaining time")
            .category(),
        ErrorCategory::SourceUnavailable
    );
    assert_eq!(budget.remaining_attempts(), 9);
    assert_eq!(
        listener.requests(),
        [
            "POST /cancel HTTP/1.1",
            "POST /too-long HTTP/1.1",
            "POST /too-long HTTP/1.1"
        ]
    );
}

#[tokio::test]
async fn read_only_body_ceiling_is_independent_of_mutations_and_debug_is_redacted() {
    const CEILING: usize = 384 * 1024;
    let listener = TlsListener::serve(loopback(), 0, match_cert(), "accepted").await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let read = substrate.begin_read(&operation).expect("logical read");
    let mut body = vec![b' '; CEILING];
    let canary = b"QUERY_PAYLOAD_CANARY";
    body[..canary.len()].copy_from_slice(canary);
    let request = HttpRequest::post_json_read_only(url(&listener, "/boundary"), body.clone())
        .expect("exact ceiling");
    assert!(!format!("{request:?}").contains("QUERY_PAYLOAD_CANARY"));
    assert_eq!(
        read.fetch(request).await.expect("accepted boundary").body(),
        b"accepted"
    );
    let error =
        HttpRequest::post_json_read_only(url(&listener, "/oversized"), vec![b'x'; CEILING + 1])
            .expect_err("ceiling plus one");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    for constructor in [HttpRequest::post_json, HttpRequest::patch_json] {
        let request = constructor(url(&listener, "/mutation"), vec![b'x'; CEILING + 1])
            .expect("mutation ceiling unchanged");
        assert_eq!(
            read.fetch(request).await.expect("mutation accepted").body(),
            b"accepted"
        );
    }
    assert_eq!(
        listener.requests(),
        [
            "POST /boundary HTTP/1.1",
            "POST /mutation HTTP/1.1",
            "PATCH /mutation HTTP/1.1"
        ]
    );
    assert_eq!(
        listener.bodies(),
        vec![body, vec![b'x'; CEILING + 1], vec![b'x'; CEILING + 1]]
    );
}

#[test]
fn read_only_replay_shares_384kib_payload_across_10000_copies() {
    let started = std::time::Instant::now();
    let body = vec![b'x'; 384 * 1024];
    let request = HttpRequest::post_json_read_only(
        Url::parse("https://example.test/query").expect("URL"),
        body,
    )
    .expect("POST");
    let original = request.body.as_ref().expect("body");
    for _ in 0..10_000 {
        let copy = std::hint::black_box(request.retry_copy().expect("read replay"));
        let replay = copy.body.as_ref().expect("body");
        assert_eq!(
            replay.as_ptr(),
            original.as_ptr(),
            "C7 retry must share, not allocate another payload"
        );
        assert_eq!(replay.len(), 384 * 1024);
    }
    let elapsed = started.elapsed();
    let ceiling = Duration::from_secs(if cfg!(debug_assertions) { 5 } else { 1 });
    eprintln!("C7 384 KiB / 10,000 replay copies: {elapsed:?}");
    assert!(
        elapsed <= ceiling,
        "C7 replay construction exceeded {ceiling:?}: {elapsed:?}"
    );
}

#[tokio::test]
async fn ordinary_mutations_never_replay_without_budget_on_retryable_status_or_disconnect() {
    let listener = TlsListener::serve_router(loopback(), 0, match_cert(), |path| match path {
        "/abort" => FixtureResponse::Abort,
        "/429" => FixtureResponse::Response {
            status: "429 Too Many Requests",
            headers: vec![("Retry-After".to_owned(), "0".to_owned())],
            body: b"busy".to_vec(),
        },
        _ => unavailable("0"),
    })
    .await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let read = substrate.begin_read(&operation).expect("logical read");
    for constructor in [HttpRequest::post_json, HttpRequest::patch_json] {
        for (path, status) in [("/503", 503), ("/429", 429)] {
            let request = constructor(url(&listener, path), b"{}".to_vec()).expect("mutation");
            assert_eq!(read.fetch(request).await.expect("status").status(), status);
        }
        let request = constructor(url(&listener, "/abort"), b"{}".to_vec()).expect("mutation");
        assert_eq!(
            read.fetch(request)
                .await
                .expect_err("disconnect")
                .category(),
            ErrorCategory::SourceUnavailable
        );
    }
    assert_eq!(
        listener.requests(),
        [
            "POST /503 HTTP/1.1",
            "POST /429 HTTP/1.1",
            "POST /abort HTTP/1.1",
            "PATCH /503 HTTP/1.1",
            "PATCH /429 HTTP/1.1",
            "PATCH /abort HTTP/1.1",
        ]
    );
}

#[tokio::test]
async fn read_only_post_requires_valid_retry_after_and_replays_transport_failure() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&attempts);
    let listener = TlsListener::serve_router(loopback(), 0, match_cert(), move |path| match path {
        "/missing" => FixtureResponse::Response {
            status: "503 Service Unavailable",
            headers: Vec::new(),
            body: b"busy".to_vec(),
        },
        "/malformed" => unavailable("not-a-delay"),
        "/transport" if counted.fetch_add(1, Ordering::SeqCst) == 0 => FixtureResponse::Abort,
        _ => FixtureResponse::Body("retried".to_owned()),
    })
    .await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let read = substrate.begin_read(&operation).expect("logical read");
    let mut budget = HttpReadBudget::new(10).expect("budget");
    for path in ["/missing", "/malformed"] {
        let request =
            HttpRequest::post_json_read_only(url(&listener, path), b"{}".to_vec()).expect("POST");
        assert_eq!(
            read.fetch_with_budget(request, &mut budget)
                .await
                .expect("invalid guidance")
                .status(),
            503
        );
    }
    let request = HttpRequest::post_json_read_only(url(&listener, "/transport"), b"{}".to_vec())
        .expect("POST");
    assert_eq!(
        read.fetch_with_budget(request, &mut budget)
            .await
            .expect("transport replay")
            .body(),
        b"retried"
    );
    assert_eq!(budget.remaining_attempts(), 6);
    assert_eq!(
        listener.requests(),
        [
            "POST /missing HTTP/1.1",
            "POST /malformed HTTP/1.1",
            "POST /transport HTTP/1.1",
            "POST /transport HTTP/1.1",
        ]
    );
    assert_eq!(listener.bodies(), vec![b"{}".to_vec(); 4]);
}

#[path = "read_acquisition_tests.rs"]
mod acquisition;
