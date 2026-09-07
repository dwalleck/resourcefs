#[path = "support/jira.rs"]
mod jira;
#[path = "support/mod.rs"]
mod session_support;
#[path = "support/tls.rs"]
mod tls;
use jira::{fixture_with_session, offset, page, project, read, response};
use resourcefs_core::{
    DiscoveryEngine, HttpCeilings, OperationGuard, PathReference, ReadEngine, ReadRequest,
    ResourceAddress, SearchEngine, SearchLimits, SearchOptions, SearchRequest, SearchTarget,
    ServerLimits, TextLimits,
};
use resourcefs_sources::{ArtifactSource, AtlassianSource, CompiledSources};
use std::{collections::HashSet, sync::Arc};

async fn compiled(
    source: AtlassianSource,
    session: &session_support::ScratchFixture,
) -> Arc<CompiledSources> {
    Arc::new(
        CompiledSources::new(
            session.filesystem.clone(),
            ArtifactSource::new(session.path_session().clone()),
            session.local.clone(),
            None,
            None,
            Some(source),
        )
        .await
        .expect("compiled fixture"),
    )
}

#[tokio::test]
async fn search_keeps_selected_page_identity() {
    let (listener, source, session) = fixture_with_session(
        |path| {
            let start = offset(path);
            let name = match start {
                0 => "First only",
                7 => "Second needle metadata",
                other => panic!("unexpected offset {other}"),
            };
            response(&page(
                vec![project(if start == 0 { "2" } else { "10" }, "P", name)],
                start,
                1,
                if start == 0 { Some(7) } else { None },
            ))
        },
        HttpCeilings::default(),
    )
    .await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 10)
        .expect("lower limits");
    let first = read(&source, "jira://acme/projects")
        .await
        .expect("first page");
    let selected = first.continuation().expect("actual source continuation");
    assert_eq!(selected, "jira://acme/projects:offset:7");
    let registry = compiled(source, &session).await;
    let discovery = DiscoveryEngine::new(
        registry.clone(),
        session.path_session().clone(),
        ServerLimits::default(),
    );
    for (pattern, expected_engine) in [
        ("Second needle", SearchEngine::RustRegex),
        ("(?<=Second )needle", SearchEngine::Pcre2),
    ] {
        let found = discovery
            .search(
                SearchRequest::new(
                    SearchTarget::resource(
                        PathReference::parse(selected).expect("selected reference"),
                    ),
                    pattern,
                    SearchOptions::default(),
                    0,
                    SearchLimits::default(),
                )
                .expect("search request"),
                &OperationGuard::new(),
            )
            .await
            .expect("selected search");
        assert_eq!(found.engine(), expected_engine);
        assert_eq!(found.total_records(), 1);
        assert_eq!(found.groups()[0].reference(), selected);
        assert!(
            found.groups()[0].lines()[0]
                .text()
                .contains("Second needle metadata")
        );
    }
    let reads = ReadEngine::new(
        registry,
        session.path_session().clone(),
        ServerLimits::default(),
    );
    let raw = reads
        .read(
            ReadRequest {
                reference: PathReference::parse("jira://acme/projects:raw").expect("raw"),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("ordinary raw projection");
    assert!(raw.content().contains("First only"));
    assert!(!raw.content().contains("Second needle"));
    assert_eq!(
        listener
            .requests()
            .iter()
            .map(|p| offset(p.split_whitespace().nth(1).expect("request target")))
            .collect::<Vec<_>>(),
        [0, 7, 7, 0]
    );
}

#[tokio::test]
async fn artifact_recovery_and_source_pages_remain_independent() {
    let (listener, source, session) = fixture_with_session(
        |path| {
            let start = offset(path);
            let name = if start == 0 {
                format!("Retained page {}", "long metadata ".repeat(10000))
            } else {
                "Native second page".into()
            };
            response(&page(
                vec![project(if start == 0 { "2" } else { "10" }, "P", &name)],
                start,
                1,
                if start == 0 { Some(7) } else { None },
            ))
        },
        HttpCeilings::default(),
    )
    .await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 10)
        .expect("lower limits");
    let original = read(&source, "jira://acme/projects")
        .await
        .expect("pre-spill oracle");
    let registry = compiled(source, &session).await;
    let engine = ReadEngine::new(
        registry,
        session.path_session().clone(),
        ServerLimits::default(),
    );
    let spilled = engine
        .read(
            ReadRequest {
                reference: PathReference::parse("jira://acme/projects").expect("collection"),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("spilled page");
    assert_eq!(
        spilled.source_continuation_reference(),
        Some("jira://acme/projects:offset:7")
    );
    let recovery = PathReference::parse(
        spilled
            .recovery_reference()
            .expect("complete retained artifact"),
    )
    .expect("recovery reference");
    let ResourceAddress::Artifact(address) = recovery.address() else {
        panic!("recovery must be an artifact")
    };
    let recovered = session
        .path_session()
        .read_artifact(address)
        .await
        .expect("read entire retained bytes");
    assert_eq!(recovered, original.content());
    let mut continuation = spilled.continuation_reference().map(str::to_owned);
    let mut recovered_suffix = spilled.content().to_owned();
    let mut visited = HashSet::new();
    while let Some(reference) = continuation {
        assert!(reference.starts_with("artifact://"));
        assert!(
            visited.insert(reference.clone()),
            "artifact continuation must advance"
        );
        let result = engine
            .read(
                ReadRequest {
                    reference: PathReference::parse(reference).expect("artifact continuation"),
                    limits: TextLimits::default(),
                    numbered: false,
                },
                &OperationGuard::new(),
            )
            .await
            .expect("artifact page");
        assert!(
            !result.content().is_empty(),
            "artifact continuation must expose retained bytes"
        );
        recovered_suffix.push_str(result.content());
        continuation = result.continuation_reference().map(str::to_owned);
    }
    assert_eq!(original.content(), recovered_suffix);
    let next = engine
        .read(
            ReadRequest {
                reference: PathReference::parse(
                    spilled
                        .source_continuation_reference()
                        .expect("native continuation"),
                )
                .expect("next page"),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("independent native page");
    assert!(next.content().contains("Native second page"));
    assert!(!next.content().contains("Retained page"));
    assert_eq!(next.continuation_reference(), None);
    assert_eq!(
        listener
            .requests()
            .iter()
            .map(|p| offset(p.split_whitespace().nth(1).expect("request target")))
            .collect::<Vec<_>>(),
        [0, 0, 7]
    );
}

fn issue_response(path: &str, large: bool) -> tls::FixtureResponse {
    let url = url::Url::parse(&format!("https://tls.invalid{path}")).expect("request");
    assert_eq!(url.path(), "/rest/api/3/search/jql");
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(
        pairs.get("jql").map(|value| value.as_ref()),
        Some("project IS NOT EMPTY ORDER BY key ASC")
    );
    let token = pairs.get("nextPageToken").map(|value| value.as_ref());
    let (id, summary, next) = match token {
        None => (
            "2",
            if large {
                format!("Retained issue {}", "long metadata ".repeat(10000))
            } else {
                "First issue only".into()
            },
            Some("opaque 雪 second"),
        ),
        Some("opaque 雪 second") => ("10", "Second needle metadata".into(), Some("opaque third")),
        Some("opaque third") => ("11", "Native final issue".into(), None),
        other => panic!("altered native token: {other:?}"),
    };
    let mut page = serde_json::json!({"issues":[{
        "id":id,"key":format!("P-{id}"),"self":format!("https://tls.invalid/rest/api/3/issue/{id}"),
        "fields":{"summary":summary,"project":{"id":"2","self":"https://tls.invalid/rest/api/3/project/2"}}
    }]});
    if let Some(next) = next {
        page["nextPageToken"] = serde_json::json!(next);
    }
    response(&page.to_string())
}

#[tokio::test]
async fn issue_search_keeps_selected_page_identity() {
    let (listener, source, session) =
        fixture_with_session(|path| issue_response(path, false), HttpCeilings::default()).await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 10)
        .expect("lower limits");
    let first = read(&source, "jira://acme/issues")
        .await
        .expect("first page");
    let selected = first.continuation().expect("actual cursor");
    let registry = compiled(source, &session).await;
    let discovery = DiscoveryEngine::new(
        registry.clone(),
        session.path_session().clone(),
        ServerLimits::default(),
    );
    for (pattern, expected_engine, total) in [
        ("Second needle", SearchEngine::RustRegex, 1),
        ("(?<=Second )needle", SearchEngine::Pcre2, 1),
        ("absent marker", SearchEngine::RustRegex, 0),
    ] {
        let found = discovery
            .search(
                SearchRequest::new(
                    SearchTarget::resource(
                        PathReference::parse(selected).expect("selected reference"),
                    ),
                    pattern,
                    SearchOptions::default(),
                    0,
                    SearchLimits::default(),
                )
                .expect("search"),
                &OperationGuard::new(),
            )
            .await
            .expect("selected page search");
        assert_eq!(found.engine(), expected_engine);
        assert_eq!(found.total_records(), total);
        if total == 1 {
            assert_eq!(found.groups()[0].reference(), selected);
            assert!(
                found.groups()[0].lines()[0]
                    .text()
                    .contains("Second needle metadata")
            );
        }
        assert!(
            found.continuation_reference().is_some(),
            "empty matches cannot discard native continuation"
        );
    }
    let reads = ReadEngine::new(
        registry,
        session.path_session().clone(),
        ServerLimits::default(),
    );
    let raw = reads
        .read(
            ReadRequest {
                reference: PathReference::parse("jira://acme/issues:raw").expect("raw"),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("raw first logical page");
    assert!(raw.content().contains("First issue only"));
    assert!(!raw.content().contains("Second needle"));
    let selected_read = reads
        .read(
            ReadRequest {
                reference: PathReference::parse(selected).expect("cursor"),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("selected read");
    assert!(selected_read.content().contains("Second needle metadata"));
    assert!(!selected_read.content().contains("First issue only"));
    assert_eq!(listener.requests().len(), 6);
}

#[tokio::test]
async fn issue_artifact_recovery_and_source_pages_remain_independent() {
    let (listener, source, session) =
        fixture_with_session(|path| issue_response(path, true), HttpCeilings::default()).await;
    let source = source
        .with_jira_browse_limits_for_test(1, 1, 10)
        .expect("lower limits");
    let original = read(&source, "jira://acme/issues")
        .await
        .expect("pre-spill oracle");
    let registry = compiled(source, &session).await;
    let engine = ReadEngine::new(
        registry,
        session.path_session().clone(),
        ServerLimits::default(),
    );
    let spilled = engine
        .read(
            ReadRequest {
                reference: PathReference::parse("jira://acme/issues").expect("collection"),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("spilled issue page");
    assert_eq!(
        spilled.source_continuation_reference(),
        original.continuation()
    );
    let recovery = PathReference::parse(spilled.recovery_reference().expect("retained artifact"))
        .expect("recovery");
    let ResourceAddress::Artifact(address) = recovery.address() else {
        panic!("artifact recovery required")
    };
    assert_eq!(
        session
            .path_session()
            .read_artifact(address)
            .await
            .expect("whole retained bytes"),
        original.content()
    );
    let mut continuation = spilled.continuation_reference().map(str::to_owned);
    let mut recovered = spilled.content().to_owned();
    let mut visited = HashSet::new();
    while let Some(reference) = continuation {
        assert!(reference.starts_with("artifact://"));
        assert!(
            visited.insert(reference.clone()),
            "artifact continuation must advance"
        );
        let result = engine
            .read(
                ReadRequest {
                    reference: PathReference::parse(reference).expect("artifact page"),
                    limits: TextLimits::default(),
                    numbered: false,
                },
                &OperationGuard::new(),
            )
            .await
            .expect("artifact continuation");
        recovered.push_str(result.content());
        continuation = result.continuation_reference().map(str::to_owned);
    }
    assert_eq!(recovered, original.content());
    let next = engine
        .read(
            ReadRequest {
                reference: PathReference::parse(
                    spilled
                        .source_continuation_reference()
                        .expect("independent native cursor"),
                )
                .expect("next source page"),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("native next page");
    assert!(next.content().contains("Second needle metadata"));
    assert_eq!(next.source_continuation_reference(), None);
    assert!(!next.content().contains("Retained issue"));
    let last = engine
        .read(
            ReadRequest {
                reference: PathReference::parse(
                    next.continuation_reference().expect("second native cursor"),
                )
                .expect("last page"),
                limits: TextLimits::default(),
                numbered: false,
            },
            &OperationGuard::new(),
        )
        .await
        .expect("last native page");
    assert!(last.content().contains("Native final issue"));
    assert_eq!(last.source_continuation_reference(), None);
    assert_eq!(last.continuation_reference(), None);
    assert_eq!(
        listener.requests().len(),
        4,
        "artifact reads must not perform native I/O"
    );
}
