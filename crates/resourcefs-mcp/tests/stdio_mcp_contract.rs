use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
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
        fs::write(root.join("wide.txt"), format!("{}\n", "x".repeat(1_024)))
            .expect("overlong-line fixture");
        fs::write(root.join("unicode.txt"), "é".repeat(10)).expect("Unicode scalar fixture");
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
        Self::start_config(roots, primary, None, None, None, None)
    }

    #[cfg(feature = "test-support")]
    fn start_gated(
        roots: &[(&str, &Path)],
        primary: Option<&str>,
        gate: &Path,
        identity: &str,
    ) -> Self {
        Self::start_config(roots, primary, Some(gate), Some(identity), None, None)
    }

    #[cfg(feature = "test-support")]
    fn start_gated_with_session_root(
        roots: &[(&str, &Path)],
        primary: Option<&str>,
        gate: &Path,
        identity: &str,
        session_root: &Path,
    ) -> Self {
        Self::start_config(
            roots,
            primary,
            Some(gate),
            Some(identity),
            Some(session_root),
            None,
        )
    }

    fn start_with_session_root(
        roots: &[(&str, &Path)],
        primary: Option<&str>,
        session_root: &Path,
    ) -> Self {
        Self::start_config(roots, primary, None, None, Some(session_root), None)
    }

    #[cfg(feature = "test-support")]
    fn start_with_storage_failure(
        roots: &[(&str, &Path)],
        primary: Option<&str>,
        session_root: &Path,
        failure: &str,
    ) -> Self {
        Self::start_config(
            roots,
            primary,
            None,
            None,
            Some(session_root),
            Some(failure),
        )
    }

    fn start_config(
        roots: &[(&str, &Path)],
        primary: Option<&str>,
        delivery_gate: Option<&Path>,
        delivery_identity: Option<&str>,
        session_root: Option<&Path>,
        storage_failure: Option<&str>,
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
        if let Some(delivery_identity) = delivery_identity {
            command.env("RESOURCEFS_TEST_DELIVERY_IDENTITY", delivery_identity);
        }
        if let Some(session_root) = session_root {
            command.env("RESOURCEFS_TEST_SESSION_ROOT", session_root);
        }
        if let Some(storage_failure) = storage_failure {
            command.env("RESOURCEFS_TEST_STORAGE_FAILURE", storage_failure);
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
        receive_response_fields(
            ResponseFields {
                stdin: &mut self.stdin,
                stdout: &mut self.stdout,
                client_roots: &mut self.client_roots,
                root_list_calls: &mut self.root_list_calls,
                cancelled_root_requests: &mut self.cancelled_root_requests,
            },
            None,
            id,
            reply_to_roots,
        )
    }

    #[cfg(feature = "test-support")]
    fn receive_response_until(&mut self, id: u64, ignored_id: u64, timeout: Duration) -> Value {
        let (sender, receiver) = std::sync::mpsc::channel();
        thread::scope(|scope| {
            let McpProcess {
                child,
                stdin,
                stdout,
                client_roots,
                root_list_calls,
                cancelled_root_requests,
                ..
            } = self;
            scope.spawn(move || {
                let response = receive_response_fields(
                    ResponseFields {
                        stdin,
                        stdout,
                        client_roots,
                        root_list_calls,
                        cancelled_root_requests,
                    },
                    Some(ignored_id),
                    id,
                    true,
                );
                let _ = sender.send(response);
            });
            match receiver.recv_timeout(timeout) {
                Ok(response) => response,
                Err(_) => {
                    let _ = child.kill();
                    panic!("resourcefs did not respond within {timeout:?} to request {id}");
                }
            }
        })
    }

    fn read_message(&mut self) -> Value {
        read_message_from(&mut self.stdout)
    }

    fn write_message(&mut self, message: &Value) {
        write_message_to(&mut self.stdin, message);
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
        self.call_read_arguments(json!({"path": path}))
    }

    fn call_read_arguments(&mut self, arguments: Value) -> Value {
        let response = self.request(
            "tools/call",
            json!({"name": "rfs_read", "arguments": arguments}),
        );
        assert!(
            response.get("error").is_none(),
            "tool call failed: {response}"
        );
        response["result"].clone()
    }

    #[cfg(feature = "test-support")]
    fn notify_cancelled(&mut self, request_id: u64) {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {"requestId": request_id},
        }));
    }

    #[cfg(feature = "test-support")]
    fn wait_for_path(&self, marker: &Path, description: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {description} at {}",
                marker.display()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(feature = "test-support")]
    fn wait_for_path_absent(&self, marker: &Path, description: &str, window: Duration) {
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            assert!(
                !marker.exists(),
                "{description} appeared at {}",
                marker.display()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(feature = "test-support")]
    fn wait_for_session_marker(&self, session_root: &Path, marker: &str) -> PathBuf {
        let session = session_directory(session_root);
        self.wait_for_path(&session.join(marker), &format!("session {marker} marker"));
        session
    }

    #[cfg(feature = "test-support")]
    fn wait_for_exit(&mut self, timeout: Duration) -> std::process::ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("poll child") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "resourcefs did not exit within {timeout:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(feature = "test-support")]
    fn drain_remaining_stdout(&mut self) -> String {
        let mut remaining = String::new();
        self.stdout
            .read_to_string(&mut remaining)
            .expect("drain remaining stdout");
        remaining
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

fn read_message_from(stdout: &mut BufReader<ChildStdout>) -> Value {
    let mut line = String::new();
    let bytes = stdout.read_line(&mut line).expect("read response");
    assert_ne!(bytes, 0, "resourcefs closed stdout before responding");
    let response: Value = serde_json::from_str(&line)
        .unwrap_or_else(|error| panic!("stdout was not JSON-RPC: {error}: {line:?}"));
    assert_eq!(response["jsonrpc"], "2.0");
    response
}

fn write_message_to(stdin: &mut Option<ChildStdin>, message: &Value) {
    let stdin = stdin.as_mut().expect("open child stdin");
    serde_json::to_writer(&mut *stdin, message).expect("serialize message");
    writeln!(stdin).expect("write message delimiter");
    stdin.flush().expect("flush message");
}

struct ResponseFields<'a> {
    stdin: &'a mut Option<ChildStdin>,
    stdout: &'a mut BufReader<ChildStdout>,
    client_roots: &'a mut Vec<Value>,
    root_list_calls: &'a mut usize,
    cancelled_root_requests: &'a mut usize,
}

fn receive_response_fields(
    fields: ResponseFields<'_>,
    ignored_id: Option<u64>,
    id: u64,
    reply_to_roots: bool,
) -> Value {
    let ResponseFields {
        stdin,
        stdout,
        client_roots,
        root_list_calls,
        cancelled_root_requests,
    } = fields;
    loop {
        let response = read_message_from(stdout);
        assert_eq!(response["jsonrpc"], "2.0");
        if response.get("method") == Some(&Value::String("roots/list".to_owned())) {
            *root_list_calls += 1;
            if reply_to_roots {
                let reply = json!({
                    "jsonrpc": "2.0",
                    "id": response["id"],
                    "result": {"roots": client_roots},
                });
                write_message_to(stdin, &reply);
            }
            continue;
        }
        if response.get("method") == Some(&Value::String("notifications/cancelled".to_owned())) {
            *cancelled_root_requests += 1;
            continue;
        }
        if response["id"].as_u64().is_some_and(|response_id| {
            ignored_id.is_some_and(|ignored_id| response_id == ignored_id)
        }) {
            if let Some(result) = response.get("result") {
                assert_eq!(
                    result["isError"], true,
                    "cancelled request returned a success result: {response}"
                );
            }
            continue;
        }
        assert_eq!(response["id"], id);
        return response;
    }
}

#[cfg(feature = "test-support")]
fn session_directory(session_root: &Path) -> PathBuf {
    let sessions = session_root.join("sessions");
    let mut directories: Vec<PathBuf> = fs::read_dir(&sessions)
        .expect("sessions directory")
        .map(|entry| entry.expect("session entry").path())
        .filter(|path| path.is_dir())
        .collect();
    assert_eq!(
        directories.len(),
        1,
        "expected exactly one Path Session under {}",
        sessions.display()
    );
    directories.remove(0)
}

#[cfg(feature = "test-support")]
fn assert_objects_empty(session_dir: &Path) {
    let entries: Vec<PathBuf> = fs::read_dir(session_dir.join("objects"))
        .expect("objects directory")
        .map(|entry| entry.expect("object entry").path())
        .collect();
    assert!(
        entries.is_empty(),
        "Path Session must not publish artifact objects: {entries:?}"
    );
}

#[cfg(feature = "test-support")]
fn assert_no_success_response(drained: &str, read_id: u64) {
    for line in drained.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let message: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("drained stdout was not JSON-RPC: {error}: {line:?}"));
        if message.get("id") != Some(&Value::Number(read_id.into())) {
            continue;
        }
        if let Some(result) = message.get("result") {
            assert_eq!(
                result["isError"], true,
                "a disconnected session must never deliver a success result: {message}"
            );
            assert_eq!(result["structuredContent"]["ok"], false);
        }
    }
}

fn accepts_type(schema: &Value, expected: &str) -> bool {
    match schema.get("type") {
        Some(Value::String(actual)) => actual == expected,
        Some(Value::Array(types)) => types.iter().any(|actual| *actual == expected),
        _ => false,
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

    let input = &tools[0]["inputSchema"];
    assert_eq!(input["type"], "object");
    assert_eq!(input["additionalProperties"], false);
    assert_eq!(input["required"], json!(["path"]), "only path is required");
    let limits_property = &input["properties"]["limits"];
    assert_eq!(
        limits_property["$ref"], "#/$defs/ReadLimitsInput",
        "limits must reference its strict object schema: {limits_property}"
    );
    let limits = &input["$defs"]["ReadLimitsInput"];
    assert_eq!(
        limits["type"], "object",
        "limits must be an object: {limits}"
    );
    assert_eq!(
        limits["additionalProperties"], false,
        "limits must reject unknown members: {limits}"
    );
    for (member, minimum, maximum) in [
        ("bytes", 1, 49_152),
        ("lines", 1, 3_000),
        ("columns", 1, 512),
    ] {
        let schema = &limits["properties"][member];
        assert!(
            accepts_type(schema, "integer"),
            "limits.{member} must be an integer schema: {schema}"
        );
        assert_eq!(schema["minimum"], minimum, "limits.{member} minimum");
        assert_eq!(schema["maximum"], maximum, "limits.{member} maximum");
    }

    let output = &tools[0]["outputSchema"];
    assert_eq!(output["type"], "object");
    assert_eq!(output["additionalProperties"], false);
    assert_eq!(
        output["$defs"]["ReadErrorOutput"]["additionalProperties"],
        false
    );
    for member in ["recoveryReference", "continuationReference"] {
        assert!(
            accepts_type(&output["properties"][member], "string"),
            "output schema must expose {member} as a string: {}",
            output["properties"][member]
        );
    }
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
            ),
            "complete success must render the header, metadata-free body, and exact content"
        );
        assert!(
            !text.contains("Recovery Reference:"),
            "unbounded text must not carry a Recovery Reference line: {text:?}"
        );
        assert!(
            !text.contains("Continuation Reference:"),
            "unbounded text must not carry a continuation line: {text:?}"
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
        assert_eq!(
            over_limit["isError"], false,
            "one byte over the ceiling must spill instead of failing: {over_limit}"
        );
        let over_structured = &over_limit["structuredContent"];
        assert_eq!(
            over_structured["content"],
            maximum_text(),
            "the first page must hold the exact maximum 49,152-byte page"
        );
        assert_eq!(over_structured["bounded"], true);
        let recovery = over_structured["recoveryReference"]
            .as_str()
            .expect("spill Recovery Reference")
            .to_owned();
        let continuation = over_structured["continuationReference"]
            .as_str()
            .expect("spill continuation")
            .to_owned();
        assert!(recovery.starts_with("artifact://"));
        assert!(continuation.starts_with("artifact://"));
        let over_text = over_limit["content"][0]["text"]
            .as_str()
            .expect("over-limit TextContent");
        assert!(!over_text.is_empty());
        assert_eq!(
            over_text,
            format!(
                "[{}#{}]\nRecovery Reference: {recovery}\nContinuation Reference: {continuation}\n{}",
                over_structured["canonicalReference"]
                    .as_str()
                    .expect("bounded canonical reference"),
                over_structured["versionTag"]
                    .as_str()
                    .expect("bounded Version Tag"),
                maximum_text(),
            ),
            "bounded text must render the header, both references, and the exact page content"
        );

        let final_page = process.call_read(&continuation);
        assert_eq!(
            final_page["isError"], false,
            "following the continuation failed: {final_page}"
        );
        let final_structured = &final_page["structuredContent"];
        assert_eq!(
            final_structured["content"], "x",
            "the continuation must address the final unterminated line"
        );
        assert_eq!(
            final_structured["bounded"], false,
            "the final page must be complete"
        );
        assert_eq!(
            final_structured["recoveryReference"], recovery,
            "the recovery root must stay stable across pages"
        );
        assert!(
            final_structured.get("continuationReference").is_none(),
            "the final page must not carry a continuation"
        );
        let final_text = final_page["content"][0]["text"]
            .as_str()
            .expect("final page TextContent");
        assert_eq!(
            final_text,
            format!(
                "[{}#{}]\nRecovery Reference: {recovery}\n{}",
                final_structured["canonicalReference"]
                    .as_str()
                    .expect("final canonical reference"),
                final_structured["versionTag"]
                    .as_str()
                    .expect("final Version Tag"),
                "x",
            ),
            "the final page must render the header, the stable Recovery Reference, and no continuation"
        );

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

    let mut process = McpProcess::start_gated(&[("launch", &removed)], None, &gate, "*");
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

fn displayed_shape(content: &str) -> (usize, usize) {
    if content.is_empty() {
        return (0, 0);
    }
    let mut lines = 0_usize;
    let mut maximum_columns = 0_usize;
    for segment in content.split_inclusive('\n') {
        lines += 1;
        let body = segment.strip_suffix('\n').unwrap_or(segment);
        let body = body.strip_suffix('\r').unwrap_or(body);
        maximum_columns = maximum_columns.max(body.chars().count());
    }
    (lines, maximum_columns)
}

fn assert_page_within_limits(content: &str, limits: &Value) {
    if let Some(bytes) = limits.get("bytes").and_then(Value::as_u64) {
        assert!(
            content.len() as u64 <= bytes,
            "page has {} bytes, over the {bytes}-byte limit",
            content.len()
        );
    }
    if let Some(lines) = limits.get("lines").and_then(Value::as_u64) {
        let (line_count, _) = displayed_shape(content);
        assert!(
            line_count as u64 <= lines,
            "page has {line_count} lines, over the {lines}-line limit"
        );
    }
    if let Some(columns) = limits.get("columns").and_then(Value::as_u64) {
        let (_, maximum_columns) = displayed_shape(content);
        assert!(
            maximum_columns as u64 <= columns,
            "page has a {maximum_columns}-column line, over the {columns}-column limit"
        );
    }
}

fn reconstruct_with_limits(process: &mut McpProcess, path: &str, limits: Value, expected: &str) {
    let mut arguments = json!({"path": path, "limits": limits});
    let mut reconstructed = String::new();
    let mut recovery: Option<String> = None;
    let mut previous_continuation: Option<String> = None;
    let mut pages = 0_usize;
    loop {
        let result = process.call_read_arguments(arguments.clone());
        assert_eq!(
            result["isError"], false,
            "bounded page read failed: {result}"
        );
        let structured = &result["structuredContent"];
        let content = structured["content"].as_str().expect("page content");
        assert_page_within_limits(content, &arguments["limits"]);
        if structured["bounded"] == true {
            assert!(!content.is_empty(), "bounded pages must make progress");
        }
        if let Some(root) = recovery.as_ref() {
            assert_eq!(
                structured["recoveryReference"].as_str().expect("recovery"),
                root,
                "the recovery root must stay stable across pages"
            );
        } else {
            recovery = Some(
                structured["recoveryReference"]
                    .as_str()
                    .expect("spill Recovery Reference")
                    .to_owned(),
            );
        }
        let continuation = structured.get("continuationReference");
        assert_eq!(
            continuation.is_some(),
            structured["bounded"] == true,
            "a continuation must be present exactly while bounded"
        );
        let text = result["content"][0]["text"].as_str().expect("TextContent");
        reconstructed.push_str(content);
        pages += 1;
        assert!(pages < 100, "continuation chain did not terminate");

        let Some(next) = continuation else {
            assert_eq!(structured["bounded"], false);
            assert!(
                text.contains(recovery.as_deref().expect("recovery")),
                "final page TextContent must contain the Recovery Reference"
            );
            break;
        };
        let next = next.as_str().expect("continuation string");
        assert_ne!(
            Some(next),
            previous_continuation.as_deref(),
            "the continuation must progress"
        );
        assert!(
            text.contains(next),
            "TextContent must contain the continuation reference"
        );
        assert!(
            text.contains(recovery.as_deref().expect("recovery")),
            "bounded page TextContent must contain the Recovery Reference"
        );
        previous_continuation = Some(next.to_owned());
        arguments["path"] = Value::String(next.to_owned());
    }
    assert_eq!(
        reconstructed, expected,
        "concatenated pages must reconstruct the exact fixture bytes"
    );
}

#[test]
fn stdio_bounded_read_reconstructs_fixture() {
    let fixture = WorkspaceFixture::new();
    let temporary = TempDir::new().expect("temporary directory");
    let session_root = temporary.path().join("cache");
    fs::create_dir(&session_root).expect("session root");
    let mut process = McpProcess::start_with_session_root(
        &[("workspace", &fixture.root)],
        Some("workspace"),
        &session_root,
    );
    process.initialize(VERSION_2026);

    reconstruct_with_limits(
        &mut process,
        "maximum.txt",
        json!({"bytes": 8_192}),
        &maximum_text(),
    );
    reconstruct_with_limits(
        &mut process,
        "maximum.txt",
        json!({"lines": 10}),
        &maximum_text(),
    );
    let wide = format!("{}\n", "x".repeat(1_024));
    reconstruct_with_limits(&mut process, "wide.txt", json!({"columns": 256}), &wide);
    reconstruct_with_limits(
        &mut process,
        "unicode.txt",
        json!({"bytes": 7}),
        &"é".repeat(10),
    );
    process.finish();
}

#[cfg(feature = "test-support")]
#[test]
fn stdio_rejects_non_lower_limits() {
    let fixture = WorkspaceFixture::new();
    let temporary = TempDir::new().expect("temporary directory");
    let gate = temporary.path().join("gate");
    let session_root = temporary.path().join("cache");
    fs::create_dir(&gate).expect("gate directory");
    fs::create_dir(&session_root).expect("session root");
    let identity = "rfs://workspace/workspace/fixture.txt";
    let mut process = McpProcess::start_gated_with_session_root(
        &[("workspace", &fixture.root)],
        Some("workspace"),
        &gate,
        identity,
        &session_root,
    );
    process.initialize(VERSION_2026);

    let invalid_limits: Vec<Value> = vec![
        json!(null),
        json!(5),
        json!([]),
        json!("tight"),
        json!({"bytes": 0}),
        json!({"bytes": -1}),
        json!({"bytes": 1.5}),
        json!({"bytes": 49_153}),
        json!({"bytes": null}),
        json!({"bytes": "10"}),
        json!({"bytes": true}),
        json!({"lines": 0}),
        json!({"lines": 3_001}),
        json!({"lines": -3}),
        json!({"columns": 0}),
        json!({"columns": 513}),
        json!({"unknown": 7}),
        json!({"bytes": 5, "lines": 0}),
    ];
    for limits in &invalid_limits {
        let response = process.request(
            "tools/call",
            json!({"name": "rfs_read", "arguments": {"path": "fixture.txt", "limits": limits}}),
        );
        assert_eq!(
            response["error"]["code"], -32602,
            "limits shape {limits} must be invalid params: {response}"
        );
        assert!(
            response.get("result").is_none(),
            "invalid limits {limits} produced a result"
        );
    }
    process.wait_for_path_absent(
        &gate.join("entered"),
        "delivery gate entry",
        Duration::from_millis(200),
    );

    let proof_id = process.send_request(
        "tools/call",
        json!({"name": "rfs_read", "arguments": {"path": "fixture.txt"}}),
    );
    process.wait_for_path(&gate.join("entered"), "delivery gate entry");
    fs::write(gate.join("release"), "release").expect("release delivery gate");
    let proof = process.receive_response(proof_id, true);
    assert!(
        proof.get("error").is_none(),
        "gate-proof read failed: {proof}"
    );
    assert_eq!(
        proof["result"]["structuredContent"]["content"], "fixture text\n",
        "a valid call must reach the source once the gate is released"
    );

    let exact = process.call_read_arguments(json!({
        "path": "fixture.txt",
        "limits": {"bytes": 49_152, "lines": 3_000, "columns": 512},
    }));
    assert_eq!(exact["isError"], false);
    assert_eq!(exact["structuredContent"]["bounded"], false);
    assert_eq!(exact["structuredContent"]["content"], "fixture text\n");

    let empty_object = process.call_read_arguments(json!({
        "path": "fixture.txt",
        "limits": {},
    }));
    assert_eq!(
        empty_object["structuredContent"]["content"],
        "fixture text\n"
    );

    let bounded = process.call_read_arguments(json!({
        "path": "fixture.txt",
        "limits": {"bytes": 5},
    }));
    assert_eq!(bounded["structuredContent"]["bounded"], true);
    assert_eq!(bounded["structuredContent"]["content"], "fixtu");
    let continuation = bounded["structuredContent"]["continuationReference"]
        .as_str()
        .expect("bounded continuation")
        .to_owned();
    assert!(
        bounded["content"][0]["text"]
            .as_str()
            .expect("bounded TextContent")
            .contains(&continuation)
    );
    let middle = process.call_read_arguments(json!({
        "path": continuation,
        "limits": {"bytes": 5},
    }));
    assert_eq!(middle["structuredContent"]["bounded"], true);
    assert_eq!(middle["structuredContent"]["content"], "re te");
    let final_continuation = middle["structuredContent"]["continuationReference"]
        .as_str()
        .expect("middle continuation")
        .to_owned();
    let final_page = process.call_read_arguments(json!({
        "path": final_continuation,
        "limits": {"bytes": 5},
    }));
    assert_eq!(final_page["isError"], false);
    assert_eq!(final_page["structuredContent"]["bounded"], false);
    assert_eq!(final_page["structuredContent"]["content"], "xt\n");

    process.finish();
}

#[cfg(feature = "test-support")]
#[test]
fn stdio_storage_write_failure_is_complete_source_unavailable() {
    let fixture = WorkspaceFixture::new();
    let temporary = TempDir::new().expect("temporary directory");
    let session_root = temporary.path().join("cache");
    fs::create_dir(&session_root).expect("session root");
    let mut process = McpProcess::start_with_storage_failure(
        &[("workspace", &fixture.root)],
        Some("workspace"),
        &session_root,
        "write",
    );
    process.initialize(VERSION_2026);

    let result = process.call_read("over-limit.txt");
    assert_tool_error(&result, "source_unavailable");
    let structured = &result["structuredContent"];
    assert!(
        structured.get("content").is_none(),
        "a failed spill must not return page content"
    );
    assert!(
        structured.get("recoveryReference").is_none(),
        "a failed spill must not return a Recovery Reference"
    );
    assert!(
        structured.get("continuationReference").is_none(),
        "a failed spill must not return a continuation"
    );
    let session_dir = session_directory(&session_root);
    assert_objects_empty(&session_dir);
    process.finish();
}

#[cfg(feature = "test-support")]
#[test]
fn stdio_disconnect_marker_failure_is_observable() {
    let fixture = WorkspaceFixture::new();
    let temporary = TempDir::new().expect("temporary directory");
    let session_root = temporary.path().join("cache");
    fs::create_dir(&session_root).expect("session root");
    let mut process = McpProcess::start_with_storage_failure(
        &[("workspace", &fixture.root)],
        Some("workspace"),
        &session_root,
        "disconnect",
    );
    process.initialize(VERSION_2026);
    let session_dir = session_directory(&session_root);

    process.stdin.take();
    let status = process.wait_for_exit(Duration::from_secs(10));
    assert!(
        !status.success(),
        "disconnect marker failure must fail the server process"
    );
    assert!(
        !session_dir.join("disconnected").exists(),
        "failed marker persistence must not claim a clean disconnect"
    );
    assert_objects_empty(&session_dir);
    assert!(
        process.drain_remaining_stdout().trim().is_empty(),
        "disconnect failure emitted unexpected protocol output"
    );
    let mut stderr = String::new();
    process
        .child
        .stderr
        .take()
        .expect("child stderr")
        .read_to_string(&mut stderr)
        .expect("read child stderr");
    assert!(
        stderr.contains("mark session disconnected"),
        "disconnect failure was not observable on stderr: {stderr}"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn stdio_request_cancel_keeps_session_active_without_artifact() {
    let fixture = WorkspaceFixture::new();
    let temporary = TempDir::new().expect("temporary directory");
    let gate = temporary.path().join("gate");
    let session_root = temporary.path().join("cache");
    fs::create_dir(&gate).expect("gate directory");
    fs::create_dir(&session_root).expect("session root");
    let identity = "rfs://workspace/workspace/over-limit.txt";
    let mut process = McpProcess::start_gated_with_session_root(
        &[("workspace", &fixture.root)],
        Some("workspace"),
        &gate,
        identity,
        &session_root,
    );
    process.initialize(VERSION_2026);

    let read_id = process.send_request(
        "tools/call",
        json!({"name": "rfs_read", "arguments": {"path": "over-limit.txt"}}),
    );
    process.wait_for_path(&gate.join("entered"), "delivery gate entry");
    process.notify_cancelled(read_id);
    thread::sleep(Duration::from_millis(20));
    fs::write(gate.join("release"), "release").expect("release delivery gate");

    let follow_up_id = process.send_request(
        "tools/call",
        json!({"name": "rfs_read", "arguments": {"path": "fixture.txt"}}),
    );
    let follow_up = process.receive_response_until(follow_up_id, read_id, Duration::from_secs(10));
    assert!(
        follow_up.get("error").is_none(),
        "follow-up read failed: {follow_up}"
    );
    let follow_up = &follow_up["result"];
    assert_eq!(
        follow_up["structuredContent"]["content"], "fixture text\n",
        "cancelling one request must not disconnect the Path Session"
    );
    thread::sleep(Duration::from_millis(100));
    assert_objects_empty(&session_directory(&session_root));
    process.finish();
}

#[cfg(feature = "test-support")]
#[test]
fn stdio_eof_disconnects_before_release_and_leaves_no_object() {
    let fixture = WorkspaceFixture::new();
    let temporary = TempDir::new().expect("temporary directory");
    let gate = temporary.path().join("gate");
    let session_root = temporary.path().join("cache");
    fs::create_dir(&gate).expect("gate directory");
    fs::create_dir(&session_root).expect("session root");
    let identity = "rfs://workspace/workspace/over-limit.txt";
    let mut process = McpProcess::start_gated_with_session_root(
        &[("workspace", &fixture.root)],
        Some("workspace"),
        &gate,
        identity,
        &session_root,
    );
    process.initialize(VERSION_2026);

    let read_id = process.send_request(
        "tools/call",
        json!({"name": "rfs_read", "arguments": {"path": "over-limit.txt"}}),
    );
    process.wait_for_path(&gate.join("entered"), "delivery gate entry");
    drop(process.stdin.take());

    let session_dir = process.wait_for_session_marker(&session_root, "disconnected");
    assert!(
        !gate.join("release").exists(),
        "the disconnected marker must appear while the gated read is still in flight"
    );
    assert_objects_empty(&session_dir);

    fs::write(gate.join("release"), "release").expect("release delivery gate");
    let status = process.wait_for_exit(Duration::from_secs(10));
    assert!(status.success(), "resourcefs exited {status}");
    let drained = process.drain_remaining_stdout();
    assert_no_success_response(&drained, read_id);
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
