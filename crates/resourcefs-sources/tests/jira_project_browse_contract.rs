#[path = "support/jira.rs"]
mod jira;
#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;
use jira::{fixture_with_session, offset, page, project, protocol_response, read, response};
use resourcefs_core::{ErrorCategory, HttpCeilings};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test]
async fn project_aliases_never_own_cached_identity() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (listener, source, _session) = fixture_with_session(
        move |path| {
            assert_eq!(path, "/rest/api/3/project/OLD");
            let id = if count.fetch_add(1, Ordering::SeqCst) == 0 {
                "2"
            } else {
                "10"
            };
            protocol_response(
                "200 OK",
                [("ETag", "\"alias\"".into())],
                project(id, "OLD", "Moving alias").to_string(),
            )
        },
        HttpCeilings::default(),
    )
    .await;
    let first = read(&source, "jira://acme/project-keys/OLD")
        .await
        .expect("first alias");
    let second = read(&source, "jira://acme/project-keys/OLD")
        .await
        .expect("rebound alias");
    assert_eq!(first.canonical_reference(), "jira://acme/projects/2");
    assert_eq!(second.canonical_reference(), "jira://acme/projects/10");
    assert_eq!(first.continuation(), None);
    assert_eq!(second.continuation(), None);
    assert_eq!(listener.requests().len(), 2);
    assert!(
        listener
            .heads()
            .iter()
            .all(|h| !h.to_ascii_lowercase().contains("if-none-match"))
    );
}

#[tokio::test]
async fn numeric_order_and_optional_metadata_survive_rendering() {
    let huge = "9".repeat(64);
    let mut two = project("2", "Z", "Two unique");
    two["projectTypeKey"] = serde_json::json!("");
    two["archived"] = serde_json::json!(false);
    two["deleted"] = serde_json::json!(true);
    let body = page(
        vec![
            project("10", "A", "Ten unique"),
            project(&huge, "M", "Huge unique"),
            two,
        ],
        0,
        100,
        None,
    );
    let (_listener, source, _session) =
        fixture_with_session(move |_| response(&body), HttpCeilings::default()).await;
    let result = read(&source, "jira://acme/projects")
        .await
        .expect("numeric page");
    let text = result.content();
    let positions = ["Two unique", "Ten unique", "Huge unique"]
        .map(|name| text.find(name).expect("row metadata"));
    assert!(positions.windows(2).all(|w| w[0] < w[1]));
    assert!(text.contains(&format!("jira://acme/projects/{huge}")));
    // Presence is semantic: explicit false/true/empty must not collapse to absent.
    let two_text = &text[positions[0]..positions[1]];
    assert!(two_text.contains("false") && two_text.contains("true"));
    for optional in [
        serde_json::json!({}),
        serde_json::json!({"archived":false}),
        serde_json::json!({"projectTypeKey":""}),
    ] {
        let mut row = project("2", "Z", "Same");
        row.as_object_mut()
            .expect("row")
            .extend(optional.as_object().expect("metadata").clone());
        let body = row.to_string();
        let (_l, s, _f) =
            fixture_with_session(move |_| response(&body), HttpCeilings::default()).await;
        let rendered = read(&s, "jira://acme/projects/2")
            .await
            .expect("optional shape");
        if optional.as_object().expect("metadata").is_empty() {
            assert!(!rendered.content().contains("false"));
        } else if optional.get("archived").is_some() {
            assert!(rendered.content().contains("false"));
        } else {
            assert!(
                rendered.content().contains("\"\""),
                "explicit empty text remains represented"
            );
        }
    }
}

#[tokio::test]
async fn offset_pages_follow_authoritative_state() {
    let (listener, source, _session) = fixture_with_session(
        |path| {
            let (rows, start, next, total) = match offset(path) {
                0 => (vec![project("10", "TEN", "First")], 0, Some(7), 1),
                7 => (vec![], 7, Some(19), 0),
                19 => (
                    vec![project("2", "TWO", "Beyond empty page")],
                    19,
                    None,
                    9000,
                ),
                n => panic!("invented native offset {n}"),
            };
            if start != 0 {
                assert!(path.contains("maxResults=7"));
            }
            let mut body: serde_json::Value =
                serde_json::from_str(&page(rows, start, 7, next)).expect("page");
            body["total"] = serde_json::json!(total);
            response(&body.to_string())
        },
        HttpCeilings::default(),
    )
    .await;
    let result = read(&source, "jira://acme/projects")
        .await
        .expect("response-driven traversal");
    assert!(result.content().contains("Beyond empty page"));
    assert_eq!(result.continuation(), None);
    assert_eq!(
        listener
            .requests()
            .iter()
            .map(|p| offset(p.split_whitespace().nth(1).expect("request target")))
            .collect::<Vec<_>>(),
        [0, 7, 19]
    );
    for next in [0, 6] {
        let (_l, s, _f) = fixture_with_session(
            move |_| response(&page(vec![], 7, 7, Some(next))),
            HttpCeilings::default(),
        )
        .await;
        assert_eq!(
            read(&s, "jira://acme/projects:offset:7")
                .await
                .expect_err("nonadvancing next")
                .category(),
            ErrorCategory::SourceUnavailable
        );
    }
    let (listener, source, _session) = fixture_with_session(
        |_| {
            response(&page(
                vec![project("2", "P", "Exactly full terminal page")],
                0,
                1,
                None,
            ))
        },
        HttpCeilings::default(),
    )
    .await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 1)
        .expect("exact shared boundary");
    assert_eq!(
        read(&source, "jira://acme/projects")
            .await
            .expect("authoritative termination at all three caps")
            .continuation(),
        None
    );
    assert_eq!(listener.requests().len(), 1);
}

#[tokio::test]
async fn duplicate_project_ids_across_pages_fail_atomically() {
    let (listener, source, _session) = fixture_with_session(
        |path| match offset(path) {
            0 => response(&page(
                vec![project("2", "TWO", "First page")],
                0,
                1,
                Some(1),
            )),
            1 => response(&page(
                vec![project("2", "DUP", "Second duplicate")],
                1,
                1,
                None,
            )),
            offset => panic!("unexpected native offset {offset}"),
        },
        HttpCeilings::default(),
    )
    .await;
    let error = read(&source, "jira://acme/projects")
        .await
        .expect_err("duplicate identity across pages must not produce a partial Resource");
    assert_eq!(error.category(), ErrorCategory::SourceUnavailable);
    assert_eq!(listener.requests().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_cache_revalidates_without_stale_success() {
    for reference in ["jira://acme/projects/2", "jira://acme/projects"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
        let release = Arc::new(std::sync::Barrier::new(2));
        let gate = release.clone();
        let (listener, source, session) = fixture_with_session(
            move |path| match count.fetch_add(1, Ordering::SeqCst) {
                0 => {
                    let row = project("2", "TWO", "Cached bytes");
                    let body = if path.contains("/search?") {
                        page(vec![row], 0, 100, None)
                    } else {
                        row.to_string()
                    };
                    protocol_response("200 OK", [("ETag", "\"v1\"".into())], body)
                }
                1 => protocol_response("304 Not Modified", [], Vec::new()),
                _ => {
                    entered_tx.send(()).expect("notify invalidator");
                    gate.wait();
                    protocol_response("304 Not Modified", [], Vec::new())
                }
            },
            HttpCeilings::default(),
        )
        .await;
        let first = read(&source, reference).await.expect("cache fill");
        assert_eq!(read(&source, reference).await.expect("304 reuse"), first);
        let pending = tokio::spawn(async move { read(&source, reference).await });
        entered_rx
            .recv()
            .await
            .expect("conditional request in flight");
        session
            .path_session()
            .cache_remove_namespace("atlassian.jira.acme")
            .await
            .expect("invalidate generation");
        release.wait();
        assert_eq!(
            pending
                .await
                .expect("read task")
                .expect_err("invalidated 304 cannot serve bytes")
                .category(),
            ErrorCategory::SourceUnavailable
        );
        assert!(
            listener.heads()[1..]
                .iter()
                .all(|h| h.to_ascii_lowercase().contains("if-none-match: \"v1\""))
        );
    }
}

#[tokio::test]
async fn production_scale_project_page() {
    let (listener, source, _session) = fixture_with_session(
        |path| {
            let start = offset(path);
            assert!(start < 1000);
            assert!(path.contains("maxResults=100"));
            let rows = (start + 1..=start + 100)
                .rev()
                .map(|n| {
                    let id = format!("8{n:063}");
                    project(
                        &id,
                        &format!("P{n}"),
                        &format!("record-{n:04} {}", "x".repeat(4096)),
                    )
                })
                .collect();
            response(&page(rows, start, 100, Some(start + 100)))
        },
        HttpCeilings::default(),
    )
    .await;
    let started = std::time::Instant::now();
    let result = read(&source, "jira://acme/projects")
        .await
        .expect("production-scale page");
    eprintln!(
        "production_scale_project_page elapsed: {:?}",
        started.elapsed()
    );
    assert_eq!(listener.requests().len(), 10);
    assert_eq!(
        result.continuation(),
        Some("jira://acme/projects:offset:1000")
    );
    assert_eq!(result.content().matches("record-").count(), 1000);
    let positions: Vec<_> = (1..=1000)
        .map(|n| {
            result
                .content()
                .find(&format!("record-{n:04} "))
                .expect("every row retained")
        })
        .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]));
    assert!(result.content().len() >= 1000 * 4096);
}
