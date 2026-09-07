#[path = "support/jira.rs"]
mod jira;
#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jira::{fixture_with_session, project, protocol_response, read, response};
use resourcefs_core::{ErrorCategory, HttpCeilings, PathReference};
use serde_json::{Value, json};

fn issue(id: &str, parent: &str, summary: &str) -> Value {
    json!({"id":id,"key":format!("P-{id}"),"self":format!("https://tls.invalid/rest/api/3/issue/{id}"),
        "fields":{"summary":summary,"project":{"id":parent,"self":format!("https://tls.invalid/rest/api/3/project/{parent}")}}})
}

fn issue_page(rows: Vec<Value>, token: Option<&str>) -> String {
    let mut page = json!({"issues":rows});
    if let Some(token) = token {
        page["nextPageToken"] = json!(token);
    }
    page.to_string()
}

fn query(path: &str, name: &str) -> Option<String> {
    let url = url::Url::parse(&format!("https://tls.invalid{path}")).expect("request URL");
    assert_eq!(url.path(), "/rest/api/3/search/jql");
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

#[tokio::test]
async fn opaque_pages_preserve_parent_membership() {
    let (listener, source, _session) = fixture_with_session(
        |path| {
            if path == "/rest/api/3/project/2" {
                return response(&project("2", "P", "Parent").to_string());
            }
            assert_eq!(
                query(path, "jql").as_deref(),
                Some("project = 2 ORDER BY key ASC")
            );
            match query(path, "nextPageToken").as_deref() {
                None => response(&issue_page(
                    vec![issue("10", "2", "Before empty page")],
                    Some("雪 opaque +/& token"),
                )),
                Some("雪 opaque +/& token") => {
                    response(&issue_page(vec![], Some("after empty Ω")))
                }
                Some("after empty Ω") => {
                    response(&issue_page(vec![issue("2", "2", "After empty page")], None))
                }
                other => panic!("token changed: {other:?}"),
            }
        },
        HttpCeilings::default(),
    )
    .await;
    let result = read(&source, "jira://acme/projects/2/issues")
        .await
        .expect("opaque traversal");
    assert_eq!(
        result.canonical_reference(),
        "jira://acme/projects/2/issues"
    );
    assert!(result.content().contains("Before empty page"));
    assert!(result.content().contains("After empty page"));
    assert_eq!(result.continuation(), None);
    assert_eq!(listener.requests().len(), 4);
    assert!(listener.requests()[0].contains("/rest/api/3/project/2 "));

    let (listener, source, _session) = fixture_with_session(
        |_| protocol_response("404 Not Found", [], Vec::new()),
        HttpCeilings::default(),
    )
    .await;
    assert_eq!(
        read(&source, "jira://acme/projects/2/issues")
            .await
            .expect_err("missing parent")
            .category(),
        ErrorCategory::NotFound
    );
    assert_eq!(listener.requests().len(), 1);
    assert!(listener.requests()[0].contains("/rest/api/3/project/2 "));

    for bad_parent in ["3", "2"] {
        let (listener, source, _session) = fixture_with_session(
            move |path| {
                if path == "/rest/api/3/project/2" {
                    return response(&project("2", "P", "Parent").to_string());
                }
                let mut row = issue("10", bad_parent, "Must not escape");
                if bad_parent == "2" {
                    row["fields"]
                        .as_object_mut()
                        .expect("fields")
                        .remove("project");
                }
                response(&issue_page(vec![row], None))
            },
            HttpCeilings::default(),
        )
        .await;
        assert_eq!(
            read(&source, "jira://acme/projects/2/issues")
                .await
                .expect_err("invalid parent membership")
                .category(),
            ErrorCategory::SourceUnavailable
        );
        assert_eq!(listener.requests().len(), 2);
    }
}

#[tokio::test]
async fn cursor_replay_is_rejected_before_egress() {
    let (listener, source, _session) = fixture_with_session(
        |path| {
            if path == "/rest/api/3/project/2" {
                return response(&project("2", "P", "Parent").to_string());
            }
            response(&issue_page(
                vec![issue("2", "2", "Cursor owner")],
                Some("opaque 雪 token"),
            ))
        },
        HttpCeilings::default(),
    )
    .await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 2)
        .expect("lower limits");
    let first = read(&source, "jira://acme/projects/2/issues")
        .await
        .expect("cursor issuer");
    let selected = first.continuation().expect("real cursor");
    assert!(
        PathReference::parse(selected)
            .expect("typed cursor")
            .projection()
            .expect("selector")
            .source_cursor()
            .is_some()
    );
    let selector = selected
        .strip_prefix("jira://acme/projects/2/issues")
        .expect("canonical owner");
    for owner in [
        "jira://acme/projects/3/issues",
        "jira://acme/issues",
        "jira://acme/projects",
    ] {
        let replay = format!("{owner}{selector}");
        match PathReference::parse(&replay) {
            Err(error) => assert_eq!(error.category(), ErrorCategory::InvalidReference),
            Ok(_) => assert_eq!(
                read(&source, &replay)
                    .await
                    .expect_err("foreign cursor owner")
                    .category(),
                ErrorCategory::InvalidReference
            ),
        }
    }
    assert_eq!(
        listener.requests().len(),
        2,
        "replay must not even look up a parent"
    );
    let (destination, rebound, _session) = fixture_with_session(
        |_| panic!("rebound cursor caused egress"),
        HttpCeilings::default(),
    )
    .await;
    assert_eq!(
        read(&rebound, selected)
            .await
            .expect_err("same site rebound to another origin")
            .category(),
        ErrorCategory::InvalidReference
    );
    assert!(destination.requests().is_empty());
    // e30 is canonical unpadded base64url for {}, but not a valid ownership envelope.
    for suffix in ["e30", "e30=", "not+base64", "", "e30:raw"] {
        let malformed = format!("jira://acme/projects/2/issues:cursor:{suffix}");
        match PathReference::parse(&malformed) {
            Err(error) => assert_eq!(error.category(), ErrorCategory::InvalidReference),
            Ok(_) => assert_eq!(
                read(&source, &malformed)
                    .await
                    .expect_err("malformed cursor")
                    .category(),
                ErrorCategory::InvalidReference
            ),
        }
    }
    // Deliberately foreign/malformed envelopes use a real issuer's origin so that
    // ownership and strict shape, rather than an unrelated origin error, reject them.
    let encoded = selector.strip_prefix(":cursor:").expect("cursor selector");
    let envelope: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).expect("issued encoding"))
            .expect("issued envelope");
    for patch in [
        json!({"owner":"jira://other/projects/2/issues"}),
        json!({"owner":"jira://acme/projects/2/issues:raw"}),
        json!({"version":2}),
        json!({"extra":true}),
        json!({"token":null}),
        json!({"token":""}),
    ] {
        let mut malformed = envelope.clone();
        malformed
            .as_object_mut()
            .expect("envelope")
            .extend(patch.as_object().expect("patch").clone());
        let reference = format!(
            "jira://acme/projects/2/issues:cursor:{}",
            URL_SAFE_NO_PAD.encode(malformed.to_string())
        );
        assert_eq!(
            read(&source, &reference)
                .await
                .expect_err("strict cursor envelope")
                .category(),
            ErrorCategory::InvalidReference
        );
    }
    let duplicate = format!(
        "{{\"owner\":{},{}",
        envelope["owner"],
        envelope.to_string().strip_prefix('{').expect("object")
    );
    let reference = format!(
        "jira://acme/projects/2/issues:cursor:{}",
        URL_SAFE_NO_PAD.encode(duplicate)
    );
    assert_eq!(
        read(&source, &reference)
            .await
            .expect_err("duplicate envelope owner")
            .category(),
        ErrorCategory::InvalidReference
    );
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test]
async fn selected_fields_enable_canonical_rows() {
    let (listener, source, _session) = fixture_with_session(
        |path| {
            assert_eq!(
                query(path, "jql").as_deref(),
                Some("project IS NOT EMPTY ORDER BY key ASC")
            );
            let fields = query(path, "fields").unwrap_or_default();
            let mut selected: Vec<_> = fields.split(',').collect();
            selected.sort_unstable();
            if selected == ["key", "project", "status", "summary"] {
                response(&issue_page(
                    vec![issue("2", "10", "Selected summary")],
                    None,
                ))
            } else {
                response(&json!({"issues":[{"id":"2"}]}).to_string())
            }
        },
        HttpCeilings::default(),
    )
    .await;
    let result = read(&source, "jira://acme/issues")
        .await
        .expect("selected fields make canonical rows possible");
    assert!(result.content().contains("jira://acme/issues/2"));
    assert!(result.content().contains("Selected summary"));
    assert!(result.content().contains("jira://acme/projects/10"));
    assert_eq!(listener.requests().len(), 1);
}

#[tokio::test]
async fn numeric_order_and_optional_metadata_survive_rendering() {
    let huge = "9".repeat(64);
    let body = issue_page(
        vec![
            issue("10", "2", "Ten unique"),
            issue(&huge, "2", "Huge unique"),
            issue("2", "2", "Two unique"),
        ],
        None,
    );
    let (_listener, source, _session) =
        fixture_with_session(move |_| response(&body), HttpCeilings::default()).await;
    let result = read(&source, "jira://acme/issues")
        .await
        .expect("numeric page");
    let positions = ["Two unique", "Ten unique", "Huge unique"]
        .map(|name| result.content().find(name).expect("metadata"));
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(
        result
            .content()
            .contains(&format!("jira://acme/issues/{huge}"))
    );
    let mut rendered = Vec::new();
    for status in [
        None,
        Some(Value::Null),
        Some(json!({})),
        Some(json!({"name":null})),
        Some(json!({"name":""})),
        Some(json!({"name":"Ready 雪"})),
    ] {
        let mut row = issue("2", "2", "Same summary");
        if let Some(status) = status {
            row["fields"]["status"] = status;
        }
        let body = issue_page(vec![row], None);
        let (_listener, source, _session) =
            fixture_with_session(move |_| response(&body), HttpCeilings::default()).await;
        rendered.push(
            read(&source, "jira://acme/issues")
                .await
                .expect("optional status shape")
                .content()
                .to_owned(),
        );
    }
    for left in 0..rendered.len() {
        for right in left + 1..rendered.len() {
            assert_ne!(
                rendered[left], rendered[right],
                "status presence shapes {left}/{right} must remain distinct"
            );
        }
    }
    assert!(rendered[5].contains("Ready 雪"));
}

#[tokio::test]
async fn issue_terminal_state_is_authoritative() {
    for markers in [json!({}), json!({"isLast":true})] {
        let mut body = json!({"issues":[]});
        body.as_object_mut()
            .expect("page")
            .extend(markers.as_object().expect("markers").clone());
        let body = body.to_string();
        let (listener, source, _session) =
            fixture_with_session(move |_| response(&body), HttpCeilings::default()).await;
        assert_eq!(
            read(&source, "jira://acme/issues")
                .await
                .expect("token absence terminates")
                .continuation(),
            None
        );
        assert_eq!(listener.requests().len(), 1);
    }
    for markers in [
        json!({"isLast":false}),
        json!({"isLast":true,"nextPageToken":"next"}),
        json!({"nextPageToken":""}),
        json!({"nextPageToken":null}),
        json!({"isLast":null}),
    ] {
        let mut body = json!({"issues":[]});
        body.as_object_mut()
            .expect("page")
            .extend(markers.as_object().expect("markers").clone());
        let body = body.to_string();
        let (listener, source, _session) =
            fixture_with_session(move |_| response(&body), HttpCeilings::default()).await;
        assert_eq!(
            read(&source, "jira://acme/issues")
                .await
                .expect_err("invalid terminal markers")
                .category(),
            ErrorCategory::SourceUnavailable
        );
        assert_eq!(listener.requests().len(), 1);
    }
    let (listener, source, _session) = fixture_with_session(
        |_| response(&issue_page(vec![], Some("repeated"))),
        HttpCeilings::default(),
    )
    .await;
    assert_eq!(
        read(&source, "jira://acme/issues")
            .await
            .expect_err("repeated token")
            .category(),
        ErrorCategory::SourceUnavailable
    );
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test]
async fn production_scale_issue_page() {
    let (listener, source, _session) = fixture_with_session(
        |path| {
            assert_eq!(query(path, "maxResults").as_deref(), Some("100"));
            let start: usize = query(path, "nextPageToken").map_or(0, |value| {
                value
                    .strip_prefix("opaque-")
                    .expect("native token")
                    .parse()
                    .expect("fixture index")
            });
            assert!(start < 1000, "no full-tenant scan");
            let rows = (start + 1..=start + 100)
                .rev()
                .map(|n| {
                    issue(
                        &format!("8{n:063}"),
                        "2",
                        &format!("record-{n:04} {}", "x".repeat(4096)),
                    )
                })
                .collect();
            response(&issue_page(rows, Some(&format!("opaque-{}", start + 100))))
        },
        HttpCeilings::default(),
    )
    .await;
    let started = std::time::Instant::now();
    let result = read(&source, "jira://acme/issues")
        .await
        .expect("production-scale page");
    eprintln!(
        "production_scale_issue_page elapsed: {:?}",
        started.elapsed()
    );
    assert_eq!(listener.requests().len(), 10);
    assert_eq!(result.content().matches("record-").count(), 1000);
    let positions: Vec<_> = (1..=1000)
        .map(|n| {
            result
                .content()
                .find(&format!("record-{n:04} "))
                .expect("every row retained")
        })
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(result.content().len() >= 1000 * 4096);
    assert!(result.content().len() < 1000 * 8192);
    let next = result
        .continuation()
        .expect("budget stop retains continuation");
    assert!(next.len() <= 65536);
    assert!(
        PathReference::parse(next)
            .expect("bounded canonical cursor")
            .projection()
            .expect("source selector")
            .source_cursor()
            .is_some()
    );
}
