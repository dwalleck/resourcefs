//! Real MCP service/stdio contracts, not shipped Atlassian profile launch.
//! The ignored host uses the production private constructor and transport.

#[path = "../../../resourcefs-sources/tests/support/mod.rs"]
mod session_support;
#[path = "../../tests/support/stdio.rs"]
mod stdio;
#[path = "../../../resourcefs-sources/tests/support/tls.rs"]
mod tls;
use stdio::{McpProcessPermit, finish_process, read_message_from, write_message_to};

use std::{
    io::BufReader,
    net::{IpAddr, Ipv4Addr},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
};

use resourcefs_core::{
    AllowedOrigin, AtlassianSiteId, HttpCeilings, JiraAddress, JiraQuery, OriginAllowlist,
    PathReference, Secret,
};
use resourcefs_sources::{AtlassianSite, AtlassianSourceMount, HttpSubstrate, OriginCredential};
use serde_json::{Value, json};

use super::*;

const HOST: &str = "server::jira_query_tests::jira_query_stdio_test_host";
const HOST_GATE: &str = "RESOURCEFS_JIRA_QUERY_TEST_HOST";
const LIVE_HOST: &str = "RESOURCEFS_JIRA_QUERY_LIVE_HOST";

// Query-specific commands use the same framing, admission and EOF lifecycle
// as the production-binary stdio contracts.
struct QueryProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    id: u64,
    origin: String,
    _receipt: tempfile::TempDir,
    _process_permit: McpProcessPermit,
}

impl QueryProcess {
    fn start(live: bool) -> Self {
        let receipt = tempfile::TempDir::new().expect("host fixture receipt");
        let permit = McpProcessPermit::acquire();
        let mut child = Command::new(std::env::current_exe().expect("libtest executable"))
            .args([
                "--ignored",
                "--exact",
                HOST,
                "--nocapture",
                "--test-threads=1",
            ])
            .env(HOST_GATE, "1")
            .env(LIVE_HOST, if live { "1" } else { "0" })
            .env(
                "RESOURCEFS_JIRA_QUERY_ORIGIN_RECEIPT",
                receipt.path().join("origin"),
            )
            .env_remove("ATLASSIAN_EMAIL")
            .env_remove("ATLASSIAN_API_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start query stdio host");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("host stdout"));
        let mut process = Self {
            child,
            stdin,
            stdout,
            id: 0,
            origin: String::new(),
            _receipt: receipt,
            _process_permit: permit,
        };
        let initialized = process.request(
            "initialize",
            json!({
                "protocolVersion": "2026-07-28", "capabilities": {},
                "clientInfo": {"name": "jira-query-contract", "version": "1"}
            }),
        );
        assert!(initialized.get("result").is_some(), "initialize succeeds");
        process.write(&json!({"jsonrpc":"2.0", "method":"notifications/initialized"}));
        if !live {
            process.origin = std::fs::read_to_string(process._receipt.path().join("origin"))
                .expect("independent fixture origin");
        }
        process
    }

    fn write(&mut self, value: &Value) {
        write_message_to(&mut self.stdin, value);
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.write(&json!({"jsonrpc":"2.0", "id":self.id, "method":method, "params":params}));
        loop {
            let response = read_message_from(&mut self.stdout, true);
            if response.get("id").is_none() {
                continue;
            }
            assert_eq!(response["id"], self.id);
            return response;
        }
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.request("tools/call", json!({"name":name, "arguments":arguments}));
        assert!(
            response.get("error").is_none(),
            "MCP tool protocol succeeds"
        );
        let result = &response["result"];
        let output = result["structuredContent"].clone();
        assert_eq!(
            result["isError"].as_bool().unwrap_or(false),
            output["ok"] == false
        );
        output
    }

    fn read(&mut self, path: &str) -> Value {
        self.call("rfs_read", json!({"path":path}))
    }

    fn finish(mut self) {
        finish_process(&mut self.child, &mut self.stdin, &mut self.stdout, true);
    }
}

fn reference(site: &str, query: &str) -> String {
    PathReference::jira(
        JiraAddress::Query {
            site: AtlassianSiteId::new(site).expect("fixture site ID"),
            query: JiraQuery::new(query).expect("nonempty query"),
        },
        None,
    )
    .expect("query reference")
    .requested()
    .to_owned()
}

fn summary(id: usize) -> String {
    format!("needle-{id} mirror mirror")
}

fn issue(id: usize) -> Value {
    json!({
        "id":id.to_string(), "key":format!("Q-{id}"),
        "self":format!("https://tls.invalid/rest/api/3/issue/{id}"),
        "fields": {"summary":summary(id), "project": {
            "id":"10", "key":"Q", "self":"https://tls.invalid/rest/api/3/project/10"
        }, "status":{"name":"In Progress"}}
    })
}

// Frozen content oracle assembled without the production decoder or renderer.
fn expected_page(origin: &str, path: &str, ids: impl IntoIterator<Item = usize>) -> String {
    let mut output = format!("# Jira Issues\n\nCanonical Reference: {path}\n\n");
    for id in ids {
        output.push_str(&format!(
            "## [\"Q-{id}\"](jira://test/issues/{id})\n\nIssue ID: {id}\nIssue Key: \"Q-{id}\"\nSummary: \"{}\"\nSelf: \"{origin}/rest/api/3/issue/{id}\"\nProject: [10](jira://test/projects/10)\nStatus: {{name: \"In Progress\"}}\nReference: jira://test/issues/{id}\n\n", summary(id)
        ));
    }
    output
}

fn fixture_response(request: &tls::FixtureRequest) -> tls::FixtureResponse {
    assert_eq!(request.target(), "/rest/api/3/search/jql", "query endpoint");
    let body: Value = serde_json::from_slice(request.body()).expect("query request JSON");
    let query = body["jql"].as_str().expect("native JQL");
    let step = body.get("nextPageToken").map_or(0, |token| {
        token
            .as_str()
            .expect("native token")
            .parse::<usize>()
            .expect("fixture token")
    });
    let (ids, next): (Vec<usize>, Option<usize>) = match query {
        "terminal" => (vec![1, 2], None),
        "bulk" => {
            let start = step * 100 + 1;
            let end = (start + 99).min(1001);
            ((start..=end).collect(), (end < 1001).then_some(step + 1))
        }
        "sparse" if step < 9 => (Vec::new(), Some(step + 1)),
        "sparse" if step == 9 => (vec![1], Some(10)),
        "sparse" => {
            let start = (step - 10) * 100 + 2;
            let end = (start + 99).min(1002);
            ((start..=end).collect(), (end < 1002).then_some(step + 1))
        }
        _ => panic!("unknown deterministic fixture query"),
    };
    let mut page =
        json!({"issues":ids.into_iter().map(issue).collect::<Vec<_>>(), "isLast":next.is_none()});
    if let Some(next) = next {
        page["nextPageToken"] = json!(next.to_string());
    }
    tls::FixtureResponse::Response {
        status: "200 OK",
        headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body: serde_json::to_vec(&page).expect("fixture page"),
    }
}

#[test]
#[ignore = "reexecuted stdio helper; not a live or standalone contract"]
fn jira_query_stdio_test_host() {
    if std::env::var(HOST_GATE).as_deref() != Ok("1") {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("host runtime")
        .block_on(async {
            let fixture = session_support::scratch_fixture().await;
            let live = std::env::var(LIVE_HOST).as_deref() == Ok("1");
            let (site_id, origin, substrate, _listener) = if live {
                assert_eq!(
                    std::env::var("RFS_LIVE").as_deref(),
                    Ok("1"),
                    "explicit live gate"
                );
                let origin = AllowedOrigin::new(
                    &std::env::var("RFS_ATLASSIAN_SITE").expect("site URL"),
                    false,
                )
                .expect("live origin");
                let token =
                    Secret::new(std::env::var("ATLASSIAN_READER_API_TOKEN").expect("reader token"))
                        .expect("reader secret");
                let credential = OriginCredential::basic(
                    origin.clone(),
                    &std::env::var("ATLASSIAN_READER_EMAIL").expect("reader email"),
                    &token,
                )
                .expect("reader credential");
                let substrate = HttpSubstrate::new(
                    OriginAllowlist::new(vec![origin.clone()]),
                    HttpCeilings::default(),
                    vec![credential],
                )
                .expect("live substrate");
                ("live".to_owned(), origin, substrate, None)
            } else {
                let port = Arc::new(AtomicU16::new(0));
                let observed_port = Arc::clone(&port);
                let listener = tls::TlsListener::serve_request_router(
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    0,
                    tls::match_cert(),
                    move |request| {
                        let mut response = fixture_response(request);
                        if let tls::FixtureResponse::Response { body, .. } = &mut response {
                            *body = String::from_utf8(body.clone())
                                .expect("fixture JSON UTF-8")
                                .replace(
                                    "https://tls.invalid/",
                                    &format!(
                                        "https://tls.invalid:{}/",
                                        observed_port.load(Ordering::Acquire)
                                    ),
                                )
                                .into_bytes();
                        }
                        response
                    },
                )
                .await;
                port.store(listener.address.port(), Ordering::Release);
                std::fs::write(
                    std::env::var("RESOURCEFS_JIRA_QUERY_ORIGIN_RECEIPT")
                        .expect("fixture receipt path"),
                    format!("https://tls.invalid:{}", listener.address.port()),
                )
                .expect("record independent fixture origin");
                let allowlist = tls::fixture_allowlist(listener.address.port(), true);
                let origin = AllowedOrigin::new(
                    &format!("https://tls.invalid:{}", listener.address.port()),
                    true,
                )
                .expect("fixture origin");
                let substrate =
                    tls::tls_substrate(allowlist, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]);
                ("test".to_owned(), origin, substrate, Some(listener))
            };
            let site = AtlassianSite::new(AtlassianSiteId::new(site_id).expect("site ID"), origin)
                .expect("site mount");
            let atlassian = AtlassianSourceMount::new(vec![site], Arc::new(substrate))
                .expect("source mount")
                .bind(fixture.path_session().clone());
            let compiled = Arc::new(
                CompiledSources::new(
                    fixture.filesystem.clone(),
                    ArtifactSource::new(fixture.path_session().clone()),
                    fixture.local.clone(),
                    None,
                    None,
                    Some(atlassian),
                )
                .await
                .expect("compiled sources"),
            );
            let session = fixture.path_session().clone();
            let limits = ServerLimits::default();
            let reads = ReadEngine::new(compiled.clone(), session.clone(), limits);
            let mutations = MutationEngine::new(compiled.clone(), session.clone());
            let discovery = DiscoveryEngine::new(compiled, session.clone(), limits);
            let disconnect = Arc::new(DisconnectState::new(session));
            let cancellations = Arc::new(RequestCancellations::default());
            let (stdin, stdout) = rmcp::transport::stdio();
            let stdio =
                rmcp::transport::async_rw::AsyncRwTransport::<RoleServer, _, _>::new(stdin, stdout);
            let transport = DisconnectTransport::new(
                stdio,
                Arc::clone(&disconnect),
                Arc::clone(&cancellations),
            );
            let running = ResourceFsServer::new(
                fixture.filesystem.clone(),
                reads,
                mutations,
                discovery,
                cancellations,
            )
            .serve(transport)
            .await
            .expect("serve MCP host");
            running.waiting().await.expect("MCP service completes");
            disconnect.finish().await.expect("disconnect session");
            fixture
                .session
                .mark_disconnected()
                .await
                .expect("retain disconnected session");
        });
}

fn recover(process: &mut QueryProcess, output: &Value) -> String {
    let Some(recovery) = output["recoveryReference"].as_str() else {
        return output["content"]
            .as_str()
            .expect("displayed content")
            .to_owned();
    };
    let mut page = process.read(recovery);
    let mut content = String::new();
    for _ in 0..100 {
        assert_eq!(page["ok"], true, "C10 artifact read succeeds");
        content.push_str(page["content"].as_str().expect("artifact content"));
        let Some(next) = page["continuationReference"].as_str() else {
            return content;
        };
        assert!(
            next.starts_with("artifact://"),
            "C10 content recovery stays in artifact chain"
        );
        page = process.read(next);
    }
    panic!("C10 bounded artifact traversal completes");
}

#[test]
fn c10_query_recovery_and_source_continuation_are_independent() {
    let mut process = QueryProcess::start(false);
    for (query, clipped, nonterminal, ids) in [
        ("terminal", false, false, 1..=2),
        ("terminal", true, false, 1..=2),
        ("sparse", false, true, 1..=1),
        ("bulk", true, true, 1..=1000),
    ] {
        let path = reference("test", query);
        let arguments = if clipped {
            json!({"path":path, "limits":{"lines":3}})
        } else {
            json!({"path":path})
        };
        let output = process.call("rfs_read", arguments);
        assert_eq!(output["ok"], true, "C10 mounted query read succeeds");
        assert_eq!(
            output["recoveryReference"].is_string(),
            clipped,
            "C10 clipping cell"
        );
        let source = if clipped {
            &output["sourceContinuationReference"]
        } else {
            &output["continuationReference"]
        };
        assert_eq!(
            source.is_string(),
            nonterminal,
            "C10 independent source continuation cell"
        );
        assert_eq!(
            recover(&mut process, &output),
            expected_page(&process.origin, &path, ids),
            "C10 complete current-page content ledger"
        );
        if let Some(next) = source.as_str() {
            assert!(next.starts_with("jira://"), "C10 next source identity");
            let next_output = process.read(next);
            let next_content = recover(&mut process, &next_output);
            let expected = if query == "bulk" {
                1001..=1001
            } else {
                2..=1001
            };
            assert_eq!(
                next_content,
                expected_page(&process.origin, &path, expected),
                "C10 next-page identity ledger differs from recovered page"
            );
            if query == "bulk" {
                assert!(next_output.get("sourceContinuationReference").is_none());
                assert!(
                    next_output.get("continuationReference").is_none(),
                    "C10 terminal successor"
                );
            } else {
                let last = next_output["sourceContinuationReference"]
                    .as_str()
                    .expect("C10 sparse fixture still advances");
                let final_output = process.read(last);
                assert_eq!(
                    recover(&mut process, &final_output),
                    expected_page(&process.origin, &path, [1002]),
                    "C10 sparse traversal covers more than 1000 rows"
                );
            }
        }
    }
    process.finish();
}

#[test]
fn c11_query_search_uses_rendered_regex_and_pcre2_and_glob_is_unsupported() {
    let mut process = QueryProcess::start(false);
    let path = reference("test", "terminal");
    let read = process.read(&path);
    assert_eq!(read["ok"], true, "C11 positive mounted read");
    let expected = expected_page(&process.origin, &path, 1..=2);
    assert_eq!(
        read["content"], expected,
        "C11 independent rendered-content control"
    );
    let lines = expected
        .lines()
        .enumerate()
        .filter(|(_, line)| line.starts_with("Summary: "))
        .map(|(index, text)| json!({"line":index+1, "text":text}))
        .collect::<Vec<_>>();
    for (pattern, engine) in [
        (r#"^Summary: "needle-[12] mirror mirror"$"#, "rust_regex"),
        (r#"^Summary: "needle-[12] (mirror) \1"$"#, "pcre2"),
    ] {
        let output = process.call("rfs_search", json!({"path":path, "pattern":pattern}));
        assert_eq!(output["ok"], true, "C11 rendered query search succeeds");
        assert_eq!(
            output["engine"], engine,
            "C11 existing regex engine selection"
        );
        assert_eq!(
            output["groups"],
            json!([{"reference":path, "lines":lines}]),
            "C11 independently computed match regions"
        );
    }
    let no_matches = process.call(
        "rfs_search",
        json!({"path":path, "pattern":"never-present-needle"}),
    );
    assert_eq!(no_matches["ok"], true, "C11 no-match search succeeds");
    assert_eq!(no_matches["groups"], json!([]), "C11 exact no-match oracle");
    let invalid = process.call("rfs_search", json!({"path":path, "pattern":"["}));
    assert_eq!(
        invalid["error"]["category"], "invalid_pattern",
        "C11 regex category"
    );
    let glob = process.call("rfs_glob", json!({"path":"jira://test/**"}));
    assert_eq!(
        glob["error"]["category"], "unsupported_projection",
        "C11 Atlassian glob category"
    );
    process.finish();
}

#[test]
#[ignore = "reader-only live Jira query MCP stdio; requires owned fixture receipt and RFS_LIVE=1"]
fn live_jira_query_stdio() {
    if std::env::var("RFS_LIVE").as_deref() != Ok("1") {
        eprintln!("RFS_LIVE is not 1; skipping live Jira query stdio");
        return;
    }
    for name in [
        "RFS_ATLASSIAN_SITE",
        "ATLASSIAN_READER_EMAIL",
        "ATLASSIAN_READER_API_TOKEN",
    ] {
        if std::env::var(name).is_err() {
            eprintln!("missing reader-only environment {name}; skipping live Jira query stdio");
            return;
        }
    }
    let receipt = std::env::var("ATLASSIAN_FIXTURE_RECEIPT")
        .unwrap_or_else(|_| ".resourcefs/atlassian-fixture-state.json".to_owned());
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(receipt).expect("owned fixture receipt"))
            .expect("fixture receipt JSON");
    assert!(
        receipt["site"]["origin"].as_str()
            == Some(
                std::env::var("RFS_ATLASSIAN_SITE")
                    .expect("site URL")
                    .trim_end_matches('/')
            ),
        "receipt belongs to configured origin"
    );
    let project = receipt["jira"]["projects"][0]["key"]
        .as_str()
        .expect("owned project key");
    let mut issues = receipt["jira"]["issues"]
        .as_array()
        .expect("owned issues")
        .iter()
        .filter(|issue| {
            issue["key"]
                .as_str()
                .expect("issue key")
                .starts_with(&format!("{project}-"))
        })
        .collect::<Vec<_>>();
    issues.sort_by_key(|issue| {
        issue["key"]
            .as_str()
            .expect("key")
            .rsplit('-')
            .next()
            .expect("key suffix")
            .parse::<u64>()
            .expect("numeric key suffix")
    });
    assert!(
        (2..=10).contains(&issues.len()),
        "owned receipt must have 2..10 query rows"
    );
    let site = "live";
    let mut process = QueryProcess::start(true);
    for descending in [false, true] {
        let ordered = if descending {
            issues.iter().rev().copied().collect::<Vec<_>>()
        } else {
            issues.clone()
        };
        let jql = format!(
            "project = {project} ORDER BY key {}",
            if descending { "DESC" } else { "ASC" }
        );
        let path = reference(site, &jql);
        let output = process.read(&path);
        assert_eq!(output["ok"], true, "live query succeeds");
        let content = recover(&mut process, &output);
        let expected_ids = ordered
            .iter()
            .map(|issue| {
                format!(
                    "Reference: jira://{site}/issues/{}",
                    issue["id"].as_str().expect("owned stable ID")
                )
            })
            .collect::<Vec<_>>();
        let actual_ids = content
            .lines()
            .filter(|line| line.starts_with("Reference: jira://"))
            .collect::<Vec<_>>();
        assert!(
            actual_ids == expected_ids,
            "live native order agrees with owned receipt"
        );
        assert!(
            output.get("sourceContinuationReference").is_none(),
            "owned query is terminal"
        );
        for issue in &ordered {
            let id = issue["id"].as_str().expect("issue ID");
            let direct = process.read(&format!("jira://{site}/issues/{id}/fields/summary"));
            assert_eq!(direct["ok"], true, "live direct selected field read");
            let summary: String =
                serde_json::from_str(direct["content"].as_str().expect("field content"))
                    .expect("summary JSON");
            let row = format!(
                "Summary: {}",
                serde_json::to_string(&summary).expect("summary oracle")
            );
            assert!(
                content.lines().any(|line| line == row),
                "query metadata agrees with direct field"
            );
        }
        let search = process.call("rfs_search", json!({"path":path, "pattern":"^Summary: "}));
        assert_eq!(search["ok"], true, "live rendered-content search");
        assert_eq!(
            search["groups"][0]["lines"]
                .as_array()
                .expect("matches")
                .len(),
            ordered.len(),
            "live summary match count"
        );
    }
    let rejected = process.read(&reference(site, "project ="));
    assert_eq!(
        rejected["error"]["category"], "invalid_pattern",
        "live native rejection category"
    );
    process.finish();
}
