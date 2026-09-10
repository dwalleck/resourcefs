//! Live full-stack smoke: Server Profile → `rfs check --probe` → `rfs serve`
//! → tools over stdio, against real upstreams.
//!
//! The adapter-level live smokes prove the Source Adapters; this one proves
//! the path an operator actually runs — profile parsing, credential
//! resolution from the environment, the startup probe, mounting, and tool
//! result rendering — with the real `rfs` binary and real GitHub and HTTPS
//! origins, GET only.
//!
//! Ignored by default; needs `RFS_LIVE=1` and `GITHUB_TOKEN`.
//! `scripts/live-smoke.sh` supplies both.
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use serde_json::{Value, json};
use tempfile::TempDir;

const PROTOCOL_VERSION: &str = "2026-07-28";
const SMALL_PR: &str = "pr://rust-lang/rust/159232";
const MULTIPAGE_PR: &str = "pr://rust-lang/rust/159628";
const BOOK_PAGE: &str = "https://doc.rust-lang.org/stable/book/ch01-01-installation.html";

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_resourcefs")
}

fn live_token() -> Option<String> {
    if std::env::var_os("RFS_LIVE").is_none_or(|value| value != "1") {
        eprintln!("RFS_LIVE is not 1; skipping the live stdio smoke");
        return None;
    }
    let token = std::env::var("GITHUB_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());
    if token.is_none() {
        eprintln!("GITHUB_TOKEN is not set; skipping the live stdio smoke");
    }
    token
}

fn native_gh(token: &str, endpoint: &str) -> Option<Value> {
    let native = match Command::new("gh")
        .args(["api", "-H", "X-GitHub-Api-Version: 2022-11-28", endpoint])
        .env("GH_TOKEN", token)
        .output()
    {
        Ok(native) => native,
        Err(error) => {
            eprintln!("gh CLI is unavailable ({error}); skipping native cross-check");
            return None;
        }
    };
    assert!(native.status.success(), "native read failed");
    Some(serde_json::from_slice(&native.stdout).expect("native JSON"))
}

/// One GitHub source and one HTTPS source, both required, so a probe failure
/// is a startup failure rather than a silently degraded mount.
fn write_profile(directory: &Path) -> PathBuf {
    write_profile_with_attempts(directory, None)
}

fn write_profile_with_attempts(directory: &Path, max_attempts: Option<u64>) -> PathBuf {
    let path = directory.join("live.json");
    let mut github = json!({
        "kind": "github",
        "id": "forge",
        "required": true,
        "allowPrivateNetwork": false,
        "credential": {"kind": "environment", "name": "GITHUB_TOKEN"},
        "repositories": [{"name": "rust-lang/rust"}]
    });
    if let Some(max_attempts) = max_attempts {
        github["acquisition"] = json!({"maxAttempts": max_attempts});
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "schemaVersion": 1,
            "session": {"cacheDirectory": "live-cache"},
            "sources": [
                github,
                {
                    "kind": "https",
                    "id": "docs",
                    "required": true,
                    "origins": [{"baseUrl": "https://doc.rust-lang.org/", "allowPrivateNetwork": false}]
                }
            ]
        }))
        .expect("profile JSON"),
    )
    .expect("write profile");
    path
}

fn write_commit_profile(directory: &Path) -> PathBuf {
    let path = directory.join("live-commit.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "schemaVersion": 1,
            "session": {"cacheDirectory": "live-commit-cache"},
            "sources": [{
                "kind": "github",
                "id": "forge",
                "required": true,
                "allowPrivateNetwork": false,
                "credential": {"kind": "environment", "name": "GITHUB_TOKEN"},
                "repositories": [{"name": "dwalleck/resourcefs"}]
            }]
        }))
        .expect("commit profile JSON"),
    )
    .expect("write commit profile");
    path
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    fn start(profile: &Path, token: &str) -> Self {
        let mut child = Command::new(binary())
            .args(["serve", "--config"])
            .arg(profile)
            .env("GITHUB_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start resourcefs");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = BufReader::new(child.stdout.take().expect("child stdout"));
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            next_id: 1,
        }
    }

    fn write(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("open child stdin");
        serde_json::to_writer(&mut *stdin, message).expect("serialize message");
        writeln!(stdin).expect("write delimiter");
        stdin.flush().expect("flush message");
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            let bytes = self.stdout.read_line(&mut line).expect("read response");
            assert_ne!(
                bytes, 0,
                "resourcefs closed stdout before responding to {method}"
            );
            let message: Value = serde_json::from_str(&line)
                .unwrap_or_else(|error| panic!("stdout was not JSON-RPC: {error}: {line:?}"));
            // This client declares no roots capability, so the server never
            // asks for them; any other server-initiated message is skipped.
            if message.get("id").and_then(Value::as_u64) == Some(id)
                && message.get("method").is_none()
            {
                assert!(message.get("error").is_none(), "{method} failed: {message}");
                return message["result"].clone();
            }
        }
    }

    fn initialize(&mut self) {
        let result = self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "resourcefs-live-smoke", "version": "1.0.0"},
            }),
        );
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        self.write(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let result = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        assert_eq!(
            result["isError"], false,
            "{tool} {arguments} returned a tool error: {result}"
        );
        assert_eq!(result["structuredContent"]["ok"], true);
        result
    }

    fn call_err(&mut self, tool: &str, arguments: Value) -> Value {
        let result = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        assert_eq!(
            result["isError"], true,
            "{tool} {arguments} unexpectedly succeeded: {result}"
        );
        result
    }

    fn finish(mut self) -> String {
        drop(self.stdin.take());
        let output = self.child.wait_with_output().expect("resourcefs exit");
        assert!(
            output.status.success(),
            "resourcefs exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stderr).into_owned()
    }
}

/// Recover one JSON document through artifact pages only. A source cursor is
/// not another representation page: return it separately for a client to
/// traverse as its own document.
fn recover_artifact_document(
    server: &mut Server,
    mut result: Value,
) -> (Value, String, Option<String>) {
    let mut bytes = String::new();
    let mut source_cursor = None;
    let mut pages = 0_usize;
    loop {
        assert_eq!(result["isError"], false, "{result}");
        let structured = &result["structuredContent"];
        bytes.push_str(structured["content"].as_str().expect("JSON page"));
        if source_cursor.is_none() {
            source_cursor = structured
                .get("sourceContinuationReference")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        pages += 1;
        assert!(pages < 1_000, "artifact recovery must progress");
        let Some(next) = structured
            .get("continuationReference")
            .and_then(Value::as_str)
        else {
            break;
        };
        if !next.starts_with("artifact://") {
            source_cursor = Some(next.to_owned());
            break;
        }
        result = server.call("rfs_read", json!({"path": next}));
    }
    let document = serde_json::from_str(&bytes).expect("one recovered JSON document");
    (document, bytes, source_cursor)
}

#[test]
#[ignore = "live full-stack smoke; needs RFS_LIVE=1 and GITHUB_TOKEN"]
fn live_stdio_profile_probe_serve_and_tools_hold_up() {
    let Some(token) = live_token() else {
        return;
    };
    let temporary = TempDir::new().expect("temporary directory");
    let profile = write_profile(temporary.path());

    // S1 — `rfs check --probe` reaches both real origins with the environment
    // credential and reports every source available.
    let check = Command::new(binary())
        .args(["check", "--probe", "--config"])
        .arg(&profile)
        .env("GITHUB_TOKEN", &token)
        .current_dir(temporary.path())
        .output()
        .expect("run rfs check");
    assert_eq!(
        check.status.code(),
        Some(0),
        "S1 check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let report: Value = serde_json::from_slice(&check.stdout).expect("S1 check report is JSON");
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["probe"], true);
    let states = report["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|source| {
            (
                source["id"].as_str().unwrap_or("?").to_owned(),
                source["state"].clone(),
            )
        })
        .collect::<Vec<_>>();
    assert!(
        states.iter().all(|(_, state)| state == "available"),
        "S1 every source is available: {states:?}"
    );
    assert!(
        !check
            .stdout
            .windows(token.len())
            .any(|window| window == token.as_bytes()),
        "S1 the credential never reaches the report"
    );
    eprintln!("S1 check --probe: {states:?}");

    // S2 — the served catalog advertises the mounted families.
    let mut server = Server::start(&profile, &token);
    server.initialize();
    let catalog = server.call("rfs_read", json!({"path": "rfs://"}));
    let catalog_text = catalog["structuredContent"]["content"]
        .as_str()
        .expect("catalog text");
    for family in ["issue://", "pr://", "https://"] {
        assert!(catalog_text.contains(family), "S2 catalog lacks {family}");
    }
    eprintln!("S2 catalog: {} bytes", catalog_text.len());

    // S3 — a GitHub Field, an Aggregate search, and a paginated collection
    // over the tool surface.
    let title = server.call("rfs_read", json!({"path": format!("{SMALL_PR}/title")}));
    let title_text = title["structuredContent"]["content"]
        .as_str()
        .expect("title text");
    assert!(!title_text.trim().is_empty());
    assert_eq!(
        title["structuredContent"]["canonicalReference"],
        format!("{SMALL_PR}/title")
    );
    let search = server.call(
        "rfs_search",
        json!({"path": SMALL_PR, "pattern": "^Kind: "}),
    );
    assert_eq!(search["structuredContent"]["totalRecords"], 1);
    assert_eq!(
        search["structuredContent"]["groups"][0]["reference"],
        SMALL_PR
    );
    let issues = server.call("rfs_read", json!({"path": "issue://rust-lang/rust"}));
    let issues_text = issues["structuredContent"]["content"]
        .as_str()
        .expect("issues text");
    assert!(issues_text.starts_with("# Issues: rust-lang/rust\n"));
    assert!(issues_text.contains("\n- issue://rust-lang/rust/"));
    let continuation = issues["structuredContent"]["continuationReference"]
        .as_str()
        .expect("S3 a ten-page collection is bounded");
    let source_continuation = issues["structuredContent"]["sourceContinuationReference"].as_str();
    if continuation.starts_with("artifact://") {
        // The page overflowed: the artifact chain progresses the result and
        // the source's next page is named alongside it (ADR-0006).
        assert_eq!(
            source_continuation,
            Some("issue://rust-lang/rust:page:11"),
            "S3 the upstream page is addressable from the first response"
        );
    } else {
        assert_eq!(continuation, "issue://rust-lang/rust:page:11");
        assert_eq!(source_continuation, None, "S3 not repeated when it fits");
    }
    eprintln!(
        "S3 github: title {title_text:?}; search 1 hit; issues {} bytes, continuation {continuation}, source continuation {source_continuation:?}",
        issues_text.len()
    );

    // S4 — an HTTPS reader-mode read over the same server.
    let page = server.call("rfs_read", json!({"path": BOOK_PAGE}));
    let page_text = page["structuredContent"]["content"]
        .as_str()
        .expect("page text");
    assert!(page_text.contains("rustup"), "S4 reader mode lacks rustup");
    assert_eq!(page["structuredContent"]["canonicalReference"], BOOK_PAGE);

    eprintln!("S4 https: {} bytes", page_text.len());

    // S5 — refusals keep their categories across the tool boundary.
    let refused = server.call_err("rfs_read", json!({"path": "issue://rust-lang/rust/159232"}));
    assert_eq!(
        refused["structuredContent"]["error"]["category"], "not_found",
        "S5 a PR under issue:// is not_found: {refused}"
    );
    let denied = server.call_err("rfs_read", json!({"path": "issue://other/repo/1"}));
    assert_eq!(
        denied["structuredContent"]["error"]["category"], "permission_denied",
        "S5 an unallowlisted repository is permission_denied: {denied}"
    );
    eprintln!("S5 refusals: not_found, permission_denied");

    let stderr = server.finish();
    assert!(
        !stderr.contains(&token),
        "the credential never reaches the diagnostics channel"
    );
}
#[test]
#[ignore = "live full-stack commit smoke; needs RFS_LIVE=1 and GITHUB_TOKEN"]
fn live_stdio_github_commit_facts_match_native_observation() {
    const COMMIT: &str = "635ab170ab57542c18272921298d575da2f8b08a";
    let Some(token) = live_token() else { return };
    let temporary = TempDir::new().expect("temporary directory");
    let profile = write_commit_profile(temporary.path());
    let check = Command::new(binary())
        .args(["check", "--probe", "--config"])
        .arg(&profile)
        .env("GITHUB_TOKEN", &token)
        .current_dir(temporary.path())
        .output()
        .expect("run commit check");
    assert_eq!(
        check.status.code(),
        Some(0),
        "commit check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let report: Value = serde_json::from_slice(&check.stdout).expect("commit check report");
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["probe"], true);
    assert_eq!(report["sources"][0]["state"], "available");
    assert!(
        !check
            .stdout
            .windows(token.len())
            .any(|window| window == token.as_bytes()),
        "the credential never reaches the commit check report"
    );
    let Some(native_repository) = native_gh(&token, "repos/dwalleck/resourcefs") else {
        return;
    };
    let Some(native_commit) = native_gh(
        &token,
        &format!("repos/dwalleck/resourcefs/commits/{COMMIT}"),
    ) else {
        return;
    };
    let mut server = Server::start(&profile, &token);
    server.initialize();
    let reference = format!("github://dwalleck/resourcefs/commits/{COMMIT}/facts");
    let first = server.call("rfs_read", json!({"path": reference}));
    let (facts, bytes, _) = recover_artifact_document(&mut server, first);
    assert_eq!(facts["schemaVersion"], json!({"major": 1, "minor": 0}));
    assert_eq!(facts["kind"], "github.commit");
    assert_eq!(facts["resource"], reference);
    assert_eq!(
        facts["request"],
        json!({"repository":{"owner":"dwalleck","name":"resourcefs"},"commitSha":COMMIT})
    );
    assert_eq!(facts["observed"]["commitSha"], native_commit["sha"]);
    assert_eq!(
        facts["observed"]["treeSha"],
        native_commit["commit"]["tree"]["sha"]
    );
    assert_eq!(
        facts["repository"]["observed"]["id"],
        native_repository["id"]
            .as_u64()
            .expect("repository id")
            .to_string()
    );
    assert_eq!(
        facts["repository"]["observed"]["fullName"],
        native_repository["full_name"]
    );
    assert_eq!(facts["data"]["sha"], native_commit["sha"]);
    assert_eq!(
        facts["data"]["treeSha"],
        native_commit["commit"]["tree"]["sha"]
    );
    assert_eq!(facts["data"]["message"], native_commit["commit"]["message"]);
    assert_eq!(
        facts["data"]["parents"]
            .as_array()
            .expect("commit parents")
            .iter()
            .map(|parent| parent["sha"].clone())
            .collect::<Vec<_>>(),
        native_commit["parents"]
            .as_array()
            .expect("native parents")
            .iter()
            .map(|parent| parent["sha"].clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        facts["data"]["author"]["name"],
        native_commit["commit"]["author"]["name"]
    );
    assert_eq!(
        facts["data"]["author"]["email"],
        native_commit["commit"]["author"]["email"]
    );
    assert_eq!(
        facts["data"]["author"]["date"],
        native_commit["commit"]["author"]["date"]
    );
    assert_eq!(
        facts["data"]["committer"]["name"],
        native_commit["commit"]["committer"]["name"]
    );
    assert_eq!(
        facts["data"]["committer"]["email"],
        native_commit["commit"]["committer"]["email"]
    );
    assert_eq!(
        facts["data"]["committer"]["date"],
        native_commit["commit"]["committer"]["date"]
    );
    assert_eq!(facts["data"]["links"]["apiUrl"], native_commit["url"]);
    assert_eq!(facts["data"]["links"]["htmlUrl"], native_commit["html_url"]);
    assert_eq!(
        facts["data"]["links"]["commentsUrl"],
        native_commit["comments_url"]
    );
    assert_eq!(
        facts["data"]["authorAccount"]["login"],
        native_commit["author"]["login"]
    );
    assert_eq!(
        facts["data"]["committerAccount"]["login"],
        native_commit["committer"]["login"]
    );
    assert!(!bytes.contains(&token));
    let stderr = server.finish();
    assert!(!stderr.contains(&token), "credential leaked to diagnostics");
}

#[test]
#[ignore = "live full-stack smoke; needs RFS_LIVE=1 and GITHUB_TOKEN"]
fn live_stdio_github_facts_match_native_observation() {
    let Some(token) = live_token() else { return };
    let temporary = TempDir::new().expect("temporary directory");
    let profile = write_profile(temporary.path());
    let Some(native) = native_gh(&token, "repos/rust-lang/rust/pulls/159232") else {
        return;
    };
    let mut server = Server::start(&profile, &token);
    server.initialize();
    let first = server.call("rfs_read", json!({"path":format!("{SMALL_PR}/facts")}));
    let (facts, bytes, source_cursor) = recover_artifact_document(&mut server, first);
    if let Some(source_cursor) = source_cursor {
        eprintln!("facts source cursor is independently addressable: {source_cursor}");
    }
    assert_eq!(facts["schemaVersion"]["major"], 1);
    assert_eq!(
        facts["data"]["id"],
        native["id"].as_u64().expect("native id").to_string()
    );
    for side in ["base", "head"] {
        assert_eq!(facts["data"][side]["commitSha"], native[side]["sha"]);
    }
    assert_eq!(facts["data"]["links"]["apiUrl"], native["url"]);
    assert_eq!(facts["data"]["links"]["htmlUrl"], native["html_url"]);
    assert_eq!(facts["acquisition"]["restApiVersion"], "2022-11-28");
    assert!(!bytes.contains(&token));
}

#[test]
#[ignore = "live full-stack smoke; needs RFS_LIVE=1 and GITHUB_TOKEN"]
fn live_stdio_github_comment_facts_match_native_observation() {
    let Some(token) = live_token() else { return };
    let temporary = TempDir::new().expect("temporary directory");
    let profile = write_profile_with_attempts(temporary.path(), Some(2));
    let Some(native_first_value) = native_gh(
        &token,
        "repos/rust-lang/rust/issues/159628/comments?per_page=100&page=1",
    ) else {
        return;
    };
    let Some(native_second_value) = native_gh(
        &token,
        "repos/rust-lang/rust/issues/159628/comments?per_page=100&page=2",
    ) else {
        return;
    };
    let native_first = native_first_value
        .as_array()
        .expect("native first comment page")
        .to_owned();
    let native_second = native_second_value
        .as_array()
        .expect("native second comment page")
        .to_owned();
    assert!(
        !native_first.is_empty(),
        "fixture PR must have first-page comments"
    );
    assert!(
        !native_second.is_empty(),
        "fixture PR must have second-page comments"
    );
    let native_ids = |page: &[Value]| {
        page.iter()
            .map(|comment| {
                comment["id"]
                    .as_u64()
                    .expect("native comment id")
                    .to_string()
            })
            .collect::<Vec<_>>()
    };

    let mut server = Server::start(&profile, &token);
    server.initialize();
    let first = server.call(
        "rfs_read",
        json!({"path": format!("{MULTIPAGE_PR}/comments/facts")}),
    );
    let (facts, bytes, source_cursor) = recover_artifact_document(&mut server, first);
    let source_cursor = source_cursor.expect("initial facts source continuation");
    assert_eq!(facts["kind"], "github.conversation_comment_collection");
    assert_eq!(facts["request"]["number"], "159628");
    assert_eq!(facts["collection"]["scope"], "initial");
    assert_eq!(facts["collection"]["state"], "incomplete");
    assert!(facts["collection"]["continuation"].is_string());
    let records = facts["data"]["records"].as_array().expect("records");
    assert_eq!(records.len(), native_first.len());
    assert_eq!(
        facts["collection"]["acceptedCount"],
        records.len(),
        "accepted count follows the observed first page"
    );
    assert_eq!(
        records
            .iter()
            .map(|record| record["id"].as_str().expect("facts comment id").to_owned())
            .collect::<Vec<_>>(),
        native_ids(&native_first)
    );

    let next = server.call("rfs_read", json!({"path": source_cursor}));
    let (next_facts, next_bytes, next_source_cursor) = recover_artifact_document(&mut server, next);
    assert!(
        next_source_cursor.is_none(),
        "the second observed page is the terminal source segment"
    );
    assert_eq!(next_facts["request"]["number"], "159628");
    assert_eq!(next_facts["collection"]["scope"], "continuation");
    assert_eq!(next_facts["collection"]["state"], "complete");
    let next_records = next_facts["data"]["records"]
        .as_array()
        .expect("continued records");
    assert_eq!(next_records.len(), native_second.len());
    assert_eq!(
        next_records
            .iter()
            .map(|record| record["id"].as_str().expect("facts comment id").to_owned())
            .collect::<Vec<_>>(),
        native_ids(&native_second)
    );
    assert!(!bytes.contains(&token));
    assert!(!next_bytes.contains(&token));

    let id = native_first[0]["id"].as_u64().expect("native comment id");
    let first = server.call(
        "rfs_read",
        json!({"path": format!("{MULTIPAGE_PR}/comments/{id}/facts")}),
    );
    let (single, _, single_source_cursor) = recover_artifact_document(&mut server, first);
    assert!(single_source_cursor.is_none());
    assert_eq!(single["data"]["id"], id.to_string());
    assert_eq!(single["data"]["nodeId"], native_first[0]["node_id"]);
    assert_eq!(single["data"]["body"], native_first[0]["body"]);
    assert!(single.get("collection").is_none());
    eprintln!(
        "S-comment conversation-comment facts: {} + {} records across source segments",
        records.len(),
        next_records.len()
    );
}

#[test]
#[ignore = "live full-stack smoke; needs RFS_LIVE=1 and GITHUB_TOKEN"]
fn live_stdio_github_review_and_inline_facts_match_native_observation() {
    let Some(token) = live_token() else { return };
    let temporary = TempDir::new().expect("temporary directory");
    let profile = write_profile_with_attempts(temporary.path(), Some(2));
    let Some(native_reviews) = native_gh(
        &token,
        "repos/rust-lang/rust/pulls/159232/reviews?per_page=100",
    ) else {
        return;
    };
    let Some(native_inline) = native_gh(
        &token,
        "repos/rust-lang/rust/pulls/159232/comments?per_page=100",
    ) else {
        return;
    };
    let native_reviews = native_reviews
        .as_array()
        .expect("native reviews")
        .to_owned();
    let native_inline = native_inline.as_array().expect("native inline").to_owned();
    assert!(
        !native_reviews.is_empty() && !native_inline.is_empty(),
        "the evidence PR must still carry reviews and inline comments"
    );

    let mut server = Server::start(&profile, &token);
    server.initialize();

    let result = server.call(
        "rfs_read",
        json!({"path": format!("{SMALL_PR}/reviews/facts")}),
    );
    let (review_facts, review_bytes, review_cursor) =
        recover_artifact_document(&mut server, result);
    assert!(review_cursor.is_none());
    assert_eq!(review_facts["kind"], "github.review_submission_collection");
    let review_records = review_facts["data"]["records"]
        .as_array()
        .expect("review records");
    for native in &native_reviews {
        let id = native["id"].as_u64().expect("native review id").to_string();
        let record = review_records
            .iter()
            .find(|record| record["id"] == json!(id))
            .unwrap_or_else(|| panic!("review {id} missing from recovered facts"));
        assert_eq!(record["commitSha"], native["commit_id"]);
        assert_eq!(record["submittedAt"], native["submitted_at"]);
        assert_eq!(
            record["links"]["pullRequestUrl"],
            native["pull_request_url"]
        );
    }

    let result = server.call(
        "rfs_read",
        json!({"path": format!("{SMALL_PR}/review-comments/facts")}),
    );
    let (inline_facts, inline_bytes, inline_cursor) =
        recover_artifact_document(&mut server, result);
    assert!(inline_cursor.is_none());
    assert_eq!(inline_facts["kind"], "github.review_comment_collection");
    let inline_records = inline_facts["data"]["records"]
        .as_array()
        .expect("inline records");
    for native in &native_inline {
        let id = native["id"].as_u64().expect("native inline id").to_string();
        let record = inline_records
            .iter()
            .find(|record| record["id"] == json!(id))
            .unwrap_or_else(|| panic!("inline comment {id} missing from recovered facts"));
        assert_eq!(record["path"], native["path"]);
        assert_eq!(record["side"], native["side"]);
        assert_eq!(record["originalLine"], native["original_line"]);
        assert_eq!(record["subjectType"], native["subject_type"]);
    }

    let id = native_inline[0]["id"].as_u64().expect("native inline id");
    let result = server.call(
        "rfs_read",
        json!({"path": format!("{SMALL_PR}/review-comments/{id}/facts")}),
    );
    let (single, single_bytes, single_cursor) = recover_artifact_document(&mut server, result);
    assert!(single_cursor.is_none());
    assert_eq!(single["data"]["id"], id.to_string());
    assert_eq!(single["data"]["diffHunk"], native_inline[0]["diff_hunk"]);
    assert_eq!(
        single["data"]["originalCommitSha"],
        native_inline[0]["original_commit_id"]
    );
    for bytes in [review_bytes, inline_bytes, single_bytes] {
        assert!(!bytes.contains(&token), "no credential may reach output");
    }
    eprintln!(
        "S-review review/inline facts: {} reviews, {} inline comments recovered before parsing",
        review_records.len(),
        inline_records.len()
    );
}
