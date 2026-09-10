use super::*;

#[tokio::test]
async fn controlled_retry_uses_one_shared_physical_attempt_ledger() {
    for (allowed, expected) in [(1, 1), (2, 2), (10, 2)] {
        let listener =
            TlsListener::serve_router(loopback(), 0, match_cert(), |_| unavailable("0")).await;
        let substrate = substrate(&listener);
        let operation = OperationGuard::new();
        let limits = resourcefs_core::ReadAcquisitionLimits::new(
            Some(allowed),
            None,
            None,
            None,
            None,
            None,
        )
        .expect("limits");
        let (read, effective) = substrate
            .begin_read_with_limits(&operation, &limits)
            .expect("read");
        let mut budget = HttpReadBudget::with_limits(&effective);
        let response = read
            .fetch_with_budget(HttpRequest::get(url(&listener, "/retry")), &mut budget)
            .await
            .expect("terminal response");
        assert_eq!(response.status(), 503);
        assert_eq!(listener.requests(), vec!["GET /retry HTTP/1.1"; expected]);
        assert_eq!(budget.used_attempts(), expected);
        for _ in expected..allowed {
            read.fetch_with_budget(HttpRequest::get(url(&listener, "/next")), &mut budget)
                .await
                .expect("remaining permit");
        }
        assert_eq!(listener.requests().len(), allowed);
        let error = read
            .fetch_with_budget(HttpRequest::get(url(&listener, "/refused")), &mut budget)
            .await
            .expect_err("hard shared boundary");
        assert_eq!(error.category(), ErrorCategory::LimitExceeded);
        assert_eq!(listener.requests().len(), allowed);
        assert_eq!(budget.used_attempts(), allowed);
    }
}

#[tokio::test]
async fn verified_http_bodies_share_cumulative_admission_without_reset() {
    for (first, second, cap) in [
        (8 * 1024 * 1024, 8 * 1024 * 1024, 16 * 1024 * 1024),
        (3, 7, 10),
    ] {
        let listener = TlsListener::serve_router(loopback(), 0, match_cert(), move |path| {
            let len = match path {
                "/first" => first,
                "/second" => second,
                _ => 1,
            };
            FixtureResponse::Stream {
                declared: Some(len),
                len,
                chunk: 64 * 1024,
                delay: Duration::ZERO,
            }
        })
        .await;
        let substrate = substrate(&listener);
        let operation = OperationGuard::new();
        let limits =
            resourcefs_core::ReadAcquisitionLimits::new(None, None, None, Some(cap), None, None)
                .expect("limits");
        let (read, effective) = substrate
            .begin_read_with_limits(&operation, &limits)
            .expect("read");
        let mut budget = HttpReadBudget::with_limits(&effective);
        for (path, bytes, total) in [("/first", first, first), ("/second", second, cap)] {
            let response = read
                .fetch_with_budget(HttpRequest::get(url(&listener, path)), &mut budget)
                .await
                .expect("individually bounded response");
            assert!(!response.truncated());
            assert_eq!(response.body().len(), bytes);
            budget
                .admit_body(response.body().len())
                .expect("verified admission");
            assert_eq!(budget.accepted_body_bytes(), total);
        }
        let response = read
            .fetch_with_budget(HttpRequest::get(url(&listener, "/one")), &mut budget)
            .await
            .expect("third HTTP body itself fits");
        assert_eq!(response.body().len(), 1);
        assert!(!response.truncated());
        let error = budget
            .admit_body(response.body().len())
            .expect_err("total, not response, ceiling");
        assert_eq!(
            error
                .details()
                .expect("detail")
                .limit()
                .expect("limit")
                .kind(),
            resourcefs_core::AcquisitionLimitKind::AcceptedBodyBytes
        );
        assert_eq!(budget.accepted_body_bytes(), cap);
        assert_eq!(budget.used_attempts(), 3);
        assert_eq!(
            listener.requests(),
            [
                "GET /first HTTP/1.1",
                "GET /second HTTP/1.1",
                "GET /one HTTP/1.1"
            ]
        );
        read.check_acceptance()
            .expect("competing clock permits success");
    }
}

#[tokio::test]
async fn effective_response_cap_bounds_stream_and_reused_body_admission() {
    let listener = TlsListener::serve_router(loopback(), 0, match_cert(), |path| {
        let len = if path == "/exact" { 7 } else { 8 };
        FixtureResponse::Stream {
            declared: None,
            len,
            chunk: 1,
            delay: Duration::ZERO,
        }
    })
    .await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let limits = resourcefs_core::ReadAcquisitionLimits::new(None, None, Some(7), None, None, None)
        .expect("limits");
    let (read, effective) = substrate
        .begin_read_with_limits(&operation, &limits)
        .expect("read");
    let mut budget = HttpReadBudget::with_limits(&effective);
    let exact = read
        .fetch_with_budget(HttpRequest::get(url(&listener, "/exact")), &mut budget)
        .await
        .expect("exact");
    assert_eq!(exact.body().len(), 7);
    assert!(!exact.truncated());
    budget.admit_body(7).expect("exact cached body accepted");
    let over = read
        .fetch_with_budget(HttpRequest::get(url(&listener, "/over")), &mut budget)
        .await
        .expect("bounded stream");
    assert_eq!(over.body().len(), 7);
    assert!(over.truncated());
    assert_eq!(
        budget
            .admit_body(8)
            .expect_err("cached body constrained")
            .details()
            .expect("details")
            .limit()
            .expect("limit")
            .kind(),
        resourcefs_core::AcquisitionLimitKind::ResponseBodyBytes
    );
    assert_eq!(budget.accepted_body_bytes(), 7);
}

#[test]
fn legacy_ledger_does_not_acquire_a_cumulative_byte_policy() {
    let mut budget = HttpReadBudget::new(10).expect("legacy budget");
    for _ in 0..3 {
        budget
            .admit_body(8 * 1024 * 1024)
            .expect("legacy admission");
    }
    assert_eq!(budget.accepted_body_bytes(), 24 * 1024 * 1024);
}

#[tokio::test]
async fn controlled_deadline_spans_body_retry_wait_and_final_acceptance() {
    let listener = TlsListener::serve_router(loopback(), 0, match_cert(), |path| match path {
        "/slow" => FixtureResponse::Stream {
            declared: Some(2),
            len: 2,
            chunk: 1,
            delay: Duration::from_secs(2),
        },
        "/wait" => FixtureResponse::Response {
            status: "503 Service Unavailable",
            headers: vec![
                ("Retry-After".to_owned(), "1".to_owned()),
                ("X-RateLimit-Reset".to_owned(), "1720000000".to_owned()),
            ],
            body: b"busy".to_vec(),
        },
        _ => FixtureResponse::Body("ok".to_owned()),
    })
    .await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let limits = resourcefs_core::ReadAcquisitionLimits::new(
        None,
        Some(Duration::from_millis(250)),
        None,
        None,
        None,
        None,
    )
    .expect("limits");
    for path in ["/slow", "/wait"] {
        let (read, effective) = substrate
            .begin_read_with_limits(&operation, &limits)
            .expect("read");
        let mut budget = HttpReadBudget::with_limits(&effective);
        let started = tokio::time::Instant::now();
        let error = read
            .fetch_with_budget(HttpRequest::get(url(&listener, path)), &mut budget)
            .await
            .expect_err("shared deadline");
        assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
        let details = error.details().expect("deadline facts");
        assert_eq!(
            details.reason(),
            resourcefs_core::ErrorReason::DeadlineExceeded
        );
        let limit = details.limit().expect("elapsed limit");
        assert_eq!(
            limit.kind(),
            resourcefs_core::AcquisitionLimitKind::ElapsedNanoseconds
        );
        assert_eq!(limit.bound(), 250_000_000);
        if path == "/wait" {
            assert_eq!(details.http_status().expect("upstream status").get(), 503);
            assert_eq!(
                details.retry_guidance(),
                Some(resourcefs_core::RetryGuidance::DelaySeconds(1))
            );
            assert_eq!(details.rate_limit_reset(), Some(1_720_000_000));
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(budget.used_attempts(), 1);
    }
    let (read, effective) = substrate
        .begin_read_with_limits(&operation, &limits)
        .expect("read");
    let mut budget = HttpReadBudget::with_limits(&effective);
    assert_eq!(
        read.fetch_with_budget(HttpRequest::get(url(&listener, "/control")), &mut budget)
            .await
            .expect("control")
            .body(),
        b"ok"
    );
    read.check_acceptance().expect("on-time acceptance");
    tokio::time::sleep(Duration::from_millis(260)).await;
    assert_eq!(
        read.check_acceptance()
            .expect_err("late result")
            .details()
            .expect("details")
            .reason(),
        resourcefs_core::ErrorReason::DeadlineExceeded
    );
    let late_fetch = read
        .fetch_with_budget(
            HttpRequest::get(url(&listener, "/after-deadline")),
            &mut budget,
        )
        .await
        .expect_err("a later fetch must not restart the logical deadline");
    assert_eq!(
        late_fetch.details().expect("late-fetch details").reason(),
        resourcefs_core::ErrorReason::DeadlineExceeded
    );
    assert_eq!(budget.used_attempts(), 1);
    let (read, _) = substrate
        .begin_read_with_limits(&operation, &limits)
        .expect("read");
    assert!(operation.cancel());
    assert_eq!(
        read.check_acceptance()
            .expect_err("cancelled acceptance")
            .category(),
        ErrorCategory::Cancelled
    );
    assert_eq!(
        read.check_acceptance()
            .expect_err("cancelled")
            .details()
            .expect("details")
            .reason(),
        resourcefs_core::ErrorReason::Cancelled
    );
    assert_eq!(
        listener.requests(),
        [
            "GET /slow HTTP/1.1",
            "GET /wait HTTP/1.1",
            "GET /control HTTP/1.1"
        ]
    );
}

#[tokio::test]
async fn substrate_ceilings_lower_effective_limits_and_send_uses_that_deadline() {
    let listener = TlsListener::serve(loopback(), 0, match_cert(), "four").await;
    let ceilings = HttpCeilings::new(resourcefs_core::HttpCeilingsInput {
        fetch_bytes: Some(3),
        timeout_millis: Some(200),
        redirect_depth: None,
    })
    .expect("substrate ceilings");
    let substrate = HttpSubstrate::with_host_lookup_and_roots(
        fixture_allowlist(listener.address.port(), true),
        ceilings,
        |_| async { Ok(vec![loopback()]) },
        &[fixture_ca()],
        Vec::new(),
    )
    .expect("substrate");
    let operation = OperationGuard::new();
    let limits =
        resourcefs_core::ReadAcquisitionLimits::new(None, None, None, None, None, Some(123))
            .expect("caller decoded cap");
    let (read, effective) = substrate
        .begin_read_with_limits(&operation, &limits)
        .expect("read");
    assert_eq!(effective.max_response_bytes(), 3);
    assert_eq!(effective.max_decoded_bytes(), 123);
    assert_eq!(effective.timeout(), Duration::from_millis(200));
    let mut budget = HttpReadBudget::with_limits(&effective);
    let response = read
        .fetch_with_budget(HttpRequest::get(url(&listener, "/body")), &mut budget)
        .await
        .expect("bounded response");
    assert_eq!(response.body(), b"fou");
    assert!(response.truncated());
    assert!(budget.admit_body(4).is_err());
    let delayed = HttpSubstrate::with_host_lookup_and_roots(
        fixture_allowlist(listener.address.port(), true),
        HttpCeilings::default(),
        |_| async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(vec![loopback()])
        },
        &[fixture_ca()],
        Vec::new(),
    )
    .expect("slow resolver substrate");
    let (read, effective) = delayed
        .begin_read_with_limits(&operation, &effective)
        .expect("read");
    let mut budget = HttpReadBudget::with_limits(&effective);
    let started = tokio::time::Instant::now();
    let error = read
        .fetch_with_budget(HttpRequest::get(url(&listener, "/send")), &mut budget)
        .await
        .expect_err("send stage shares effective deadline");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(listener.requests(), ["GET /body HTTP/1.1"]);
}

#[tokio::test]
async fn controlled_deadline_interrupts_delayed_headers_with_reachable_control() {
    // The fixture yields before sending any headers, so the same runtime can
    // drive the client deadline. Its physical request log precedes the delay.
    let listener = TlsListener::serve_router(loopback(), 0, match_cert(), |_| {
        FixtureResponse::DelayedBody {
            delay: Duration::from_millis(500),
            body: "eventual".to_owned(),
        }
    })
    .await;
    let substrate = substrate(&listener);
    let operation = OperationGuard::new();
    let control = resourcefs_core::ReadAcquisitionLimits::new(
        None,
        Some(Duration::from_secs(3)),
        None,
        None,
        None,
        None,
    )
    .expect("control limits");
    let (read, effective) = substrate
        .begin_read_with_limits(&operation, &control)
        .expect("read");
    let mut budget = HttpReadBudget::with_limits(&effective);
    assert_eq!(
        read.fetch_with_budget(HttpRequest::get(url(&listener, "/control")), &mut budget)
            .await
            .expect("same delayed headers succeed")
            .body(),
        b"eventual"
    );
    let limits = resourcefs_core::ReadAcquisitionLimits::new(
        None,
        Some(Duration::from_millis(150)),
        None,
        None,
        None,
        None,
    )
    .expect("lower timeout");
    let (read, effective) = substrate
        .begin_read_with_limits(&operation, &limits)
        .expect("read");
    let mut budget = HttpReadBudget::with_limits(&effective);
    let started = tokio::time::Instant::now();
    let error = read
        .fetch_with_budget(HttpRequest::get(url(&listener, "/headers")), &mut budget)
        .await
        .expect_err("headers exceed deadline");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert_eq!(
        error.details().expect("deadline facts").reason(),
        resourcefs_core::ErrorReason::DeadlineExceeded
    );
    assert!(started.elapsed() < Duration::from_millis(450));
    assert_eq!(
        listener.requests(),
        ["GET /control HTTP/1.1", "GET /headers HTTP/1.1"]
    );
    assert_eq!(budget.used_attempts(), 1);
}
