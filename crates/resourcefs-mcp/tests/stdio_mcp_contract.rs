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
    client_roots: Vec<Value>,
    root_list_calls: usize,
    cancelled_root_requests: usize,
}

impl McpProcess {
    fn start(root: &Path) -> Self {
        Self::start_with_roots(&[("workspace", root)], Some("workspace"))
    }

    fn start_with_roots(roots: &[(&str, &Path)], primary: Option<&str>) -> Self {
        Self::start_config(roots, primary, None)
    }

    #[cfg(feature = "test-support")]
    fn start_with_delivery_gate(
        roots: &[(&str, &Path)],
        primary: Option<&str>,
        gate: &Path,
    ) -> Self {
        Self::start_config(roots, primary, Some(gate))
    }

    fn start_config(
        roots: &[(&str, &Path)],
        primary: Option<&str>,
        delivery_gate: Option<&Path>,
    ) -> Self {
        let mut arguments = vec!["serve".to_owned()];
        for (id, path) in roots {
            arguments.push("--root".to_owned());
            arguments.push(format!("{id}={}", path.display()));
        }
        if let Some(primary) = primary {
            arguments.push("--primary-root".to_owned());
            arguments.push(primary.to_owned());
        }
        let mut command = Command::new(binary());
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(delivery_gate) = delivery_gate {
            command.env("RESOURCEFS_TEST_DELIVERY_GATE", delivery_gate);
        }
        let mut child = command.spawn().expect("start resourcefs");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = BufReader::new(child.stdout.take().expect("child stdout"));
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            next_id: 1,
            client_roots: Vec::new(),
            root_list_calls: 0,
            cancelled_root_requests: 0,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.request_handling_roots(method, params, true)
    }

    fn request_without_roots_reply(&mut self, method: &str, params: Value) -> Value {
        self.request_handling_roots(method, params, false)
    }

    fn request_handling_roots(
        &mut self,
        method: &str,
        params: Value,
        reply_to_roots: bool,
    ) -> Value {
        let id = self.send_request(method, params);
        self.receive_response(id, reply_to_roots)
    }

    fn send_request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }));
        id
    }

    fn receive_response(&mut self, id: u64, reply_to_roots: bool) -> Value {
        loop {
            let response = self.read_message();
            assert_eq!(response["jsonrpc"], "2.0");
            if response.get("method") == Some(&Value::String("roots/list".to_owned())) {
                self.root_list_calls += 1;
                if reply_to_roots {
                    let reply = json!({
                        "jsonrpc": "2.0",
                        "id": response["id"],
                        "result": {"roots": self.client_roots},
                    });
                    self.write_message(&reply);
                }
                continue;
            }
            if response.get("method") == Some(&Value::String("notifications/cancelled".to_owned()))
            {
                self.cancelled_root_requests += 1;
                continue;
            }
            assert_eq!(response["id"], id);
            return response;
        }
    }

    fn read_message(&mut self) -> Value {
        let mut line = String::new();
        let bytes = self.stdout.read_line(&mut line).expect("read response");
        assert_ne!(bytes, 0, "resourcefs closed stdout before responding");
        let response: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("stdout was not JSON-RPC: {error}: {line:?}"));
        assert_eq!(response["jsonrpc"], "2.0");
        response
    }

    fn write_message(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("open child stdin");
        serde_json::to_writer(&mut *stdin, message).expect("serialize message");
        writeln!(stdin).expect("write message delimiter");
        stdin.flush().expect("flush message");
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
        self.initialize_with_capabilities(version, json!({}))
    }

    fn initialize_with_roots(&mut self, version: &str, roots: Vec<Value>) -> Value {
        self.client_roots = roots;
        self.initialize_with_capabilities(version, json!({"roots": {"listChanged": true}}))
    }

    fn initialize_with_capabilities(&mut self, version: &str, capabilities: Value) -> Value {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": version,
                "capabilities": capabilities,
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

    fn change_client_roots(&mut self, roots: Vec<Value>) {
        self.client_roots = roots;
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/roots/list_changed",
        }));
        thread::sleep(Duration::from_millis(20));
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

fn mcp_root(path: &Path, name: &str) -> Value {
    json!({
        "uri": url::Url::from_directory_path(path)
            .expect("MCP root file URI")
            .to_string(),
        "name": name,
    })
}

#[test]
fn client_roots_replace_refresh_and_restore_launch_roots() {
    let temporary = TempDir::new().expect("temporary directory");
    let launch = temporary.path().join("launch");
    let alpha = temporary.path().join("alpha");
    let beta = temporary.path().join("beta");
    for root in [&launch, &alpha, &beta] {
        fs::create_dir(root).expect("root fixture");
    }
    fs::write(launch.join("shared.txt"), "launch").expect("launch fixture");
    fs::write(alpha.join("shared.txt"), "alpha").expect("alpha fixture");
    fs::write(beta.join("shared.txt"), "beta").expect("beta fixture");

    let mut process = McpProcess::start_with_roots(&[("launch", &launch)], Some("beta"));
    process.initialize_with_roots(
        VERSION_2026,
        vec![mcp_root(&alpha, "alpha"), mcp_root(&beta, "beta")],
    );
    let selected = process.call_read("shared.txt");
    assert_eq!(selected["structuredContent"]["content"], "beta");
    let canonical = selected["structuredContent"]["canonicalReference"]
        .as_str()
        .expect("canonical reference")
        .to_owned();
    assert!(canonical.starts_with("rfs://workspace/client-"));
    assert!(!canonical.contains(&beta.to_string_lossy() as &str));
    assert_eq!(process.root_list_calls, 1);
    process.change_client_roots(Vec::new());
    assert_eq!(
        process.call_read("shared.txt")["structuredContent"]["content"],
        "launch"
    );

    assert_tool_error(&process.call_read(&canonical), "invalid_reference");
    assert_eq!(process.root_list_calls, 2);
    process.change_client_roots(vec![json!({
        "uri": "https://example.com/not-local",
        "name": "invalid",
    })]);
    assert_tool_error(&process.call_read("shared.txt"), "source_unavailable");
    assert_eq!(process.root_list_calls, 3);

    process.change_client_roots(vec![mcp_root(&alpha, "alpha")]);
    assert_eq!(
        process.call_read("shared.txt")["structuredContent"]["content"],
        "alpha"
    );
    assert_eq!(process.root_list_calls, 4);
    process.finish();
}

#[test]
fn client_roots_work_under_legacy_protocol_revision() {
    let temporary = TempDir::new().expect("temporary directory");
    let launch = temporary.path().join("launch");
    let client = temporary.path().join("client");
    fs::create_dir(&launch).expect("launch root");
    fs::create_dir(&client).expect("client root");
    fs::write(launch.join("shared.txt"), "launch").expect("launch fixture");
    fs::write(client.join("shared.txt"), "client").expect("client fixture");

    let mut process = McpProcess::start_with_roots(&[("launch", &launch)], None);
    process.initialize_with_roots(VERSION_2025, vec![mcp_root(&client, "client")]);
    let selected = process.call_read("shared.txt");
    assert_eq!(
        selected["structuredContent"]["content"], "client",
        "legacy client roots were not applied: {selected}"
    );
    assert_eq!(process.root_list_calls, 1);
    process.finish();
}

#[test]
fn concurrent_requests_share_one_serialized_root_acquisition() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize_with_roots(VERSION_2026, vec![mcp_root(&fixture.root, "workspace")]);
    let params = json!({"name": "rfs_read", "arguments": {"path": "fixture.txt"}});

    let first_id = process.send_request("tools/call", params.clone());
    let roots_request = process.read_message();
    assert_eq!(roots_request["method"], "roots/list");
    process.root_list_calls += 1;
    let second_id = process.send_request("tools/call", params);
    thread::sleep(Duration::from_millis(20));
    process.write_message(&json!({
        "jsonrpc": "2.0",
        "id": roots_request["id"],
        "result": {"roots": process.client_roots},
    }));

    let mut responses = Vec::with_capacity(2);
    while responses.len() < 2 {
        let response = process.read_message();
        assert_ne!(
            response.get("method"),
            Some(&Value::String("roots/list".to_owned())),
            "concurrent request started a second roots acquisition"
        );
        if response.get("method") == Some(&Value::String("notifications/cancelled".to_owned())) {
            process.cancelled_root_requests += 1;
            continue;
        }
        responses.push(response);
    }
    for id in [first_id, second_id] {
        let response = responses
            .iter()
            .find(|response| response["id"] == id)
            .expect("tool response");
        assert!(
            response.get("error").is_none(),
            "tool call failed: {response}"
        );
        assert_eq!(
            response["result"]["structuredContent"]["content"],
            "fixture text\n"
        );
    }
    assert_eq!(process.root_list_calls, 1);
    assert_eq!(process.cancelled_root_requests, 0);
    process.finish();
}

#[test]
fn client_root_acquisition_times_out_fail_closed() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize_with_roots(VERSION_2026, vec![mcp_root(&fixture.root, "workspace")]);

    let started = Instant::now();
    let response = process.request_without_roots_reply(
        "tools/call",
        json!({"name": "rfs_read", "arguments": {"path": "fixture.txt"}}),
    );
    let elapsed = started.elapsed();
    assert!(
        response.get("error").is_none(),
        "tool call failed: {response}"
    );
    assert_tool_error(&response["result"], "source_unavailable");
    assert_eq!(process.root_list_calls, 1);
    assert_eq!(process.cancelled_root_requests, 1);
    assert!(
        elapsed >= Duration::from_millis(4_900),
        "root acquisition returned before its 5-second deadline: {elapsed:?}"
    );
    assert!(
        elapsed <= Duration::from_millis(6_500),
        "root acquisition exceeded its bounded deadline: {elapsed:?}"
    );
    process.finish();
}

#[cfg(feature = "test-support")]
#[test]
fn removed_inflight_result_fails_after_root_generation_changes() {
    let temporary = TempDir::new().expect("temporary directory");
    let removed = temporary.path().join("removed");
    let retained = temporary.path().join("retained");
    let gate = temporary.path().join("gate");
    fs::create_dir(&removed).expect("removed root");
    fs::create_dir(&retained).expect("retained root");
    fs::create_dir(&gate).expect("delivery gate");
    fs::write(removed.join("gated.txt"), "removed content").expect("gated fixture");
    fs::write(retained.join("gated.txt"), "retained content").expect("retained fixture");

    let mut process = McpProcess::start_with_delivery_gate(&[("launch", &removed)], None, &gate);
    process.initialize_with_roots(VERSION_2026, vec![mcp_root(&removed, "workspace")]);
    let list = process.request("tools/list", json!({}));
    assert!(
        list.get("error").is_none(),
        "initial root refresh failed: {list}"
    );

    let read_id = process.send_request(
        "tools/call",
        json!({"name": "rfs_read", "arguments": {"path": "gated.txt"}}),
    );
    let entered = gate.join("entered");
    let entered_deadline = Instant::now() + Duration::from_secs(2);
    while !entered.exists() {
        assert!(
            Instant::now() < entered_deadline,
            "filesystem delivery gate was not entered"
        );
        thread::sleep(Duration::from_millis(10));
    }

    process.change_client_roots(vec![mcp_root(&retained, "workspace")]);
    let refresh_id = process.send_request("tools/list", json!({}));
    let refreshed = process.receive_response(refresh_id, true);
    assert!(
        refreshed.get("error").is_none(),
        "changed root refresh failed: {refreshed}"
    );
    fs::write(gate.join("release"), "release").expect("release delivery gate");

    let response = process.receive_response(read_id, true);
    assert!(
        response.get("error").is_none(),
        "tool call failed: {response}"
    );
    assert_tool_error(&response["result"], "invalid_reference");
    process.finish();
}

#[test]
fn launch_primary_selection_is_exact() {
    let temporary = TempDir::new().expect("temporary directory");
    let alpha = temporary.path().join("alpha");
    let beta = temporary.path().join("beta");

    fs::create_dir(&alpha).expect("alpha root");
    fs::create_dir(&beta).expect("beta root");
    fs::write(alpha.join("shared.txt"), "alpha").expect("alpha fixture");
    fs::write(beta.join("shared.txt"), "beta").expect("beta fixture");
    let roots = [("alpha", alpha.as_path()), ("beta", beta.as_path())];

    let mut no_primary = McpProcess::start_with_roots(&roots, None);
    no_primary.initialize(VERSION_2026);
    assert_tool_error(&no_primary.call_read("shared.txt"), "ambiguous_reference");
    assert_eq!(
        no_primary.call_read("rfs://workspace/alpha/shared.txt")["structuredContent"]["content"],
        "alpha"
    );
    assert_eq!(
        no_primary.call_read("rfs://workspace/beta/shared.txt")["structuredContent"]["content"],
        "beta"
    );
    no_primary.finish();

    let mut selected = McpProcess::start_with_roots(&roots, Some("beta"));
    selected.initialize(VERSION_2026);
    assert_eq!(
        selected.call_read("shared.txt")["structuredContent"]["content"],
        "beta"
    );
    selected.finish();

    let mut unmatched = McpProcess::start_with_roots(&roots, Some("missing"));
    unmatched.initialize(VERSION_2026);
    assert_tool_error(&unmatched.call_read("shared.txt"), "ambiguous_reference");
    unmatched.finish();

    let one_root = [("alpha", alpha.as_path())];
    let mut implicit = McpProcess::start_with_roots(&one_root, None);
    implicit.initialize(VERSION_2026);
    assert_eq!(
        implicit.call_read("shared.txt")["structuredContent"]["content"],
        "alpha"
    );
    implicit.finish();
}

#[test]
fn canonical_identity_stays_private() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize(VERSION_2026);
    let result = process.call_read("fixture.txt");
    let structured = &result["structuredContent"];
    assert_eq!(
        structured["canonicalReference"],
        "rfs://workspace/workspace/fixture.txt"
    );
    assert!(structured.get("backingFileUri").is_none());
    assert!(
        !serde_json::to_string(&result)
            .expect("serialize result")
            .contains(&fixture.root.to_string_lossy() as &str)
    );
    process.finish();
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
            "--root <ID=PATH>",
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
            "Workspace Root canonical URIs must be unique",
        ),
        (
            vec!["serve", "--root", &file_root, "--primary-root", "workspace"],
            "failed to open Workspace Root 'workspace'",
        ),
        (
            vec![
                "serve",
                "--root",
                &missing_root,
                "--primary-root",
                "workspace",
            ],
            "failed to open Workspace Root 'workspace'",
        ),
        (
            vec![
                "serve",
                "--root",
                &invalid_name,
                "--primary-root",
                "workspace",
            ],
            "Workspace Root ID must start",
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
