use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tempfile::TempDir;

const VERSION_2026: &str = "2026-07-28";
const VERSION_2025: &str = "2025-11-25";

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_resourcefs")
}
fn maximum_text() -> String {
    let mut content = String::with_capacity(48 * 1024);
    for _ in 0..96 {
        content.extend(std::iter::repeat_n('x', 511));
        content.push('\n');
    }
    content
}

struct WorkspaceFixture {
    _temporary: TempDir,
    root: PathBuf,
}

impl WorkspaceFixture {
    fn new() -> Self {
        let temporary = TempDir::new().expect("temporary directory");
        let root = temporary.path().join("workspace root");
        fs::create_dir(&root).expect("workspace root");
        fs::write(root.join("fixture.txt"), "fixture text\n").expect("text fixture");
        fs::write(root.join("empty.txt"), "").expect("empty fixture");
        fs::write(root.join("unicode space.txt"), "héllo from space\n").expect("Unicode fixture");
        fs::write(root.join("maximum.txt"), maximum_text()).expect("maximum fixture");
        fs::write(root.join("binary.bin"), [0xff, 0xfe]).expect("binary fixture");
        fs::create_dir(root.join("directory")).expect("directory fixture");

        let mut over_limit = Vec::with_capacity(48 * 1024 + 1);
        for _ in 0..96 {
            over_limit.extend(std::iter::repeat_n(b'x', 511));
            over_limit.push(b'\n');
        }
        over_limit.push(b'x');
        fs::write(root.join("over-limit.txt"), over_limit).expect("limit fixture");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = temporary.path().join("outside.txt");
            fs::write(&outside, "outside secret").expect("outside fixture");
            symlink(outside, root.join("escape.txt")).expect("escaping symlink");
        }

        Self {
            _temporary: temporary,
            root,
        }
    }
}

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl McpProcess {
    fn start(root: &Path) -> Self {
        let root_argument = format!("workspace={}", root.display());
        let mut child = Command::new(binary())
            .args([
                "serve",
                "--root",
                &root_argument,
                "--primary-root",
                "workspace",
            ])
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

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let stdin = self.stdin.as_mut().expect("open child stdin");
        serde_json::to_writer(&mut *stdin, &request).expect("serialize request");
        writeln!(stdin).expect("write request delimiter");
        stdin.flush().expect("flush request");

        let mut line = String::new();
        let bytes = self.stdout.read_line(&mut line).expect("read response");
        assert_ne!(bytes, 0, "resourcefs closed stdout before responding");
        let response: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("stdout was not JSON-RPC: {error}: {line:?}"));
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], id);
        response
    }

    fn notify_initialized(&mut self) {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        let stdin = self.stdin.as_mut().expect("open child stdin");
        serde_json::to_writer(&mut *stdin, &notification).expect("serialize notification");
        writeln!(stdin).expect("write notification delimiter");
        stdin.flush().expect("flush notification");
    }

    fn initialize(&mut self, version: &str) -> Value {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": {"name": "resourcefs-contract-test", "version": "1.0.0"},
            }),
        );
        assert!(
            response.get("error").is_none(),
            "initialize failed: {response}"
        );
        self.notify_initialized();
        response["result"].clone()
    }

    fn call_read(&mut self, path: &str) -> Value {
        let response = self.request(
            "tools/call",
            json!({"name": "rfs_read", "arguments": {"path": path}}),
        );
        assert!(
            response.get("error").is_none(),
            "tool call failed: {response}"
        );
        response["result"].clone()
    }

    fn finish(&mut self) -> String {
        self.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("poll child") {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "resourcefs did not exit after stdin closed"
            );
            thread::sleep(Duration::from_millis(10));
        };

        let mut remaining_stdout = String::new();
        self.stdout
            .read_to_string(&mut remaining_stdout)
            .expect("remaining stdout");
        assert!(
            remaining_stdout.trim().is_empty(),
            "unexpected protocol stdout after final response: {remaining_stdout:?}"
        );

        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .expect("child stderr")
            .read_to_string(&mut stderr)
            .expect("read child stderr");
        assert!(status.success(), "resourcefs exited {status}: {stderr}");
        stderr
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let should_terminate = match self.child.try_wait() {
            Ok(Some(_status)) => false,
            Ok(None) => true,
            Err(error) => {
                eprintln!("failed to poll resourcefs test process during cleanup: {error}");
                true
            }
        };
        if should_terminate {
            let _kill_result = self.child.kill();
            let _wait_result = self.child.wait();
        }
    }
}

fn assert_tool_error(result: &Value, category: &str) {
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["ok"], false);
    assert_eq!(result["structuredContent"]["error"]["category"], category);
    let text = result["content"][0]["text"]
        .as_str()
        .expect("error TextContent");
    assert!(!text.is_empty());
    assert!(text.contains(category));
}

#[test]
fn negotiates_required_revisions() {
    let fixture = WorkspaceFixture::new();
    for version in [VERSION_2026, VERSION_2025] {
        let mut process = McpProcess::start(&fixture.root);
        let initialized = process.initialize(version);
        assert_eq!(initialized["protocolVersion"], version);
        assert_eq!(initialized["serverInfo"]["name"], "resourcefs");
        assert_eq!(initialized["serverInfo"]["version"], "1.0.0");
        assert!(initialized["capabilities"].get("tools").is_some());
        for absent in ["resources", "prompts", "logging", "completions", "tasks"] {
            assert!(initialized["capabilities"].get(absent).is_none());
        }
        let stderr = process.finish();
        assert!(stderr.is_empty(), "unexpected server diagnostics: {stderr}");
    }

    let mut fallback = McpProcess::start(&fixture.root);
    let initialized = fallback.initialize("2099-01-01");
    assert_eq!(initialized["protocolVersion"], VERSION_2026);
    fallback.finish();
}

#[test]
fn lists_object_root_read_tool_only() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize(VERSION_2026);
    let response = process.request("tools/list", json!({}));
    let tools = response["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "rfs_read");
    assert_eq!(tools[0]["inputSchema"]["type"], "object");
    assert_eq!(tools[0]["inputSchema"]["additionalProperties"], false);
    assert_eq!(tools[0]["outputSchema"]["type"], "object");
    assert_eq!(tools[0]["outputSchema"]["additionalProperties"], false);
    assert_eq!(
        tools[0]["outputSchema"]["$defs"]["ReadErrorOutput"]["additionalProperties"],
        false
    );
    process.finish();
}

#[test]
fn renders_complete_success_and_errors() {
    let fixture = WorkspaceFixture::new();
    for version in [VERSION_2026, VERSION_2025] {
        let mut process = McpProcess::start(&fixture.root);
        process.initialize(version);

        let success = process.call_read("fixture.txt");
        assert_eq!(success["isError"], false);
        let text = success["content"][0]["text"].as_str().expect("TextContent");
        let structured = &success["structuredContent"];
        assert_eq!(structured["ok"], true);
        assert_eq!(structured["contractVersion"], "1.0.0");
        assert_eq!(
            structured["canonicalReference"],
            "rfs://workspace/workspace/fixture.txt"
        );
        assert_eq!(structured["contentType"], "text/plain; charset=utf-8");
        assert_eq!(structured["content"], "fixture text\n");
        assert_eq!(structured["mutable"], false);
        assert_eq!(structured["bounded"], false);
        assert!(
            structured["versionTag"]
                .as_str()
                .is_some_and(|tag| tag.starts_with("sha256:") && tag.len() == 71)
        );
        assert!(structured.get("recoveryReference").is_none());
        assert_eq!(
            text,
            format!(
                "[{}#{}]\n{}",
                structured["canonicalReference"]
                    .as_str()
                    .expect("canonical reference"),
                structured["versionTag"].as_str().expect("Version Tag"),
                structured["content"].as_str().expect("structured content")
            )
        );

        let canonical = process.call_read("rfs://workspace/workspace/fixture.txt");
        assert_eq!(canonical["structuredContent"]["content"], "fixture text\n");

        let unicode = process.call_read("unicode space.txt");
        assert_eq!(
            unicode["structuredContent"]["content"],
            "héllo from space\n"
        );

        let empty = process.call_read("empty.txt");
        assert_eq!(empty["structuredContent"]["content"], "");
        assert!(
            !empty["content"][0]["text"]
                .as_str()
                .expect("empty TextContent")
                .is_empty()
        );

        let started = Instant::now();
        let maximum = process.call_read("maximum.txt");
        let elapsed = started.elapsed();
        assert_eq!(
            maximum["structuredContent"]["content"],
            maximum_text(),
            "maximum-sized content changed"
        );
        assert!(
            elapsed <= Duration::from_millis(250),
            "maximum-sized MCP read took {elapsed:?}"
        );

        for (path, category) in [
            ("missing.txt", "not_found"),
            ("directory", "unsupported_projection"),
            ("binary.bin", "unsupported_projection"),
            ("../secret", "permission_denied"),
            ("https://example.com/file", "invalid_reference"),
        ] {
            assert_tool_error(&process.call_read(path), category);
        }
        #[cfg(unix)]
        assert_tool_error(&process.call_read("escape.txt"), "permission_denied");

        let over_limit = process.call_read("over-limit.txt");
        assert_tool_error(&over_limit, "limit_exceeded");
        assert!(over_limit["structuredContent"].get("content").is_none());

        let malformed = process.request("tools/call", json!({"name": "rfs_read", "arguments": {}}));
        assert_eq!(malformed["error"]["code"], -32602);
        assert!(
            malformed["error"]["message"]
                .as_str()
                .is_some_and(|text| text.contains("missing field `path`"))
        );
        assert!(malformed.get("result").is_none());

        let extra_argument = process.request(
            "tools/call",
            json!({
                "name": "rfs_read",
                "arguments": {"path": "fixture.txt", "extra": true}
            }),
        );
        assert_eq!(extra_argument["error"]["code"], -32602);
        assert!(
            extra_argument["error"]["message"]
                .as_str()
                .is_some_and(|text| text.contains("unknown field `extra`"))
        );
        assert!(extra_argument.get("result").is_none());

        let unknown = process.request(
            "tools/call",
            json!({"name": "unknown_tool", "arguments": {}}),
        );
        assert_eq!(unknown["error"]["code"], -32602);

        let unknown_method = process.request("unknown/method", json!({}));
        assert_eq!(unknown_method["error"]["code"], -32601);

        process.finish();
    }
}

fn run_invalid(args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .output()
        .expect("run invalid resourcefs command")
}

#[test]
fn rejects_invalid_root_configuration() {
    let temporary = TempDir::new().expect("temporary directory");
    let root = temporary.path().join("root");
    fs::create_dir(&root).expect("root fixture");
    let file = temporary.path().join("file.txt");
    fs::write(&file, "file").expect("file fixture");
    let missing = temporary.path().join("missing");
    let valid_root = format!("workspace={}", root.display());
    let second_root = format!("other={}", root.display());
    let file_root = format!("workspace={}", file.display());
    let missing_root = format!("workspace={}", missing.display());
    let invalid_name = format!("bad/name={}", root.display());

    let cases: Vec<(Vec<&str>, &str)> = vec![
        (
            vec!["serve", "--primary-root", "workspace"],
            "--root <NAME=PATH>",
        ),
        (
            vec!["serve", "--root", &valid_root],
            "--primary-root <NAME>",
        ),
        (
            vec![
                "serve",
                "--root",
                &valid_root,
                "--root",
                &second_root,
                "--primary-root",
                "workspace",
            ],
            "exactly one launch Workspace Root is required",
        ),
        (
            vec!["serve", "--root", &valid_root, "--primary-root", "other"],
            "does not match configured root",
        ),
        (
            vec!["serve", "--root", &file_root, "--primary-root", "workspace"],
            "configured Workspace Root is not a directory",
        ),
        (
            vec![
                "serve",
                "--root",
                &missing_root,
                "--primary-root",
                "workspace",
            ],
            "failed to canonicalize Workspace Root",
        ),
        (
            vec![
                "serve",
                "--root",
                &invalid_name,
                "--primary-root",
                "workspace",
            ],
            "Workspace Root name must start",
        ),
    ];

    for (args, expected_diagnostic) in cases {
        let output = run_invalid(&args);
        assert!(
            !output.status.success(),
            "invalid command succeeded: {args:?}"
        );
        assert!(
            output.stdout.is_empty(),
            "invalid command wrote stdout: {args:?}"
        );
        let stderr = String::from_utf8(output.stderr).expect("UTF-8 process diagnostics");
        assert!(
            stderr.contains(expected_diagnostic),
            "invalid command omitted {expected_diagnostic:?}: {args:?}: {stderr}"
        );
    }
}
