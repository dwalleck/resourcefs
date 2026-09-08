#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;

use resourcefs_core::{
    AllowedOrigin, ErrorCategory, ErrorReason, HttpCeilings, OperationGuard, PathReference,
    ReadAcquisitionLimits, Secret, SourceAdapter, SourceResource,
};
use resourcefs_sources::{
    GithubConfig, GithubDeployment, GithubRepository, GithubSource, GithubSourceMount,
    HttpSubstrate, MutationGrants, OriginCredential, SecretReference,
};
use serde_json::{Value, json};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tls::{FixtureResponse, TlsListener};

const NATIVE: &str = include_str!("../../../.rfs-0n97/oracles/pr-native.json");
const RESOURCE: &str = "pr://owner/repo/7/facts";

// Same system-allocation accounting as filesystem_resource_limits_contract.
// Charge 32 extra bytes per live allocation as conservative allocator overhead;
// this measures incremental heap ownership, not process RSS or allocator arenas.
struct FactsCountingAllocator;
static FACTS_CURRENT_HEAP: AtomicUsize = AtomicUsize::new(0);
static FACTS_PEAK_HEAP: AtomicUsize = AtomicUsize::new(0);
const ALLOCATION_OVERHEAD: usize = 32;
#[global_allocator]
static FACTS_ALLOCATOR: FactsCountingAllocator = FactsCountingAllocator;

fn record_facts_allocation(bytes: usize) {
    let current = FACTS_CURRENT_HEAP.fetch_add(bytes, Ordering::AcqRel) + bytes;
    FACTS_PEAK_HEAP.fetch_max(current, Ordering::AcqRel);
}

unsafe impl GlobalAlloc for FactsCountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forward the exact layout to the system allocator.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_facts_allocation(layout.size() + ALLOCATION_OVERHEAD);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: this pointer and layout originated in the delegated allocator.
        unsafe { System.dealloc(pointer, layout) };
        FACTS_CURRENT_HEAP.fetch_sub(layout.size() + ALLOCATION_OVERHEAD, Ordering::AcqRel);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forward the original allocation and requested replacement size.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() {
            if new_size >= layout.size() {
                record_facts_allocation(new_size - layout.size());
            } else {
                FACTS_CURRENT_HEAP.fetch_sub(layout.size() - new_size, Ordering::AcqRel);
            }
        }
        replacement
    }
}
fn response(status: &'static str, body: String, headers: Vec<(String, String)>) -> FixtureResponse {
    FixtureResponse::Response {
        status,
        headers,
        body: body.into_bytes(),
    }
}
async fn fixture<F>(transform: F, operator: ReadAcquisitionLimits) -> (TlsListener, GithubSource)
where
    F: Fn(usize, &str, &str) -> FixtureResponse + Send + Sync + 'static,
{
    let (listener, source, _session) = fixture_with_session(transform, operator).await;
    (listener, source)
}
async fn fixture_with_session<F>(
    transform: F,
    operator: ReadAcquisitionLimits,
) -> (TlsListener, GithubSource, session_support::ScratchFixture)
where
    F: Fn(usize, &str, &str) -> FixtureResponse + Send + Sync + 'static,
{
    let count = AtomicUsize::new(0);
    let listener = TlsListener::serve_request_router(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        tls::match_cert(),
        move |request| {
            assert_eq!(request.method(), "GET", "facts cannot write");
            assert_eq!(
                request.target(),
                "/repos/owner/repo/pulls/7",
                "no link/fork egress"
            );
            let host = request
                .head()
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
                        .map(|(_, value)| value.trim())
                })
                .expect("Host header");
            let native = NATIVE.replace("@API@", &format!("https://{host}/"));
            transform(
                count.fetch_add(1, Ordering::SeqCst),
                &native,
                request.head(),
            )
        },
    )
    .await;
    let port = listener.address.port();
    let api = format!("https://{}:{port}/", tls::FIXTURE_HOST);
    let origin = AllowedOrigin::new(&api, true).expect("origin");
    let secret = Secret::new("facts-secret-sentinel".to_owned()).expect("secret");
    let credential = OriginCredential::new(origin, "Authorization", Some("Bearer"), &secret)
        .expect("credential");
    let substrate = HttpSubstrate::with_host_lookup_and_roots(
        tls::fixture_allowlist(port, true),
        HttpCeilings::default(),
        |_| async { Ok::<_, std::io::Error>(vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]) },
        &[tls::fixture_ca()],
        vec![credential],
    )
    .expect("TLS substrate");
    let config = GithubConfig::new(
        "github-facts",
        false,
        MutationGrants::default(),
        GithubDeployment::new(Some(api), Some("https://github.example".into()))
            .expect("deployment"),
        true,
        SecretReference::environment("UNUSED_FIXTURE_TOKEN").expect("reference"),
        vec![GithubRepository::new("owner/repo", MutationGrants::default()).expect("repository")],
        operator,
    )
    .expect("config");
    let session = session_support::scratch_fixture().await;
    let source = GithubSourceMount::new(config, Arc::new(substrate))
        .bind(session.path_session().clone())
        .expect("source");
    (listener, source, session)
}
async fn read(
    source: &GithubSource,
    limits: Option<&ReadAcquisitionLimits>,
) -> Result<SourceResource, resourcefs_core::ResourceError> {
    source
        .read(
            &PathReference::parse(RESOURCE).expect("facts reference"),
            &OperationGuard::new(),
            limits,
        )
        .await
}
fn document(resource: &SourceResource) -> Value {
    serde_json::from_str(resource.content()).expect("complete JSON")
}
fn limits(
    response: Option<usize>,
    accepted: Option<usize>,
    representation: Option<usize>,
) -> ReadAcquisitionLimits {
    ReadAcquisitionLimits::new(None, None, response, accepted, representation)
        .expect("valid limits")
}

#[tokio::test]
async fn facts_native_identity_consumer_and_observed_request() {
    let (listener, source) = fixture(
        |_, native, _| {
            response(
                "200 OK",
                native.into(),
                vec![
                    ("ETag".into(), "\"original\"".into()),
                    ("Date".into(), "Tue, 01 Jan 2019 00:00:00 GMT".into()),
                ],
            )
        },
        ReadAcquisitionLimits::default(),
    )
    .await;
    let resource = read(&source, None).await.expect("facts");
    assert!(resource.continuation().is_none());
    let dir = tempfile::tempdir().expect("oracle directory");
    let native = NATIVE.replace(
        "@API@",
        &format!("https://{}:{}/", tls::FIXTURE_HOST, listener.address.port()),
    );
    std::fs::write(dir.path().join("native.json"), &native).expect("native input");
    std::fs::write(dir.path().join("facts.json"), resource.content()).expect("production output");
    let observations = json!({"upstream.body.status":200,"upstream.body.etag":"\"original\"","acquisition.usage.attemptedRequests":1,"acquisition.usage.acceptedBodyBytes":native.len()});
    std::fs::write(
        dir.path().join("observations.json"),
        observations.to_string(),
    )
    .expect("observations");
    let status = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.rfs-0n97/oracles/facts_consumer.py"
        ))
        .arg(dir.path().join("facts.json"))
        .arg(dir.path().join("native.json"))
        .arg(dir.path().join("observations.json"))
        .status()
        .expect("independent consumer");
    assert!(
        status.success(),
        "C05/C06 production consumer rejected output"
    );
    assert_eq!(
        listener.requests(),
        ["GET /repos/owner/repo/pulls/7 HTTP/1.1"]
    );
    let head = listener.heads()[0].to_ascii_lowercase();
    for header in [
        "authorization: bearer facts-secret-sentinel",
        "accept: application/vnd.github+json",
        "x-github-api-version: 2022-11-28",
    ] {
        assert!(head.contains(header), "missing {header}");
    }
    assert!(!resource.content().contains("facts-secret-sentinel"));
}

#[tokio::test]
async fn facts_required_identity_contradictions_and_duplicates_are_rejected() {
    let cases = [
        ("/id", json!(0)),
        ("/id", Value::Null),
        ("/id", json!("9007199254740993")),
        ("/number", json!(8)),
        ("/number", json!(0)),
        (
            "/url",
            json!("https://foreign.invalid/repos/owner/repo/pulls/7"),
        ),
        ("/base/sha", json!("111")),
        (
            "/head/sha",
            json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        ),
        (
            "/head/sha",
            json!("gggggggggggggggggggggggggggggggggggggggg"),
        ),
        ("/base/repo/full_name", json!("other/repo")),
        ("/base/repo/name", json!("wrong")),
        ("/base/repo/owner/login", json!("wrong")),
    ];
    for (pointer, replacement) in cases {
        let (listener, source) = fixture(
            move |_, native, _| {
                let mut value: Value = serde_json::from_str(native).unwrap();
                *value.pointer_mut(pointer).unwrap() = replacement.clone();
                response("200 OK", value.to_string(), vec![])
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        let error = read(&source, None).await.expect_err(pointer);
        assert!(
            matches!(
                error.details().expect("typed identity error").reason(),
                ErrorReason::UpstreamMalformed | ErrorReason::UpstreamIdentityMismatch
            ),
            "{pointer}: {error:?}"
        );
        assert_eq!(listener.requests().len(), 1);
    }
    for field in ["id", "number", "url", "base", "head"] {
        let (listener, source) = fixture(
            move |_, native, _| {
                let mut value: Value = serde_json::from_str(native).unwrap();
                value.as_object_mut().unwrap().remove(field);
                response("200 OK", value.to_string(), vec![])
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        assert!(read(&source, None).await.is_err(), "missing {field}");
        assert_eq!(listener.requests().len(), 1);
    }
    for duplicate in [
        "\"id\":1,",
        "\"number\":7,",
        "\"url\":\"secret-sentinel\",",
        "\"_links\":{\"self\":{},\"self\":{}},",
        "\"html_url\":null,",
    ] {
        let (listener, source) = fixture(
            move |_, native, _| {
                response(
                    "200 OK",
                    native.replacen('{', &format!("{{{duplicate}"), 1),
                    vec![],
                )
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        assert!(read(&source, None).await.is_err(), "duplicate {duplicate}");
        assert_eq!(listener.requests().len(), 1);
    }
}

#[tokio::test]
async fn facts_independent_optional_presence_type_matrix() {
    let fields = [
        ("body", "body"),
        ("title", "title"),
        ("state", "state"),
        ("node_id", "nodeId"),
        ("user", "author"),
        ("created_at", "createdAt"),
        ("updated_at", "updatedAt"),
        ("closed_at", "closedAt"),
        ("merged_at", "mergedAt"),
        ("draft", "draft"),
        ("merged", "merged"),
    ];
    for (native_key, owned_key) in fields {
        for mode in 0..4 {
            let (listener, source) = fixture(
                move |_, native, _| {
                    let mut value: Value = serde_json::from_str(native).unwrap();
                    match mode {
                        0 => {
                            value.as_object_mut().unwrap().remove(native_key);
                        }
                        1 => value[native_key] = Value::Null,
                        2 => {}
                        _ => value[native_key] = json!(["wrong-type-secret"]),
                    }
                    response("200 OK", value.to_string(), vec![])
                },
                ReadAcquisitionLimits::default(),
            )
            .await;
            let result = read(&source, None).await;
            if mode == 3 {
                assert!(result.is_err(), "{native_key} wrong type");
            } else {
                let output = document(&result.expect(native_key));
                assert_eq!(
                    output["data"].get(owned_key).is_some(),
                    mode != 0,
                    "{native_key}"
                );
                if mode == 1 {
                    assert!(output["data"][owned_key].is_null(), "{native_key}");
                }
                if mode < 2 {
                    assert!(
                        output["unavailableFacts"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|entry| entry["field"] == owned_key
                                && entry["reason"] == if mode == 0 { "omitted" } else { "null" }),
                        "{native_key} knowledge"
                    );
                }
            }
            assert_eq!(listener.requests().len(), 1);
        }
    }
    for side in ["base", "head"] {
        for field in ["ref", "repo"] {
            for mode in 0..4 {
                let (listener, source) = fixture(
                    move |_, native, _| {
                        let mut value: Value = serde_json::from_str(native).unwrap();
                        match mode {
                            0 => {
                                value[side].as_object_mut().unwrap().remove(field);
                            }
                            1 => value[side][field] = Value::Null,
                            2 => {}
                            _ => value[side][field] = json!(["wrong"]),
                        };
                        response("200 OK", value.to_string(), vec![])
                    },
                    ReadAcquisitionLimits::default(),
                )
                .await;
                let result = read(&source, None).await;
                if mode == 3 {
                    assert!(result.is_err(), "{side}.{field}");
                } else {
                    let output = document(&result.expect("optional branch"));
                    let owned = if field == "ref" {
                        "refName"
                    } else {
                        "repository"
                    };
                    assert_eq!(output["data"][side].get(owned).is_some(), mode != 0);
                    if mode == 1 {
                        assert!(output["data"][side][owned].is_null());
                    }
                    if field == "repo" {
                        assert_eq!(
                            output["data"][side]["repositoryAvailability"],
                            ["omitted", "null", "present"][mode]
                        );
                    }
                }
                assert_eq!(listener.requests().len(), 1, "no fork reads");
            }
        }
    }
}

#[tokio::test]
async fn facts_cache_provenance_changed_body_and_failed_revalidation() {
    let (listener, source) = fixture(
        |index, native, head| {
            if index == 1 || index == 2 {
                assert!(
                    head.to_ascii_lowercase()
                        .contains("if-none-match: \"body-one\"")
                );
            }
            if index == 3 {
                assert!(
                    head.to_ascii_lowercase()
                        .contains("if-none-match: \"body-two\"")
                );
            }
            match index {
                0 => response(
                    "200 OK",
                    native.into(),
                    vec![
                        ("ETag".into(), "\"body-one\"".into()),
                        ("Date".into(), "Tue, 01 Jan 2019 00:00:00 GMT".into()),
                    ],
                ),
                1 => response(
                    "304 Not Modified",
                    String::new(),
                    vec![("Date".into(), "Wed, 01 Jan 2020 00:00:00 GMT".into())],
                ),
                2 => response(
                    "200 OK",
                    native.replace("Independent title", "Changed title"),
                    vec![("ETag".into(), "\"body-two\"".into())],
                ),
                _ => response("403 Forbidden", "private-provider-secret".into(), vec![]),
            }
        },
        ReadAcquisitionLimits::default(),
    )
    .await;
    let first = read(&source, None).await.expect("fresh");
    let second = read(&source, None).await.expect("304");
    let a = document(&first);
    let b = document(&second);
    assert_eq!(a["data"], b["data"]);
    assert_eq!(a["upstream"]["body"], b["upstream"]["body"]);
    assert_eq!(b["upstream"]["revalidation"]["status"], 304);
    assert_eq!(
        b["upstream"]["revalidation"]["date"],
        "Wed, 01 Jan 2020 00:00:00 GMT"
    );
    assert_ne!(first.version_tag(), second.version_tag());
    let changed = document(&read(&source, None).await.expect("changed"));
    assert_eq!(changed["data"]["title"], "Changed title λ");
    assert_eq!(changed["upstream"]["body"]["etag"], "\"body-two\"");
    let error = read(&source, None).await.expect_err("no stale fallback");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    assert!(!format!("{error:?}").contains("private-provider-secret"));
    assert_eq!(listener.requests().len(), 4);
}

#[tokio::test]
async fn facts_body_and_cached_readmission_exact_plus_one() {
    for cached in [false, true] {
        for accepted in [false, true] {
            for plus_one in [false, true] {
                let (listener, source) = fixture(
                    move |index, native, _| {
                        if cached && index > 0 {
                            response("304 Not Modified", String::new(), vec![])
                        } else {
                            response(
                                "200 OK",
                                native.into(),
                                vec![("ETag".into(), "\"cache\"".into())],
                            )
                        }
                    },
                    ReadAcquisitionLimits::default(),
                )
                .await;
                if cached {
                    read(&source, None).await.expect("warm cache");
                }
                let bytes = NATIVE
                    .replace(
                        "@API@",
                        &format!("https://{}:{}/", tls::FIXTURE_HOST, listener.address.port()),
                    )
                    .len();
                let cap = bytes - usize::from(plus_one);
                let control = if accepted {
                    limits(None, Some(cap), None)
                } else {
                    limits(Some(cap), None, None)
                };
                let result = read(&source, Some(&control)).await;
                if plus_one {
                    let error = result.expect_err("plus-one admission");
                    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
                } else {
                    assert_eq!(
                        document(&result.expect("exact admission"))["acquisition"]["usage"]["acceptedBodyBytes"],
                        bytes
                    );
                }
                assert_eq!(listener.requests().len(), if cached { 2 } else { 1 });
            }
        }
    }
}

#[tokio::test]
async fn facts_representation_includes_escaping_and_envelope() {
    let body = "\u{0001}\"\\雪\n".repeat(300);
    let expected = body.clone();
    let (listener, source) = fixture(
        move |_, native, _| {
            let mut value: Value = serde_json::from_str(native).unwrap();
            value["body"] = json!(body);
            response("200 OK", value.to_string(), vec![])
        },
        ReadAcquisitionLimits::default(),
    )
    .await;
    let full = read(&source, None).await.expect("unrestricted");
    assert_eq!(document(&full)["data"]["body"], expected);
    let cap = limits(None, None, Some(expected.len()));
    let error = read(&source, Some(&cap))
        .await
        .expect_err("unescaped data length cannot admit full envelope");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test]
async fn facts_errors_are_status_derived_and_sanitized() {
    for (status, headers, category, reason) in [
        (
            "401 Unauthorized",
            vec![],
            ErrorCategory::PermissionDenied,
            ErrorReason::UpstreamDenied,
        ),
        (
            "403 Forbidden",
            vec![],
            ErrorCategory::PermissionDenied,
            ErrorReason::UpstreamDenied,
        ),
        (
            "403 Forbidden",
            vec![("X-RateLimit-Remaining".into(), "0".into())],
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamRateLimited,
        ),
        (
            "404 Not Found",
            vec![],
            ErrorCategory::NotFound,
            ErrorReason::UpstreamNotFoundOrHidden,
        ),
        (
            "429 Too Many Requests",
            vec![],
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamRateLimited,
        ),
        (
            "500 Internal Server Error",
            vec![],
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamUnavailable,
        ),
        (
            "503 Service Unavailable",
            vec![],
            ErrorCategory::SourceUnavailable,
            ErrorReason::UpstreamUnavailable,
        ),
    ] {
        let (listener, source) = fixture(
            move |_, _, _| {
                response(
                    status,
                    "token-secret: expired rate limit not found".into(),
                    headers.clone(),
                )
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        let one = ReadAcquisitionLimits::new(Some(1), None, None, None, None).unwrap();
        let error = read(&source, Some(&one)).await.expect_err(status);
        assert_eq!(error.category(), category, "{status}");
        let details = error.details().expect("machine details");
        assert_eq!(details.reason(), reason, "{status}");
        if status.starts_with("404") {
            assert!(details.access_ambiguity().is_some());
        }
        assert!(!format!("{error:?}").contains("token-secret"));
        assert_eq!(listener.requests().len(), 1);
    }
}

#[tokio::test]
async fn facts_denial_and_precancel_have_no_egress_with_positive_control() {
    let (listener, source) = fixture(
        |_, native, _| response("200 OK", native.into(), vec![]),
        ReadAcquisitionLimits::default(),
    )
    .await;
    read(&source, None)
        .await
        .expect("authorized positive control");
    let denied = source
        .read(
            &PathReference::parse("pr://other/repo/7/facts").unwrap(),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect_err("repository denied");
    assert_eq!(denied.category(), ErrorCategory::PermissionDenied);
    let operation = OperationGuard::new();
    operation.cancel();
    assert_eq!(
        source
            .read(&PathReference::parse(RESOURCE).unwrap(), &operation, None)
            .await
            .expect_err("cancelled")
            .category(),
        ErrorCategory::Cancelled
    );
    assert_eq!(listener.requests().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn facts_inflight_cancel_and_deadline_refuse_barrier_delayed_response() {
    for cancel in [false, true] {
        let arrived = Arc::new(tokio::sync::Notify::new());
        let notify = Arc::clone(&arrived);
        let (release, blocked) = std::sync::mpsc::channel();
        let blocked = std::sync::Mutex::new(blocked);
        let (listener, source) = fixture(
            move |index, native, _| {
                if index == 0 {
                    notify.notify_one();
                    match blocked.lock() {
                        Ok(receiver) => receiver
                            .recv_timeout(Duration::from_secs(5))
                            .expect("release barrier"),
                        Err(poisoned) => panic!("response barrier poisoned: {poisoned}"),
                    }
                }
                response("200 OK", native.into(), vec![])
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        let source = Arc::new(source);
        let reading = Arc::clone(&source);
        let operation = OperationGuard::new();
        let worker_operation = operation.clone();
        let task = tokio::spawn(async move {
            let controls = ReadAcquisitionLimits::new(
                None,
                if cancel {
                    None
                } else {
                    Some(Duration::from_millis(100))
                },
                None,
                None,
                None,
            )
            .unwrap();
            reading
                .read(
                    &PathReference::parse(RESOURCE).unwrap(),
                    &worker_operation,
                    Some(&controls),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), arrived.notified())
            .await
            .expect("actual HTTP request arrived");
        if cancel {
            operation.cancel();
        }
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("bounded response")
            .expect("reader task");
        release.send(()).expect("release response");
        let error = result.expect_err("late response cannot publish");
        assert_eq!(
            error.category(),
            if cancel {
                ErrorCategory::Cancelled
            } else {
                ErrorCategory::SourceUnavailable
            }
        );
        assert_eq!(
            error.details().expect("typed refusal").reason(),
            if cancel {
                ErrorReason::Cancelled
            } else {
                ErrorReason::DeadlineExceeded
            }
        );
        assert_eq!(listener.requests().len(), 1);
        read(&source, None)
            .await
            .expect("same source remains usable after independent failed read");
        assert_eq!(listener.requests().len(), 2);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn facts_inflight_generation_change_refuses_publication() {
    for invalidate in [false, true] {
        let arrived = Arc::new(tokio::sync::Notify::new());
        let notify = Arc::clone(&arrived);
        let (release, blocked) = std::sync::mpsc::channel();
        let blocked = std::sync::Mutex::new(blocked);
        let (listener, source, session_fixture) = fixture_with_session(
            move |index, native, _| {
                if index == 0 {
                    notify.notify_one();
                    blocked
                        .lock()
                        .expect("response barrier")
                        .recv_timeout(Duration::from_secs(5))
                        .expect("release barrier");
                }
                response(
                    "200 OK",
                    native.into(),
                    vec![("ETag".into(), "\"body\"".into())],
                )
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        let source = Arc::new(source);
        let reading = Arc::clone(&source);
        let task = tokio::spawn(async move { read(&reading, None).await });
        tokio::time::timeout(Duration::from_secs(3), arrived.notified())
            .await
            .expect("actual request arrived");
        if invalidate {
            session_fixture
                .path_session()
                .cache_remove_namespace("github-http")
                .await
                .expect("source namespace invalidation");
        }
        release.send(()).expect("release response");
        let result = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .expect("bounded source read")
            .expect("reader task");
        if invalidate {
            let error = result.expect_err("invalidated observation cannot publish");
            assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
            assert_eq!(
                error.details().expect("typed refusal").reason(),
                ErrorReason::UpstreamUnavailable
            );
        } else {
            assert_eq!(
                document(&result.expect("unchanged generation"))["data"]["id"],
                "9007199254740993"
            );
        }
        read(&source, None)
            .await
            .expect("fresh generation remains readable");
        assert_eq!(listener.requests().len(), 2);
    }
}

#[tokio::test]
async fn facts_operator_intersection_is_observed_for_each_independent_dimension() {
    let operator = ReadAcquisitionLimits::new(
        Some(2),
        Some(Duration::from_secs(4)),
        Some(20000),
        Some(30000),
        Some(40000),
    )
    .unwrap();
    for (caller, expected) in [
        (
            ReadAcquisitionLimits::new(
                Some(1),
                Some(Duration::from_secs(5)),
                Some(19000),
                Some(31000),
                Some(39000),
            )
            .unwrap(),
            [1.0, 4000.0, 19000.0, 30000.0, 39000.0],
        ),
        (
            ReadAcquisitionLimits::new(
                Some(3),
                Some(Duration::from_secs(3)),
                Some(21000),
                Some(29000),
                Some(41000),
            )
            .unwrap(),
            [2.0, 3000.0, 20000.0, 29000.0, 40000.0],
        ),
        (
            ReadAcquisitionLimits::new(
                Some(2),
                Some(Duration::from_secs(4)),
                Some(20000),
                Some(30000),
                Some(40000),
            )
            .unwrap(),
            [2.0, 4000.0, 20000.0, 30000.0, 40000.0],
        ),
    ] {
        let (listener, source) = fixture(
            |_, native, _| response("200 OK", native.into(), vec![]),
            operator,
        )
        .await;
        let facts = document(&read(&source, Some(&caller)).await.expect("lower-only read"));
        for (field, minimum) in [
            "maxAttempts",
            "timeoutMs",
            "maxResponseBytes",
            "maxAcceptedBodyBytes",
            "maxRepresentationBytes",
        ]
        .into_iter()
        .zip(expected)
        {
            assert_eq!(
                facts["acquisition"]["limits"][field]
                    .as_f64()
                    .expect("numeric limit"),
                minimum,
                "{field}"
            );
        }
        assert_eq!(listener.requests().len(), 1);
    }
    let (listener, source) = fixture(
        |_, native, _| response("200 OK", native.into(), vec![]),
        limits(Some(1), None, None),
    )
    .await;
    assert_eq!(
        read(&source, Some(&ReadAcquisitionLimits::default()))
            .await
            .expect_err("operator body ceiling")
            .category(),
        ErrorCategory::LimitExceeded
    );
    assert_eq!(listener.requests().len(), 1);
}

#[tokio::test]
async fn facts_link_and_nested_native_optional_matrix() {
    let paths = [
        "/html_url",
        "/diff_url",
        "/patch_url",
        "/issue_url",
        "/_links",
        "/user/id",
        "/user/node_id",
        "/user/login",
        "/user/url",
        "/user/html_url",
        "/base/repo/id",
        "/base/repo/node_id",
        "/base/repo/name",
        "/base/repo/full_name",
        "/base/repo/owner",
        "/base/repo/url",
        "/base/repo/html_url",
        "/head/repo/id",
        "/head/repo/node_id",
        "/head/repo/name",
        "/head/repo/full_name",
        "/head/repo/owner",
        "/head/repo/url",
        "/head/repo/html_url",
        "/base/repo/owner/id",
        "/base/repo/owner/login",
        "/head/repo/owner/id",
        "/head/repo/owner/login",
    ];
    for path in paths {
        for mode in 0..4 {
            let (listener, source) = fixture(
                move |_, native, _| {
                    let mut value: Value = serde_json::from_str(native).unwrap();
                    let (parent, key) = path.rsplit_once('/').unwrap();
                    let object = if parent.is_empty() {
                        &mut value
                    } else {
                        value.pointer_mut(parent).unwrap()
                    };
                    match mode {
                        0 => {
                            object.as_object_mut().unwrap().remove(key);
                        }
                        1 => object[key] = Value::Null,
                        2 => {}
                        _ => object[key] = json!(["invalid-type"]),
                    }
                    response("200 OK", value.to_string(), vec![])
                },
                ReadAcquisitionLimits::default(),
            )
            .await;
            let result = read(&source, None).await;
            if mode == 3 {
                assert!(result.is_err(), "{path} wrong type");
            } else {
                let output = document(&result.expect(path));
                let owned = path
                    .replace("/user", "/author")
                    .replace("/repo/", "/repository/")
                    .replace("/node_id", "/nodeId")
                    .replace("/full_name", "/fullName")
                    .replace("/html_url", "/links/htmlUrl")
                    .replace("/diff_url", "/links/diffUrl")
                    .replace("/patch_url", "/links/patchUrl")
                    .replace("/issue_url", "/links/issueUrl")
                    .replace("/_links", "/links/relations")
                    .replace("/url", "/links/apiUrl");
                let pointer = format!("/data{owned}");
                assert_eq!(output.pointer(&pointer).is_some(), mode != 0, "{path}");
                if mode == 1 {
                    assert!(output.pointer(&pointer).unwrap().is_null(), "{path}");
                }
            }
            assert_eq!(listener.requests().len(), 1);
        }
    }
}

#[tokio::test]
async fn facts_retry_attempts_and_redirects_are_observed_not_inferred() {
    for attempts in [1, 2] {
        let (listener, source) = fixture(
            |index, native, _| {
                if index == 0 {
                    response(
                        "503 Service Unavailable",
                        "secret-retry-body".into(),
                        vec![("Retry-After".into(), "0".into())],
                    )
                } else {
                    response("200 OK", native.into(), vec![])
                }
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        let control = ReadAcquisitionLimits::new(Some(attempts), None, None, None, None).unwrap();
        let result = read(&source, Some(&control)).await;
        if attempts == 1 {
            assert!(result.is_err());
        } else {
            assert_eq!(
                document(&result.expect("permitted retry"))["acquisition"]["usage"]["attemptedRequests"],
                2
            );
        }
        assert_eq!(listener.requests().len(), attempts);
    }
    for location in [
        "/repos/owner/repo/pulls/8",
        "https://forbidden.invalid/token-leak",
    ] {
        let (listener, source) = fixture(
            move |_, _, _| FixtureResponse::Redirect(location.into()),
            ReadAcquisitionLimits::default(),
        )
        .await;
        assert!(
            read(&source, None).await.is_err(),
            "redirect cannot grant authority"
        );
        assert_eq!(listener.requests().len(), 1);
    }
}

#[tokio::test]
async fn facts_malformed_bodies_and_uncached_304_never_publish() {
    for body in [
        vec![],
        b"{malformed-secret".to_vec(),
        vec![0xff, 0xfe],
        b"{\"id\":18446744073709551616}".to_vec(),
    ] {
        let (listener, source) = fixture(
            move |_, _, _| FixtureResponse::Response {
                status: "200 OK",
                headers: vec![],
                body: body.clone(),
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        let error = read(&source, None)
            .await
            .expect_err("malformed native body");
        assert_eq!(
            error.details().unwrap().reason(),
            ErrorReason::UpstreamMalformed
        );
        assert!(!format!("{error:?}").contains("malformed-secret"));
        assert_eq!(listener.requests().len(), 1);
    }
    let (listener, source) = fixture(
        |_, _, _| response("304 Not Modified", String::new(), vec![]),
        ReadAcquisitionLimits::default(),
    )
    .await;
    assert!(
        read(&source, None).await.is_err(),
        "304 without cache cannot identify a PR"
    );
    assert_eq!(listener.requests().len(), 1);
}

#[tokio::test]
async fn facts_legacy_cache_does_not_fabricate_original_observation() {
    let (listener, source, fixture_session) = fixture_with_session(
        |index, native, head| {
            if index == 0 {
                response(
                    "200 OK",
                    native.into(),
                    vec![("ETag".into(), "\"legacy\"".into())],
                )
            } else {
                assert!(
                    head.to_ascii_lowercase()
                        .contains("if-none-match: \"legacy\"")
                );
                response(
                    "304 Not Modified",
                    String::new(),
                    vec![("Date".into(), "Wed, 01 Jan 2020 00:00:00 GMT".into())],
                )
            }
        },
        ReadAcquisitionLimits::default(),
    )
    .await;
    source
        .read(
            &PathReference::parse("pr://owner/repo/7/title").unwrap(),
            &OperationGuard::new(),
            None,
        )
        .await
        .expect("legacy cache warm");
    let session = fixture_session.path_session();
    let key = resourcefs_core::SessionCacheKey::new(
        "github-http",
        format!(
            "application/vnd.github+json\nhttps://{}:{}/repos/owner/repo/pulls/7",
            tls::FIXTURE_HOST,
            listener.address.port()
        ),
    )
    .expect("existing media/URL cache key");
    let original = session
        .cache_get(&key)
        .await
        .expect("cache access")
        .expect("warm response retained");
    let native = NATIVE.replace(
        "@API@",
        &format!("https://{}:{}/", tls::FIXTURE_HOST, listener.address.port()),
    );
    assert_eq!(
        original.content(),
        native.as_bytes(),
        "retained native body"
    );
    let original_metadata: Value =
        serde_json::from_slice(original.metadata()).expect("warm metadata");
    assert_eq!(original_metadata["etag"], "\"legacy\"");
    // Recreate the historical metadata shape, not a freshly observed title read.
    // Cache identity, content and validator remain exactly those actually acquired.
    let historical_metadata = serde_json::to_vec(
        &json!({"etag":original_metadata["etag"], "link":original_metadata["link"]}),
    )
    .expect("historical etag/link metadata");
    let historical =
        resourcefs_core::SessionCacheEntry::new(historical_metadata, original.content().to_vec())
            .expect("historical cache entry");
    let generation = session
        .cache_generation("github-http")
        .await
        .expect("cache generation");
    assert!(
        session
            .cache_put_if_generation(key, historical, generation)
            .await
            .expect("replace metadata without changing generation")
    );
    let facts = document(&read(&source, None).await.expect("legacy body revalidated"));
    assert!(facts["upstream"]["body"].get("status").is_none());
    assert!(facts["upstream"]["body"].get("observedAtUnixMs").is_none());
    assert_eq!(facts["upstream"]["revalidation"]["status"], 304);
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test]
async fn facts_duplicate_nested_identity_and_relation_keys_are_not_last_value_wins() {
    for (needle, replacement) in [
        (
            "\"id\": 9007199254740997",
            "\"id\":1,\"id\":9007199254740997",
        ),
        (
            "\"sha\": \"2222222222222222222222222222222222222222\"",
            "\"sha\":\"1111111111111111111111111111111111111111\",\"sha\":\"2222222222222222222222222222222222222222\"",
        ),
        ("\"self\": {\"href\":", "\"self\":{},\"self\":{\"href\":"),
        (
            "\"href\": \"https://github.example/Owner/Repo/pull/7\"",
            "\"href\":\"https://secret.invalid\",\"href\":\"https://github.example/Owner/Repo/pull/7\"",
        ),
    ] {
        let (listener, source) = fixture(
            move |_, native, _| {
                assert!(native.contains(needle), "fixture mutation must apply");
                response("200 OK", native.replacen(needle, replacement, 1), vec![])
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        assert!(read(&source, None).await.is_err(), "duplicate {needle}");
        assert_eq!(listener.requests().len(), 1);
    }
}

#[tokio::test]
async fn facts_recognized_optional_parent_links_reject_contradictions_but_opaque_links_survive() {
    for path in [
        "/html_url",
        "/diff_url",
        "/patch_url",
        "/issue_url",
        "/_links/self/href",
        "/_links/html/href",
    ] {
        for bad_parent in [false, true] {
            let (listener, source) = fixture(
                move |_, native, _| {
                    let mut value: Value = serde_json::from_str(native).unwrap();
                    let link = value.pointer_mut(path).expect("fixture link");
                    let original = link.as_str().unwrap();
                    *link = json!(if bad_parent {
                        original.replace("/7", "/8")
                    } else {
                        "opaque-provider-observation".to_owned()
                    });
                    response("200 OK", value.to_string(), vec![])
                },
                ReadAcquisitionLimits::default(),
            )
            .await;
            let result = read(&source, None).await;
            if bad_parent {
                assert_eq!(
                    result.expect_err(path).details().unwrap().reason(),
                    ErrorReason::UpstreamIdentityMismatch
                );
            } else {
                assert!(
                    result
                        .expect("opaque link")
                        .content()
                        .contains("opaque-provider-observation")
                );
            }
            assert_eq!(listener.requests().len(), 1);
        }
    }
    for relation in ["issue", "comments", "review_comments"] {
        let (listener, source) = fixture(
            move |_, native, _| {
                let mut value: Value = serde_json::from_str(native).unwrap();
                let parent = value["url"].as_str().unwrap();
                let wrong = match relation {
                    "issue" => parent.replace("/pulls/7", "/issues/8"),
                    "comments" => format!("{}/comments", parent.replace("/pulls/7", "/issues/8")),
                    _ => format!("{}/comments", parent.replace("/pulls/7", "/pulls/8")),
                };
                value["_links"][relation] = json!({"href":wrong});
                response("200 OK", value.to_string(), vec![])
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        assert_eq!(
            read(&source, None)
                .await
                .expect_err(relation)
                .details()
                .unwrap()
                .reason(),
            ErrorReason::UpstreamIdentityMismatch
        );
        assert_eq!(listener.requests().len(), 1);
    }
    let (listener, source) = fixture(
        |_, native, _| {
            let mut value: Value = serde_json::from_str(native).unwrap();
            for key in ["html_url", "diff_url", "patch_url", "issue_url"] {
                value[key] = json!("");
            }
            value["_links"]["future"] = json!({"href":"https://opaque.invalid/{template}"});
            response("200 OK", value.to_string(), vec![])
        },
        ReadAcquisitionLimits::default(),
    )
    .await;
    let facts = document(&read(&source, None).await.expect("empty and future links"));
    for key in ["htmlUrl", "diffUrl", "patchUrl", "issueUrl"] {
        assert_eq!(facts["data"]["links"][key], "");
    }
    assert_eq!(
        facts["data"]["links"]["relations"]["future"]["href"],
        "https://opaque.invalid/{template}"
    );
    assert_eq!(listener.requests().len(), 1);
}

#[tokio::test]
async fn facts_native_objects_reject_empty_arrays() {
    for path in [
        "/user",
        "/base",
        "/head",
        "/base/repo",
        "/head/repo",
        "/base/repo/owner",
        "/head/repo/owner",
        "/_links",
        "/_links/self",
        "/_links/html",
    ] {
        let (listener, source) = fixture(
            move |_, native, _| {
                let mut value: Value = serde_json::from_str(native).unwrap();
                *value.pointer_mut(path).expect("native object") = json!([]);
                response("200 OK", value.to_string(), vec![])
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        let error = read(&source, None).await.expect_err(path);
        assert_eq!(
            error.details().expect("typed malformed error").reason(),
            ErrorReason::UpstreamMalformed,
            "{path}"
        );
        assert_eq!(listener.requests().len(), 1);
    }
}

#[tokio::test]
async fn facts_fork_links_validate_own_identity_without_fetching_fork() {
    for (path, wrong) in [
        ("/head/repo/url", "@API@repos/owner/repo"),
        ("/head/repo/url", "https://foreign.invalid/repos/Other/Fork"),
        ("/head/repo/html_url", "https://github.example/Owner/Repo"),
        ("/head/repo/html_url", "https://foreign.invalid/Other/Fork"),
    ] {
        let (listener, source) = fixture(
            move |_, native, _| {
                let mut value: Value = serde_json::from_str(native).unwrap();
                let api = value["url"]
                    .as_str()
                    .unwrap()
                    .strip_suffix("repos/owner/repo/pulls/7")
                    .unwrap()
                    .to_owned();
                *value.pointer_mut(path).unwrap() = json!(wrong.replace("@API@", &api));
                response("200 OK", value.to_string(), vec![])
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        let error = read(&source, None).await.expect_err(path);
        assert_eq!(
            error.details().unwrap().reason(),
            ErrorReason::UpstreamIdentityMismatch
        );
        assert_eq!(listener.requests().len(), 1);
    }
    let (listener, source) = fixture(
        |_, native, _| response("200 OK", native.into(), vec![]),
        ReadAcquisitionLimits::default(),
    )
    .await;
    let facts = document(&read(&source, None).await.expect("valid distinct fork"));
    assert_eq!(
        facts["data"]["head"]["repository"]["fullName"],
        "Other/Fork"
    );
    assert_eq!(
        facts["data"]["head"]["repository"]["id"],
        "9007199254740999"
    );
    assert_eq!(
        listener.requests().len(),
        1,
        "fork metadata never grants a second GET"
    );
}

#[tokio::test]
async fn facts_repository_links_preserve_templates_opaque_and_empty_values() {
    for side in ["base", "head"] {
        for native_key in ["url", "html_url"] {
            for supplied in [
                "",
                "opaque-native-link",
                "https://foreign.invalid/repos/{owner}/{repo}",
                "https://foreign.invalid/{owner}/{repo}",
            ] {
                let (listener, source) = fixture(
                    move |_, native, _| {
                        let mut value: Value = serde_json::from_str(native).unwrap();
                        value[side]["repo"][native_key] = json!(supplied);
                        response("200 OK", value.to_string(), vec![])
                    },
                    ReadAcquisitionLimits::default(),
                )
                .await;
                let facts = document(&read(&source, None).await.expect("inert repository link"));
                let owned_key = if native_key == "url" {
                    "apiUrl"
                } else {
                    "htmlUrl"
                };
                assert_eq!(
                    facts["data"][side]["repository"]["links"][owned_key],
                    supplied
                );
                assert_eq!(listener.requests().len(), 1);
            }
        }
    }
}

#[tokio::test]
#[ignore = "checkpointed-build production-scale budget"]
async fn github_facts_production_budget() -> Result<(), &'static str> {
    if cfg!(debug_assertions) {
        return Err("run this budget in release mode");
    }
    const NATIVE_BYTES: usize = 8 * 1024 * 1024;
    const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
    const HEAP_LIMIT: usize = 96 * 1024 * 1024;

    // Two independent cold reads: wall time first, incremental heap second.
    // --exact and --test-threads=1 isolate the process-wide allocator census.
    for measure_heap in [false, true] {
        let body = Arc::new(OnceLock::<String>::new());
        let served = Arc::clone(&body);
        let (listener, source) = fixture(
            move |_, _, _| {
                response(
                    "200 OK",
                    served.get().expect("prepared native body").clone(),
                    vec![("ETag".into(), "\"production-budget\"".into())],
                )
            },
            ReadAcquisitionLimits::default(),
        )
        .await;
        // Independent native fixture, never derived from the production encoder.
        // All construction and JSON oracle work is outside measured regions.
        let native = NATIVE.replace(
            "@API@",
            &format!("https://{}:{}/", tls::FIXTURE_HOST, listener.address.port()),
        );
        let mut value: Value = serde_json::from_str(&native).expect("native oracle");
        let mut payload = "é\n\"\\".repeat(256 * 1024);
        value["body"] = json!(&payload);
        let encoded_len = serde_json::to_vec(&value).expect("fixture size").len();
        assert!(encoded_len < NATIVE_BYTES);
        payload.extend(std::iter::repeat_n('x', NATIVE_BYTES - encoded_len));
        value["body"] = json!(&payload);
        let encoded = serde_json::to_string(&value).expect("native fixture");
        assert_eq!(encoded.len(), NATIVE_BYTES, "independent wire byte count");
        body.set(encoded).expect("one prepared body");
        drop(value);
        drop(native);
        let reference = PathReference::parse(RESOURCE).expect("facts reference");
        let operation = OperationGuard::new();

        let baseline = FACTS_CURRENT_HEAP.load(Ordering::Acquire);
        FACTS_PEAK_HEAP.store(baseline, Ordering::Release);
        let started = Instant::now();
        let result = source.read(&reference, &operation, None).await;
        let elapsed = started.elapsed();
        let incremental_heap = FACTS_PEAK_HEAP
            .load(Ordering::Acquire)
            .saturating_sub(baseline);

        let resource = result.expect("production-size Facts through public adapter");
        let output_bytes = resource.content().len();
        assert!(output_bytes >= payload.len(), "complete owned payload");
        assert!(output_bytes <= OUTPUT_LIMIT, "owned document byte ceiling");
        assert!(
            resource.continuation().is_none(),
            "complete source document"
        );
        let facts = document(&resource);
        assert_eq!(facts["data"]["body"].as_str(), Some(payload.as_str()));
        assert_eq!(
            facts["acquisition"]["usage"]["acceptedBodyBytes"],
            NATIVE_BYTES
        );
        assert_eq!(listener.requests().len(), 1, "one cold native GET");
        if measure_heap {
            // Includes server response clone, TLS/HTTP buffers, native decode,
            // cache admission and owned output; excludes prepared fixtures and
            // post-read JSON assertions. Fixture overhead makes this conservative.
            println!(
                "facts_budget phase=heap native_bytes={NATIVE_BYTES} output_bytes={output_bytes} incremental_heap_bytes={incremental_heap} per_allocation_overhead_bytes={ALLOCATION_OVERHEAD} heap_limit_bytes={HEAP_LIMIT}"
            );
            assert!(
                incremental_heap <= HEAP_LIMIT,
                "incremental heap {incremental_heap}"
            );
        } else {
            // Full cold loopback read is a conservative UPPER BOUND on local
            // decode/validate/project/serialize work, not a decode-only timer.
            // Includes TLS/network/cache and allocator instrumentation; no
            // synthetic server delay or fixture construction is timed/subtracted.
            println!(
                "facts_budget phase=wall native_bytes={NATIVE_BYTES} output_bytes={output_bytes} cold_loopback_upper_bound_ns={} local_processing_limit_ns=1000000000",
                elapsed.as_nanos()
            );
            assert!(
                elapsed <= Duration::from_secs(1),
                "local processing upper bound {elapsed:?}"
            );
        }
    }
    Ok(())
}
