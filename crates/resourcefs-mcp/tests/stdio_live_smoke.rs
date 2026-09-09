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

/// One GitHub source and one HTTPS source, both required, so a probe failure
/// is a startup failure rather than a silently degraded mount.
fn write_profile(directory: &Path) -> PathBuf {
    let path = directory.join("live.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "schemaVersion": 1,
            "session": {"cacheDirectory": "live-cache"},
            "sources": [
                {
                    "kind": "github",
                    "id": "forge",
                    "required": true,
                    "allowPrivateNetwork": false,
                    "credential": {"kind": "environment", "name": "GITHUB_TOKEN"},
                    "repositories": [{"name": "rust-lang/rust"}]
                },
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
#[ignore = "live full-stack smoke; needs RFS_LIVE=1 and GITHUB_TOKEN"]
fn live_stdio_github_facts_match_native_observation() {
    let Some(token) = live_token() else { return };
    let temporary = TempDir::new().expect("temporary directory");
    let profile = write_profile(temporary.path());
    // `gh` is a dependency beyond the RFS_LIVE/GITHUB_TOKEN gate, so a runner
    // without it skips this cross-check rather than failing the row.
    let native = match Command::new("gh")
        .args([
            "api",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            "repos/rust-lang/rust/pulls/159232",
        ])
        .env("GH_TOKEN", &token)
        .output()
    {
        Ok(native) => native,
        Err(error) => {
            eprintln!("gh CLI is unavailable ({error}); skipping native cross-check");
            return;
        }
    };
    assert!(native.status.success(), "native observation failed");
    let native: Value = serde_json::from_slice(&native.stdout).expect("native JSON");
    let mut server = Server::start(&profile, &token);
    server.initialize();
    let mut result = server.call("rfs_read", json!({"path":format!("{SMALL_PR}/facts")}));
    let mut bytes = String::new();
    loop {
        assert_eq!(result["isError"], false);
        bytes.push_str(
            result["structuredContent"]["content"]
                .as_str()
                .expect("facts page"),
        );
        let Some(next) = result["structuredContent"]["continuationReference"].as_str() else {
            break;
        };
        result = server.call("rfs_read", json!({"path":next}));
    }
    let facts: Value = serde_json::from_str(&bytes).expect("complete facts");
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
    let profile = write_profile(temporary.path());
    let native = Command::new("gh")
        .args([
            "api",
            "-H",
            "X-GitHub-Api-Version: 2022-11-28",
            "repos/rust-lang/rust/issues/159232/comments?per_page=100&page=1",
        ])
        .env("GH_TOKEN", &token)
        .output()
        .expect("native gh observation");
    assert!(native.status.success(), "native observation failed");
    let native: Vec<Value> = serde_json::from_slice(&native.stdout).expect("native comments");
    assert!(!native.is_empty(), "fixture PR must have comments");
    let mut server = Server::start(&profile, &token);
    server.initialize();
    let mut result = server.call(
        "rfs_read",
        json!({"path": format!("{SMALL_PR}/comments/facts")}),
    );
    let mut bytes = String::new();
    loop {
        assert_eq!(result["isError"], false);
        bytes.push_str(
            result["structuredContent"]["content"]
                .as_str()
                .expect("facts page"),
        );
        let Some(next) = result["structuredContent"]["continuationReference"].as_str() else {
            break;
        };
        result = server.call("rfs_read", json!({"path": next}));
    }
    let facts: Value = serde_json::from_str(&bytes).expect("complete collection");
    assert_eq!(facts["kind"], "github.conversation_comment_collection");
    assert_eq!(facts["request"]["number"], "159232");
    let records = facts["data"]["records"].as_array().expect("records");
    assert!(!records.is_empty());
    assert_eq!(
        facts["collection"]["acceptedCount"]
            .as_u64()
            .expect("count"),
        records.len() as u64
    );
    assert_eq!(
        records[0]["id"],
        native[0]["id"].as_u64().expect("id").to_string()
    );
    assert_eq!(records[0]["nodeId"], native[0]["node_id"]);
    assert_eq!(records[0]["body"], native[0]["body"]);
    assert_eq!(records[0]["parent"]["number"], "159232");

    // The single-comment read through the real binary matches the same record.
    // A large comment body overflows the inline page, so recovery is followed
    // before parsing exactly as a client must.
    let id = native[0]["id"].as_u64().expect("id");
    let mut result = server.call(
        "rfs_read",
        json!({"path": format!("{SMALL_PR}/comments/{id}/facts")}),
    );
    let mut single = String::new();
    loop {
        assert_eq!(result["isError"], false, "{result}");
        single.push_str(
            result["structuredContent"]["content"]
                .as_str()
                .expect("comment JSON page"),
        );
        let Some(next) = result["structuredContent"]["continuationReference"].as_str() else {
            break;
        };
        result = server.call("rfs_read", json!({"path": next}));
    }
    let single: Value = serde_json::from_str(&single).expect("comment facts");
    assert_eq!(single["data"]["id"], id.to_string());
    assert_eq!(single["data"]["nodeId"], native[0]["node_id"]);
    assert!(single.get("collection").is_none());
    assert!(!bytes.contains(&token));
    eprintln!(
        "S-comment conversation-comment facts: {} records",
        records.len()
    );
}
