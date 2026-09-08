#[path = "support/jira.rs"]
mod jira;
#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jira::{fixture_with_request_session, protocol_response, read, response};
use resourcefs_core::{
    ErrorCategory, HttpCeilings, HttpCeilingsInput, OperationGuard, PathReference, SourceAdapter,
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const JQL: &str = "project = P ORDER BY key DESC";
fn reference(query: &str) -> String {
    let mut encoded = String::from("jira://acme/search/");
    for byte in query.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").expect("string formatting");
        }
    }
    encoded
}
fn issue(id: &str, summary: &str) -> Value {
    json!({"id":id,"key":format!("P-{id}"),"self":format!("https://tls.invalid/rest/api/3/issue/{id}"),
        "fields":{"summary":summary,"status":{"name":"Open"},"project":{"id":"2","self":"https://tls.invalid/rest/api/3/project/2"}}})
}
fn page(rows: Vec<Value>, token: Option<&str>) -> String {
    let mut value = json!({"issues":rows,"isLast":token.is_none()});
    if let Some(token) = token {
        value["nextPageToken"] = json!(token);
    }
    value.to_string()
}
fn payload(request: &tls::FixtureRequest, query: &str, maximum: usize) -> Option<String> {
    assert_eq!(request.method(), "POST", "C4 read-only POST");
    assert_eq!(
        request.target(),
        "/rest/api/3/search/jql",
        "C4 fixed endpoint, no URL JQL"
    );
    assert!(
        request
            .head()
            .to_ascii_lowercase()
            .contains("content-type: application/json")
    );
    let body: Value = serde_json::from_slice(request.body()).expect("JSON request");
    let token = body
        .get("nextPageToken")
        .map(|v| v.as_str().expect("opaque string").to_owned());
    let mut expected =
        json!({"jql":query,"maxResults":maximum,"fields":["key","summary","status","project"]});
    if let Some(token) = &token {
        expected["nextPageToken"] = json!(token);
    }
    assert_eq!(body, expected, "C4 exact native payload");
    token
}
fn ids(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            line.strip_prefix("Reference: jira://acme/issues/")
                .map(str::to_owned)
        })
        .collect()
}

#[tokio::test]
async fn query_post_preserves_native_order_through_short_and_empty_pages() {
    let (listener, source, _session) = fixture_with_request_session(
        |request| match payload(request, JQL, 100).as_deref() {
            None => response(&page(
                vec![issue("90", "first"), issue("10", "second")],
                Some("雪 +/& opaque"),
            )),
            Some("雪 +/& opaque") => response(&page(vec![], Some("after empty Ω"))),
            Some("after empty Ω") => response(&page(vec![issue("2", "third")], None)),
            token => panic!("C4 unexpected token {token:?}"),
        },
        HttpCeilings::default(),
    )
    .await;
    let result = read(&source, &reference(JQL))
        .await
        .expect("C4 native sequence");
    assert_eq!(ids(result.content()), ["90", "10", "2"]);
    assert_eq!(result.continuation(), None);
    assert_eq!(listener.requests().len(), 3);
}

#[tokio::test]
async fn query_cursor_owner_cells_refuse_before_egress_and_valid_owner_advances() {
    let owner = reference(JQL);
    let (listener, source, _session) = fixture_with_request_session(
        |request| match payload(request, JQL, 1).as_deref() {
            None => response(&page(vec![issue("9", "issuer")], Some("opaque 雪"))),
            Some("opaque 雪") => response(&page(vec![issue("2", "valid continuation")], None)),
            token => panic!("C3 changed token {token:?}"),
        },
        HttpCeilings::default(),
    )
    .await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 1)
        .expect("lower limits");
    let first = read(&source, &owner).await.expect("C3 issuer");
    let next = first.continuation().expect("bound cursor");
    let suffix = next.strip_prefix(&owner).expect("owner prefix");
    let encoded = suffix.strip_prefix(":cursor:").expect("typed cursor");
    let envelope: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).expect("base64"))
        .expect("envelope");
    for patch in [
        json!({"owner":reference("project = P ORDER BY key ASC")}),
        json!({"owner":owner.replace("acme", "other")}),
        json!({"owner":"jira://acme/issues"}),
        json!({"origin":"0".repeat(64)}),
        json!({"version":2}),
        json!({"token":""}),
        json!({"token":null}),
        json!({"extra":true}),
    ] {
        let mut bad = envelope.clone();
        bad.as_object_mut()
            .expect("object")
            .extend(patch.as_object().expect("patch").clone());
        let bad = format!("{owner}:cursor:{}", URL_SAFE_NO_PAD.encode(bad.to_string()));
        assert_eq!(
            read(&source, &bad)
                .await
                .expect_err("C3 bound owner")
                .category(),
            ErrorCategory::InvalidReference
        );
    }
    for changed in [
        reference("project = P ORDER BY key ASC"),
        "jira://acme/issues".into(),
    ] {
        assert_eq!(
            read(&source, &format!("{changed}{suffix}"))
                .await
                .expect_err("C3 changed resource")
                .category(),
            ErrorCategory::InvalidReference
        );
    }
    for encoded in ["e30", "_w", "e30=", "not+base64"] {
        let bad = format!("{owner}:cursor:{encoded}");
        let error = match PathReference::parse(&bad) {
            Err(error) => error,
            Ok(parsed) => source
                .read(&parsed, &OperationGuard::new(), None)
                .await
                .expect_err("C3 malformed envelope"),
        };
        assert_eq!(error.category(), ErrorCategory::InvalidReference);
    }
    assert_eq!(listener.requests().len(), 1, "C3 no rejected cursor egress");
    let (destination, rebound, _other_session) = fixture_with_request_session(
        |_| response(&page(vec![issue("2", "new mount")], None)),
        HttpCeilings::default(),
    )
    .await;
    assert_eq!(
        read(&rebound, next)
            .await
            .expect_err("C3 changed configured origin")
            .category(),
        ErrorCategory::InvalidReference
    );
    assert!(destination.requests().is_empty());
    assert_eq!(
        ids(read(&rebound, &owner)
            .await
            .expect("C3 destination positive control")
            .content()),
        ["2"]
    );
    assert_eq!(
        ids(read(&source, next)
            .await
            .expect("C3 valid original owner")
            .content()),
        ["2"]
    );
    assert_eq!(listener.requests().len(), 2);
    // Same HTTPS origin under another configured Site ID must not inherit ownership.
    let origin = resourcefs_core::AllowedOrigin::new(
        &format!("https://tls.invalid:{}/", listener.address.port()),
        true,
    )
    .expect("fixture origin");
    let substrate = tls::tls_substrate(
        resourcefs_core::OriginAllowlist::new(vec![origin.clone()]),
        vec![std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)],
    );
    let alias = resourcefs_sources::AtlassianSourceMount::new(
        vec![
            resourcefs_sources::AtlassianSite::new(
                resourcefs_core::AtlassianSiteId::new("other").expect("site ID"),
                origin,
            )
            .expect("site"),
        ],
        Arc::new(substrate),
    )
    .expect("alias mount")
    .bind(_session.path_session().clone())
    .with_jira_browse_limits_for_test(1, 1, 1)
    .expect("lower limits");
    assert_eq!(
        read(&alias, &next.replace("jira://acme/", "jira://other/"))
            .await
            .expect_err("C3 same origin, different site")
            .category(),
        ErrorCategory::InvalidReference
    );
    assert_eq!(listener.requests().len(), 2);
    let alias_first = read(&alias, &owner.replace("jira://acme/", "jira://other/"))
        .await
        .expect("C3 alias positive control");
    assert!(
        alias_first
            .content()
            .contains("Reference: jira://other/issues/9\n")
    );
    assert_eq!(listener.requests().len(), 3);
}

#[tokio::test]
async fn query_stable_authority_and_later_corruption_are_atomic() {
    let huge = "18446744073709551616000000000000000001";
    let valid = issue(huge, "large stable identity");
    let mut corruptions = vec![
        ("{wrong".to_owned(), ErrorCategory::SourceUnavailable),
        (
            page(vec![issue("90", "duplicate across pages")], None),
            ErrorCategory::SourceUnavailable,
        ),
        (
            page(vec![valid.clone(), valid.clone()], None),
            ErrorCategory::SourceUnavailable,
        ),
    ];
    for (pointer, value) in [
        ("/self", json!("https://foreign.invalid/rest/api/3/issue/2")),
        ("/self", json!("https://tls.invalid/rest/api/3/issue/3")),
        ("/self", json!("https://tls.invalid/rest/api/3/issue/P-3")),
        (
            "/fields/project/self",
            json!("https://foreign.invalid/rest/api/3/project/2"),
        ),
        (
            "/fields/project/self",
            json!("https://tls.invalid/rest/api/3/project/3"),
        ),
        ("/key", json!("P/2")),
        ("/fields/summary", json!(42)),
    ] {
        let mut bad = issue("2", "must not escape");
        *bad.pointer_mut(pointer).expect("fixture pointer") = value;
        corruptions.push((page(vec![bad], None), ErrorCategory::SourceUnavailable));
    }
    corruptions.push((
        page(
            (1..=101)
                .map(|id| issue(&id.to_string(), "over return"))
                .collect(),
            None,
        ),
        ErrorCategory::LimitExceeded,
    ));
    // Same first page and otherwise-valid second page provide the positive control.
    for (last, expected) in corruptions
        .into_iter()
        .map(|(body, category)| (body, Some(category)))
        .chain([(page(vec![valid], None), None)])
    {
        let (listener, source, _session) = fixture_with_request_session(
            move |request| {
                if payload(request, JQL, 100).is_none() {
                    response(&page(
                        vec![issue("90", "unpublished prefix")],
                        Some("later"),
                    ))
                } else {
                    response(&last)
                }
            },
            HttpCeilings::default(),
        )
        .await;
        let result = read(&source, &reference(JQL)).await;
        if let Some(category) = expected {
            assert_eq!(
                result.expect_err("C5/C6 atomic failure").category(),
                category
            );
        } else {
            assert_eq!(
                ids(result.expect("C5 valid stable links").content()),
                ["90", huge]
            );
        }
        assert_eq!(listener.requests().len(), 2);
    }
}

#[tokio::test]
async fn query_terminal_metadata_and_effective_page_size_are_authoritative() {
    for last in [
        None,
        Some(Value::Null),
        Some(json!(false)),
        Some(json!(true)),
        Some(json!("false")),
    ] {
        for token in [None, Some(json!("")), Some(json!("next")), Some(json!(17))] {
            let mut body = json!({"issues":[]});
            if let Some(last) = &last {
                body["isLast"] = last.clone();
            }
            if let Some(token) = &token {
                body["nextPageToken"] = token.clone();
            }
            let valid = match (&last, &token) {
                (None | Some(Value::Bool(true)), None) => true,
                (None | Some(Value::Bool(false)), Some(Value::String(s))) if !s.is_empty() => true,
                _ => false,
            };
            let (listener, source, _session) = fixture_with_request_session(
                move |_| response(&body.to_string()),
                HttpCeilings::default(),
            )
            .await;
            let source = source
                .with_jira_browse_limits_for_test(1, 1, 1)
                .expect("lower limits");
            let result = read(&source, &reference(JQL)).await;
            if valid {
                let result = result.expect("C4 valid terminal shape");
                assert_eq!(result.continuation().is_some(), token.is_some());
            } else {
                assert_eq!(
                    result.expect_err("C4 malformed terminal shape").category(),
                    ErrorCategory::SourceUnavailable
                );
            }
            assert_eq!(listener.requests().len(), 1);
        }
    }
    for maximum in [
        json!(0),
        json!(-1),
        json!(101),
        json!("100"),
        Value::Null,
        json!(1),
    ] {
        let valid = maximum == json!(1);
        let body =
            json!({"issues":[issue("2", "effective maximum")],"isLast":true,"maxResults":maximum});
        let (listener, source, _session) = fixture_with_request_session(
            move |r| {
                payload(r, JQL, 100);
                response(&body.to_string())
            },
            HttpCeilings::default(),
        )
        .await;
        let result = read(&source, &reference(JQL)).await;
        if valid {
            assert_eq!(
                ids(result.expect("C6 smaller effective maximum").content()),
                ["2"]
            );
        } else {
            assert_eq!(
                result.expect_err("C6 invalid effective maximum").category(),
                ErrorCategory::SourceUnavailable
            );
        }
        assert_eq!(listener.requests().len(), 1);
    }
    for next_maximum in [1, 2] {
        let attempts = AtomicUsize::new(0);
        let (listener, source, _session) = fixture_with_request_session(
            move |request| match attempts.fetch_add(1, Ordering::SeqCst) {
                0 => {
                    assert_eq!(payload(request, JQL, 100), None);
                    response(
                        &json!({
                            "issues": [issue("90", "before native clamp")],
                            "isLast": false, "nextPageToken": "clamped", "maxResults": 1
                        })
                        .to_string(),
                    )
                }
                1 => {
                    assert_eq!(payload(request, JQL, 1).as_deref(), Some("clamped"));
                    response(
                        &json!({
                            "issues": [issue("2", "after native clamp")],
                            "isLast": true, "maxResults": next_maximum
                        })
                        .to_string(),
                    )
                }
                _ => panic!("unexpected request after terminal native page"),
            },
            HttpCeilings::default(),
        )
        .await;
        let result = read(&source, &reference(JQL)).await;
        if next_maximum == 1 {
            assert_eq!(
                ids(result.expect("C6 downward native clamp").content()),
                ["90", "2"]
            );
        } else {
            assert_eq!(
                result
                    .expect_err("C6 native maximum cannot rise again")
                    .category(),
                ErrorCategory::SourceUnavailable
            );
        }
        assert_eq!(listener.requests().len(), 2);
    }
}

#[tokio::test]
async fn query_attempt_budget_counts_retry_once_across_native_pages() {
    for retry in [false, true] {
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = attempts.clone();
        let (listener, source, _session) = fixture_with_request_session(
            move |request| {
                payload(request, JQL, 100);
                let n = count.fetch_add(1, Ordering::SeqCst);
                if retry && n == 0 {
                    return protocol_response(
                        "503 Service Unavailable",
                        [("Retry-After", "0".into())],
                        Vec::new(),
                    );
                }
                response(&page(
                    vec![issue(&(100 - n).to_string(), "budget row")],
                    Some(&format!("token-{n}")),
                ))
            },
            HttpCeilings::default(),
        )
        .await;
        let result = read(&source, &reference(JQL))
            .await
            .expect("C6 bounded page");
        assert_eq!(listener.requests().len(), 10, "C6 physical attempts");
        let expected: Vec<_> = (usize::from(retry)..10)
            .map(|n| (100 - n).to_string())
            .collect();
        assert_eq!(ids(result.content()), expected);
        assert!(result.continuation().is_some());
        if retry {
            assert_eq!(
                listener.bodies()[0],
                listener.bodies()[1],
                "C7 exact replay bytes"
            );
        }
    }
    let count = Arc::new(AtomicUsize::new(0));
    let attempts = count.clone();
    let (listener, source, _session) = fixture_with_request_session(
        move |_| match attempts.fetch_add(1, Ordering::SeqCst) {
            0 | 2 => protocol_response(
                "503 Service Unavailable",
                [("Retry-After", "0".into())],
                Vec::new(),
            ),
            1 => response(&page(vec![issue("2", "retry already spent")], Some("next"))),
            _ => panic!("C7 second retry escaped"),
        },
        HttpCeilings::default(),
    )
    .await;
    assert_eq!(
        read(&source, &reference(JQL))
            .await
            .expect_err("C7 one logical retry")
            .category(),
        ErrorCategory::SourceUnavailable
    );
    assert_eq!(listener.requests().len(), 3);
}

#[tokio::test]
async fn query_repeated_and_unrepresentable_tokens_fail_without_partial_rows() {
    for token in ["same".to_owned(), "x".repeat(65_536), "雪".repeat(17_000)] {
        let repeated = token == "same";
        let (listener, source, _session) = fixture_with_request_session(
            move |request| {
                let prior = payload(request, JQL, 100);
                response(&page(
                    vec![issue(
                        if prior.is_none() { "9" } else { "2" },
                        "atomic token",
                    )],
                    Some(&token),
                ))
            },
            HttpCeilings::default(),
        )
        .await;
        let error = read(&source, &reference(JQL))
            .await
            .expect_err("C6 token cannot be lost or loop");
        assert_eq!(
            error.category(),
            if repeated {
                ErrorCategory::SourceUnavailable
            } else {
                ErrorCategory::LimitExceeded
            }
        );
        assert_eq!(listener.requests().len(), if repeated { 2 } else { 1 });
    }
}

#[tokio::test]
async fn query_native_rejection_is_structural_redacted_and_uncached() {
    let query = "text ~ \"private-query-canary\"";
    for (body, valid) in [
        (json!({"errorMessages":["private-query-canary", "token-canary"],"errors":{"private-field-canary":"response-canary"},"status":400}).to_string(), true),
        ("{}".into(), true), (json!({"errorMessages":[],"errors":{}}).to_string(), true),
        (json!({"errorMessages":"response-canary"}).to_string(), false),
        (json!({"errors":{"field":["response-canary"]}}).to_string(), false),
        (json!({"status":400.5}).to_string(), false), ("not JSON".into(), false),
    ] {
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = attempts.clone();
        let (listener, source, _session) = fixture_with_request_session(move |request| {
            payload(request, query, 100);
            if count.fetch_add(1, Ordering::SeqCst) == 0 { protocol_response("400 Bad Request", [("Content-Type", "application/json".into())], body.as_bytes().to_vec()) }
            else { response(&page(vec![issue("2", "success after rejection")], None)) }
        }, HttpCeilings::default()).await;
        let error = read(&source, &reference(query)).await.expect_err("C8 native rejection");
        assert_eq!(error.category(), if valid { ErrorCategory::InvalidPattern } else { ErrorCategory::SourceUnavailable });
        if valid {
            assert!(error.message().contains("400"), "C8 HTTP status context");
        }
        for secret in ["private-query-canary", "private-field-canary", "response-canary", "token-canary", "agent@example.com"] {
            assert!(!format!("{error:?} {error}").contains(secret), "C8 leaked diagnostic canary");
        }
        assert!(String::from_utf8_lossy(&listener.bodies()[0]).contains("private-query-canary"), "C8 positive wire control");
        assert_eq!(ids(read(&source, &reference(query)).await.expect("C9 no negative cache").content()), ["2"]);
        assert_eq!(listener.requests().len(), 2);
    }
}

#[tokio::test]
async fn query_changed_validator_rows_are_not_cached_and_shared_controls_apply() {
    let count = Arc::new(AtomicUsize::new(0));
    let attempts = count.clone();
    let (listener, source, _session) = fixture_with_request_session(
        move |request| {
            payload(request, JQL, 100);
            assert!(
                !request
                    .head()
                    .to_ascii_lowercase()
                    .contains("if-none-match")
            );
            let id = if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                "9"
            } else {
                "2"
            };
            protocol_response(
                "200 OK",
                [
                    ("Content-Type", "application/json".into()),
                    ("ETag", "\"unchanged-validator\"".into()),
                ],
                page(vec![issue(id, "fresh")], None).into_bytes(),
            )
        },
        HttpCeilings::default(),
    )
    .await;
    assert_eq!(
        ids(read(&source, &reference(JQL))
            .await
            .expect("C9 first")
            .content()),
        ["9"]
    );
    assert_eq!(
        ids(read(&source, &reference(JQL))
            .await
            .expect("C9 fresh second")
            .content()),
        ["2"]
    );
    let guard = OperationGuard::new();
    guard.cancel();
    assert_eq!(
        source
            .read(
                &PathReference::parse(reference(JQL)).expect("reference"),
                &guard,
                None
            )
            .await
            .expect_err("C9 cancellation")
            .category(),
        ErrorCategory::Cancelled
    );
    assert_eq!(
        read(&source, &reference(JQL).replace("acme", "missing"))
            .await
            .expect_err("C9 unmounted")
            .category(),
        ErrorCategory::PermissionDenied
    );
    assert_eq!(listener.requests().len(), 2);
    for oversized in [false, true] {
        let (listener, source, _session) = fixture_with_request_session(
            move |_| {
                response(&page(
                    vec![issue("2", &"x".repeat(if oversized { 2048 } else { 16 }))],
                    None,
                ))
            },
            HttpCeilings::new(HttpCeilingsInput {
                fetch_bytes: Some(1024),
                ..Default::default()
            })
            .expect("lower body bound"),
        )
        .await;
        let result = read(&source, &reference(JQL)).await;
        if oversized {
            assert_eq!(
                result.expect_err("C9 body ceiling").category(),
                ErrorCategory::LimitExceeded
            );
        } else {
            assert_eq!(
                ids(result.expect("C9 bounded positive control").content()),
                ["2"]
            );
        }
        assert_eq!(listener.requests().len(), 1);
    }
}

#[tokio::test]
async fn query_production_scale_thousand_rows_preserves_order_and_wall_bound() {
    let query = format!("text ~ \"{}\" ORDER BY key DESC", "q".repeat(4096));
    let native_query = query.clone();
    let summary = "s".repeat(4096);
    let rows: Vec<_> = (1..=1000)
        .rev()
        .map(|id| issue(&id.to_string(), &summary))
        .collect();
    let pages: Vec<_> = rows
        .chunks(100)
        .enumerate()
        .map(|(index, chunk)| page(chunk.to_vec(), Some(&format!("雪-page-{}", index + 1))))
        .collect();
    let (listener, source, _session) = fixture_with_request_session(
        move |request| {
            let index = payload(request, &native_query, 100)
                .map(|s| {
                    s.strip_prefix("雪-page-")
                        .expect("native Unicode token")
                        .parse::<usize>()
                        .expect("page index")
                })
                .unwrap_or(0);
            if index == 10 {
                response(&page(vec![], None))
            } else {
                response(&pages[index])
            }
        },
        HttpCeilings::default(),
    )
    .await;
    let start = Instant::now();
    let result = read(&source, &reference(&query))
        .await
        .expect("C6 production-scale page");
    let elapsed = start.elapsed();
    assert_eq!(
        ids(result.content()),
        (1..=1000)
            .rev()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(result.content().matches(&summary).count(), 1000);
    assert_eq!(listener.requests().len(), 10);
    let bound = Duration::from_secs(if cfg!(debug_assertions) { 10 } else { 2 });
    eprintln!("C6/C9 1000 rows x 4KiB: {elapsed:?}; bound {bound:?} (includes loopback TLS)");
    assert!(elapsed <= bound, "C6/C9 production wall bound");
    let next = result.continuation().expect("C6 exact budget continuation");
    let terminal = read(&source, next)
        .await
        .expect("C4 authoritative empty termination");
    assert_eq!(ids(terminal.content()), Vec::<String>::new());
    assert_eq!(terminal.continuation(), None);
    assert_eq!(listener.requests().len(), 11);
}

#[tokio::test]
async fn query_lower_row_limit_partitions_without_loss_or_over_return() {
    for limit in [999, 1000] {
        for over_return in [false, true] {
            let (listener, source, _session) = fixture_with_request_session(
                move |request| {
                    let body: Value = serde_json::from_slice(request.body()).expect("request");
                    let start = body
                        .get("nextPageToken")
                        .map(|v| {
                            v.as_str()
                                .expect("token")
                                .parse::<usize>()
                                .expect("offset in opaque fixture token")
                        })
                        .unwrap_or(0);
                    let maximum = 100.min(limit - start);
                    payload(request, JQL, maximum);
                    let returned = maximum + usize::from(over_return && start == 900);
                    response(&page(
                        (start..start + returned)
                            .map(|n| issue(&(2000 - n).to_string(), "partition"))
                            .collect(),
                        Some(&(start + returned).to_string()),
                    ))
                },
                HttpCeilings::default(),
            )
            .await;
            let source = source
                .with_jira_browse_limits_for_test(100, limit, 10)
                .expect("lower row limit");
            let result = read(&source, &reference(JQL)).await;
            if over_return {
                assert_eq!(
                    result
                        .expect_err("C6 over-return fails atomically")
                        .category(),
                    ErrorCategory::LimitExceeded
                );
            } else {
                let result = result.expect("C6 exact logical partition");
                assert_eq!(
                    ids(result.content()),
                    (0..limit)
                        .map(|n| (2000 - n).to_string())
                        .collect::<Vec<_>>()
                );
                assert!(result.continuation().is_some());
            }
            assert_eq!(listener.requests().len(), 10);
        }
    }
}

#[tokio::test]
async fn query_transport_status_and_retry_deadline_keep_shared_categories() {
    for (status, category) in [
        ("304 Not Modified", ErrorCategory::SourceUnavailable),
        ("401 Unauthorized", ErrorCategory::PermissionDenied),
        ("403 Forbidden", ErrorCategory::PermissionDenied),
        ("404 Not Found", ErrorCategory::NotFound),
        ("410 Gone", ErrorCategory::NotFound),
        ("413 Payload Too Large", ErrorCategory::LimitExceeded),
        ("429 Too Many Requests", ErrorCategory::SourceUnavailable),
        (
            "500 Internal Server Error",
            ErrorCategory::SourceUnavailable,
        ),
    ] {
        let (listener, source, _session) = fixture_with_request_session(
            move |_| protocol_response(status, [], b"private-query-canary token-canary".to_vec()),
            HttpCeilings::default(),
        )
        .await;
        let error = read(&source, &reference(JQL))
            .await
            .expect_err("C9 status refusal");
        assert_eq!(error.category(), category);
        assert!(!format!("{error:?}").contains("canary"));
        assert_eq!(listener.requests().len(), 1);
    }
    for wait in ["0", "60"] {
        let count = Arc::new(AtomicUsize::new(0));
        let attempts = count.clone();
        let (listener, source, _session) = fixture_with_request_session(
            move |_| {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    protocol_response(
                        "503 Service Unavailable",
                        [("Retry-After", wait.into())],
                        Vec::new(),
                    )
                } else {
                    response(&page(vec![issue("2", "deadline positive control")], None))
                }
            },
            HttpCeilings::new(HttpCeilingsInput {
                timeout_millis: Some(1800),
                ..Default::default()
            })
            .expect("lower deadline"),
        )
        .await;
        let result = read(&source, &reference(JQL)).await;
        if wait == "0" {
            assert_eq!(
                ids(result.expect("C9 wait within deadline").content()),
                ["2"]
            );
            assert_eq!(listener.requests().len(), 2);
        } else {
            assert_eq!(
                result
                    .expect_err("C9 wait exceeds original deadline")
                    .category(),
                ErrorCategory::SourceUnavailable
            );
            assert_eq!(listener.requests().len(), 1);
        }
    }
}

#[tokio::test]
async fn query_optional_status_and_required_identity_fields_keep_wire_contract() {
    let mut missing = issue("9", "absent status");
    missing["fields"]
        .as_object_mut()
        .expect("fields")
        .remove("status");
    let mut null = issue("2", "null status");
    null["fields"]["status"] = Value::Null;
    let (listener, source, _session) = fixture_with_request_session(
        move |request| {
            payload(request, JQL, 100);
            response(&page(
                vec![missing.clone(), null.clone(), issue("1", "present")],
                None,
            ))
        },
        HttpCeilings::default(),
    )
    .await;
    let result = read(&source, &reference(JQL))
        .await
        .expect("C5 optional status");
    assert_eq!(ids(result.content()), ["9", "2", "1"]);
    assert!(result.content().contains("absent status"));
    assert!(result.content().contains("null status"));
    assert!(result.content().contains("present"));
    assert_eq!(listener.requests().len(), 1);
    for field in ["summary", "project"] {
        for null in [false, true] {
            let mut bad = issue("2", "required summary");
            if null {
                bad["fields"][field] = Value::Null;
            } else {
                bad["fields"].as_object_mut().expect("fields").remove(field);
            }
            let (listener, source, _session) = fixture_with_request_session(
                move |_| response(&page(vec![bad.clone()], None)),
                HttpCeilings::default(),
            )
            .await;
            assert_eq!(
                read(&source, &reference(JQL))
                    .await
                    .expect_err("C5 required compact fields")
                    .category(),
                ErrorCategory::SourceUnavailable
            );
            assert_eq!(listener.requests().len(), 1);
        }
    }
}

#[tokio::test]
async fn query_cursor_reference_boundary_round_trips_or_fails_atomically() {
    let native = Arc::new(std::sync::Mutex::new("seed".to_owned()));
    let upstream = native.clone();
    let (listener, source, _session) = fixture_with_request_session(
        move |request| {
            let token = upstream.lock().expect("native fixture token");
            match payload(request, JQL, 1) {
                None => response(&page(vec![issue("9", "issuer")], Some(&token))),
                Some(received) => {
                    assert_eq!(received, *token, "C6 maximum token bytes preserved");
                    response(&page(vec![issue("2", "boundary continuation")], None))
                }
            }
        },
        HttpCeilings::default(),
    )
    .await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 1)
        .expect("force source continuation");
    let owner = reference(JQL);
    let seed = read(&source, &owner).await.expect("C6 initial issuer");
    let encoded = seed
        .continuation()
        .expect("seed cursor")
        .strip_prefix(&format!("{owner}:cursor:"))
        .expect("owner");
    let mut envelope: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).expect("base64"))
            .expect("envelope");
    let mut low = 1;
    let mut high = resourcefs_core::MAX_PATH_REFERENCE_BYTES;
    while low < high {
        let middle = (low + high).div_ceil(2);
        envelope["token"] = json!("x".repeat(middle));
        let length =
            owner.len() + ":cursor:".len() + URL_SAFE_NO_PAD.encode(envelope.to_string()).len();
        if length <= resourcefs_core::MAX_PATH_REFERENCE_BYTES {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    *native.lock().expect("token") = "x".repeat(low);
    let boundary = read(&source, &owner)
        .await
        .expect("C6 maximum representable continuation");
    let next = boundary.continuation().expect("boundary cursor");
    assert!(next.len() <= resourcefs_core::MAX_PATH_REFERENCE_BYTES);
    assert_eq!(
        ids(read(&source, next)
            .await
            .expect("C6 boundary round trip")
            .content()),
        ["2"]
    );
    *native.lock().expect("token") = "x".repeat(low + 1);
    assert_eq!(
        read(&source, &owner)
            .await
            .expect_err("C6 one-byte unrepresentable token")
            .category(),
        ErrorCategory::LimitExceeded
    );
    assert_eq!(listener.requests().len(), 4);
}
