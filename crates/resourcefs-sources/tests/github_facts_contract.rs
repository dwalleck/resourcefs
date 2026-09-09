#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;

use resourcefs_core::{
    AllowedOrigin, ErrorCategory, ErrorReason, HttpCeilings, OperationGuard, PathReference,
    ReadAcquisitionLimits, ReadRequest, Secret, SourceAdapter, SourceResource, TextLimits,
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
        atomic::{AtomicBool, AtomicUsize, Ordering},
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

fn host_from_head(head: &str) -> &str {
    head.lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
                .map(|(_, value)| value.trim())
        })
        .expect("Host header")
}
async fn fixture<F>(
    transform: F,
    operator: ReadAcquisitionLimits,
) -> (TlsListener, GithubSource, session_support::ScratchFixture)
where
    F: Fn(usize, &str, &str) -> FixtureResponse + Send + Sync + 'static,
{
    let (listener, source, session) = fixture_with_session(transform, operator).await;
    (listener, source, session)
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
            let host = host_from_head(request.head());
            let native = NATIVE.replace("@API@", &format!("https://{host}/"));
            transform(
                count.fetch_add(1, Ordering::SeqCst),
                &native,
                request.head(),
            )
        },
    )
    .await;
    let (source, session) = build_source(&listener, operator).await;
    (listener, source, session)
}
async fn build_source(
    listener: &TlsListener,
    operator: ReadAcquisitionLimits,
) -> (GithubSource, session_support::ScratchFixture) {
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
    (source, session)
}
async fn read(
    source: &GithubSource,
    limits: Option<&ReadAcquisitionLimits>,
) -> Result<SourceResource, resourcefs_core::ResourceError> {
    read_reference(source, RESOURCE, limits).await
}
async fn read_reference(
    source: &GithubSource,
    reference: &str,
    limits: Option<&ReadAcquisitionLimits>,
) -> Result<SourceResource, resourcefs_core::ResourceError> {
    source
        .read(
            &PathReference::parse(reference)?,
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
async fn session_scratch_retain_and_read_engine_recover_artifact_while_owner_lives() {
    let fixture = session_support::scratch_fixture().await;
    let operation = OperationGuard::new();
    let content = "session scratch artifact bytes\n";
    let address = fixture
        .session
        .path_session()
        .retain(content, &operation)
        .await
        .expect("retain scratch artifact");
    let reference = PathReference::artifact(address, None).expect("artifact reference");
    let recovered = fixture
        .reads
        .read(
            ReadRequest {
                reference,
                limits: TextLimits::default(),
                numbered: false,
                acquisition: None,
            },
            &operation,
        )
        .await
        .expect("read retained artifact through ReadEngine");
    assert_eq!(recovered.content(), content);
    assert!(recovered.recovery_reference().is_none());
    assert!(recovered.continuation_reference().is_none());
}

#[tokio::test]
async fn facts_native_identity_consumer_and_observed_request() {
    let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
            let (listener, source, _session) = fixture(
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
                let (listener, source, _session) = fixture(
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
    let (listener, source, _session) = fixture(
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
                let (listener, source, _session) = fixture(
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
    let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
    let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
    let (listener, source, _session) = fixture(
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
            let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
            let error = result.expect_err("parent does not retry beyond the attempt bound");
            assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
            let details = error.details().expect("typed retry refusal");
            assert_eq!(details.reason(), ErrorReason::UpstreamUnavailable);
            assert_eq!(
                details.retry_guidance(),
                Some(resourcefs_core::RetryGuidance::DelaySeconds(0))
            );
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
        let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
    let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
            let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
    let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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
    let (listener, source, _session) = fixture(
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
                let (listener, source, _session) = fixture(
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
        let (listener, source, _session) = fixture(
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

const COMMENT_NATIVE: &str = include_str!("../../../.rfs-bfwa/oracles/comment-native.json");
const COMMENT_RESOURCE: &str = "pr://owner/repo/7/comments/9001/facts";

/// Routes the parent pull request and one conversation comment, so the two
/// requests of a comment read can be observed independently.
async fn comment_fixture<F>(
    transform: F,
) -> (TlsListener, GithubSource, session_support::ScratchFixture)
where
    F: Fn(&str, usize, &str) -> FixtureResponse + Send + Sync + 'static,
{
    let count = AtomicUsize::new(0);
    let listener = TlsListener::serve_request_router(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        tls::match_cert(),
        move |request| {
            assert_eq!(request.method(), "GET", "facts cannot write");
            let host = host_from_head(request.head());
            let target = request.target().to_owned();
            let template = if target.starts_with("/repos/owner/repo/pulls/") {
                NATIVE
            } else if target.starts_with("/repos/owner/repo/issues/comments/") {
                COMMENT_NATIVE
            } else {
                panic!("unexpected conversation-comment facts request {target}");
            };
            let native = template
                .replace("@API@", &format!("https://{host}/"))
                .replace(
                    "https://github.example/Owner/Repo/pull/7",
                    "https://github.example/owner/repo/pull/7",
                );
            transform(&target, count.fetch_add(1, Ordering::SeqCst), &native)
        },
    )
    .await;
    let (source, session) = build_source(&listener, ReadAcquisitionLimits::default()).await;
    (listener, source, session)
}

#[tokio::test]
async fn single_comment_facts_shape() {
    let (listener, source, _session) =
        comment_fixture(|_, _, native| response("200 OK", native.into(), vec![])).await;
    let resource = read_reference(&source, COMMENT_RESOURCE, None)
        .await
        .expect("single comment facts");
    assert!(resource.continuation().is_none());
    let facts = document(&resource);
    assert_eq!(facts["kind"], "github.conversation_comment");
    assert_eq!(facts["resource"], COMMENT_RESOURCE);
    assert!(
        facts.get("collection").is_none(),
        "singular reads carry no collection object"
    );
    assert_eq!(facts["data"]["kind"], "github.conversation_comment");
    assert_eq!(facts["data"]["id"], "9001");
    assert_eq!(facts["data"]["nodeId"], "IC_native");
    assert_eq!(facts["data"]["parent"]["kind"], "github.pull_request");
    assert_eq!(facts["data"]["parent"]["id"], "9007199254740993");
    assert_eq!(facts["data"]["parent"]["number"], "7");
    assert_eq!(facts["data"]["parent"]["nodeId"], "PR_native");
    assert_eq!(
        facts["data"]["body"],
        "Conversation quote \" slash \\ newline\n雪"
    );
    assert_eq!(facts["data"]["author"]["login"], "Commenter");
    assert_eq!(facts["data"]["createdAt"], "2004-04-04T00:00:00Z");
    assert_eq!(facts["data"]["updatedAt"], "2005-05-05T00:00:00Z");
    assert_eq!(
        facts["data"]["links"]["issueUrl"],
        format!(
            "https://{}:{}/repos/owner/repo/issues/7",
            tls::FIXTURE_HOST,
            listener.address.port()
        )
    );
    assert_eq!(facts["acquisition"]["usage"]["attemptedRequests"], 2);
    assert_eq!(facts["repository"]["observed"]["name"], "Repo");

    // A cursor belongs to the collection only; native page and line selectors
    // parse but the adapter refuses them.
    assert_eq!(
        PathReference::parse(format!("{COMMENT_RESOURCE}:cursor:e30"))
            .expect_err("cursor on a singular comment")
            .category(),
        ErrorCategory::InvalidReference
    );
    for selector in [":raw", ":page:2", ":1-2"] {
        let error = read_reference(&source, &format!("{COMMENT_RESOURCE}{selector}"), None)
            .await
            .expect_err("selector refused");
        assert_eq!(
            error.category(),
            ErrorCategory::UnsupportedProjection,
            "{selector}"
        );
    }
    assert_eq!(listener.requests().len(), 2, "refusals acquire nothing");
}

#[tokio::test]
async fn comment_records_preserve_native_presence() {
    let cases = [
        ("/body", json!(null), "null"),
        ("/user", json!(null), "null"),
        ("/node_id", json!(null), "null"),
        ("/body", json!(""), "empty"),
        ("/body", json!("snow 雪 \"quoted\" \\ back"), "unicode"),
    ];
    for (pointer, replacement, label) in cases {
        let supplied = replacement.clone();
        let (_, source, _session) = comment_fixture(move |target, _, native| {
            if !target.starts_with("/repos/owner/repo/issues/comments/") {
                return response("200 OK", native.into(), vec![]);
            }
            let mut value: Value = serde_json::from_str(native).unwrap();
            *value.pointer_mut(pointer).unwrap() = supplied.clone();
            response("200 OK", value.to_string(), vec![])
        })
        .await;
        let facts = document(
            &read_reference(&source, COMMENT_RESOURCE, None)
                .await
                .expect(label),
        );
        let field = pointer.trim_start_matches('/');
        let output = match field {
            "user" => "author",
            "node_id" => "nodeId",
            "created_at" => "createdAt",
            other => other,
        };
        match replacement {
            Value::Null => {
                // A supplied null stays present as null; it is not the same as
                // an unsupplied optional field.
                let slot = facts["data"]
                    .get(output)
                    .unwrap_or_else(|| panic!("{pointer} {label} present in {facts}"));
                assert!(slot.is_null(), "{label} supplied null stays null");
            }
            _ => assert_eq!(
                facts["data"][output], replacement,
                "{label} supplied value survives"
            ),
        }
    }

    // Absent optional fields stay absent and are named in unavailableFacts.
    for (pointer, output, unavailable) in [
        ("/body", "body", "body"),
        ("/user", "author", "author"),
        ("/node_id", "nodeId", "nodeId"),
        ("/created_at", "createdAt", "createdAt"),
        ("/url", "links/apiUrl", "links.apiUrl"),
    ] {
        let (_, source, _session) = comment_fixture(move |target, _, native| {
            if !target.starts_with("/repos/owner/repo/issues/comments/") {
                return response("200 OK", native.into(), vec![]);
            }
            let mut value: Value = serde_json::from_str(native).unwrap();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            let object = if parent.is_empty() {
                &mut value
            } else {
                value.pointer_mut(parent).unwrap()
            };
            object.as_object_mut().unwrap().remove(key);
            response("200 OK", value.to_string(), vec![])
        })
        .await;
        let facts = document(
            &read_reference(&source, COMMENT_RESOURCE, None)
                .await
                .expect(pointer),
        );
        let pointer_path = format!("/data/{}", output.replace('.', "/"));
        assert!(
            facts.pointer(&pointer_path).is_none(),
            "{pointer} absent stays absent"
        );
        assert!(
            facts["unavailableFacts"]
                .as_array()
                .expect("unavailable facts")
                .iter()
                .any(|entry| entry["field"] == unavailable && entry["reason"] == "omitted"),
            "{pointer} named unavailable"
        );
    }

    // Unknown native keys are ignored; a matching id above 2^53 stays exact.
    let large_id = 9_007_199_254_740_993_u64;
    let large_reference = format!("pr://owner/repo/7/comments/{large_id}/facts");
    let (_, source, _session) = comment_fixture(move |target, _, native| {
        if !target.starts_with("/repos/owner/repo/issues/comments/") {
            return response("200 OK", native.into(), vec![]);
        }
        let mut value: Value = serde_json::from_str(native).unwrap();
        value["future_native_field"] = json!({"nested": true});
        value["id"] = json!(large_id);
        value["url"] = json!(format!(
            "{}/{large_id}",
            value["url"]
                .as_str()
                .expect("comment api URL")
                .rsplit_once('/')
                .expect("comment api URL segment")
                .0
        ));
        value["html_url"] = json!(format!(
            "https://github.example/owner/repo/pull/7#issuecomment-{large_id}"
        ));
        response("200 OK", value.to_string(), vec![])
    })
    .await;
    let facts = document(
        &read_reference(&source, &large_reference, None)
            .await
            .expect("unknown key and matching large id"),
    );
    assert_eq!(facts["data"]["id"], "9007199254740993");
    assert_eq!(facts["request"]["commentId"], "9007199254740993");
    assert!(facts["data"].get("future_native_field").is_none());

    // A positive provider id that differs from the addressed id is not accepted.
    let (_, source, _session) = comment_fixture(move |target, _, native| {
        if !target.starts_with("/repos/owner/repo/issues/comments/") {
            return response("200 OK", native.into(), vec![]);
        }
        let mut value: Value = serde_json::from_str(native).unwrap();
        value["id"] = json!(9002_u64);
        value["url"] = json!(format!(
            "{}/9002",
            value["url"]
                .as_str()
                .expect("comment api URL")
                .rsplit_once('/')
                .expect("comment api URL segment")
                .0
        ));
        value["html_url"] = json!("https://github.example/owner/repo/pull/7#issuecomment-9002");
        response("200 OK", value.to_string(), vec![])
    })
    .await;
    let error = read_reference(&source, COMMENT_RESOURCE, None)
        .await
        .expect_err("wrong observed comment id");
    assert_eq!(
        error.details().expect("typed identity refusal").reason(),
        ErrorReason::UpstreamIdentityMismatch
    );

    // Duplicate recognized keys and non-object records are rejected.
    for body in [
        r#"{"id":1,"id":2,"issue_url":"@API@repos/owner/repo/issues/7"}"#.to_owned(),
        r#"[{"id":1}]"#.to_owned(),
    ] {
        let (_, source, _session) = comment_fixture(move |target, _, native| {
            if target.starts_with("/repos/owner/repo/issues/comments/") {
                return response("200 OK", body.clone(), vec![]);
            }
            response("200 OK", native.into(), vec![])
        })
        .await;
        let error = read_reference(&source, COMMENT_RESOURCE, None)
            .await
            .expect_err("malformed comment record");
        assert_eq!(
            error.details().expect("typed").reason(),
            ErrorReason::UpstreamMalformed
        );
    }
}

#[tokio::test]
async fn wrong_parent_rejects_whole_component() {
    // The comment names another issue/PR in the same repository.
    let (_, source, _session) = comment_fixture(|target, _, native| {
        if !target.starts_with("/repos/owner/repo/issues/comments/") {
            return response("200 OK", native.into(), vec![]);
        }
        let mut value: Value = serde_json::from_str(native).unwrap();
        let issue_url = value["issue_url"]
            .as_str()
            .unwrap()
            .replace("/issues/7", "/issues/8");
        value["issue_url"] = json!(issue_url);
        response("200 OK", value.to_string(), vec![])
    })
    .await;
    assert_eq!(
        read_reference(&source, COMMENT_RESOURCE, None)
            .await
            .expect_err("wrong parent")
            .category(),
        ErrorCategory::NotFound
    );

    // The comment names a parent in another repository.
    let (_, source, _session) = comment_fixture(|target, _, native| {
        if !target.starts_with("/repos/owner/repo/issues/comments/") {
            return response("200 OK", native.into(), vec![]);
        }
        let mut value: Value = serde_json::from_str(native).unwrap();
        let issue_url = value["issue_url"]
            .as_str()
            .unwrap()
            .replace("/repos/owner/repo/issues/7", "/repos/other/repo/issues/7");
        value["issue_url"] = json!(issue_url);
        response("200 OK", value.to_string(), vec![])
    })
    .await;
    assert_eq!(
        read_reference(&source, COMMENT_RESOURCE, None)
            .await
            .expect_err("foreign parent")
            .category(),
        ErrorCategory::NotFound
    );

    // The addressed number is not the pull request the comment belongs to.
    let (_, source, _session) =
        comment_fixture(|_, _, native| response("200 OK", native.into(), vec![])).await;
    let error = read_reference(&source, "pr://owner/repo/8/comments/9001/facts", None)
        .await
        .expect_err("parent identity mismatch");
    assert!(matches!(
        error.details().expect("typed").reason(),
        ErrorReason::UpstreamIdentityMismatch | ErrorReason::UpstreamMalformed
    ));

    // An issue number is not a pull request: the parent read is not found.
    let (_, source, _session) = comment_fixture(|target, _, native| {
        if target.starts_with("/repos/owner/repo/pulls/") {
            return response("404 Not Found", "{}".into(), vec![]);
        }
        response("200 OK", native.into(), vec![])
    })
    .await;
    assert_eq!(
        read_reference(&source, COMMENT_RESOURCE, None)
            .await
            .expect_err("issue number")
            .category(),
        ErrorCategory::NotFound
    );
}

#[tokio::test]
async fn facts_identity_corpus_has_explicit_verdicts_not_human_parity() {
    // Facts intentionally has its own verdict table: unlike a human adapter's
    // later-page policy, these singular link contradictions are always hard
    // refusals, while empty actors remain losslessly acceptable.
    for (field, replacement) in [
        (
            "html_url",
            "https://foreign.invalid/owner/repo/pull/7#issuecomment-9001",
        ),
        ("html_url", "file:///tmp/comment"),
        (
            "html_url",
            "https://user:pass@github.example/owner/repo/pull/7#issuecomment-9001",
        ),
        (
            "html_url",
            "https://github.example/owner/repo/issues/8#issuecomment-9001",
        ),
        (
            "html_url",
            "https://github.example/owner/repo/issues/7#issuecomment-9002",
        ),
    ] {
        let (listener, source, _session) = comment_fixture(move |target, _, native| {
            if !target.starts_with("/repos/owner/repo/issues/comments/") {
                return response("200 OK", native.into(), vec![]);
            }
            let mut value: Value = serde_json::from_str(native).unwrap();
            value[field] = json!(replacement);
            response("200 OK", value.to_string(), vec![])
        })
        .await;
        assert!(
            read_reference(&source, COMMENT_RESOURCE, None)
                .await
                .is_err(),
            "facts singular rejects {field}={replacement}"
        );
        assert_eq!(listener.requests().len(), 2);
    }
    // GitHub's issue-shaped web URL is an accepted spelling of the same
    // conversation comment identity.
    let (_, source, _session) = comment_fixture(|target, _, native| {
        if !target.starts_with("/repos/owner/repo/issues/comments/") {
            return response("200 OK", native.into(), vec![]);
        }
        let mut value: Value = serde_json::from_str(native).unwrap();
        value["html_url"] = json!("https://github.example/owner/repo/issues/7#issuecomment-9001");
        response("200 OK", value.to_string(), vec![])
    })
    .await;
    let facts = document(
        &read_reference(&source, COMMENT_RESOURCE, None)
            .await
            .expect("issue-shaped web URL remains accepted"),
    );
    assert_eq!(
        facts["data"]["links"]["htmlUrl"],
        "https://github.example/owner/repo/issues/7#issuecomment-9001"
    );

    for kind in ["off-origin", "file", "userinfo", "self-id"] {
        let (listener, source, _session) = comment_fixture(move |target, _, native| {
            if !target.starts_with("/repos/owner/repo/issues/comments/") {
                return response("200 OK", native.into(), vec![]);
            }
            let mut value: Value = serde_json::from_str(native).unwrap();
            let link = match kind {
                "off-origin" => {
                    "https://foreign.invalid/repos/owner/repo/issues/comments/9001".to_owned()
                }
                "file" => "file:///tmp/comment".to_owned(),
                "userinfo" => {
                    let current = value["url"].as_str().expect("api URL");
                    format!(
                        "https://user:pass@{}",
                        current.trim_start_matches("https://")
                    )
                }
                _ => {
                    let current = value["url"].as_str().expect("api URL");
                    format!(
                        "{}/9002",
                        current.rsplit_once('/').expect("comment api URL segment").0
                    )
                }
            };
            value["url"] = json!(link);
            response("200 OK", value.to_string(), vec![])
        })
        .await;
        assert!(
            read_reference(&source, COMMENT_RESOURCE, None)
                .await
                .is_err(),
            "facts singular rejects {kind} API link"
        );
        assert_eq!(listener.requests().len(), 2);
    }

    // Empty actor values are an intentional facts distinction, not a human
    // adapter parity requirement.
    let (_, source, _session) = comment_fixture(|target, _, native| {
        if !target.starts_with("/repos/owner/repo/issues/comments/") {
            return response("200 OK", native.into(), vec![]);
        }
        let mut value: Value = serde_json::from_str(native).unwrap();
        value["user"]["login"] = json!("");
        response("200 OK", value.to_string(), vec![])
    })
    .await;
    let facts = document(
        &read_reference(&source, COMMENT_RESOURCE, None)
            .await
            .expect("empty actor login remains observable"),
    );
    assert_eq!(facts["data"]["author"]["login"], "");

    // The same identity corpus on a later collection page rejects the whole
    // component; ordinary malformed records use the retained-prefix fence.
    for (field, replacement) in [
        (
            "html_url",
            "https://foreign.invalid/owner/repo/pull/7#issuecomment-101",
        ),
        ("html_url", "file:///tmp/comment"),
        (
            "html_url",
            "https://user:pass@github.example/owner/repo/pull/7#issuecomment-101",
        ),
        (
            "html_url",
            "https://github.example/owner/repo/pull/7#issuecomment-102",
        ),
        (
            "html_url",
            "https://github.example/owner/repo/issues/8#issuecomment-101",
        ),
        (
            "html_url",
            "https://github.example/owner/repo/issues/7#issuecomment-102",
        ),
    ] {
        let (_, source, _session) = collection_fixture(move |target, api, _| {
            if target.contains("page=2") {
                let mut page = vec![comment_json(101, api)];
                page[0][field] = json!(replacement);
                return response(
                    "200 OK",
                    serde_json::to_string(&page).expect("identity page"),
                    vec![],
                );
            }
            let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
            comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
        })
        .await;
        assert!(
            read_reference(&source, COLLECTION_RESOURCE, None)
                .await
                .is_err(),
            "facts later-page identity contradiction rejects {field}={replacement}"
        );
    }
    let (_, source, _session) = collection_fixture(|target, api, _| {
        if target.contains("page=2") {
            let mut page = vec![comment_json(101, api)];
            page[0]["html_url"] =
                json!("https://github.example/owner/repo/issues/7#issuecomment-101");
            return response(
                "200 OK",
                serde_json::to_string(&page).expect("accepted issue-shaped page"),
                vec![],
            );
        }
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
    })
    .await;
    let facts = document(
        &read_reference(&source, COLLECTION_RESOURCE, None)
            .await
            .expect("issue-shaped later comment URL remains accepted"),
    );
    assert_eq!(
        facts["data"]["records"].as_array().expect("records").len(),
        101
    );
    assert_eq!(
        facts["data"]["records"][100]["links"]["htmlUrl"],
        "https://github.example/owner/repo/issues/7#issuecomment-101"
    );

    for kind in ["off-origin", "file", "userinfo", "self-id"] {
        let (listener, source, _session) = collection_fixture(move |target, api, _| {
            if target.contains("page=2") {
                let mut page = vec![comment_json(101, api)];
                let current = page[0]["url"].as_str().expect("api URL").to_owned();
                page[0]["url"] = json!(match kind {
                    "off-origin" => {
                        "https://foreign.invalid/repos/owner/repo/issues/comments/101".to_owned()
                    }
                    "file" => "file:///tmp/comment".to_owned(),
                    "userinfo" => format!(
                        "https://user:pass@{}",
                        current.trim_start_matches("https://")
                    ),
                    _ => format!(
                        "{}/102",
                        current.rsplit_once('/').expect("comment api URL segment").0
                    ),
                });
                return response(
                    "200 OK",
                    serde_json::to_string(&page).expect("identity API page"),
                    vec![],
                );
            }
            let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
            comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
        })
        .await;
        assert!(
            read_reference(&source, COLLECTION_RESOURCE, None)
                .await
                .is_err(),
            "facts later-page API contradiction rejects {kind}"
        );
        assert_eq!(listener.requests().len(), 3);
    }
    // A later native Link containing userinfo is an authority violation: it
    // must not be softened into a retained prefix.
    let (listener, source, _session) = collection_fixture(|target, api, _| {
        if target.contains("page=2") {
            let hostile = format!(
                "https://user:pass@{}repos/owner/repo/issues/7/comments?per_page=100&page=3",
                api.trim_start_matches("https://")
            );
            let page = vec![comment_json(101, api)];
            return response(
                "200 OK",
                serde_json::to_string(&page).expect("hostile Link page"),
                vec![("Link".into(), format!("<{hostile}>; rel=\"next\""))],
            );
        }
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
    })
    .await;
    let error = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect_err("userinfo authority violation");
    assert_eq!(error.category(), ErrorCategory::PermissionDenied);
    assert_eq!(listener.requests().len(), 3, "whole-read fence");
}
const COLLECTION_RESOURCE: &str = "pr://owner/repo/7/comments/facts";

fn comment_json(id: u64, api: &str) -> Value {
    json!({
        "id": id,
        "node_id": format!("IC_{id}"),
        "url": format!("{api}repos/owner/repo/issues/comments/{id}"),
        "html_url": format!("https://github.example/owner/repo/pull/7#issuecomment-{id}"),
        "issue_url": format!("{api}repos/owner/repo/issues/7"),
        "body": format!("comment {id}"),
        "user": {
            "id": 500 + id,
            "node_id": "U_commenter",
            "login": "commenter",
            "url": format!("{api}users/commenter"),
            "html_url": "https://github.example/commenter"
        },
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-02T00:00:00Z"
    })
}

fn comment_page(ids: &[u64], api: &str, next: Option<&str>) -> FixtureResponse {
    let records: Vec<Value> = ids.iter().map(|id| comment_json(*id, api)).collect();
    let mut headers = Vec::new();
    if let Some(next) = next {
        headers.push(("Link".to_string(), format!("<{next}>; rel=\"next\"")));
    }
    response(
        "200 OK",
        serde_json::to_string(&records).expect("page JSON"),
        headers,
    )
}

/// Routes the parent pull request and the conversation-comment collection,
/// handing the transform the API base so it can build native pages.
async fn collection_fixture<F>(
    transform: F,
) -> (TlsListener, GithubSource, session_support::ScratchFixture)
where
    F: Fn(&str, &str, usize) -> FixtureResponse + Send + Sync + 'static,
{
    let count = AtomicUsize::new(0);
    let listener = TlsListener::serve_request_router(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        tls::match_cert(),
        move |request| {
            assert_eq!(request.method(), "GET", "facts cannot write");
            let host = host_from_head(request.head());
            let api = format!("https://{host}/");
            let target = request.target().to_owned();
            if target.starts_with("/repos/owner/repo/pulls/") {
                return response("200 OK", NATIVE.replace("@API@", &api), vec![]);
            }
            transform(&target, &api, count.fetch_add(1, Ordering::SeqCst))
        },
    )
    .await;
    let (source, session) = build_source(&listener, ReadAcquisitionLimits::default()).await;
    (listener, source, session)
}

async fn read_with_operation(
    source: &GithubSource,
    reference: &str,
    limits: Option<&ReadAcquisitionLimits>,
    operation: &OperationGuard,
) -> Result<SourceResource, resourcefs_core::ResourceError> {
    source
        .read(
            &PathReference::parse(reference).expect("facts reference"),
            operation,
            limits,
        )
        .await
}

#[tokio::test]
async fn collection_admits_whole_pages_atomically() {
    // 600 + 400 records are admitted whole.
    let (listener, source, _session) = collection_fixture(|target, api, _| {
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        if target.contains("page=2") {
            comment_page(&(601..=1000).collect::<Vec<_>>(), api, None)
        } else {
            comment_page(&(1..=600).collect::<Vec<_>>(), api, Some(&page_two))
        }
    })
    .await;
    let resource = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect("600+400 admitted");
    let facts = document(&resource);
    assert_eq!(facts["schemaVersion"]["minor"], 1);
    assert_eq!(facts["collection"]["scope"], "initial");
    assert_eq!(facts["collection"]["state"], "complete");
    assert_eq!(facts["collection"]["acceptedCount"], 1000);
    assert_eq!(
        facts["data"]["records"].as_array().expect("records").len(),
        1000
    );
    assert!(facts["collection"].get("localLimit").is_none());
    assert_eq!(facts["acquisition"]["usage"]["attemptedRequests"], 3);
    assert_eq!(listener.requests().len(), 3);

    // 600 followed by 401 retains only the first page.
    let (listener, source, _session) = collection_fixture(|target, api, _| {
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        if target.contains("page=2") {
            comment_page(&(601..=1001).collect::<Vec<_>>(), api, None)
        } else {
            comment_page(&(1..=600).collect::<Vec<_>>(), api, Some(&page_two))
        }
    })
    .await;
    let resource = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect("first page retained");
    let facts = document(&resource);
    let records = facts["data"]["records"].as_array().expect("records");
    assert_eq!(facts["schemaVersion"]["minor"], 1);
    assert_eq!(facts["collection"]["scope"], "initial");
    assert_eq!(facts["collection"]["state"], "incomplete");
    assert_eq!(facts["collection"]["acceptedCount"], 600);
    assert_eq!(records.len(), 600);
    assert_eq!(records.first().expect("first record")["id"], "1");
    assert_eq!(records.last().expect("last record")["id"], "600");
    assert_eq!(
        facts["collection"]["localLimit"]["kind"],
        "collection_records"
    );
    assert_eq!(facts["collection"]["localLimit"]["bound"], 1000);
    assert_eq!(facts["collection"]["localLimit"]["observed"], 1001);
    let continuation = facts["collection"]["continuation"]
        .as_str()
        .expect("refused page continuation");
    assert_eq!(resource.continuation(), Some(continuation));
    assert_eq!(facts["acquisition"]["usage"]["attemptedRequests"], 3);
    let before_resume = listener.requests().len();
    let resumed = read_reference(&source, continuation, None)
        .await
        .expect("resume refused page");
    let resumed_facts = document(&resumed);
    assert_eq!(resumed_facts["collection"]["scope"], "continuation");
    assert_eq!(resumed_facts["collection"]["state"], "complete");
    assert_eq!(resumed_facts["collection"]["acceptedCount"], 401);
    assert_eq!(
        resumed_facts["data"]["records"]
            .as_array()
            .expect("resumed records")
            .first()
            .expect("resumed first")["id"],
        "601"
    );
    assert_eq!(listener.requests().len(), before_resume + 2);

    // A 1,001-record first page is a typed failure, never an empty success.
    let (_, source, _session) =
        collection_fixture(|_, api, _| comment_page(&(1..=1001).collect::<Vec<_>>(), api, None))
            .await;
    let error = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect_err("unreturnable first page");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(
        error
            .details()
            .expect("typed limit")
            .limit()
            .expect("limit detail")
            .kind()
            .as_str(),
        "collection_records"
    );
}

#[tokio::test]
async fn empty_complete_is_not_inaccessible() {
    let (_, source, _session) = collection_fixture(|_, _, _| comment_page(&[], "", None)).await;
    let facts = document(
        &read_reference(&source, COLLECTION_RESOURCE, None)
            .await
            .expect("empty complete collection"),
    );
    assert_eq!(facts["schemaVersion"]["minor"], 1);
    assert_eq!(facts["collection"]["scope"], "initial");
    assert_eq!(facts["collection"]["state"], "complete");
    assert_eq!(facts["collection"]["acceptedCount"], 0);
    assert_eq!(
        facts["data"]["records"].as_array().expect("records").len(),
        0
    );

    for status in ["404 Not Found", "403 Forbidden"] {
        let (_, source, _session) =
            collection_fixture(move |_, _, _| response(status, "{}".into(), vec![])).await;
        let error = read_reference(&source, COLLECTION_RESOURCE, None)
            .await
            .expect_err("inaccessible collection");
        assert!(matches!(
            error.category(),
            ErrorCategory::NotFound | ErrorCategory::PermissionDenied
        ));
    }
}

#[tokio::test]
async fn partial_retention_and_rejection_precedence() {
    // A malformed later page retains the verified first page with honest
    // coverage; the collection is not reported complete.
    let (listener, source, _session) = collection_fixture(|target, api, _| {
        if target.contains("page=2") {
            return response("200 OK", "{malformed".into(), vec![]);
        }
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
    })
    .await;
    let resource = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect("first page retained");
    let facts = document(&resource);
    assert_eq!(facts["collection"]["state"], "incomplete");
    assert_eq!(facts["collection"]["acceptedCount"], 100);
    assert_eq!(
        facts["collection"]["failure"]["category"],
        "source_unavailable"
    );
    assert_eq!(
        facts["collection"]["failure"]["reason"],
        "upstream_malformed"
    );
    assert!(facts["collection"].get("continuation").is_none());
    assert!(resource.continuation().is_none());
    assert_eq!(listener.requests().len(), 3);

    // A later page missing its required id retains the verified prefix but
    // never issues a cursor for a page whose records were malformed.
    let (_, source, _session) = collection_fixture(|target, api, _| {
        if target.contains("page=2") {
            let mut page = vec![comment_json(101, api)];
            page[0]
                .as_object_mut()
                .expect("comment object")
                .remove("id");
            return response(
                "200 OK",
                serde_json::to_string(&page).expect("malformed record page"),
                vec![],
            );
        }
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
    })
    .await;
    let resource = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect("malformed later record retains prefix");
    let facts = document(&resource);
    assert_eq!(facts["collection"]["acceptedCount"], 100);
    assert_eq!(
        facts["collection"]["failure"]["reason"],
        "upstream_malformed"
    );
    assert!(facts["collection"].get("continuation").is_none());
    assert!(resource.continuation().is_none());

    // A wrong parent is an authority contradiction, so it rejects all pages.
    let (_, source, _session) = collection_fixture(|target, api, _| {
        if target.contains("page=2") {
            let mut page = vec![comment_json(101, api)];
            page[0]["issue_url"] = json!(format!("{api}repos/owner/repo/issues/8"));
            return response(
                "200 OK",
                serde_json::to_string(&page).expect("wrong-parent page"),
                vec![],
            );
        }
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
    })
    .await;
    assert!(
        read_reference(&source, COLLECTION_RESOURCE, None)
            .await
            .is_err(),
        "wrong parent rejects the whole collection"
    );

    // Duplicate ids on the first page are not a complete collection.
    let (_, source, _session) =
        collection_fixture(|_, api, _| comment_page(&[1, 1], api, None)).await;
    assert!(
        read_reference(&source, COLLECTION_RESOURCE, None)
            .await
            .is_err(),
        "duplicate first-page ids are refused"
    );

    // A malformed first page is a typed failure, not an empty collection.
    let (_, source, _session) =
        collection_fixture(|_, _, _| response("200 OK", "{malformed".into(), vec![])).await;
    let error = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect_err("unusable first page");
    assert_eq!(
        error.details().expect("typed").reason(),
        ErrorReason::UpstreamMalformed
    );

    // Cancellation after a verified page rejects every page of this read.
    let operation = Arc::new(OperationGuard::new());
    let cancelling = Arc::clone(&operation);
    let (listener, source, _session) = collection_fixture(move |_target, api, index| {
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        if index == 0 {
            return comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two));
        }
        cancelling.cancel();
        comment_page(&[], api, None)
    })
    .await;
    let error = read_with_operation(&source, COLLECTION_RESOURCE, None, &operation)
        .await
        .expect_err("cancellation rejects the component");
    assert_eq!(error.category(), ErrorCategory::Cancelled);
    assert_eq!(
        listener.requests().len(),
        3,
        "parent and both pages were requested before cancellation was observed"
    );
}

#[tokio::test]
async fn collection_final_failure_rolls_back_last_verified_page() {
    let healthy = Arc::new(AtomicBool::new(false));
    let fixture_healthy = Arc::clone(&healthy);
    let (listener, source, _session) = collection_fixture(move |target, api, _| {
        if target.contains("page=3") {
            if fixture_healthy.load(Ordering::SeqCst) {
                return comment_page(&[201], api, None);
            }
            return response(
                "503 Service Unavailable",
                "retry secret".into(),
                vec![("Retry-After".into(), "3600".into())],
            );
        }
        if target.contains("page=2") {
            let page_three = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=3");
            let mut records: Vec<Value> = (101..=200).map(|id| comment_json(id, api)).collect();
            records[0]
                .as_object_mut()
                .expect("comment object")
                .remove("body");
            return response(
                "200 OK",
                serde_json::to_string(&records).expect("page JSON"),
                vec![("Link".into(), format!("<{page_three}>; rel=\"next\""))],
            );
        }
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
    })
    .await;

    let wide_controls =
        ReadAcquisitionLimits::new(Some(4), None, None, None, None).expect("four attempts");
    let first = read_reference(&source, COLLECTION_RESOURCE, Some(&wide_controls))
        .await
        .expect("two verified pages plus final failure");
    let first_facts = document(&first);
    assert_eq!(first_facts["collection"]["scope"], "initial");
    assert_eq!(first_facts["collection"]["state"], "incomplete");
    assert_eq!(first_facts["collection"]["acceptedCount"], 200);
    assert_eq!(
        first_facts["data"]["records"]
            .as_array()
            .expect("first records")
            .len(),
        200
    );
    assert_eq!(
        first_facts["collection"]["failure"]["category"],
        "source_unavailable"
    );
    assert_eq!(
        first_facts["collection"]["failure"]["reason"],
        "upstream_unavailable"
    );
    assert_eq!(first_facts["collection"]["failure"]["httpStatus"], 503);
    assert_eq!(
        first_facts["collection"]["failure"]["retryGuidance"]["delaySeconds"],
        3600
    );
    let first_cursor = first_facts["collection"]["continuation"]
        .as_str()
        .expect("failed page continuation")
        .to_owned();
    assert_eq!(first.continuation(), Some(first_cursor.as_str()));
    let first_cursor_payload = cursor_parts(
        first_cursor
            .rsplit(":cursor:")
            .next()
            .expect("encoded first cursor"),
    )
    .0;
    assert!(
        first_cursor_payload["target"]
            .as_str()
            .expect("first cursor target")
            .contains("page=3")
    );
    assert_eq!(
        first_facts["unavailableFacts"]
            .as_array()
            .expect("first unavailable facts")
            .iter()
            .filter(|entry| entry["field"] == "body" && entry["reason"] == "omitted")
            .count(),
        1
    );
    assert_eq!(first_facts["acquisition"]["usage"]["attemptedRequests"], 4);
    assert_eq!(listener.requests().len(), 4);

    let mut candidate = first_facts.clone();
    let candidate_collection = candidate["collection"]
        .as_object_mut()
        .expect("candidate collection");
    assert!(candidate_collection.remove("failure").is_some());
    assert!(candidate_collection.get("localLimit").is_none());
    assert!(candidate_collection.get("continuation").is_some());
    let candidate_bytes = serde_json::to_vec_pretty(&candidate)
        .expect("candidate JSON")
        .len();
    let full_failure_bytes = first.content().len();
    let failure_overhead = full_failure_bytes
        .checked_sub(candidate_bytes)
        .expect("failure makes the document larger");
    assert!(
        failure_overhead > 128,
        "failure field must add substantial representation bytes"
    );
    let cap = candidate_bytes.checked_add(64).expect("representation cap");
    assert!(
        cap < full_failure_bytes.saturating_sub(64),
        "derived cap leaves the required failure-growth margin"
    );
    let controls =
        ReadAcquisitionLimits::new(Some(4), None, None, None, Some(cap)).expect("derived cap");

    let limited = read_reference(&source, COLLECTION_RESOURCE, Some(&controls))
        .await
        .expect("rollback publishes the verified prefix");
    let limited_facts = document(&limited);
    assert_eq!(listener.requests().len(), 8, "four requests per read");
    assert_eq!(
        limited_facts["acquisition"]["usage"]["attemptedRequests"],
        4
    );
    assert_eq!(limited_facts["collection"]["scope"], "initial");
    assert_eq!(limited_facts["collection"]["state"], "incomplete");
    assert_eq!(limited_facts["collection"]["acceptedCount"], 100);
    assert!(limited_facts["collection"].get("failure").is_none());
    assert_eq!(
        limited_facts["collection"]["localLimit"]["kind"],
        "representation_bytes"
    );
    assert_eq!(limited_facts["collection"]["localLimit"]["bound"], cap);
    assert!(
        limited_facts["collection"]["localLimit"]["observed"]
            .as_u64()
            .expect("observed representation bytes")
            > cap as u64
    );
    assert!(limited.content().len() <= cap);
    let limited_records = limited_facts["data"]["records"]
        .as_array()
        .expect("retained records");
    assert_eq!(limited_records.len(), 100);
    assert_eq!(
        limited_records
            .iter()
            .map(|record| record["id"].as_str().expect("comment ID"))
            .collect::<Vec<_>>(),
        (1..=100).map(|id| id.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(
        limited_facts["upstream"]["pages"]
            .as_array()
            .expect("retained upstream pages")
            .len(),
        1
    );
    assert!(
        !limited_facts["unavailableFacts"]
            .as_array()
            .expect("rolled-back unavailable facts")
            .iter()
            .any(|entry| entry["field"] == "body" && entry["reason"] == "omitted"),
        "rolled-back page metadata must be removed"
    );
    let limited_cursor = limited_facts["collection"]["continuation"]
        .as_str()
        .expect("rollback continuation")
        .to_owned();
    assert_eq!(limited.continuation(), Some(limited_cursor.as_str()));
    let limited_cursor_payload = cursor_parts(
        limited_cursor
            .rsplit(":cursor:")
            .next()
            .expect("encoded rollback cursor"),
    )
    .0;
    assert!(
        limited_cursor_payload["target"]
            .as_str()
            .expect("rollback cursor target")
            .contains("page=2")
    );
    assert!(
        !limited_cursor_payload["target"]
            .as_str()
            .expect("rollback cursor target")
            .contains("page=3")
    );

    healthy.store(true, Ordering::SeqCst);
    let resumed = read_reference(&source, &limited_cursor, Some(&controls))
        .await
        .expect("resume reaches the repaired final page");
    let resumed_facts = document(&resumed);
    assert_eq!(resumed_facts["collection"]["scope"], "continuation");
    assert_eq!(resumed_facts["collection"]["state"], "complete");
    assert_eq!(resumed_facts["collection"]["acceptedCount"], 101);
    assert!(resumed_facts["collection"].get("failure").is_none());
    assert!(resumed_facts["collection"].get("continuation").is_none());
    assert!(resumed.continuation().is_none());
    let resumed_records = resumed_facts["data"]["records"]
        .as_array()
        .expect("resumed records");
    assert_eq!(resumed_records.len(), 101);
    assert_eq!(resumed_records.first().expect("resumed first")["id"], "101");
    assert_eq!(resumed_records.last().expect("resumed last")["id"], "201");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn collection_generation_change_between_pages_rejects_verified_prefix() {
    let page_two_arrived = Arc::new(tokio::sync::Notify::new());
    let notify = Arc::clone(&page_two_arrived);
    let (release, blocked) = std::sync::mpsc::channel();
    let blocked = std::sync::Mutex::new(blocked);
    let (listener, source, session_fixture) = collection_fixture(move |_, api, index| {
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        if index == 0 {
            return comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two));
        }
        notify.notify_one();
        blocked
            .lock()
            .expect("generation barrier")
            .recv_timeout(Duration::from_secs(5))
            .expect("release generation barrier");
        comment_page(&(101..=200).collect::<Vec<_>>(), api, None)
    })
    .await;
    let source = Arc::new(source);
    let reading = Arc::clone(&source);
    let task =
        tokio::spawn(async move { read_reference(&reading, COLLECTION_RESOURCE, None).await });
    tokio::time::timeout(Duration::from_secs(3), page_two_arrived.notified())
        .await
        .expect("second page request arrived");
    session_fixture
        .path_session()
        .cache_remove_namespace("github-http")
        .await
        .expect("invalidate GitHub generation");
    release.send(()).expect("release second page");
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("bounded generation read")
        .expect("reader task");
    assert!(result.is_err(), "generation change rejects all prior pages");
    assert!(listener.requests().len() >= 3);
}
#[tokio::test]
async fn collection_coverage_vocabulary_is_honest() {
    let (_, source, _session) =
        collection_fixture(|_, api, _| comment_page(&[1, 2, 3], api, None)).await;
    let facts = document(
        &read_reference(&source, COLLECTION_RESOURCE, None)
            .await
            .expect("complete collection"),
    );
    let collection = facts["collection"].as_object().expect("collection object");
    assert_eq!(facts["schemaVersion"]["minor"], 1);
    assert_eq!(collection["scope"], "initial");
    for key in ["state", "acceptedCount", "scope"] {
        assert!(collection.contains_key(key), "{key}");
    }
    // This family has no provider cap and no supplied total; neither is invented.
    for absent in ["providerCap", "reportedTotal", "continuation"] {
        assert!(!collection.contains_key(absent), "{absent} must be absent");
    }
    assert_eq!(facts["kind"], "github.conversation_comment_collection");
    assert_eq!(
        facts["data"]["records"][0]["kind"],
        "github.conversation_comment"
    );
    assert_eq!(facts["data"]["records"][0]["parent"]["number"], "7");
}

#[tokio::test]
async fn one_budget_covers_parent_and_pages() {
    let (listener, source, _session) = collection_fixture(|target, api, _| {
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        if target.contains("page=2") {
            comment_page(&(101..=200).collect::<Vec<_>>(), api, None)
        } else {
            comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
        }
    })
    .await;
    let controls =
        ReadAcquisitionLimits::new(Some(2), None, None, None, None).expect("two attempts");
    let resource = read_reference(&source, COLLECTION_RESOURCE, Some(&controls))
        .await
        .expect("parent plus one page fit the budget");
    let facts = document(&resource);
    assert_eq!(facts["schemaVersion"]["minor"], 1);
    assert_eq!(facts["collection"]["scope"], "initial");
    assert_eq!(facts["collection"]["state"], "incomplete");
    assert_eq!(facts["collection"]["acceptedCount"], 100);
    assert_eq!(facts["collection"]["failure"]["category"], "limit_exceeded");
    assert_eq!(facts["collection"]["failure"]["limit"]["kind"], "attempts");
    let continuation = facts["collection"]["continuation"]
        .as_str()
        .expect("budget continuation");
    assert_eq!(resource.continuation(), Some(continuation));
    assert_eq!(facts["acquisition"]["usage"]["attemptedRequests"], 2);
    assert_eq!(listener.requests().len(), 2, "the attempt budget is shared");
}

#[tokio::test]
async fn collection_early_retry_does_not_fit_publishes_prefix_while_deadline_remains() {
    let (listener, source, _session) = collection_fixture(|target, api, _| {
        if target.contains("page=2") {
            return response(
                "503 Service Unavailable",
                "retry secret".into(),
                vec![("Retry-After".into(), "3600".into())],
            );
        }
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
    })
    .await;
    let controls = ReadAcquisitionLimits::new(None, Some(Duration::from_secs(5)), None, None, None)
        .expect("deadline controls");
    let resource = read_reference(&source, COLLECTION_RESOURCE, Some(&controls))
        .await
        .expect("retry that cannot fit publishes verified prefix");
    let facts = document(&resource);
    assert_eq!(facts["schemaVersion"]["minor"], 1);
    assert_eq!(facts["collection"]["scope"], "initial");
    assert_eq!(facts["collection"]["state"], "incomplete");
    assert_eq!(facts["collection"]["acceptedCount"], 100);
    assert_eq!(
        facts["collection"]["failure"]["reason"],
        "deadline_exceeded"
    );
    assert_eq!(
        facts["collection"]["failure"]["retryGuidance"]["delaySeconds"],
        3600
    );
    let continuation = facts["collection"]["continuation"]
        .as_str()
        .expect("retry target remains resumable");
    assert_eq!(resource.continuation(), Some(continuation));
    assert_eq!(listener.requests().len(), 3);
}
#[tokio::test]
async fn collection_response_and_aggregate_limits_keep_distinct_full_details() {
    fn page_body(api: &str, first: u64, last: u64) -> String {
        let records: Vec<Value> = (first..=last).map(|id| comment_json(id, api)).collect();
        serde_json::to_string(&records).expect("page JSON")
    }

    #[derive(Clone, Copy)]
    enum Case {
        ResponseTooLarge,
        FreshAggregateTooSmall,
        FreshAggregateFits,
    }

    for case in [
        Case::ResponseTooLarge,
        Case::FreshAggregateTooSmall,
        Case::FreshAggregateFits,
    ] {
        let (listener, source, _session) = collection_fixture(|target, api, _| {
            if target.contains("page=2") {
                return response("200 OK", page_body(api, 101, 200), vec![]);
            }
            let next = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
            response(
                "200 OK",
                page_body(api, 1, 100),
                vec![("Link".into(), format!("<{next}>; rel=\"next\""))],
            )
        })
        .await;
        let api = format!("https://{}:{}/", tls::FIXTURE_HOST, listener.address.port());
        let parent_bytes = NATIVE.replace("@API@", &api).len();
        let first_bytes = page_body(&api, 1, 100).len();
        let second_bytes = page_body(&api, 101, 200).len();
        assert!(
            first_bytes < second_bytes,
            "the first page must fit every case"
        );
        assert!(
            parent_bytes < second_bytes - 1,
            "the parent must fit every case"
        );
        let (response_bound, aggregate_bound, kind, bound, observed) = match case {
            Case::ResponseTooLarge => (
                Some(second_bytes - 1),
                None,
                "response_body_bytes",
                second_bytes - 1,
                second_bytes,
            ),
            Case::FreshAggregateTooSmall => (
                None,
                Some(parent_bytes + second_bytes - 1),
                "accepted_body_bytes",
                parent_bytes + second_bytes - 1,
                parent_bytes + first_bytes + second_bytes,
            ),
            Case::FreshAggregateFits => (
                None,
                Some(parent_bytes + second_bytes),
                "accepted_body_bytes",
                parent_bytes + second_bytes,
                parent_bytes + first_bytes + second_bytes,
            ),
        };
        let controls =
            ReadAcquisitionLimits::new(None, None, response_bound, aggregate_bound, None)
                .expect("native-byte-derived limit");
        let resource = read_reference(&source, COLLECTION_RESOURCE, Some(&controls))
            .await
            .expect("verified first page survives the later limit");
        let facts = document(&resource);
        assert_eq!(facts["collection"]["scope"], "initial");
        assert_eq!(facts["collection"]["state"], "incomplete");
        assert_eq!(facts["collection"]["acceptedCount"], 100);
        assert_eq!(facts["collection"]["failure"]["category"], "limit_exceeded");
        assert_eq!(facts["collection"]["failure"]["reason"], "limit_exceeded");
        assert_eq!(
            facts["collection"]["failure"]["limit"],
            json!({"kind":kind,"bound":bound,"observed":observed})
        );
        let ids = facts["data"]["records"]
            .as_array()
            .expect("retained records")
            .iter()
            .map(|record| record["id"].as_str().expect("comment ID"))
            .collect::<Vec<_>>();
        assert_eq!(ids, (1..=100).map(|id| id.to_string()).collect::<Vec<_>>());
        assert_eq!(listener.requests().len(), 3);

        if !matches!(case, Case::FreshAggregateFits) {
            assert!(
                resource.continuation().is_none(),
                "a fresh read cannot fix this limit"
            );
            assert!(facts["collection"].get("continuation").is_none());
            continue;
        }
        let continuation = facts["collection"]["continuation"]
            .as_str()
            .expect("fresh parent plus second page fits");
        assert_eq!(resource.continuation(), Some(continuation));
        let resumed = read_reference(&source, continuation, Some(&controls))
            .await
            .expect("same-bound continuation succeeds");
        let resumed_facts = document(&resumed);
        assert_eq!(resumed_facts["collection"]["scope"], "continuation");
        assert_eq!(resumed_facts["collection"]["state"], "complete");
        assert_eq!(resumed_facts["collection"]["acceptedCount"], 100);
        let resumed_ids = resumed_facts["data"]["records"]
            .as_array()
            .expect("resumed records")
            .iter()
            .map(|record| record["id"].as_str().expect("comment ID"))
            .collect::<Vec<_>>();
        assert_eq!(
            resumed_ids,
            (101..=200).map(|id| id.to_string()).collect::<Vec<_>>()
        );
        assert!(resumed.continuation().is_none());
        assert_eq!(listener.requests().len(), 5);
    }
}
#[tokio::test]
async fn representation_ceiling_includes_outcome_overhead() {
    // Measure the actual pretty document, including nested records, long
    // provenance, unavailable/null fields and revalidation observations.
    let etag = format!("\"{}\"", "e".repeat(8_190));
    let date = "Tue, 01 Jan 2019 00:00:00 GMT".to_owned();
    let (listener, source, _session) = collection_fixture({
        let etag = etag.clone();
        let date = date.clone();
        move |target, api, index| {
            let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
            let page = if target.contains("page=2") { 2 } else { 1 };
            let ids: Vec<u64> = if page == 1 {
                (1..=100).collect()
            } else {
                (101..=200).collect()
            };
            if index >= 2 {
                return response(
                    "304 Not Modified",
                    String::new(),
                    vec![("Date".into(), date.clone())],
                );
            }
            let mut records: Vec<Value> = ids.iter().map(|id| comment_json(*id, api)).collect();
            records.iter_mut().for_each(|record| {
                record["body"] = json!("nested pretty body ".repeat(32));
            });
            if page == 1 {
                records[0]["body"] = Value::Null;
                records[1].as_object_mut().expect("record").remove("body");
                records[2]["user"] = Value::Null;
                records[3].as_object_mut().expect("record").remove("user");
                records[4]["user"]["login"] = json!("");
            }
            let mut headers = vec![("ETag".into(), etag.clone()), ("Date".into(), date.clone())];
            headers.push(("Link".into(), format!("<{page_two}>; rel=\"next\"")));
            if page == 2 {
                headers.pop();
            }
            response(
                "200 OK",
                serde_json::to_string(&records).expect("large page JSON"),
                headers,
            )
        }
    })
    .await;

    let first = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect("two real pages");
    let first_facts = document(&first);
    assert_eq!(
        first_facts["data"]["records"]
            .as_array()
            .expect("records")
            .len(),
        200
    );
    assert_eq!(first_facts["collection"]["acceptedCount"], 200);
    let revalidated = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect("revalidated two real pages");
    let revalidated_facts = document(&revalidated);
    assert_eq!(
        revalidated_facts["upstream"]["pages"][0]["revalidation"]["status"], 304,
        "revalidation observations: {}",
        revalidated_facts["upstream"]
    );
    assert_eq!(
        revalidated_facts["upstream"]["pages"][0]["body"]["etag"]
            .as_str()
            .expect("8KiB ETag")
            .len(),
        8_192
    );

    // Lower the observed complete-document ceiling enough to refuse page two,
    // while leaving a large margin for the first 100-record pretty document.
    let cap = revalidated.content().len().saturating_sub(8 * 1024);
    assert!(cap > 100_000, "cap must leave room for one full page");
    let controls = ReadAcquisitionLimits::new(None, None, None, None, Some(cap)).expect("cap");
    let limited = read_reference(&source, COLLECTION_RESOURCE, Some(&controls))
        .await
        .expect("whole first page under measured lower ceiling");
    assert!(limited.content().len() <= cap);
    let limited_facts = document(&limited);
    let records = limited_facts["data"]["records"]
        .as_array()
        .expect("records");
    assert_eq!(records.len(), 100);
    assert_eq!(records.first().expect("first record")["id"], "1");
    assert_eq!(records.last().expect("last record")["id"], "100");
    assert_eq!(limited_facts["collection"]["scope"], "initial");
    assert_eq!(limited_facts["collection"]["state"], "incomplete");
    assert_eq!(
        limited_facts["collection"]["localLimit"]["kind"],
        "representation_bytes"
    );
    assert_eq!(limited_facts["collection"]["acceptedCount"], 100);
    let continuation = limited_facts["collection"]["continuation"]
        .as_str()
        .expect("representation continuation");
    assert_eq!(limited.continuation(), Some(continuation));

    let resumed = read_reference(&source, continuation, None)
        .await
        .expect("resume representation-refused page");
    let resumed_facts = document(&resumed);
    assert_eq!(resumed_facts["collection"]["scope"], "continuation");
    assert_eq!(resumed_facts["collection"]["state"], "complete");
    assert_eq!(resumed_facts["collection"]["acceptedCount"], 100);
    assert_eq!(
        resumed_facts["data"]["records"][0]["id"], "101",
        "resume starts at refused page without skipping"
    );
    assert!(listener.requests().len() >= 9);
}

#[tokio::test]
async fn repeated_pagination_is_explicit() {
    let (listener, source, _session) = collection_fixture(|target, api, _| {
        // Every page names the same next target, so traversal cannot progress.
        let repeated = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        if target.contains("page=2") {
            comment_page(&(101..=200).collect::<Vec<_>>(), api, Some(&repeated))
        } else {
            comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&repeated))
        }
    })
    .await;
    let facts = document(
        &read_reference(&source, COLLECTION_RESOURCE, None)
            .await
            .expect("repeated pagination is reported, not looped"),
    );
    assert_eq!(facts["collection"]["state"], "unknown");
    assert_eq!(facts["collection"]["scope"], "initial");
    assert_eq!(
        facts["collection"]["inconsistency"]["reason"],
        "repeated_pagination"
    );
    assert_eq!(facts["collection"]["acceptedCount"], 200);
    assert_eq!(listener.requests().len(), 3, "bounded request count");
}

#[tokio::test]
#[ignore = "checkpointed-build production-scale budget"]
async fn github_collection_production_budget() {
    // Nine pages of 100 records: one shared attempt budget spends the first
    // attempt on the parent, so nine data pages is the reachable maximum.
    let body = "x".repeat(6_000);
    let (_, source, _session) = collection_fixture(move |target, api, _| {
        // `per_page=100` also contains `page=`, so take the last occurrence.
        let page: u64 = target
            .rsplit("page=")
            .next()
            .and_then(|value| value.split('&').next())
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        let start = (page - 1) * 100 + 1;
        let ids: Vec<u64> = (start..start + 100).collect();
        let mut records: Vec<Value> = ids.iter().map(|id| comment_json(*id, api)).collect();
        for record in &mut records {
            record["body"] = json!(body.clone());
        }
        let mut headers = Vec::new();
        if page < 9 {
            headers.push((
                "Link".to_string(),
                format!(
                    "<{api}repos/owner/repo/issues/7/comments?per_page=100&page={}>; rel=\"next\"",
                    page + 1
                ),
            ));
        }
        response(
            "200 OK",
            serde_json::to_string(&records).expect("page JSON"),
            headers,
        )
    })
    .await;
    let started = Instant::now();
    let resource = read_reference(&source, COLLECTION_RESOURCE, None)
        .await
        .expect("production-size collection");
    let elapsed = started.elapsed();
    let facts = document(&resource);
    assert_eq!(facts["collection"]["acceptedCount"], 900);
    assert_eq!(facts["collection"]["state"], "complete");
    let output_bytes = resource.content().len();
    assert!(
        output_bytes > 6_000_000,
        "production-size output: {output_bytes}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "local processing took {elapsed:?}"
    );
    eprintln!(
        "collection_budget phase=wall records=900 output_bytes={output_bytes} local_processing_ns={} limit_ns=1000000000",
        elapsed.as_nanos()
    );
}

/// Minimal base64url (no padding) codec so the fence can tamper with an
/// opaque handle without reaching into production internals.
fn b64_decode(value: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut bits = 0_u32;
    let mut width = 0_u32;
    let mut out = Vec::new();
    for byte in value.bytes() {
        let digit = ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .expect("canonical base64url alphabet") as u32;
        bits = (bits << 6) | digit;
        width += 6;
        if width >= 8 {
            width -= 8;
            out.push((bits >> width) as u8);
            bits &= (1 << width) - 1;
        }
    }
    out
}
fn b64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut block = [0_u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let value = u32::from_be_bytes([0, block[0], block[1], block[2]]);
        for index in 0..chunk.len() + 1 {
            let digit = (value >> (18 - 6 * index)) & 0x3f;
            out.push(ALPHABET[digit as usize] as char);
        }
    }
    out
}

fn cursor_parts(encoded: &str) -> (Value, Vec<u8>) {
    let bytes = b64_decode(encoded);
    assert!(bytes.len() > 32, "cursor payload and tag");
    let split = bytes.len() - 32;
    (
        serde_json::from_slice(&bytes[..split]).expect("cursor payload"),
        bytes[split..].to_vec(),
    )
}

fn cursor_spelling(resource: &str, payload: &Value, tag: &[u8]) -> String {
    let mut bytes = serde_json::to_vec(payload).expect("cursor payload JSON");
    bytes.extend_from_slice(tag);
    format!("{resource}:cursor:{}", b64_encode(&bytes))
}

/// A truncated collection whose next page is never fetched, so the handle is
/// issued from the attempt budget rather than from a failed page.
async fn truncated_collection() -> (TlsListener, GithubSource, session_support::ScratchFixture) {
    collection_fixture(|target, api, _| {
        let page_two = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
        if target.contains("page=2") {
            comment_page(&(101..=200).collect::<Vec<_>>(), api, None)
        } else {
            comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&page_two))
        }
    })
    .await
}

#[tokio::test]
async fn continuation_encoding_is_canonical_and_bounded() {
    let (listener, source, _session) = truncated_collection().await;
    let controls = ReadAcquisitionLimits::new(Some(2), None, None, None, None).expect("budget");
    let resource = read_reference(&source, COLLECTION_RESOURCE, Some(&controls))
        .await
        .expect("truncated collection");
    let facts = document(&resource);
    assert_eq!(facts["schemaVersion"]["minor"], 1);
    assert_eq!(facts["collection"]["scope"], "initial");
    let continuation = facts["collection"]["continuation"]
        .as_str()
        .expect("continuation named");
    assert_eq!(resource.continuation(), Some(continuation));
    assert!(continuation.len() <= resourcefs_core::MAX_PATH_REFERENCE_BYTES);
    let parsed = PathReference::parse(continuation).expect("handle parses");
    assert!(
        parsed
            .projection()
            .and_then(resourcefs_core::ProjectionSelector::source_cursor)
            .is_some(),
        "handle is a typed source cursor"
    );

    // v2 is payload JSON followed by a raw 32-byte HMAC tag.
    let encoded = continuation
        .rsplit(":cursor:")
        .next()
        .expect("cursor spelling");
    assert!(!encoded.contains('='));
    let (payload, tag) = cursor_parts(encoded);
    assert_eq!(tag.len(), 32);
    assert_eq!(payload["version"], 2);
    assert_eq!(payload["source"], "github-facts");
    assert_eq!(payload["resource"], COLLECTION_RESOURCE);
    let expected_api = format!("https://{}:{}/", tls::FIXTURE_HOST, listener.address.port());
    assert_eq!(payload["api"], expected_api);
    assert!(
        payload["target"]
            .as_str()
            .expect("target")
            .contains("page=2")
    );
    assert_eq!(
        b64_encode(&b64_decode(encoded)),
        encoded,
        "encoded cursor is canonical unpadded base64url"
    );

    // A padded or unsigned handle is rejected without any upstream request.
    for malformed in [
        format!("{COLLECTION_RESOURCE}:cursor:{encoded}="),
        format!(
            "{COLLECTION_RESOURCE}:cursor:{}",
            b64_encode(&serde_json::to_vec(&payload).expect("unsigned payload"))
        ),
        format!("{COLLECTION_RESOURCE}:cursor:e30"),
    ] {
        let before = listener.requests().len();
        let error = read_reference(&source, &malformed, None)
            .await
            .expect_err("malformed cursor");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
        assert_eq!(listener.requests().len(), before);
    }
}

#[tokio::test]
async fn continuation_is_session_and_authority_bound() {
    use aws_lc_rs::hmac;
    use sha2::{Digest, Sha256};

    let (listener, source, session_fixture) = truncated_collection().await;
    let controls = ReadAcquisitionLimits::new(Some(2), None, None, None, None).expect("budget");
    let first = read_reference(&source, COLLECTION_RESOURCE, Some(&controls))
        .await
        .expect("first read");
    let first_facts = document(&first);
    let cursor = first_facts["collection"]["continuation"]
        .as_str()
        .expect("handle")
        .to_owned();
    let encoded = cursor.rsplit(":cursor:").next().expect("cursor");
    let (payload, tag) = cursor_parts(encoded);

    // A clone shares the issuing source's key and session authority.
    let before_clone = listener.requests().len();
    let resumed = read_reference(&source.clone(), &cursor, None)
        .await
        .expect("clone resumes");
    let resumed_facts = document(&resumed);
    assert_eq!(resumed_facts["collection"]["scope"], "continuation");
    assert_eq!(resumed_facts["data"]["records"][0]["id"], "101");
    assert_eq!(resumed_facts["collection"]["acceptedCount"], 100);
    assert_eq!(listener.requests().len(), before_clone + 2);

    // A different Path Session acquires nothing.
    let (other, _other_session) = build_source(&listener, ReadAcquisitionLimits::default()).await;
    let quiet = listener.requests().len();
    let error = read_reference(&other, &cursor, None)
        .await
        .expect_err("foreign session");
    assert_eq!(error.category(), ErrorCategory::InvalidReference);
    assert_eq!(
        listener.requests().len(),
        quiet,
        "foreign session has no egress"
    );

    // A different resource is refused before its parent is fetched.
    let error = read_reference(&source, "pr://owner/repo/8/comments/facts:cursor:AAA", None)
        .await
        .expect_err("foreign resource");
    assert_eq!(error.category(), ErrorCategory::InvalidReference);
    assert_eq!(listener.requests().len(), quiet);

    let api = payload["api"].as_str().expect("api").to_owned();
    let userinfo_target = api.replacen("https://", "https://cursor-user:cursor-pass@", 1)
        + "repos/owner/repo/issues/7/comments?per_page=100&page=2";
    let numeric_target =
        format!("{api}repositories/724712/issues/159628/comments?per_page=100&page=2");
    let edits = vec![
        ("version", json!(1)),
        ("source", json!("other-source")),
        ("api", json!("https://foreign.invalid/")),
        ("resource", json!("pr://owner/repo/8/comments/facts")),
        ("target", json!(numeric_target)),
        ("target", json!(userinfo_target)),
    ];
    // Public artifact authority must not be usable as the signing key, either
    // directly or through the old published SHA-256 session correlator.
    let artifact = session_fixture
        .session
        .path_session()
        .retain("known artifact token", &OperationGuard::new())
        .await
        .expect("artifact token");
    let known_token = artifact.session_token().to_string();
    let token_hash = Sha256::digest(known_token.as_bytes());
    let payload_bytes = serde_json::to_vec(&payload).expect("forged payload bytes");
    let payload_text = std::str::from_utf8(&payload_bytes).expect("JSON payload");
    assert!(!payload_text.contains(&known_token));
    assert!(!payload_text.contains(&format!("{token_hash:x}")));
    for public_key in [known_token.as_bytes(), token_hash.as_ref()] {
        let key = hmac::Key::new(hmac::HMAC_SHA256, public_key);
        let forged_tag = hmac::sign(&key, &payload_bytes);
        let forged = cursor_spelling(COLLECTION_RESOURCE, &payload, forged_tag.as_ref());
        let before = listener.requests().len();
        let error = read_reference(&source, &forged, None)
            .await
            .expect_err("public artifact authority cannot forge a source cursor");
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
        assert_eq!(
            listener.requests().len(),
            before,
            "forgery must precede egress"
        );
    }

    for (field, value) in edits {
        let mut edited = payload.clone();
        edited[field] = value;
        let tampered = cursor_spelling(COLLECTION_RESOURCE, &edited, &tag);
        let before = listener.requests().len();
        let error = read_reference(&source, &tampered, None)
            .await
            .expect_err(field);
        assert_eq!(error.category(), ErrorCategory::InvalidReference, "{field}");
        assert_eq!(
            listener.requests().len(),
            before,
            "{field} mutation must not fetch parent or page"
        );
    }

    // Removing or modifying the tag must fail even when the target stays on
    // the configured API origin; an origin-only confine is not sufficient.
    let unsigned = cursor_spelling(COLLECTION_RESOURCE, &payload, &[]);
    let before = listener.requests().len();
    assert_eq!(
        read_reference(&source, &unsigned, None)
            .await
            .expect_err("missing HMAC tag")
            .category(),
        ErrorCategory::InvalidReference
    );
    assert_eq!(listener.requests().len(), before);

    let mut altered_tag = tag.clone();
    altered_tag[0] ^= 0x80;
    let same_origin = cursor_spelling(COLLECTION_RESOURCE, &payload, &altered_tag);
    let before = listener.requests().len();
    assert_eq!(
        read_reference(&source, &same_origin, None)
            .await
            .expect_err("invalid HMAC tag")
            .category(),
        ErrorCategory::InvalidReference
    );
    assert_eq!(
        listener.requests().len(),
        before,
        "MAC failure on same-origin target must not fetch"
    );

    // A legacy unsigned v1 envelope is not a valid v2 handle.
    let legacy_payload = json!({
        "version": 1,
        "resource": COLLECTION_RESOURCE,
        "origin": "0".repeat(64),
        "session": "0".repeat(64),
        "next": payload["target"],
    });
    let legacy_bytes = serde_json::to_vec(&legacy_payload).expect("legacy payload");
    let legacy = format!("{COLLECTION_RESOURCE}:cursor:{}", b64_encode(&legacy_bytes));
    let before = listener.requests().len();
    assert_eq!(
        read_reference(&source, &legacy, None)
            .await
            .expect_err("legacy unsigned cursor")
            .category(),
        ErrorCategory::InvalidReference
    );
    assert_eq!(listener.requests().len(), before);
}

#[tokio::test]
async fn human_and_facts_comment_routes_share_identity_guards_not_partial_policy() {
    #[derive(Clone, Copy)]
    enum IdentityCase {
        Matching,
        WrongId,
        WrongParent,
    }
    for case in [
        IdentityCase::Matching,
        IdentityCase::WrongId,
        IdentityCase::WrongParent,
    ] {
        let (listener, source, _session) = comment_fixture(move |target, _, native| {
            if !target.starts_with("/repos/owner/repo/issues/comments/") {
                return response("200 OK", native.into(), vec![]);
            }
            let mut comment: Value = serde_json::from_str(native).expect("native comment");
            match case {
                IdentityCase::Matching => {}
                IdentityCase::WrongId => {
                    comment["id"] = json!(9002);
                    comment["url"] = json!(
                        comment["url"]
                            .as_str()
                            .expect("api URL")
                            .replace("/9001", "/9002")
                    );
                    comment["html_url"] = json!(
                        comment["html_url"]
                            .as_str()
                            .expect("web URL")
                            .replace("issuecomment-9001", "issuecomment-9002")
                    );
                }
                IdentityCase::WrongParent => {
                    comment["issue_url"] = json!(
                        comment["issue_url"]
                            .as_str()
                            .expect("parent URL")
                            .replace("/issues/7", "/issues/8")
                    );
                }
            }
            response("200 OK", comment.to_string(), vec![])
        })
        .await;
        let human = read_reference(&source, "pr://owner/repo/7/comments/9001", None).await;
        let facts = read_reference(&source, COMMENT_RESOURCE, None).await;
        match case {
            IdentityCase::Matching => {
                let native: Value = serde_json::from_str(COMMENT_NATIVE).expect("native control");
                assert!(
                    human
                        .expect("matching human comment")
                        .content()
                        .contains(native["body"].as_str().expect("native body"))
                );
                assert_eq!(
                    document(&facts.expect("matching facts comment"))["data"]["body"],
                    native["body"]
                );
            }
            IdentityCase::WrongId | IdentityCase::WrongParent => {
                let expected = match case {
                    IdentityCase::WrongId => ErrorCategory::SourceUnavailable,
                    IdentityCase::WrongParent => ErrorCategory::NotFound,
                    IdentityCase::Matching => unreachable!("handled matching control"),
                };
                assert_eq!(
                    human.expect_err("human identity guard").category(),
                    expected
                );
                let error = facts.expect_err("facts identity guard");
                assert_eq!(error.category(), expected);
                assert_eq!(
                    error.details().expect("typed identity").reason(),
                    ErrorReason::UpstreamIdentityMismatch
                );
            }
        }
        assert_eq!(
            listener.requests().len(),
            4,
            "both routes inspect the same native record"
        );
    }

    #[derive(Clone, Copy)]
    enum DuplicateCase {
        FirstPage,
        LaterPage,
    }
    for case in [DuplicateCase::FirstPage, DuplicateCase::LaterPage] {
        let (listener, source, _session) = collection_fixture(move |target, api, _| match case {
            DuplicateCase::FirstPage => comment_page(&[1, 1], api, None),
            DuplicateCase::LaterPage if target.contains("page=2") => {
                comment_page(&[100, 101], api, None)
            }
            DuplicateCase::LaterPage => {
                let next = format!("{api}repos/owner/repo/issues/7/comments?per_page=100&page=2");
                comment_page(&(1..=100).collect::<Vec<_>>(), api, Some(&next))
            }
        })
        .await;
        let human = read_reference(&source, "pr://owner/repo/7/comments", None).await;
        assert_eq!(
            human.expect_err("human duplicate guard").category(),
            ErrorCategory::SourceUnavailable
        );
        let facts = read_reference(&source, COLLECTION_RESOURCE, None).await;
        match case {
            DuplicateCase::FirstPage => {
                assert_eq!(
                    facts.expect_err("facts duplicate guard").category(),
                    ErrorCategory::SourceUnavailable
                );
                assert_eq!(listener.requests().len(), 4);
            }
            DuplicateCase::LaterPage => {
                // This difference is intentional: facts retain a verified
                // prefix, but must not claim complete coverage or offer a cursor.
                let resource = facts.expect("facts retain the earlier verified page");
                let document = document(&resource);
                assert_eq!(document["collection"]["state"], "unknown");
                assert_eq!(
                    document["collection"]["inconsistency"]["reason"],
                    "duplicate_record_id"
                );
                assert_eq!(document["collection"]["acceptedCount"], 100);
                let ids = document["data"]["records"]
                    .as_array()
                    .expect("verified records")
                    .iter()
                    .map(|record| record["id"].as_str().expect("id"))
                    .collect::<Vec<_>>();
                assert_eq!(ids, (1..=100).map(|id| id.to_string()).collect::<Vec<_>>());
                assert!(resource.continuation().is_none());
                assert!(document["collection"].get("continuation").is_none());
                assert_eq!(listener.requests().len(), 6);
            }
        }
    }
}

// --- Review-submission and inline review-comment families -------------------

const REVIEW_RESOURCE: &str = "pr://owner/repo/7/reviews/facts";
const INLINE_RESOURCE: &str = "pr://owner/repo/7/review-comments/facts";
const REVIEW_ITEM: &str = "pr://owner/repo/7/reviews/4988827796/facts";
const INLINE_ITEM: &str = "pr://owner/repo/7/review-comments/3826494362/facts";

fn actor_json(id: u64, login: &str) -> Value {
    json!({
        "id": id,
        "node_id": format!("U_{login}"),
        "login": login,
        "url": format!("@API@users/{login}"),
        "html_url": format!("https://github.example/{login}")
    })
}

/// The native review shape observed at REST 2022-11-28: no top-level `url`,
/// and a supplied `commit_id` that stays with the submission after a push.
fn review_json(id: u64, api: &str, commit: &str) -> Value {
    json!({
        "id": id,
        "node_id": format!("PRR_{id}"),
        "user": actor_json(501, "reviewer"),
        "body": format!("review {id}"),
        "state": "COMMENTED",
        "html_url": format!("https://github.example/owner/repo/pull/7#pullrequestreview-{id}"),
        "pull_request_url": format!("{api}repos/owner/repo/pulls/7"),
        "author_association": "CONTRIBUTOR",
        "submitted_at": "2026-08-21T01:01:16Z",
        "commit_id": commit
    })
}

fn review_page(ids: &[u64], api: &str, next: Option<&str>) -> FixtureResponse {
    let records: Vec<Value> = ids
        .iter()
        .map(|id| review_json(*id, api, "a8395d4c879b743dda521b3d3929b6819c8ad3b8"))
        .collect();
    let mut headers = Vec::new();
    if let Some(next) = next {
        headers.push(("Link".to_string(), format!("<{next}>; rel=\"next\"")));
    }
    response(
        "200 OK",
        serde_json::to_string(&records).expect("page JSON"),
        headers,
    )
}

/// The native inline shape observed at REST 2022-11-28: current `line` is
/// supplied null while `original_line`/`original_position` survive, and a
/// thread root omits `in_reply_to_id` entirely.
fn inline_json(id: u64, api: &str) -> Value {
    json!({
        "id": id,
        "node_id": format!("PRRC_{id}"),
        "url": format!("{api}repos/owner/repo/pulls/comments/{id}"),
        "html_url": format!("https://github.example/owner/repo/pull/7#discussion_r{id}"),
        "pull_request_url": format!("{api}repos/owner/repo/pulls/7"),
        "pull_request_review_id": 4988827796u64,
        "body": format!("inline {id}"),
        "user": actor_json(502, "reviewer"),
        "created_at": "2026-08-21T01:01:16Z",
        "updated_at": "2026-08-21T01:01:16Z",
        "path": "compiler/rustc_middle/src/ty/print/pretty.rs",
        "diff_hunk": "@@ -1,1 +1,1 @@\n-old\n+new",
        "commit_id": "a8395d4c879b743dda521b3d3929b6819c8ad3b8",
        "original_commit_id": "a8395d4c879b743dda521b3d3929b6819c8ad3b8",
        "side": "RIGHT",
        "line": null,
        "start_side": null,
        "start_line": null,
        "original_line": 2741,
        "original_start_line": null,
        "position": 1,
        "original_position": 20,
        "subject_type": "line"
    })
}

fn inline_page(ids: &[u64], api: &str, next: Option<&str>) -> FixtureResponse {
    let records: Vec<Value> = ids.iter().map(|id| inline_json(*id, api)).collect();
    let mut headers = Vec::new();
    if let Some(next) = next {
        headers.push(("Link".to_string(), format!("<{next}>; rel=\"next\"")));
    }
    response(
        "200 OK",
        serde_json::to_string(&records).expect("page JSON"),
        headers,
    )
}

/// Routes the exact pull-request parent, then hands every other target to the
/// transform so each test declares the collection and item it serves.
async fn family_fixture<F>(
    transform: F,
) -> (TlsListener, GithubSource, session_support::ScratchFixture)
where
    F: Fn(&str, &str, usize) -> FixtureResponse + Send + Sync + 'static,
{
    let count = AtomicUsize::new(0);
    let listener = TlsListener::serve_request_router(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        tls::match_cert(),
        move |request| {
            assert_eq!(request.method(), "GET", "facts cannot write");
            let host = host_from_head(request.head());
            let api = format!("https://{host}/");
            let target = request.target().to_owned();
            if target == "/repos/owner/repo/pulls/7" {
                return response("200 OK", NATIVE.replace("@API@", &api), vec![]);
            }
            transform(&target, &api, count.fetch_add(1, Ordering::SeqCst))
        },
    )
    .await;
    let (source, session) = build_source(&listener, ReadAcquisitionLimits::default()).await;
    (listener, source, session)
}

fn fixture_api(listener: &TlsListener) -> String {
    format!("https://{}:{}/", tls::FIXTURE_HOST, listener.address.port())
}

#[tokio::test]
async fn review_facts_collection_and_item_preserve_native_facts() {
    let (listener, source, _session) = family_fixture(|target, api, _| {
        if target.starts_with("/repos/owner/repo/pulls/7/reviews/") {
            return response(
                "200 OK",
                review_json(4988827796, api, "a8395d4c879b743dda521b3d3929b6819c8ad3b8")
                    .to_string(),
                vec![],
            );
        }
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/reviews"),
            "unexpected target {target}"
        );
        review_page(&[4988827796, 5020553314], api, None)
    })
    .await;
    let api = fixture_api(&listener);
    let facts = document(
        &read_reference(&source, REVIEW_RESOURCE, None)
            .await
            .expect("review collection"),
    );
    assert_eq!(facts["kind"], "github.review_submission_collection");
    assert_eq!(facts["schemaVersion"]["minor"], 1);
    assert_eq!(facts["collection"]["state"], "complete");
    assert_eq!(facts["collection"]["acceptedCount"], 2);
    assert_eq!(facts["observed"]["parent"]["number"], "7");
    let records = facts["data"]["records"].as_array().expect("records");
    assert_eq!(records.len(), 2);
    let first = &records[0];
    assert_eq!(first["kind"], "github.review_submission");
    assert_eq!(first["id"], "4988827796");
    assert_eq!(first["nodeId"], "PRR_4988827796");
    assert_eq!(first["parent"]["number"], "7");
    assert_eq!(first["body"], "review 4988827796");
    assert_eq!(first["author"]["login"], "reviewer");
    assert_eq!(first["state"], "COMMENTED");
    assert_eq!(
        first["commitSha"],
        "a8395d4c879b743dda521b3d3929b6819c8ad3b8"
    );
    assert_eq!(first["submittedAt"], "2026-08-21T01:01:16Z");
    assert_eq!(
        first["links"]["pullRequestUrl"],
        format!("{api}repos/owner/repo/pulls/7")
    );
    assert!(
        first["links"].get("apiUrl").is_none(),
        "an unsupplied native url must stay absent rather than become empty"
    );
    let unavailable = facts["unavailableFacts"].as_array().expect("unavailable");
    assert!(
        unavailable
            .iter()
            .any(|entry| entry["field"] == "links.apiUrl" && entry["reason"] == "omitted"),
        "unsupplied api url is recorded: {unavailable:?}"
    );

    // The item read yields the same native record as the collection row.
    let item = document(
        &read_reference(&source, REVIEW_ITEM, None)
            .await
            .expect("review item"),
    );
    assert_eq!(item["kind"], "github.review_submission");
    assert_eq!(item["schemaVersion"]["minor"], 0);
    assert_eq!(item["request"]["reviewId"], "4988827796");
    assert_eq!(item["data"], *first, "item and collection agree");
}

#[tokio::test]
async fn inline_facts_preserve_native_anchor_presence() {
    let (listener, source, _session) = family_fixture(|target, api, _| {
        if target.starts_with("/repos/owner/repo/pulls/comments/") {
            return response("200 OK", inline_json(3826494362, api).to_string(), vec![]);
        }
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/comments"),
            "unexpected target {target}"
        );
        let mut records = vec![inline_json(3826494362, api), inline_json(3854357897, api)];
        // A reply carries the supplied thread relationship.
        records[1]["in_reply_to_id"] = json!(3826494362u64);
        // Unknown native values must survive verbatim, and a file-level comment
        // keeps its null line instead of a fabricated coordinate.
        let mut third = inline_json(3854357999, api);
        third["side"] = json!("BOTH");
        third["subject_type"] = json!("file");
        records.push(third);
        // A LEFT-side multiline anchor keeps both its start and current
        // coordinates instead of collapsing to a single line.
        let mut fourth = inline_json(3854358001, api);
        fourth["side"] = json!("LEFT");
        fourth["start_side"] = json!("LEFT");
        fourth["line"] = json!(10);
        fourth["start_line"] = json!(5);
        fourth["original_line"] = json!(9);
        fourth["original_start_line"] = json!(4);
        fourth["original_position"] = json!(7);
        records.push(fourth);
        response(
            "200 OK",
            serde_json::to_string(&records).expect("inline page"),
            vec![],
        )
    })
    .await;
    let api = fixture_api(&listener);
    let facts = document(
        &read_reference(&source, INLINE_RESOURCE, None)
            .await
            .expect("inline collection"),
    );
    assert_eq!(facts["kind"], "github.review_comment_collection");
    assert_eq!(facts["schemaVersion"]["minor"], 1);
    let records = facts["data"]["records"].as_array().expect("records");
    assert_eq!(records.len(), 4);
    let root = &records[0];
    assert_eq!(root["kind"], "github.review_comment");
    assert_eq!(root["id"], "3826494362");
    assert_eq!(root["parent"]["number"], "7");
    assert_eq!(root["reviewId"], "4988827796");
    assert!(
        root.get("replyToId").is_none(),
        "a thread root omits the supplied reply relationship"
    );
    assert_eq!(root["side"], "RIGHT");
    assert_eq!(root["line"], Value::Null, "supplied null stays null");
    assert_eq!(root["startSide"], Value::Null);
    assert_eq!(root["startLine"], Value::Null);
    assert_eq!(root["originalLine"], 2741);
    assert_eq!(root["originalStartLine"], Value::Null);
    assert_eq!(root["position"], 1);
    assert_eq!(root["originalPosition"], 20);
    assert_eq!(root["subjectType"], "line");
    assert_eq!(root["diffHunk"], "@@ -1,1 +1,1 @@\n-old\n+new");
    assert_eq!(root["path"], "compiler/rustc_middle/src/ty/print/pretty.rs");
    assert_eq!(root["commitSha"], root["originalCommitSha"]);
    assert_eq!(
        root["links"]["apiUrl"],
        format!("{api}repos/owner/repo/pulls/comments/3826494362")
    );
    assert_eq!(
        root["links"]["pullRequestUrl"],
        format!("{api}repos/owner/repo/pulls/7")
    );
    let unavailable = facts["unavailableFacts"].as_array().expect("unavailable");
    assert!(
        unavailable
            .iter()
            .any(|entry| entry["field"] == "line" && entry["reason"] == "null"),
        "a supplied null is recorded as null, not omitted: {unavailable:?}"
    );

    let reply = &records[1];
    assert_eq!(reply["replyToId"], "3826494362");
    let unknown = &records[2];
    assert_eq!(unknown["side"], "BOTH", "unknown native value is verbatim");
    assert_eq!(unknown["subjectType"], "file");
    assert_eq!(unknown["line"], Value::Null);
    let multiline = &records[3];
    assert_eq!(multiline["side"], "LEFT");
    assert_eq!(multiline["startSide"], "LEFT");
    assert_eq!(multiline["line"], 10);
    assert_eq!(multiline["startLine"], 5);
    assert_eq!(multiline["originalLine"], 9);
    assert_eq!(multiline["originalStartLine"], 4);
    assert_eq!(multiline["originalPosition"], 7);

    let item = document(
        &read_reference(&source, INLINE_ITEM, None)
            .await
            .expect("inline item"),
    );
    assert_eq!(item["kind"], "github.review_comment");
    assert_eq!(item["request"]["commentId"], "3826494362");
    assert_eq!(item["data"], *root, "item and collection agree");
}

#[tokio::test]
async fn review_and_inline_identity_contradictions_reject_the_component() {
    // A review naming another pull request is a not-found contradiction.
    let (_, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/reviews"),
            "unexpected target {target}"
        );
        let mut record = review_json(4988827796, api, "a8395d4c879b743dda521b3d3929b6819c8ad3b8");
        record["pull_request_url"] = json!(format!("{api}repos/owner/repo/pulls/9"));
        response("200 OK", json!([record]).to_string(), vec![])
    })
    .await;
    assert_eq!(
        read_reference(&source, REVIEW_RESOURCE, None)
            .await
            .expect_err("review parent contradiction")
            .category(),
        ErrorCategory::NotFound
    );

    // A review item served under a different native id is an identity mismatch.
    let (_, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/reviews/4988827796"),
            "unexpected target {target}"
        );
        response(
            "200 OK",
            review_json(5020553314, api, "a8395d4c879b743dda521b3d3929b6819c8ad3b8").to_string(),
            vec![],
        )
    })
    .await;
    let error = read_reference(&source, REVIEW_ITEM, None)
        .await
        .expect_err("review id mismatch");
    assert!(matches!(
        error.details().expect("typed").reason(),
        ErrorReason::UpstreamIdentityMismatch | ErrorReason::UpstreamMalformed
    ));

    // The repository-wide inline item endpoint must not serve a comment that
    // belongs to another pull request.
    let (_, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/comments/3826494362"),
            "unexpected target {target}"
        );
        let mut record = inline_json(3826494362, api);
        record["pull_request_url"] = json!(format!("{api}repos/owner/repo/pulls/9"));
        response("200 OK", record.to_string(), vec![])
    })
    .await;
    assert_eq!(
        read_reference(&source, INLINE_ITEM, None)
            .await
            .expect_err("inline parent contradiction")
            .category(),
        ErrorCategory::NotFound
    );

    // A later page naming another pull request rejects the whole component,
    // including the page that was already acquired.
    let (listener, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/comments"),
            "unexpected target {target}"
        );
        if target.contains("page=2") {
            let mut record = inline_json(3854357897, api);
            record["pull_request_url"] = json!(format!("{api}repos/owner/repo/pulls/9"));
            return response("200 OK", json!([record]).to_string(), vec![]);
        }
        let next = format!("{api}repos/owner/repo/pulls/7/comments?per_page=100&page=2");
        inline_page(&[3826494362], api, Some(&next))
    })
    .await;
    assert_eq!(
        read_reference(&source, INLINE_RESOURCE, None)
            .await
            .expect_err("later-page parent contradiction")
            .category(),
        ErrorCategory::NotFound
    );
    assert_eq!(
        listener.requests().len(),
        3,
        "parent plus two pages; no document and no further request"
    );
}

#[tokio::test]
async fn review_collection_ceiling_and_failure_outcomes_are_atomic() {
    // Exactly 1,000 review records are admitted whole on one shared budget.
    let (_, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/reviews"),
            "unexpected target {target}"
        );
        review_page(&(1..=1000).collect::<Vec<_>>(), api, None)
    })
    .await;
    let facts = document(
        &read_reference(&source, REVIEW_RESOURCE, None)
            .await
            .expect("1,000 admitted"),
    );
    assert_eq!(facts["collection"]["state"], "complete");
    assert_eq!(facts["collection"]["acceptedCount"], 1000);
    assert_eq!(
        facts["acquisition"]["usage"]["attemptedRequests"], 2,
        "parent plus one page share the read budget"
    );

    // A 1,001-record first page is a typed limit, never a partial success.
    let (_, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/reviews"),
            "unexpected target {target}"
        );
        review_page(&(1..=1001).collect::<Vec<_>>(), api, None)
    })
    .await;
    let error = read_reference(&source, REVIEW_RESOURCE, None)
        .await
        .expect_err("over the record ceiling");
    assert_eq!(error.category(), ErrorCategory::LimitExceeded);
    assert_eq!(
        error
            .details()
            .expect("typed limit")
            .limit()
            .expect("limit detail")
            .kind()
            .as_str(),
        "collection_records"
    );

    // A malformed first page is a typed failure, not an empty collection.
    let (_, source, _session) =
        family_fixture(|_, _, _| response("200 OK", "{malformed".into(), vec![])).await;
    let error = read_reference(&source, REVIEW_RESOURCE, None)
        .await
        .expect_err("malformed first page");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert_eq!(
        error.details().expect("typed refusal").reason(),
        ErrorReason::UpstreamMalformed
    );

    // A malformed later page keeps the verified prefix and issues no cursor.
    let (listener, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/reviews"),
            "unexpected target {target}"
        );
        if target.contains("page=2") {
            return response("200 OK", "{malformed".into(), vec![]);
        }
        let next = format!("{api}repos/owner/repo/pulls/7/reviews?per_page=100&page=2");
        review_page(&[4988827796], api, Some(&next))
    })
    .await;
    let resource = read_reference(&source, REVIEW_RESOURCE, None)
        .await
        .expect("prefix retained");
    let facts = document(&resource);
    assert_eq!(facts["collection"]["state"], "incomplete");
    assert_eq!(facts["collection"]["acceptedCount"], 1);
    assert_eq!(
        facts["collection"]["failure"]["reason"],
        "upstream_malformed"
    );
    assert!(facts["collection"].get("continuation").is_none());
    assert!(resource.continuation().is_none());
    assert_eq!(listener.requests().len(), 3);
}

#[tokio::test]
async fn inline_collection_inconsistency_and_representation_ceiling() {
    // A later page repeating an acquired native id is an explicit
    // inconsistency, not a complete read.
    let (_, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/comments"),
            "unexpected target {target}"
        );
        if target.contains("page=2") {
            return inline_page(&[3826494362], api, None);
        }
        let next = format!("{api}repos/owner/repo/pulls/7/comments?per_page=100&page=2");
        inline_page(&[3826494362], api, Some(&next))
    })
    .await;
    let facts = document(
        &read_reference(&source, INLINE_RESOURCE, None)
            .await
            .expect("duplicate ids across pages"),
    );
    assert_eq!(facts["collection"]["state"], "unknown");
    assert_eq!(
        facts["collection"]["inconsistency"]["reason"],
        "duplicate_record_id"
    );

    // A repeated pagination target is the same kind of unknown coverage.
    let (_, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/comments"),
            "unexpected target {target}"
        );
        let next = format!("{api}repos/owner/repo/pulls/7/comments?per_page=100&page=1");
        inline_page(&[3826494362], api, Some(&next))
    })
    .await;
    let facts = document(
        &read_reference(&source, INLINE_RESOURCE, None)
            .await
            .expect("repeated pagination"),
    );
    assert_eq!(facts["collection"]["state"], "unknown");
    assert_eq!(
        facts["collection"]["inconsistency"]["reason"],
        "repeated_pagination"
    );

    // A candidate page that cannot fit the serialized ceiling is never partly
    // admitted: the verified prefix survives with a continuation to it.
    let controls = limits(None, None, Some(9_000));
    let (listener, source, _session) = family_fixture(|target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/comments"),
            "unexpected target {target}"
        );
        if target.contains("page=2") {
            let mut records: Vec<Value> = (0..3)
                .map(|index| inline_json(9_000_000 + index, api))
                .collect();
            for record in &mut records {
                record["diff_hunk"] = json!("x".repeat(4_000));
            }
            return response(
                "200 OK",
                serde_json::to_string(&records).expect("oversized page"),
                vec![],
            );
        }
        let next = format!("{api}repos/owner/repo/pulls/7/comments?per_page=100&page=2");
        inline_page(&[3826494362, 3854357897, 3854357999], api, Some(&next))
    })
    .await;
    let resource = read_reference(&source, INLINE_RESOURCE, Some(&controls))
        .await
        .expect("prefix retained");
    let facts = document(&resource);
    assert_eq!(facts["collection"]["state"], "incomplete");
    assert_eq!(facts["collection"]["acceptedCount"], 3);
    assert_eq!(
        facts["collection"]["localLimit"]["kind"],
        "representation_bytes"
    );
    assert_eq!(
        facts["data"]["records"].as_array().expect("records").len(),
        3
    );
    let continuation = facts["collection"]["continuation"]
        .as_str()
        .expect("continuation to the rejected page");
    assert_eq!(resource.continuation(), Some(continuation));

    // A cursor issued by one discussion family is refused by another before
    // any egress.
    let handle = continuation.rsplit(":cursor:").next().expect("cursor");
    let quiet = listener.requests().len();
    let error = read_reference(
        &source,
        &format!("pr://owner/repo/7/reviews/facts:cursor:{handle}"),
        None,
    )
    .await
    .expect_err("foreign family cursor");
    assert_eq!(error.category(), ErrorCategory::InvalidReference);
    assert_eq!(
        listener.requests().len(),
        quiet,
        "a foreign cursor acquires nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inline_collection_cancellation_rejects_acquired_pages() {
    let arrived = Arc::new(tokio::sync::Notify::new());
    let notify = Arc::clone(&arrived);
    let (release, blocked) = std::sync::mpsc::channel();
    let blocked = std::sync::Mutex::new(blocked);
    let (listener, source, _session) = family_fixture(move |target, api, _| {
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/comments"),
            "unexpected target {target}"
        );
        if target.contains("page=2") {
            notify.notify_one();
            match blocked.lock() {
                Ok(receiver) => receiver
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release barrier"),
                Err(poisoned) => panic!("response barrier poisoned: {poisoned}"),
            }
            return inline_page(&[3854357897], api, None);
        }
        let next = format!("{api}repos/owner/repo/pulls/7/comments?per_page=100&page=2");
        inline_page(&[3826494362], api, Some(&next))
    })
    .await;
    let source = Arc::new(source);
    let reading = Arc::clone(&source);
    let operation = OperationGuard::new();
    let worker_operation = operation.clone();
    let task = tokio::spawn(async move {
        reading
            .read(
                &PathReference::parse(INLINE_RESOURCE).expect("reference"),
                &worker_operation,
                None,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), arrived.notified())
        .await
        .expect("page two request arrived after page one was acquired");
    operation.cancel();
    release.send(()).expect("release page two");
    let error = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("bounded response")
        .expect("reader task")
        .expect_err("cancellation rejects the acquired prefix");
    assert_eq!(error.category(), ErrorCategory::Cancelled);
    assert_eq!(
        error.details().expect("typed refusal").reason(),
        ErrorReason::Cancelled
    );
    assert_eq!(listener.requests().len(), 3, "parent plus two pages");
}

#[tokio::test]
async fn colliding_ids_across_discussion_families_stay_distinct() {
    const SHARED_ID: u64 = 4_242;
    let (_, source, _session) = family_fixture(|target, api, _| {
        if target.starts_with("/repos/owner/repo/issues/7/comments") {
            return comment_page(&[SHARED_ID], api, None);
        }
        if target.starts_with("/repos/owner/repo/pulls/7/reviews") {
            return review_page(&[SHARED_ID], api, None);
        }
        assert!(
            target.starts_with("/repos/owner/repo/pulls/7/comments"),
            "unexpected target {target}"
        );
        inline_page(&[SHARED_ID], api, None)
    })
    .await;
    let conversation = document(
        &read_reference(&source, "pr://owner/repo/7/comments/facts", None)
            .await
            .expect("conversation collection"),
    );
    let review = document(
        &read_reference(&source, REVIEW_RESOURCE, None)
            .await
            .expect("review collection"),
    );
    let inline = document(
        &read_reference(&source, INLINE_RESOURCE, None)
            .await
            .expect("inline collection"),
    );
    // The same native number names three unrelated records; each family keeps
    // its own kind and document identity rather than collapsing into one.
    for (facts, collection_kind, record_kind) in [
        (
            &conversation,
            "github.conversation_comment_collection",
            "github.conversation_comment",
        ),
        (
            &review,
            "github.review_submission_collection",
            "github.review_submission",
        ),
        (
            &inline,
            "github.review_comment_collection",
            "github.review_comment",
        ),
    ] {
        assert_eq!(facts["kind"], collection_kind);
        let record = &facts["data"]["records"][0];
        assert_eq!(record["kind"], record_kind);
        assert_eq!(record["id"], SHARED_ID.to_string());
        assert_eq!(record["parent"]["number"], "7");
    }
    assert_eq!(
        review["data"]["records"][0]["commitSha"],
        "a8395d4c879b743dda521b3d3929b6819c8ad3b8"
    );
    assert_eq!(inline["data"]["records"][0]["side"], "RIGHT");
    assert_ne!(
        conversation["data"]["records"][0]["kind"],
        review["data"]["records"][0]["kind"]
    );
    assert_ne!(
        review["data"]["records"][0]["kind"],
        inline["data"]["records"][0]["kind"]
    );
}
