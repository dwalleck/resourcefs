use std::{
    fs,
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tempfile::TempDir;

const VERSION_2026: &str = "2026-07-28";

#[cfg(feature = "test-support")]
#[path = "support/profile_tls.rs"]
mod profile_tls;

#[path = "support/stdio.rs"]
mod stdio;
use stdio::{McpProcessPermit, finish_process, read_message_from, write_message_to};

const VERSION_2025: &str = "2025-11-25";
const SOURCE_CATALOG_TEXT: &str = concat!(
    "Mounted sources\n",
    "Next discovery step: rfs_read rfs://workspace\n",
    "Selectors: :N | :N-M | :N- | comma-separated ranges | :raw | :page:N\n",
    "artifact:// — artifact://<session>-<id>[:selector] — artifact://00000000000000000000000000000000-1\nlocal:// — local://<name>[:selector] (flat names; bare local:// lists this Path Session\'s scratch) — local://plan.md\n",
    "rfs://workspace — <relative-path> | rfs://workspace/<root>/<path>[:selector] | file://<absolute-path> (relative paths use the Primary Workspace Root) — rfs://workspace/workspace/src/lib.rs\n",
);
const WORKSPACE_CATALOG_TEXT: &str = "rfs://workspace/workspace/ (primary)\n";

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
    allow_harness_output: bool,
    _process_permit: McpProcessPermit,
    // Drop runs child shutdown before fields (including this directory) are released.
    _session_directory: Option<TempDir>,
}

impl McpProcess {
    fn start(root: &Path) -> Self {
        Self::start_with_roots(&[("workspace", root)], Some("workspace"))
    }

    fn start_with_roots(roots: &[(&str, &Path)], primary: Option<&str>) -> Self {
        Self::start_config(roots, primary, None, None, None, None)
    }

    fn start_profile(profile: &Path, current_directory: &Path) -> Self {
        Self::start_arguments(
            vec![
                "serve".to_owned(),
                "--config".to_owned(),
                profile.display().to_string(),
            ],
            None,
            None,
            None,
            None,
            Some(current_directory),
            &[],
        )
    }
    fn start_profile_with_env(
        profile: &Path,
        current_directory: &Path,
        environment: &[(&str, &str)],
    ) -> Self {
        Self::start_arguments(
            vec![
                "serve".to_owned(),
                "--config".to_owned(),
                profile.display().to_string(),
            ],
            None,
            None,
            None,
            None,
            Some(current_directory),
            environment,
        )
    }

    #[cfg(feature = "test-support")]
    fn start_profile_with_https_root(profile: &Path, current_directory: &Path) -> Self {
        Self::start_profile_with_https_root_and_env(profile, current_directory, &[])
    }

    #[cfg(feature = "test-support")]
    fn start_profile_with_https_root_and_env(
        profile: &Path,
        current_directory: &Path,
        environment: &[(&str, &str)],
    ) -> Self {
        let root_path = current_directory.join("profile-https-ca.der");
        fs::write(&root_path, profile_tls::root_certificate()).expect("write profile fixture CA");
        let executable = std::env::current_exe().expect("stdio contract test executable");
        let mut command = Command::new(executable);
        command
            .envs(environment.iter().copied())
            .args([
                "--ignored",
                "--exact",
                "profile_https_test_server",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(PROFILE_HTTPS_HELPER, "1")
            .env(PROFILE_HTTPS_CONFIG, profile)
            .env(PROFILE_HTTPS_ROOT, root_path)
            .current_dir(current_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let permit = McpProcessPermit::acquire();
        Self::from_child(
            command.spawn().expect("start profile HTTPS test server"),
            true,
            permit,
        )
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
        Self::start_arguments(
            arguments,
            delivery_gate,
            delivery_identity,
            session_root,
            storage_failure,
            None,
            &[],
        )
    }

    fn start_arguments(
        arguments: Vec<String>,
        delivery_gate: Option<&Path>,
        delivery_identity: Option<&str>,
        session_root: Option<&Path>,
        storage_failure: Option<&str>,
        current_directory: Option<&Path>,
        environment: &[(&str, &str)],
    ) -> Self {
        // Ordinary CLI fixtures must not sweep the user's shared session cache.
        // Profile storage and explicit per-test overrides remain authoritative.
        let session_directory = if session_root.is_none()
            && !arguments.iter().any(|argument| argument == "--config")
            && !environment
                .iter()
                .any(|(name, _)| *name == "RESOURCEFS_TEST_SESSION_ROOT")
        {
            Some(TempDir::new().expect("isolated child session directory"))
        } else {
            None
        };
        let mut command = Command::new(binary());
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = &session_directory {
            command.env("RESOURCEFS_TEST_SESSION_ROOT", directory.path());
        }
        if let Some(current_directory) = current_directory {
            command.current_dir(current_directory);
        }
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
        for (name, value) in environment {
            command.env(name, value);
        }
        let permit = McpProcessPermit::acquire();
        let mut process =
            Self::from_child(command.spawn().expect("start resourcefs"), false, permit);
        process._session_directory = session_directory;
        process
    }

    fn from_child(
        mut child: Child,
        allow_harness_output: bool,
        process_permit: McpProcessPermit,
    ) -> Self {
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
            allow_harness_output,
            _process_permit: process_permit,
            _session_directory: None,
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
                allow_harness_output: self.allow_harness_output,
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
                allow_harness_output,
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
                        allow_harness_output: *allow_harness_output,
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
        read_message_from(&mut self.stdout, self.allow_harness_output)
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
        finish_process(
            &mut self.child,
            &mut self.stdin,
            &mut self.stdout,
            self.allow_harness_output,
        )
    }
}

struct ResponseFields<'a> {
    stdin: &'a mut Option<ChildStdin>,
    stdout: &'a mut BufReader<ChildStdout>,
    client_roots: &'a mut Vec<Value>,
    root_list_calls: &'a mut usize,
    cancelled_root_requests: &'a mut usize,
    allow_harness_output: bool,
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
        allow_harness_output,
    } = fields;
    loop {
        let response = read_message_from(stdout, allow_harness_output);
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
    let sessions = session_root.join("resourcefs");
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
        let panicking = thread::panicking();
        let observed_status = self.child.try_wait();
        let should_terminate = !matches!(&observed_status, Ok(Some(_)));
        if should_terminate {
            let _kill_result = self.child.kill();
            let _wait_result = self.child.wait();
        }
        // Reporting a failed diagnostic write would use the same broken stderr.
        // Ignore only these write failures: panicking again during unwinding
        // would abort the test binary and lose the original failure.
        if panicking {
            let _ = writeln!(
                io::stderr(),
                "resourcefs child status before cleanup: {observed_status:?}"
            );
            if let Some(mut stderr) = self.child.stderr.take() {
                let mut diagnostics = String::new();
                if let Err(error) = stderr.read_to_string(&mut diagnostics) {
                    let _ = writeln!(
                        io::stderr(),
                        "failed to read resourcefs child stderr: {error}"
                    );
                }
                let _ = writeln!(
                    io::stderr(),
                    "resourcefs child stderr after failure:\n{diagnostics}"
                );
            }
        } else if let Err(error) = &observed_status {
            let _ = writeln!(
                io::stderr(),
                "failed to poll resourcefs test process during cleanup: {error}"
            );
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

#[cfg(feature = "test-support")]
fn assert_tool_success(result: &Value) -> &Value {
    let structured = &result["structuredContent"];
    assert_eq!(structured["ok"], true, "expected tool success: {result:#}");
    structured
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
fn initialization_and_all_tool_descriptions_advertise_catalog() {
    const CUE: &str = "Start with rfs_read of rfs:// to discover mounted sources.";
    let fixture = WorkspaceFixture::new();

    for version in [VERSION_2026, VERSION_2025] {
        let mut process = McpProcess::start(&fixture.root);
        let initialized = process.initialize(version);
        assert!(
            initialized["instructions"]
                .as_str()
                .is_some_and(|instructions| instructions.starts_with(CUE)),
            "{version}: {initialized}"
        );
        let response = process.request("tools/list", json!({}));
        let tools = response["result"]["tools"].as_array().expect("tools array");
        let mut names = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "rfs_edit",
                "rfs_glob",
                "rfs_read",
                "rfs_search",
                "rfs_write"
            ],
            "{version}"
        );
        for tool in tools {
            assert!(
                tool["description"]
                    .as_str()
                    .is_some_and(|description| description.starts_with(CUE)),
                "{version} {}: {tool}",
                tool["name"]
            );
        }
        process.finish();
    }
}

#[test]
fn catalog_root_lines_parse_and_teach() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize(VERSION_2026);
    let expected = "Resource 'rfs://workspace/workspace/' is a directory; enumerate it with rfs_glob or read a file below it; bounded directory listings are tracked by rfs-hwlm";

    for spelling in ["rfs://workspace/workspace/", "rfs://workspace/workspace"] {
        let result = process.call_read(spelling);
        assert_tool_error(&result, "unsupported_projection");
        assert_eq!(
            result["structuredContent"]["error"]["message"], expected,
            "{spelling}"
        );
        assert!(
            result["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains(expected)),
            "{spelling}: {result}"
        );
    }

    let stderr = process.finish();
    assert!(stderr.is_empty(), "unexpected server diagnostics: {stderr}");
}

#[test]
fn lists_and_calls_discovery_tools() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize(VERSION_2026);
    let response = process.request("tools/list", json!({}));
    let tools = response["result"]["tools"].as_array().expect("tools array");
    let mut names = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "rfs_edit",
            "rfs_glob",
            "rfs_read",
            "rfs_search",
            "rfs_write"
        ]
    );

    let tool = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing {name} schema"))
    };
    let read = tool("rfs_read");
    let search = tool("rfs_search");
    let glob = tool("rfs_glob");
    let write = tool("rfs_write");
    let edit = tool("rfs_edit");
    for (name, schema, required) in [
        ("rfs_read", &read["inputSchema"], json!(["path"])),
        ("rfs_search", &search["inputSchema"], json!(["pattern"])),
        ("rfs_glob", &glob["inputSchema"], json!(["path"])),
        (
            "rfs_write",
            &write["inputSchema"],
            json!(["path", "content"]),
        ),
        ("rfs_edit", &edit["inputSchema"], json!(["patch"])),
    ] {
        assert_eq!(schema["type"], "object", "{name} input root");
        assert_eq!(schema["additionalProperties"], false, "{name} strict input");
        assert_eq!(schema["required"], required, "{name} required fields");
    }
    let search_path = &search["inputSchema"]["properties"]["path"];
    assert!(
        accepts_type(search_path, "string") && !accepts_type(search_path, "null"),
        "rfs_search optional path must reject explicit null: {search_path}"
    );
    for field in ["ifVersion", "operationId"] {
        let property = &write["inputSchema"]["properties"][field];
        assert!(
            accepts_type(property, "string") && !accepts_type(property, "null"),
            "[C3] rfs_write optional {field} must reject explicit null: {property}"
        );
    }

    let assert_limits =
        |name: &str, schema: &Value, definition: &str, fields: &[(&str, u64, u64)]| {
            assert_eq!(
                schema["properties"]["limits"]["$ref"],
                format!("#/$defs/{definition}"),
                "{name} limits reference"
            );
            let limits = &schema["$defs"][definition];
            assert_eq!(limits["type"], "object", "{name} limits root");
            assert_eq!(
                limits["additionalProperties"], false,
                "{name} strict limits"
            );
            for (member, minimum, maximum) in fields {
                let property = &limits["properties"][member];
                assert!(
                    accepts_type(property, "integer"),
                    "{name} limits.{member} integer schema: {property}"
                );
                assert_eq!(property["minimum"], *minimum, "{name} limits.{member} min");
                assert_eq!(property["maximum"], *maximum, "{name} limits.{member} max");
            }
        };
    assert_limits(
        "rfs_read",
        &read["inputSchema"],
        "ReadLimitsInput",
        &[
            ("bytes", 1, 49_152),
            ("lines", 1, 3_000),
            ("columns", 1, 512),
        ],
    );
    assert_limits(
        "rfs_search",
        &search["inputSchema"],
        "SearchLimitsInput",
        &[
            ("maxResults", 1, 1_000),
            ("maxBytes", 1, 49_152),
            ("maxLines", 1, 3_000),
            ("maxColumns", 1, 512),
        ],
    );
    assert_limits(
        "rfs_glob",
        &glob["inputSchema"],
        "GlobLimitsInput",
        &[("maxResults", 1, 1_000)],
    );
    for (name, schema) in [
        ("rfs_read", &read["outputSchema"]),
        ("rfs_search", &search["outputSchema"]),
        ("rfs_glob", &glob["outputSchema"]),
    ] {
        assert_eq!(schema["type"], "object", "{name} output root");
        assert_eq!(
            schema["additionalProperties"], false,
            "{name} output strict"
        );
        for member in ["recoveryReference", "continuationReference"] {
            assert!(
                accepts_type(&schema["properties"][member], "string"),
                "{name} output must expose {member}: {}",
                schema["properties"][member]
            );
        }
        // Reads and searches can cross a paginated source; glob cannot.
        let names_source_continuation = accepts_type(
            &schema["properties"]["sourceContinuationReference"],
            "string",
        );
        assert_eq!(
            names_source_continuation,
            name != "rfs_glob",
            "{name} output sourceContinuationReference: {}",
            schema["properties"]["sourceContinuationReference"]
        );
    }
    for (name, schema) in [
        ("rfs_write", &write["outputSchema"]),
        ("rfs_edit", &edit["outputSchema"]),
    ] {
        assert_eq!(schema["type"], "object", "{name} output root");
        assert_eq!(
            schema["additionalProperties"], false,
            "{name} output strict"
        );
        for member in [
            "operation",
            "canonicalReference",
            "versionTag",
            "displayedRanges",
            "displayedEof",
        ] {
            assert!(
                schema["properties"].get(member).is_some(),
                "{name} output must expose {member}: {schema}"
            );
        }
    }

    let search_response = process.request(
        "tools/call",
        json!({
            "name": "rfs_search",
            "arguments": {"path": "fixture.txt", "pattern": "fixture"}
        }),
    );
    assert!(search_response.get("error").is_none(), "{search_response}");
    let search_result = &search_response["result"];
    assert_eq!(search_result["isError"], false);
    assert_eq!(search_result["structuredContent"]["ok"], true);
    assert_eq!(search_result["structuredContent"]["engine"], "rust_regex");
    assert_eq!(
        search_result["structuredContent"]["groups"][0]["reference"],
        "rfs://workspace/workspace/fixture.txt"
    );
    assert_eq!(
        search_result["structuredContent"]["groups"][0]["lines"][0],
        json!({"line": 1, "text": "fixture text"})
    );
    assert!(
        search_result["content"][0]["text"]
            .as_str()
            .expect("search TextContent")
            .contains("fixture text")
    );

    let glob_response = process.request(
        "tools/call",
        json!({"name": "rfs_glob", "arguments": {"path": "fixture.txt"}}),
    );
    assert!(glob_response.get("error").is_none(), "{glob_response}");
    let glob_result = &glob_response["result"];
    assert_eq!(glob_result["isError"], false);
    assert_eq!(glob_result["structuredContent"]["ok"], true);
    assert_eq!(
        glob_result["structuredContent"]["entries"][0],
        json!({
            "reference": "rfs://workspace/workspace/fixture.txt",
            "kind": "file"
        })
    );
    assert!(
        glob_result["content"][0]["text"]
            .as_str()
            .expect("glob TextContent")
            .contains("rfs://workspace/workspace/fixture.txt")
    );

    let unknown = process.request(
        "tools/call",
        json!({"name": "rfs_missing", "arguments": {}}),
    );
    assert_eq!(unknown["error"]["code"], -32602);
    process.finish();
}

#[test]
fn versioned_write_receipts_and_catalog_policy_work_over_stdio() {
    let fixture = WorkspaceFixture::new();
    let profile = fixture.root.join("mutation-profile.json");
    fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "workspace": {
                "roots": [{
                    "id": "workspace",
                    "path": fixture.root,
                    "grants": {"create": true, "update": true, "delete": true}
                }],
                "primaryRoot": "workspace"
            }
        }))
        .expect("profile JSON"),
    )
    .expect("mutation profile");
    let mut process = McpProcess::start_profile(&profile, &fixture.root);
    process.initialize(VERSION_2026);

    let created = process.request(
        "tools/call",
        json!({
            "name": "rfs_write",
            "arguments": {"path": "created.txt", "content": "secret authored body\n"}
        }),
    );
    assert!(created.get("error").is_none(), "{created}");
    let created = &created["result"];
    assert_eq!(created["isError"], false);
    assert_eq!(created["structuredContent"]["operation"], "created");
    assert_eq!(
        created["structuredContent"]["canonicalReference"],
        "rfs://workspace/workspace/created.txt"
    );
    assert_eq!(
        created["structuredContent"]["displayedRanges"],
        json!([{"startLine": 1, "endLine": 1}])
    );
    assert_eq!(created["structuredContent"]["displayedEof"], true);
    let first_tag = created["structuredContent"]["versionTag"]
        .as_str()
        .expect("created Version Tag")
        .to_owned();
    assert!(
        !created["content"][0]["text"]
            .as_str()
            .expect("receipt text")
            .contains("secret authored body"),
        "receipt must not echo Resource content"
    );
    assert_eq!(
        process.call_read("created.txt")["structuredContent"]["content"],
        "secret authored body\n"
    );

    let replaced = process.request(
        "tools/call",
        json!({
            "name": "rfs_write",
            "arguments": {
                "path": "created.txt",
                "content": "replacement\n",
                "ifVersion": first_tag
            }
        }),
    );
    assert!(replaced.get("error").is_none(), "{replaced}");
    let replaced = &replaced["result"];
    assert_eq!(replaced["isError"], false);
    assert_eq!(replaced["structuredContent"]["operation"], "replaced");
    let replacement_tag = replaced["structuredContent"]["versionTag"]
        .as_str()
        .expect("replacement Version Tag");

    let stale = process.request(
        "tools/call",
        json!({
            "name": "rfs_write",
            "arguments": {
                "path": "created.txt",
                "content": "stale\n",
                "ifVersion": first_tag
            }
        }),
    );
    assert!(stale.get("error").is_none(), "{stale}");
    assert_tool_error(&stale["result"], "version_conflict");
    assert_eq!(
        process.call_read("created.txt")["structuredContent"]["content"],
        "replacement\n"
    );

    let catalog = process.request(
        "tools/call",
        json!({
            "name": "rfs_write",
            "arguments": {"path": "rfs://", "content": "forbidden"}
        }),
    );
    assert!(catalog.get("error").is_none(), "{catalog}");
    assert_tool_error(&catalog["result"], "permission_denied");

    let edit = process.request(
        "tools/call",
        json!({
            "name": "rfs_edit",
            "arguments": {
                "patch": format!(
                    "[rfs://workspace/workspace/created.txt#{replacement_tag}]\nPUT 1.=1:\n+edited"
                )
            }
        }),
    );
    assert!(edit.get("error").is_none(), "{edit}");
    assert_eq!(edit["result"]["isError"], false);
    assert_eq!(edit["result"]["structuredContent"]["operation"], "edited");
    assert_eq!(
        process.call_read("created.txt")["structuredContent"]["content"],
        "edited\n"
    );
    let edited_tag = edit["result"]["structuredContent"]["versionTag"]
        .as_str()
        .expect("edited Version Tag");
    let rem = process.request(
        "tools/call",
        json!({
            "name": "rfs_edit",
            "arguments": {
                "patch": format!(
                    "[rfs://workspace/workspace/created.txt#{edited_tag}]\nREM"
                )
            }
        }),
    );
    assert!(rem.get("error").is_none(), "{rem}");
    assert_eq!(rem["result"]["isError"], false);
    assert_eq!(rem["result"]["structuredContent"]["operation"], "deleted");
    assert!(
        rem["result"]["structuredContent"]
            .get("versionTag")
            .is_none()
    );
    assert!(
        rem["result"]["structuredContent"]
            .get("displayedRanges")
            .is_none()
    );
    assert_tool_error(&process.call_read("created.txt"), "not_found");

    let move_source = process.request(
        "tools/call",
        json!({
            "name": "rfs_write",
            "arguments": {"path": "move-source.txt", "content": "moved content\n"}
        }),
    );
    assert!(move_source.get("error").is_none(), "{move_source}");
    let move_source = &move_source["result"]["structuredContent"];
    let moved = process.request(
        "tools/call",
        json!({
            "name": "rfs_edit",
            "arguments": {
                "patch": format!(
                    "[{}#{}]\nMV moved.txt",
                    move_source["canonicalReference"]
                        .as_str()
                        .expect("move source reference"),
                    move_source["versionTag"].as_str().expect("move source tag")
                )
            }
        }),
    );
    assert!(moved.get("error").is_none(), "{moved}");
    assert_eq!(moved["result"]["isError"], false);
    assert_eq!(moved["result"]["structuredContent"]["operation"], "moved");
    assert_eq!(
        moved["result"]["structuredContent"]["sourceReference"],
        "rfs://workspace/workspace/move-source.txt"
    );
    assert_eq!(
        moved["result"]["structuredContent"]["canonicalReference"],
        "rfs://workspace/workspace/moved.txt"
    );
    assert_tool_error(&process.call_read("move-source.txt"), "not_found");
    assert_eq!(
        process.call_read("moved.txt")["structuredContent"]["content"],
        "moved content\n"
    );

    process.finish();
}

#[test]
fn invalid_operation_id_precedes_dispatch() {
    let fixture = WorkspaceFixture::new();
    let profile = fixture.root.join("operation-id-profile.json");
    fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "workspace": {
                "roots": [{
                    "id": "workspace",
                    "path": fixture.root,
                    "grants": {"create": true}
                }],
                "primaryRoot": "workspace"
            }
        }))
        .expect("[C3] profile JSON"),
    )
    .expect("[C3] profile");
    let mut process = McpProcess::start_profile(&profile, &fixture.root);
    process.initialize(VERSION_2026);

    for (operation_id, path) in [
        ("invalid id", "invalid-id.txt"),
        ("local-op", "non-creation-target.txt"),
    ] {
        let response = process.request(
            "tools/call",
            json!({
                "name": "rfs_write",
                "arguments": {
                    "path": path,
                    "content": "must not land",
                    "operationId": operation_id
                }
            }),
        );
        assert!(response.get("error").is_none(), "[C3] {response}");
        assert_tool_error(&response["result"], "invalid_reference");
        assert!(
            !fixture.root.join(path).exists(),
            "[C3] rejected before filesystem adapter access: {path}"
        );
    }

    process.finish();
}

#[test]
fn github_non_target_mutation_is_refused() {
    let fixture = WorkspaceFixture::new();
    let profile = fixture.root.join("github-mutation-profile.json");
    fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "sources": [{
                "kind": "github",
                "id": "github",
                "allowPrivateNetwork": false,
                "required": true,
                "grants": {"update": true},
                "credential": {"kind": "environment", "name": "RFS_GITHUB_TEST_TOKEN"},
                "repositories": [{
                    "name": "owner/repo",
                    "grants": {"update": true}
                }]
            }]
        }))
        .expect("[C14] profile JSON"),
    )
    .expect("[C14] profile");
    let mut process = McpProcess::start_profile_with_env(
        &profile,
        &fixture.root,
        &[("RFS_GITHUB_TEST_TOKEN", "fixture-token")],
    );
    process.initialize(VERSION_2026);
    let tag = format!("sha256:{}", "0".repeat(64));

    let aggregate = process.request(
        "tools/call",
        json!({
            "name": "rfs_write",
            "arguments": {
                "path": "issue://owner/repo/42",
                "content": "forbidden",
                "ifVersion": tag
            }
        }),
    );
    assert!(aggregate.get("error").is_none(), "[C14] {aggregate}");
    assert_tool_error(&aggregate["result"], "unsupported_mutation");
    let facts = process.request("tools/call", json!({
        "name":"rfs_write","arguments":{"path":"pr://owner/repo/7/facts","content":"forbidden","ifVersion":tag}
    }));
    assert_tool_error(&facts["result"], "unsupported_mutation");

    let edit = process.request(
        "tools/call",
        json!({
            "name": "rfs_edit",
            "arguments": {
                "patch": format!(
                    "[issue://owner/repo/42/title#{}]\nPUT 1.=1:\n+forbidden",
                    format!("sha256:{}", "0".repeat(64))
                )
            }
        }),
    );
    assert!(edit.get("error").is_none(), "[C14] {edit}");
    assert_tool_error(&edit["result"], "unsupported_mutation");
    process.finish();
}

#[test]
fn github_mutation_schema_and_receipts_match() {
    let fixture = WorkspaceFixture::new();
    let profile = fixture.root.join("github-creation-profile.json");
    let profile_json = |create: bool| {
        json!({
            "schemaVersion": 1,
            "sources": [{
                "kind": "github",
                "id": "github",
                "required": true,
                "allowPrivateNetwork": false,
                "grants": {"create": create, "update": true},
                "credential": {"kind": "environment", "name": "RFS_GITHUB_TEST_TOKEN"},
                "repositories": [{
                    "name": "owner/repo",
                    "grants": {"create": create, "update": true}
                }]
            }]
        })
    };
    fs::write(
        &profile,
        serde_json::to_vec(&profile_json(true)).expect("[C16] profile JSON"),
    )
    .expect("[C16] profile");
    let mut process = McpProcess::start_profile_with_env(
        &profile,
        &fixture.root,
        &[("RFS_GITHUB_TEST_TOKEN", "fixture-token")],
    );
    process.initialize(VERSION_2026);

    let listed = process.request("tools/list", json!({}));
    let write = listed["result"]["tools"]
        .as_array()
        .expect("[C16] tools")
        .iter()
        .find(|tool| tool["name"] == "rfs_write")
        .expect("[C16] rfs_write");
    let operation_id = &write["inputSchema"]["properties"]["operationId"];
    assert!(
        accepts_type(operation_id, "string") && !accepts_type(operation_id, "null"),
        "[C16] optional strict operationId schema: {operation_id}"
    );
    let version_tag = &write["outputSchema"]["properties"]["versionTag"];
    assert!(
        accepts_type(version_tag, "string") && accepts_type(version_tag, "null"),
        "[C16] mutation output versionTag is nullable: {version_tag}"
    );
    assert!(
        write["description"]
            .as_str()
            .is_some_and(|description| description.contains("operationId")),
        "[C16] tool description explains operationId"
    );

    for (arguments, category) in [
        (
            json!({
                "path": "issue://owner/repo/new",
                "content": "---\ntitle: missing close",
                "operationId": "malformed"
            }),
            "invalid_reference",
        ),
        (
            json!({
                "path": "issue://owner/repo/42",
                "content": "forbidden",
                "ifVersion": format!("sha256:{}", "0".repeat(64))
            }),
            "unsupported_mutation",
        ),
    ] {
        let response = process.request(
            "tools/call",
            json!({"name": "rfs_write", "arguments": arguments}),
        );
        assert!(response.get("error").is_none(), "[C16] {response}");
        assert_tool_error(&response["result"], category);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "[C16] non-empty operational error text"
        );
    }
    process.finish();

    fs::write(
        &profile,
        serde_json::to_vec(&profile_json(false)).expect("[C16] denied profile JSON"),
    )
    .expect("[C16] denied profile");
    let mut denied = McpProcess::start_profile_with_env(
        &profile,
        &fixture.root,
        &[("RFS_GITHUB_TEST_TOKEN", "fixture-token")],
    );
    denied.initialize(VERSION_2026);
    let response = denied.request(
        "tools/call",
        json!({
            "name": "rfs_write",
            "arguments": {
                "path": "issue://owner/repo/new",
                "content": "---\ntitle: Denied\n---\n",
                "operationId": "denied"
            }
        }),
    );
    assert!(response.get("error").is_none(), "[C16] {response}");
    assert_tool_error(&response["result"], "permission_denied");
    denied.finish();
}

#[cfg(feature = "test-support")]
#[test]
fn discovery_argument_matrix_precedes_io() {
    let fixture = WorkspaceFixture::new();
    let cases = [
        (
            "rfs_search",
            vec![
                json!({}),
                json!({"pattern": null}),
                json!({"pattern": 7}),
                json!({"pattern": "fixture", "unknown": true}),
                json!({"pattern": "fixture", "path": null}),
                json!({"pattern": "fixture", "caseSensitive": null}),
                json!({"pattern": "fixture", "gitignore": null}),
                json!({"pattern": "fixture", "hidden": null}),
                json!({"pattern": "fixture", "skip": null}),
                json!({"pattern": "fixture", "skip": -1}),
                json!({"pattern": "fixture", "limits": null}),
                json!({"pattern": "fixture", "limits": []}),
                json!({"pattern": "fixture", "limits": {"unknown": 1}}),
                json!({"pattern": "fixture", "limits": {"maxResults": null}}),
                json!({"pattern": "fixture", "limits": {"maxResults": 0}}),
                json!({"pattern": "fixture", "limits": {"maxResults": 1_001}}),
                json!({"pattern": "fixture", "limits": {"maxBytes": 49_153}}),
                json!({"pattern": "fixture", "limits": {"maxLines": 3_001}}),
                json!({"pattern": "fixture", "limits": {"maxColumns": 513}}),
            ],
            vec![
                (json!({"pattern": ""}), "invalid_pattern"),
                (json!({"pattern": "("}), "invalid_pattern"),
                (json!({"pattern": "x".repeat(65_537)}), "limit_exceeded"),
            ],
            json!({
                "pattern": "x".repeat(65_536),
                "caseSensitive": true,
                "gitignore": true,
                "hidden": false,
                "skip": 0,
                "limits": {
                    "maxResults": 1_000,
                    "maxBytes": 49_152,
                    "maxLines": 3_000,
                    "maxColumns": 512
                }
            }),
            json!({"path": "fixture.txt", "pattern": "fixture"}),
        ),
        (
            "rfs_glob",
            vec![
                json!({}),
                json!({"path": null}),
                json!({"path": 7}),
                json!({"path": "*.txt", "unknown": true}),
                json!({"path": "*.txt", "caseSensitive": null}),
                json!({"path": "*.txt", "gitignore": null}),
                json!({"path": "*.txt", "hidden": null}),
                json!({"path": "*.txt", "skip": null}),
                json!({"path": "*.txt", "skip": -1}),
                json!({"path": "*.txt", "limits": null}),
                json!({"path": "*.txt", "limits": []}),
                json!({"path": "*.txt", "limits": {"unknown": 1}}),
                json!({"path": "*.txt", "limits": {"maxResults": null}}),
                json!({"path": "*.txt", "limits": {"maxResults": 0}}),
                json!({"path": "*.txt", "limits": {"maxResults": 1_001}}),
            ],
            vec![
                (json!({"path": ""}), "invalid_pattern"),
                (json!({"path": "\\"}), "invalid_pattern"),
            ],
            json!({
                "path": "*.txt",
                "caseSensitive": true,
                "gitignore": true,
                "hidden": false,
                "skip": 0,
                "limits": {"maxResults": 1_000}
            }),
            json!({"path": "fixture.txt"}),
        ),
    ];

    for (tool, invalid_shapes, operational_errors, boundary, defaults) in cases {
        let temporary = TempDir::new().expect("C16 gate directory");
        let gate = temporary.path().join("gate");
        fs::create_dir(&gate).expect("C16 gate");
        let mut process = McpProcess::start_gated(
            &[("workspace", &fixture.root)],
            Some("workspace"),
            &gate,
            "*",
        );
        process.initialize(VERSION_2026);
        let invalid_gate = gate.clone();
        let (stop_watcher, watcher_stop) = std::sync::mpsc::channel();
        let invalid_io_watcher = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if watcher_stop.try_recv().is_ok() {
                    return false;
                }
                if invalid_gate.join("entered").exists() {
                    fs::write(invalid_gate.join("release"), "release")
                        .expect("C16 release unexpectedly reached gate");
                    return true;
                }
                thread::sleep(Duration::from_millis(5));
            }
            false
        });

        for arguments in invalid_shapes {
            let response =
                process.request("tools/call", json!({"name": tool, "arguments": arguments}));
            assert_eq!(
                response["error"]["code"], -32602,
                "{tool} schema failure was not MCP invalid_params: {response}"
            );
        }
        stop_watcher.send(()).expect("C16 stop invalid-I/O watcher");
        assert!(
            !invalid_io_watcher.join().expect("C16 invalid-I/O watcher"),
            "{tool} invalid schema input reached source I/O"
        );
        for (arguments, category) in operational_errors {
            let response =
                process.request("tools/call", json!({"name": tool, "arguments": arguments}));
            assert!(
                response.get("error").is_none(),
                "{tool} operational validation became a protocol error: {response}"
            );
            assert_tool_error(&response["result"], category);
        }
        process.wait_for_path_absent(
            &gate.join("entered"),
            &format!("{tool} invalid input I/O gate"),
            Duration::from_millis(200),
        );

        let boundary_id =
            process.send_request("tools/call", json!({"name": tool, "arguments": boundary}));
        process.wait_for_path(&gate.join("entered"), &format!("{tool} boundary I/O"));
        fs::write(gate.join("release"), "release").expect("C16 release gate");
        let boundary_response = process.receive_response(boundary_id, true);
        assert!(
            boundary_response.get("error").is_none(),
            "{tool} exact boundary protocol failure: {boundary_response}"
        );
        assert_eq!(
            boundary_response["result"]["structuredContent"]["ok"], true,
            "{tool} exact boundary result"
        );

        let default_response =
            process.request("tools/call", json!({"name": tool, "arguments": defaults}));
        assert!(
            default_response.get("error").is_none(),
            "{tool} default call failed: {default_response}"
        );
        assert_eq!(
            default_response["result"]["structuredContent"]["totalRecords"], 1,
            "{tool} defaults hand table"
        );
        process.finish();
    }
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
        assert_eq!(structured["contractVersion"], "1.1.0");
        assert_eq!(
            structured["canonicalReference"],
            "rfs://workspace/workspace/fixture.txt"
        );
        assert_eq!(structured["contentType"], "text/plain; charset=utf-8");
        assert_eq!(structured["content"], "fixture text\n");
        assert_eq!(structured["mutable"], false);
        assert_eq!(structured["bounded"], false);
        assert_eq!(
            structured["displayedRanges"],
            json!([{"startLine": 1, "endLine": 1}])
        );
        assert_eq!(structured["displayedEof"], true);
        assert!(
            structured["versionTag"]
                .as_str()
                .is_some_and(|tag| tag.starts_with("sha256:") && tag.len() == 71)
        );
        assert!(structured.get("recoveryReference").is_none());
        assert_eq!(
            text,
            format!(
                "[{}#{}]\nDisplayed Lines: 1-1\nDisplayed EOF: true\n{}",
                structured["canonicalReference"]
                    .as_str()
                    .expect("canonical reference"),
                structured["versionTag"].as_str().expect("Version Tag"),
                structured["content"].as_str().expect("structured content")
            ),
            "complete success must render the header, coordinate metadata, and exact content"
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

        let numbered = process.call_read_arguments(json!({
            "path": "fixture.txt",
            "numbered": true
        }));
        assert_eq!(
            numbered["structuredContent"]["content"], "fixture text\n",
            "numbering never changes structured content"
        );
        assert_eq!(
            numbered["content"][0]["text"]
                .as_str()
                .expect("numbered text"),
            format!(
                "[{}#{}]\nDisplayed Lines: 1-1\nDisplayed EOF: true\n1:fixture text\n",
                numbered["structuredContent"]["canonicalReference"]
                    .as_str()
                    .expect("canonical reference"),
                numbered["structuredContent"]["versionTag"]
                    .as_str()
                    .expect("Version Tag"),
            )
        );

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
            // The HTTPS adapter is mounted, but this server declares no origins,
            // so a read reports an unconfigured source rather than a missing
            // one. Mutation of the same reference is `unsupported_mutation`
            // regardless of configuration — see `https_is_read_only`.
            ("https://example.com/file", "source_unavailable"),
            // A scheme with no family still fails at the grammar.
            ("ftp://example.com/file", "invalid_reference"),
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
                "[{}#{}]\nDisplayed Lines: 1-96\nDisplayed EOF: false\nRecovery Reference: {recovery}\nContinuation Reference: {continuation}\n{}",
                over_structured["canonicalReference"]
                    .as_str()
                    .expect("bounded canonical reference"),
                over_structured["versionTag"]
                    .as_str()
                    .expect("bounded Version Tag"),
                maximum_text(),
            ),
            "bounded text must render coordinate metadata, both references, and exact page content"
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
                "[{}#{}]\nDisplayed Lines: 1-1\nDisplayed EOF: true\nRecovery Reference: {recovery}\n{}",
                final_structured["canonicalReference"]
                    .as_str()
                    .expect("final canonical reference"),
                final_structured["versionTag"]
                    .as_str()
                    .expect("final Version Tag"),
                "x",
            ),
            "the final page must render coordinate metadata, the stable Recovery Reference, and no continuation"
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
fn namespace_catalog_tracks_client_root_replacement() {
    let temporary = TempDir::new().expect("temporary directory");
    let launch = temporary.path().join("launch");
    let alpha = temporary.path().join("alpha");
    let beta = temporary.path().join("beta");
    for root in [&launch, &alpha, &beta] {
        fs::create_dir(root).expect("root fixture");
    }
    fs::write(alpha.join("shared.txt"), "alpha").expect("alpha fixture");
    fs::write(beta.join("shared.txt"), "beta").expect("beta fixture");

    let mut process = McpProcess::start_with_roots(&[("launch", &launch)], None);
    process.initialize_with_roots(VERSION_2026, vec![mcp_root(&alpha, "alpha")]);
    let initial_catalog = process.call_read("rfs://workspace");
    assert_eq!(process.root_list_calls, 1);
    let alpha_file = process.call_read("shared.txt");
    let alpha_root = alpha_file["structuredContent"]["canonicalReference"]
        .as_str()
        .expect("alpha canonical reference")
        .strip_suffix("shared.txt")
        .expect("alpha root prefix");
    assert_eq!(
        initial_catalog["structuredContent"]["content"],
        format!("{alpha_root} (primary)\n")
    );
    let initial_tag = initial_catalog["structuredContent"]["versionTag"]
        .as_str()
        .expect("initial catalog tag")
        .to_owned();

    process.change_client_roots(vec![mcp_root(&beta, "beta")]);
    let replacement_catalog = process.call_read("rfs://workspace");
    assert_eq!(process.root_list_calls, 2);
    let beta_file = process.call_read("shared.txt");
    let beta_root = beta_file["structuredContent"]["canonicalReference"]
        .as_str()
        .expect("beta canonical reference")
        .strip_suffix("shared.txt")
        .expect("beta root prefix");
    assert_eq!(
        replacement_catalog["structuredContent"]["content"],
        format!("{beta_root} (primary)\n")
    );
    assert!(
        !replacement_catalog["structuredContent"]["content"]
            .as_str()
            .expect("replacement content")
            .contains(alpha_root)
    );
    assert_ne!(
        replacement_catalog["structuredContent"]["versionTag"],
        initial_tag
    );
    process.finish();
}

#[test]
fn catalog_search_and_glob_redirect_before_io() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize_with_roots(VERSION_2026, vec![mcp_root(&fixture.root, "workspace")]);

    for (tool, path) in [
        ("rfs_search", "rfs://"),
        ("rfs_search", "rfs://workspace"),
        ("rfs_glob", "rfs://"),
        ("rfs_glob", "rfs://workspace"),
    ] {
        let arguments = if tool == "rfs_search" {
            json!({"path": path, "pattern": "needle"})
        } else {
            json!({"path": path})
        };
        let response = process.request("tools/call", json!({"name": tool, "arguments": arguments}));
        assert!(response.get("error").is_none(), "{tool} {path}: {response}");
        let result = &response["result"];
        assert_tool_error(result, "unsupported_projection");
        let expected = format!("catalog references are read-only discovery; use rfs_read {path}");
        assert_eq!(
            result["structuredContent"]["error"]["message"], expected,
            "{tool} {path}"
        );
        assert!(
            result["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains(&expected)),
            "{tool} {path}: {result}"
        );
    }
    assert_eq!(process.root_list_calls, 0);
    process.finish();
}

#[test]
fn profile_launch_authority_reaches_stdio_and_scratch_profiles_have_no_cwd_root() {
    let temporary = TempDir::new().expect("profile stdio fixture");
    let profile_directory = temporary.path().join("profile");
    let launch_directory = temporary.path().join("launch");
    let profile_root = profile_directory.join("workspace");
    let client_root = temporary.path().join("client");
    fs::create_dir_all(&profile_root).expect("profile workspace");
    fs::create_dir(&launch_directory).expect("launch directory");
    fs::create_dir(&client_root).expect("client workspace");
    fs::write(profile_root.join("shared.txt"), "profile").expect("profile fixture");
    fs::write(launch_directory.join("shared.txt"), "launch").expect("launch fixture");
    fs::write(client_root.join("shared.txt"), "client").expect("client fixture");
    let profile = profile_directory.join("server.json");
    fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "schemaVersion":1,
            "workspace":{
                "roots":[{"id":"profile","path":"workspace"}],
                "primaryRoot":"profile"
            },
            "session":{"cacheDirectory":"cache","retentionTtlSeconds":0}
        }))
        .expect("serialize profile"),
    )
    .expect("write profile");

    let mut process = McpProcess::start_profile(&profile, &launch_directory);
    process.initialize_with_roots(VERSION_2026, Vec::new());
    assert_eq!(
        process.call_read("shared.txt")["structuredContent"]["content"],
        "profile"
    );
    process.change_client_roots(vec![mcp_root(&client_root, "client")]);
    assert_eq!(
        process.call_read("shared.txt")["structuredContent"]["content"],
        "client"
    );
    process.change_client_roots(Vec::new());
    assert_eq!(
        process.call_read("shared.txt")["structuredContent"]["content"],
        "profile"
    );
    process.finish();

    let scratch = profile_directory.join("scratch.json");
    fs::write(
        &scratch,
        serde_json::to_vec(&json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"scratch-cache","retentionTtlSeconds":0}
        }))
        .expect("serialize scratch profile"),
    )
    .expect("write scratch profile");
    let mut scratch_process = McpProcess::start_profile(&scratch, &launch_directory);
    scratch_process.initialize_with_roots(VERSION_2026, Vec::new());
    assert_tool_error(
        &scratch_process.call_read("shared.txt"),
        "ambiguous_reference",
    );
    scratch_process.finish();
}

#[test]
fn profile_limits_bound_live_mcp_reads_and_discovery() {
    let temporary = TempDir::new().expect("profile limits fixture");
    let workspace = temporary.path().join("workspace");
    let launch_directory = temporary.path().join("launch");
    fs::create_dir(&workspace).expect("workspace");
    fs::create_dir(&launch_directory).expect("launch directory");
    fs::write(workspace.join("first.txt"), "fixture first\n".repeat(20)).expect("first fixture");
    fs::write(workspace.join("second.txt"), "fixture third\n").expect("second fixture");
    let profile = temporary.path().join("limits.json");
    fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "schemaVersion":1,
            "workspace":{
                "roots":[{"id":"workspace","path":"workspace"}],
                "primaryRoot":"workspace"
            },
            "limits":{
                "text":{"bytes":100},
                "discovery":{"searchMatches":1,"globEntries":1,"listingEntries":1}
            },
            "session":{"cacheDirectory":"cache","retentionTtlSeconds":0}
        }))
        .expect("serialize bounded profile"),
    )
    .expect("write bounded profile");

    let mut process = McpProcess::start_profile(&profile, &launch_directory);
    process.initialize(VERSION_2026);
    let read = process.call_read("first.txt");
    assert_eq!(
        read["structuredContent"]["content"].as_str().map(str::len),
        Some(100)
    );
    assert_eq!(read["structuredContent"]["bounded"], true);
    let lower = process.call_read_arguments(json!({
        "path":"first.txt",
        "limits":{"bytes":3}
    }));
    assert_eq!(lower["structuredContent"]["content"], "fix");

    for (tool, arguments) in [
        ("rfs_search", json!({"pattern":"fixture"})),
        ("rfs_glob", json!({"path":"*.txt"})),
    ] {
        let response = process.request("tools/call", json!({"name":tool,"arguments":arguments}));
        assert!(response.get("error").is_none(), "{tool}: {response}");
        assert_eq!(
            response["result"]["structuredContent"]["returnedRecords"], 1,
            "{tool}: {response}"
        );
    }
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

#[cfg(feature = "test-support")]
#[test]
fn root_change_fences_discovery() {
    let temporary = TempDir::new().expect("C18 temporary directory");
    let removed = temporary.path().join("removed-discovery");
    let retained = temporary.path().join("retained-discovery");
    let launch = temporary.path().join("launch-discovery");
    fs::create_dir(&removed).expect("C18 removed root");
    fs::create_dir(&retained).expect("C18 retained root");
    fs::create_dir(&launch).expect("C18 launch root");
    fs::write(removed.join("gated.txt"), "removed needle\n").expect("C18 removed fixture");
    fs::write(retained.join("gated.txt"), "retained needle\n").expect("C18 retained fixture");

    for (tool, arguments) in [
        ("rfs_search", json!({"pattern": "needle"})),
        ("rfs_glob", json!({"path": "*.txt"})),
    ] {
        let gate = temporary.path().join(format!("{tool}-root-gate"));
        fs::create_dir(&gate).expect("C18 delivery gate");
        let mut process = McpProcess::start_gated(&[("launch", &launch)], None, &gate, "*");
        process.initialize_with_roots(VERSION_2026, vec![mcp_root(&removed, "workspace")]);

        let request_id =
            process.send_request("tools/call", json!({"name": tool, "arguments": arguments}));
        let roots_request = process.read_message();
        assert_eq!(
            roots_request["method"], "roots/list",
            "C18 {tool} did not acquire client roots before source I/O: {roots_request}"
        );
        process.root_list_calls += 1;
        process.write_message(&json!({
            "jsonrpc": "2.0",
            "id": roots_request["id"],
            "result": {"roots": process.client_roots},
        }));
        process.wait_for_path(&gate.join("entered"), &format!("{tool} delivery gate"));
        process.change_client_roots(vec![mcp_root(&retained, "workspace")]);
        let refresh_id = process.send_request("tools/list", json!({}));
        let refreshed = process.receive_response(refresh_id, true);
        assert!(
            refreshed.get("error").is_none(),
            "{tool} changed root refresh failed: {refreshed}"
        );
        fs::write(gate.join("release"), "release").expect("C18 release delivery gate");

        let response = process.receive_response(request_id, true);
        assert!(
            response.get("error").is_none(),
            "{tool} call failed at the protocol layer: {response}"
        );
        assert_tool_error(&response["result"], "invalid_reference");

        let follow_up = process.request(
            "tools/call",
            json!({
                "name": tool,
                "arguments": if tool == "rfs_search" {
                    json!({"pattern": "retained"})
                } else {
                    json!({"path": "*.txt"})
                }
            }),
        );
        assert_eq!(
            follow_up["result"]["structuredContent"]["totalRecords"], 1,
            "{tool} did not use the refreshed authoritative root"
        );
        assert!(
            follow_up["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("gated.txt")),
            "{tool} follow-up did not return the retained fixture"
        );
        process.finish();
    }
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
#[test]
fn catalogs_use_common_read_result_shape() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize(VERSION_2026);

    for (path, canonical, expected_content) in [
        ("rfs://", "rfs://", SOURCE_CATALOG_TEXT),
        ("rfs://workspace", "rfs://workspace", WORKSPACE_CATALOG_TEXT),
    ] {
        let result = process.call_read(path);
        assert_eq!(result["isError"], false, "{path}");
        let structured = result["structuredContent"]
            .as_object()
            .expect("structured catalog result");
        let mut keys = structured.keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "bounded",
                "canonicalReference",
                "content",
                "contentType",
                "contractVersion",
                "displayedEof",
                "displayedRanges",
                "mutable",
                "ok",
                "requestedPath",
                "versionTag",
            ],
            "{path}"
        );
        assert_eq!(structured["ok"], true, "{path}");
        assert_eq!(structured["requestedPath"], path, "{path}");
        assert_eq!(structured["canonicalReference"], canonical, "{path}");
        assert_eq!(
            structured["contentType"], "text/plain; charset=utf-8",
            "{path}"
        );
        assert_eq!(structured["mutable"], false, "{path}");
        assert_eq!(structured["bounded"], false, "{path}");
        assert_eq!(structured["content"], expected_content, "{path}");
        let displayed = structured["displayedRanges"]
            .as_array()
            .expect("catalog displayed ranges");
        assert_eq!(displayed.len(), 1, "{path}");
        assert_eq!(displayed[0]["startLine"], 1, "{path}");
        assert_eq!(structured["displayedEof"], true, "{path}");
        let end_line = displayed[0]["endLine"]
            .as_u64()
            .expect("catalog displayed end line");
        let tag = structured["versionTag"].as_str().expect("Version Tag");
        assert_eq!(
            result["content"][0]["text"],
            format!(
                "[{canonical}#{tag}]\nDisplayed Lines: 1-{end_line}\nDisplayed EOF: true\n{expected_content}"
            ),
            "{path}"
        );
    }
    process.finish();
}

/// C11 — Session Scratch reads report `mutable: true` through the public tools,
/// and the family root's synthetic listing reports `mutable: false`.
#[test]
fn scratch_reads_report_mutability() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize(VERSION_2026);

    // The family root is readable and read-only even before any scratch exists.
    let root = process.call_read("local://");
    assert_eq!(root["isError"], false);
    let root_structured = root["structuredContent"]
        .as_object()
        .expect("structured root listing");
    assert_eq!(root_structured["canonicalReference"], "local://");
    assert_eq!(
        root_structured["mutable"], false,
        "the scratch listing is a synthetic projection"
    );
    assert!(
        !root_structured["content"]
            .as_str()
            .expect("listing content")
            .trim()
            .is_empty(),
        "an empty scratch session still renders a status line"
    );

    // A named scratch Resource, created with no Workspace Mutation grant, is
    // readable and reports itself mutable.
    let write = process.request(
        "tools/call",
        json!({
            "name": "rfs_write",
            "arguments": {"path": "local://plan.md", "content": "scratch line\n"}
        }),
    );
    assert!(
        write.get("error").is_none(),
        "scratch write failed: {write}"
    );
    assert_eq!(write["result"]["isError"], false, "{write}");

    let read = process.call_read("local://plan.md");
    assert_eq!(read["isError"], false);
    let structured = read["structuredContent"]
        .as_object()
        .expect("structured scratch result");
    assert_eq!(structured["canonicalReference"], "local://plan.md");
    assert_eq!(
        structured["mutable"], true,
        "named Session Scratch is writable under session authority"
    );
    assert_eq!(structured["content"], "scratch line\n");

    // The root now lists it, and the entry re-parses to the same Resource.
    let listed = process.call_read("local://");
    let listing = listed["structuredContent"]["content"]
        .as_str()
        .expect("listing content");
    assert!(
        listing.lines().any(|line| line == "local://plan.md"),
        "the listing must name the created Resource: {listing}"
    );

    process.finish();
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

/// Reconstruct one JSON document from artifact pages without following a
/// source traversal cursor. A source cursor is returned separately so callers
/// can consume that next segment as its own document.
fn recover_artifact_document(
    process: &mut McpProcess,
    mut result: Value,
    limits: Value,
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
        result = process.call_read_arguments(json!({"path": next, "limits": limits.clone()}));
    }
    let document = serde_json::from_str(&bytes).expect("one recovered JSON document");
    (document, bytes, source_cursor)
}

#[test]
fn catalog_reads_obey_common_limits_and_reconstruct() {
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

    for limits in [
        json!({"bytes": 64}),
        json!({"lines": 1}),
        json!({"columns": 32}),
    ] {
        reconstruct_with_limits(&mut process, "rfs://", limits, SOURCE_CATALOG_TEXT);
    }
    reconstruct_with_limits(
        &mut process,
        "rfs://workspace",
        json!({"bytes": 16}),
        WORKSPACE_CATALOG_TEXT,
    );
    process.finish();
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
fn discovery_cancellation_preempts_client_root_acquisition() {
    let temporary = TempDir::new().expect("C17 root-acquisition temporary directory");
    let launch = temporary.path().join("launch");
    let client = temporary.path().join("client");
    fs::create_dir(&launch).expect("C17 launch root");
    fs::create_dir(&client).expect("C17 client root");
    fs::write(client.join("fixture.txt"), "needle\n").expect("C17 client fixture");

    for (tool, arguments) in [
        ("rfs_search", json!({"pattern": "needle"})),
        ("rfs_glob", json!({"path": "*.txt"})),
    ] {
        let mut process = McpProcess::start_with_roots(&[("launch", &launch)], None);
        process.initialize_with_roots(VERSION_2026, vec![mcp_root(&client, "workspace")]);
        let request_id = process.send_request(
            "tools/call",
            json!({"name": tool, "arguments": arguments.clone()}),
        );
        let roots_request = process.read_message();
        assert_eq!(
            roots_request["method"], "roots/list",
            "C17 {tool} did not enter client-root acquisition"
        );
        process.root_list_calls += 1;

        let started = Instant::now();
        process.notify_cancelled(request_id);
        let response = process.receive_response(request_id, true);
        let elapsed = started.elapsed();
        assert!(
            elapsed <= Duration::from_millis(250),
            "{tool} root-acquisition cancellation exceeded 250 ms: {elapsed:?}"
        );
        assert!(
            response.get("error").is_none(),
            "{tool} root-acquisition cancellation became a protocol error: {response}"
        );
        assert_tool_error(&response["result"], "cancelled");
        assert_eq!(
            process.cancelled_root_requests, 1,
            "{tool} leaked the abandoned roots/list request"
        );

        let follow_up =
            process.request("tools/call", json!({"name": tool, "arguments": arguments}));
        assert_eq!(
            follow_up["result"]["structuredContent"]["totalRecords"], 1,
            "{tool} did not recover root acquisition after cancellation: {follow_up}"
        );
        assert_eq!(
            process.root_list_calls, 2,
            "{tool} did not retry root acquisition after cancellation"
        );
        process.finish();
    }
}

#[cfg(feature = "test-support")]
#[test]
fn cancelled_discovery_returns_promptly_without_late_artifact() {
    let temporary = TempDir::new().expect("C17 temporary directory");
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).expect("C17 workspace");
    for index in 0..1_100 {
        fs::write(root.join(format!("f{index:04}.txt")), "needle\n")
            .expect("C17 discovery fixture");
    }

    for (tool, arguments, follow_up) in [
        (
            "rfs_search",
            json!({"pattern": "needle"}),
            json!({"path": "f0000.txt", "pattern": "needle"}),
        ),
        (
            "rfs_glob",
            json!({"path": "*.txt"}),
            json!({"path": "f0000.txt"}),
        ),
    ] {
        let gate = temporary.path().join(format!("{tool}-gate"));
        let session_root = temporary.path().join(format!("{tool}-cache"));
        fs::create_dir(&gate).expect("C17 gate");
        fs::create_dir(&session_root).expect("C17 session root");
        let mut process = McpProcess::start_gated_with_session_root(
            &[("workspace", &root)],
            Some("workspace"),
            &gate,
            "*",
            &session_root,
        );
        process.initialize(VERSION_2026);

        let request_id =
            process.send_request("tools/call", json!({"name": tool, "arguments": arguments}));
        process.wait_for_path(&gate.join("entered"), &format!("{tool} delivery gate"));
        let cancellation_gate = gate.clone();
        let (stop_watchdog, watchdog_stop) = std::sync::mpsc::channel();
        let cancellation_watchdog = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(300);
            while Instant::now() < deadline {
                if watchdog_stop.try_recv().is_ok() {
                    return false;
                }
                thread::sleep(Duration::from_millis(5));
            }
            fs::write(cancellation_gate.join("release"), "release").expect("C17 watchdog release");
            true
        });
        let started = Instant::now();
        process.notify_cancelled(request_id);
        let response = process.receive_response(request_id, true);
        let _ = stop_watchdog.send(());
        assert!(
            !cancellation_watchdog
                .join()
                .expect("C17 cancellation watchdog"),
            "{tool} did not return before the gated operation was released"
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed <= Duration::from_millis(250),
            "{tool} cancellation exceeded 250 ms: {elapsed:?}"
        );
        assert!(
            response.get("error").is_none(),
            "{tool} cancellation became a protocol error: {response}"
        );
        assert_tool_error(&response["result"], "cancelled");

        fs::write(gate.join("release"), "release").expect("C17 release delivery gate");
        let follow_up_response =
            process.request("tools/call", json!({"name": tool, "arguments": follow_up}));
        assert!(
            follow_up_response.get("error").is_none(),
            "{tool} follow-up failed: {follow_up_response}"
        );
        assert_eq!(
            follow_up_response["result"]["structuredContent"]["totalRecords"], 1,
            "{tool} cancellation damaged the Path Session"
        );
        thread::sleep(Duration::from_millis(100));
        assert_objects_empty(&session_directory(&session_root));
        process.finish();
    }
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

/// C13 (read-only half) — every mutation of an `https://` reference is refused
/// as `unsupported_mutation` through the public tools.
///
/// Distinct from the catalog and Artifact families, which are refused as
/// `permission_denied` because they exist and forbid writes; HTTPS is a
/// read-only *family*, so the refusal does not depend on whether any origin is
/// configured. That independence is the point: a build with no HTTPS origins
/// must still answer "this family is read-only" rather than "no such source".
#[test]
fn https_is_read_only() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize(VERSION_2026);

    let write = process.request(
        "tools/call",
        json!({
            "name": "rfs_write",
            "arguments": {
                "path": "https://example.com/doc",
                "content": "forbidden"
            }
        }),
    );
    assert!(write.get("error").is_none(), "{write}");
    assert_tool_error(&write["result"], "unsupported_mutation");

    let edit = process.request(
        "tools/call",
        json!({
            "name": "rfs_edit",
            "arguments": {
                "patch": "[https://example.com/doc#sha256:0000000000000000000000000000000000000000000000000000000000000000]\nPUT 1.=1:\n+forbidden"
            }
        }),
    );
    assert!(edit.get("error").is_none(), "{edit}");
    assert_tool_error(&edit["result"], "unsupported_mutation");
}

/// C14 — an optional source that fails its startup probe degrades: the server
/// starts, every other source keeps serving, and only that source's references
/// fail with `source_unavailable`.
///
/// Anchored at the tool, not at the launch helper: the claim is about what an
/// `rfs_read` returns, and the hops between a mounted source and the tool are
/// exactly where a degradation could be lost.
///
/// Three assertions carry it, none sufficient alone. The workspace read is the
/// **positive control** — it proves the server genuinely started and serves, so
/// the HTTPS refusal cannot be "nothing works".
///
/// The refusal **message** is asserted, not just its category, and that is
/// load-bearing rather than belt-and-braces. Deleting degradation and letting
/// the origin mount normally still yields `source_unavailable` here — the fetch
/// simply leaves and fails against the closed port with "request failed: error
/// sending request". Category alone is therefore not a unique explanation: a
/// dead port and a refused-before-egress degradation are indistinguishable at
/// that granularity, so a category-only fence would pass with the whole feature
/// removed. Only the startup-specific phrasing separates the two.
///
/// The final block is the controlled comparison: the same profile shape against
/// a live TLS origin must serve `/doc`. That proves the degraded assertion is
/// attributable to unreachability rather than to the source never mounting.
#[cfg(feature = "test-support")]
#[test]
fn optional_https_degrades_while_other_sources_serve() {
    let fixture = WorkspaceFixture::new();
    fs::write(fixture.root.join("local.txt"), "workspace still serves\n").expect("workspace file");

    let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("closed-port listener");
    let closed_port = closed.local_addr().expect("closed address").port();
    drop(closed);
    let closed_base = format!("https://localhost:{closed_port}/");
    let profile = write_https_profile(&fixture, "degraded", &closed_base, false);

    let mut process = McpProcess::start_profile_with_https_root(&profile, &fixture.root);
    process.initialize_with_roots(VERSION_2026, Vec::new());

    // Positive control: the server is alive and another source serves.
    assert_eq!(
        process.call_read("local.txt")["structuredContent"]["content"],
        "workspace still serves\n",
        "a degraded optional source must not stop other sources serving"
    );

    let refused = process.call_read(&format!("{closed_base}doc"));
    assert_tool_error(&refused, "source_unavailable");
    let text = refused["content"][0]["text"]
        .as_str()
        .expect("error TextContent")
        .to_owned();
    assert!(
        text.contains("unreachable at startup"),
        "the refusal must name degradation, not absence of configuration; got: {text}"
    );
    process.finish();

    let server = profile_tls::ProfileTlsServer::start(
        "<html><body><p>healthy origin served</p></body></html>",
    );
    let live_base = server.base_url();
    let reachable = write_https_profile(&fixture, "reachable", &live_base, false);
    let mut healthy = McpProcess::start_profile_with_https_root(&reachable, &fixture.root);
    healthy.initialize_with_roots(VERSION_2026, Vec::new());
    let served = healthy.call_read(&format!("{live_base}doc"));
    assert_tool_success(&served);
    assert_eq!(
        served["structuredContent"]["content"], "healthy origin served\n",
        "a reachable optional origin must serve a real document"
    );
    healthy.finish();
    assert!(
        server.accepts() <= 6,
        "healthy control exceeded its six-connection fixture budget"
    );
    assert!(
        server.requests().iter().any(|target| target == "/doc"),
        "the healthy control must observe the document request server-side"
    );
}

/// C14 — probing happens once at startup and never on the read path.
///
/// The listener is the oracle. A startup probe is a bare TCP connect, so it
/// shows up as exactly one accepted connection before the server answers
/// `initialize`; ordinary reads must add none. Counting accepts rather than
/// timing anything makes the claim observable from outside the process.
///
/// The reads here are of a workspace file, which never touches the network on
/// its own. That is the point: nothing about a local read *should* reach the
/// listener, so any connection it produces is a re-probe — which is exactly the
/// regression this guards, since re-validating source health per request is a
/// natural thing to add and would be invisible from inside the read's result.
#[test]
fn probing_happens_once_at_startup_not_per_read() {
    let fixture = WorkspaceFixture::new();
    fs::write(fixture.root.join("local.txt"), "served locally\n").expect("workspace file");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("probe listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let port = listener.local_addr().expect("listener address").port();

    let profile = fixture.root.join("probe-once.json");
    fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "schemaVersion":1,
            "workspace":{
                "roots":[{"id":"workspace","path":"."}],
                "primaryRoot":"workspace"
            },
            "session":{"cacheDirectory":"probe-once-cache","retentionTtlSeconds":0},
            "sources":[{
                // `required` here also makes this the plan's fourth stress
                // profile — required + reachable — whose expected outcome is a
                // server that starts and serves normally.
                "kind":"https","id":"web","required":true,
                "origins":[{
                    "baseUrl":format!("https://127.0.0.1:{port}/"),
                    "allowPrivateNetwork":true
                }]
            }]
        }))
        .expect("serialize probe-once profile"),
    )
    .expect("write probe-once profile");

    let drain = |listener: &std::net::TcpListener| {
        let mut count = 0;
        while let Ok((stream, _)) = listener.accept() {
            drop(stream);
            count += 1;
        }
        count
    };

    let mut process = McpProcess::start_profile(&profile, &fixture.root);
    process.initialize_with_roots(VERSION_2026, Vec::new());
    assert_eq!(
        drain(&listener),
        1,
        "startup must probe the configured origin exactly once"
    );

    for _ in 0..3 {
        assert_eq!(
            process.call_read("local.txt")["structuredContent"]["content"],
            "served locally\n"
        );
    }
    assert_eq!(
        drain(&listener),
        0,
        "an ordinary read must not re-probe a configured source"
    );
    process.finish();
}

#[cfg(feature = "test-support")]
const PROFILE_HTTPS_HELPER: &str = "RESOURCEFS_PROFILE_HTTPS_HELPER";
#[cfg(feature = "test-support")]
const PROFILE_HTTPS_CONFIG: &str = "RESOURCEFS_PROFILE_HTTPS_CONFIG";
#[cfg(feature = "test-support")]
const PROFILE_HTTPS_ROOT: &str = "RESOURCEFS_PROFILE_HTTPS_ROOT";

#[cfg(feature = "test-support")]
fn write_https_profile(
    fixture: &WorkspaceFixture,
    name: &str,
    base_url: &str,
    required: bool,
) -> PathBuf {
    let profile = fixture.root.join(format!("{name}.json"));
    fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "workspace": {
                "roots": [{"id": "workspace", "path": "."}],
                "primaryRoot": "workspace"
            },
            "session": {
                "cacheDirectory": format!("{name}-cache"),
                "retentionTtlSeconds": 0
            },
            "sources": [{
                "kind": "https",
                "id": "web",
                "required": required,
                "origins": [{
                    "baseUrl": base_url,
                    "allowPrivateNetwork": true
                }]
            }]
        }))
        .expect("serialize profile HTTPS fixture"),
    )
    .expect("write profile HTTPS fixture");
    profile
}

#[cfg(feature = "test-support")]
#[test]
#[ignore = "re-executed by profile HTTPS contract tests as the child MCP server"]
fn profile_https_test_server() {
    if std::env::var_os(PROFILE_HTTPS_HELPER).is_none() {
        return;
    }
    let profile =
        PathBuf::from(std::env::var_os(PROFILE_HTTPS_CONFIG).expect("profile helper config path"));
    let root_path =
        PathBuf::from(std::env::var_os(PROFILE_HTTPS_ROOT).expect("profile helper CA path"));
    let root_der = fs::read(root_path).expect("read profile fixture CA");
    let root = resourcefs_mcp::test_support::TestRootCertificate::from_der(&root_der)
        .expect("profile fixture CA");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("profile helper runtime");
    runtime
        .block_on(resourcefs_mcp::test_support::serve_profile_with_https_root(
            &profile, root,
        ))
        .expect("profile HTTPS test server");
}

#[test]
fn read_acquisition_validates_before_roots_and_reports_unsupported() {
    let fixture = WorkspaceFixture::new();
    let mut process = McpProcess::start(&fixture.root);
    process.initialize_with_roots(VERSION_2026, vec![mcp_root(&fixture.root, "workspace")]);
    for acquisition in [
        json!(null),
        json!([]),
        json!({"unknown": 1}),
        json!({"maxAttempts": null}),
        json!({"maxAttempts": 0}),
        json!({"maxAttempts": -1}),
        json!({"maxAttempts": 1.5}),
        json!({"maxAttempts": 11}),
    ] {
        let response = process.request(
            "tools/call",
            json!({
                "name": "rfs_read", "arguments": {"path": "fixture.txt", "acquisition": acquisition}
            }),
        );
        assert_eq!(response["error"]["code"], -32602, "{response}");
        assert_eq!(
            process.root_list_calls, 0,
            "malformed acquisition refreshed roots"
        );
    }
    let rejected = process.call_read_arguments(json!({
        "path": "fixture.txt", "acquisition": {"maxAttempts": 1}
    }));
    assert_eq!(rejected["isError"], true, "{rejected}");
    assert_eq!(
        rejected["structuredContent"]["error"]["category"],
        "unsupported_projection"
    );
    assert_eq!(
        rejected["structuredContent"]["error"]["details"]["reason"],
        "acquisition_controls_unsupported"
    );
    let unchanged = process.call_read("fixture.txt");
    assert_eq!(unchanged["structuredContent"]["content"], "fixture text\n");
    process.finish();
}

#[cfg(feature = "test-support")]
#[test]
fn github_profile_trust_reaches_real_stdio_reads_only_when_explicit() {
    let fixture = WorkspaceFixture::new();
    let body = json!({
        "id": 8001, "number": 7, "state": "open", "title": "Native fixture title",
        "body": "Native fixture body", "user": {"login": "author", "id": 2},
        "html_url": "https://github.example/owner/repo/pull/7",
        "created_at": "2026-08-20T01:02:03Z", "updated_at": "2026-08-21T02:03:04Z",
        "draft": false, "merged": false, "head": {"ref": "feature"}, "base": {"ref": "main"}
    })
    .to_string();
    let server = profile_tls::ProfileTlsServer::start_native(vec![
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/pulls/7".into(),
            body: body.clone(),
            headers: Vec::new(),
        },
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/pulls/7".into(),
            body,
            headers: Vec::new(),
        },
    ]);
    let profile = fixture.root.join("github-trust.json");
    fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "sources": [{
                "kind": "github", "id": "github", "required": true,
                "apiBaseUrl": server.base_url(), "allowPrivateNetwork": true,
                "credential": {"kind": "environment", "name": "RFS_GITHUB_TEST_TOKEN"},
                "repositories": [{"name": "owner/repo"}]
            }]
        }))
        .expect("GitHub profile JSON"),
    )
    .expect("GitHub profile");
    let environment = [("RFS_GITHUB_TEST_TOKEN", "fixture-token")];
    let mut trusted =
        McpProcess::start_profile_with_https_root_and_env(&profile, &fixture.root, &environment);
    trusted.initialize(VERSION_2026);
    let title = trusted.call_read("pr://owner/repo/7/title");
    assert_eq!(
        title["structuredContent"]["content"],
        "Native fixture title"
    );
    let body = trusted.call_read("pr://owner/repo/7/body");
    assert_eq!(body["structuredContent"]["content"], "Native fixture body");
    trusted.finish();
    let requests = server.recorded_requests();
    assert_eq!(requests.len(), 2);
    for (index, request) in requests.iter().enumerate() {
        assert_eq!(request.sequence, index + 1);
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/repos/owner/repo/pulls/7");
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "authorization" && value == "Bearer fixture-token")
        );
        assert!(
            request
                .headers
                .iter()
                .any(|(name, value)| name == "accept" && value == "application/vnd.github+json")
        );
    }
    // Even injecting the helper's environment cannot select trust in the shipped CLI.
    let root_path = fixture.root.join("profile-https-ca.der");
    let root_text = root_path.to_str().expect("fixture CA path");
    let mut ordinary = McpProcess::start_profile_with_env(
        &profile,
        &fixture.root,
        &[
            environment[0],
            (PROFILE_HTTPS_HELPER, "1"),
            (PROFILE_HTTPS_ROOT, root_text),
        ],
    );
    ordinary.initialize(VERSION_2026);
    let refused = ordinary.call_read("pr://owner/repo/7/title");
    assert_eq!(refused["isError"], true, "{refused}");
    ordinary.finish();
    assert_eq!(
        server.recorded_requests().len(),
        2,
        "system trust must refuse before HTTP"
    );
    let mut invalid_profile: Value =
        serde_json::from_slice(&fs::read(&profile).expect("profile bytes"))
            .expect("profile document");
    invalid_profile["sources"][0]["httpsRoot"] = json!(root_text);
    fs::write(
        &profile,
        serde_json::to_vec(&invalid_profile).expect("invalid profile JSON"),
    )
    .expect("invalid profile");
    let profile_text = profile.to_str().expect("profile path");
    let rejected_profile = run_invalid(&["check", "--config", profile_text]);
    assert!(
        !rejected_profile.status.success(),
        "profile cannot select fixture trust"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn https_uses_common_read_shape() {
    const EXPECTED_MARKDOWN: &str = "# Profile Héllo\n\nserved end to end.\n";
    const EXPECTED_TAG: &str =
        "sha256:516f99a604023e18d9e0ea55ed2f8a8e2d0f4b1f5ecfda0c84a52d70213acaef";

    let fixture = WorkspaceFixture::new();
    let server = profile_tls::ProfileTlsServer::start(
        "<html><body><h1>Profile Héllo</h1><p>served end to end.</p></body></html>",
    );
    let base_url = server.base_url();
    let profile = write_https_profile(&fixture, "common-shape", &base_url, true);
    let reference = format!("{base_url}doc");

    let mut process = McpProcess::start_profile_with_https_root(&profile, &fixture.root);
    process.initialize_with_roots(VERSION_2026, Vec::new());
    let result = process.call_read(&reference);
    let structured = assert_tool_success(&result)
        .as_object()
        .expect("structured HTTPS result");
    let mut keys = structured.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "bounded",
            "canonicalReference",
            "content",
            "contentType",
            "contractVersion",
            "displayedEof",
            "displayedRanges",
            "mutable",
            "ok",
            "requestedPath",
            "versionTag",
        ]
    );
    assert_eq!(structured["canonicalReference"], reference);
    assert_eq!(structured["contentType"], "text/markdown; charset=utf-8");
    assert_eq!(structured["versionTag"], EXPECTED_TAG);
    assert_eq!(structured["mutable"], false);
    assert_eq!(structured["bounded"], false);
    assert_eq!(structured["content"], EXPECTED_MARKDOWN);
    assert_eq!(
        structured["displayedRanges"],
        json!([{"startLine": 1, "endLine": 3}])
    );
    assert_eq!(structured["displayedEof"], true);
    assert!(structured.get("recoveryReference").is_none());
    assert_eq!(
        result["content"][0]["text"],
        format!(
            "[{reference}#{EXPECTED_TAG}]\nDisplayed Lines: 1-3\nDisplayed EOF: true\n{EXPECTED_MARKDOWN}"
        )
    );
    process.finish();
    assert!(
        server.accepts() <= 6,
        "common-shape read exceeded its six-connection fixture budget"
    );
    assert!(
        server.requests().iter().any(|target| target == "/doc"),
        "the profile-launched read must reach the TLS fixture"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn https_profile_read_recovers_bounded_markdown() {
    let fixture = WorkspaceFixture::new();
    let line = "x".repeat(511);
    let mut html = String::from("<html><body>");
    let mut expected = String::new();
    for index in 0..97 {
        html.push_str("<p>");
        html.push_str(&line);
        html.push_str("</p>");
        expected.push_str(&line);
        expected.push('\n');
        if index != 96 {
            expected.push('\n');
        }
    }
    html.push_str("</body></html>");
    assert!(
        expected.len() > 48 * 1024,
        "stress fixture must exceed the inline ceiling"
    );

    let server = profile_tls::ProfileTlsServer::start(&html);
    let base_url = server.base_url();
    let profile = write_https_profile(&fixture, "bounded-shape", &base_url, true);
    let reference = format!("{base_url}large");
    let mut process = McpProcess::start_profile_with_https_root(&profile, &fixture.root);
    process.initialize_with_roots(VERSION_2026, Vec::new());

    let first = process.call_read(&reference);
    let first_structured = assert_tool_success(&first);
    assert_eq!(first_structured["bounded"], true);
    assert_eq!(first_structured["displayedEof"], false);
    let recovery = first_structured["recoveryReference"]
        .as_str()
        .expect("bounded HTTPS recovery")
        .to_owned();
    assert!(recovery.starts_with("artifact://"));
    let mut continuation = Some(
        first_structured["continuationReference"]
            .as_str()
            .expect("bounded HTTPS continuation")
            .to_owned(),
    );
    let mut reconstructed = first_structured["content"]
        .as_str()
        .expect("first Markdown page")
        .to_owned();

    for _ in 0..4 {
        let Some(next) = continuation.take() else {
            break;
        };
        let page = process.call_read(&next);
        let structured = assert_tool_success(&page);
        assert_eq!(structured["recoveryReference"], recovery);
        reconstructed.push_str(
            structured["content"]
                .as_str()
                .expect("continued Markdown page"),
        );
        continuation = structured
            .get("continuationReference")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    assert!(
        continuation.is_none(),
        "artifact continuation must reach EOF within the fixture bound"
    );
    assert_eq!(
        reconstructed, expected,
        "artifact pages must recover every Markdown byte"
    );
    process.finish();
    assert!(
        server.accepts() <= 6,
        "bounded read exceeded its six-connection fixture budget"
    );
    assert!(
        server.requests().iter().any(|target| target == "/large"),
        "the bounded read must fetch the real TLS document"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn github_facts_stdio_reconstructs_native_json_without_reacquisition() {
    for large in [false, true] {
        let fixture = WorkspaceFixture::new();
        let mut native: Value =
            serde_json::from_str(include_str!("../../../.rfs-0n97/oracles/pr-native.json"))
                .expect("native fixture");
        if large {
            native["body"] = json!("雪 \"escaped\"\n".repeat(1500));
        }
        let server =
            profile_tls::ProfileTlsServer::start_native(vec![profile_tls::NativeResponse {
                path: "/repos/owner/repo/pulls/7".into(),
                body: native.to_string(),
                headers: Vec::new(),
            }]);
        let profile = fixture.root.join("github-facts.json");
        fs::write(
            &profile,
            json!({"schemaVersion":1,"sources":[{
                "kind":"github","id":"github","required":true,"apiBaseUrl":server.base_url(),
                "webOrigin":"https://github.example","allowPrivateNetwork":true,
                "credential":{"kind":"environment","name":"RFS_GITHUB_TEST_TOKEN"},
                "repositories":[{"name":"owner/repo"}],"acquisition":{"maxAttempts":1}
            }]})
            .to_string(),
        )
        .expect("profile");
        let mut process = McpProcess::start_profile_with_https_root_and_env(
            &profile,
            &fixture.root,
            &[("RFS_GITHUB_TEST_TOKEN", "facts-stdio-secret")],
        );
        process.initialize(VERSION_2026);
        let mut result = process.call_read_arguments(json!({"path":"pr://owner/repo/7/facts","limits":{"bytes": if large { 1024 } else { 49152 }}}));
        let mut bytes = String::new();
        let mut pages = 0;
        let mut root = None;
        loop {
            assert_eq!(result["isError"], false, "{result}");
            let output = &result["structuredContent"];
            bytes.push_str(output["content"].as_str().expect("JSON page"));
            pages += 1;
            assert!(pages < 1000, "recovery must progress");
            if root.is_none() {
                root = output["recoveryReference"].as_str().map(str::to_owned);
            }
            let Some(next) = output["continuationReference"].as_str() else {
                break;
            };
            assert_eq!(server.recorded_requests().len(), 1);
            result = process.call_read_arguments(json!({"path":next,"limits":{"bytes":4096}}));
        }
        assert_eq!(
            pages > 1,
            large,
            "small facts inline; large facts recovered"
        );
        let facts: Value = serde_json::from_str(&bytes).expect("reconstructed full JSON");
        assert_eq!(facts["schemaVersion"]["major"], 1);
        assert_eq!(facts["data"]["body"], native["body"]);
        assert_eq!(facts["data"]["id"], "9007199254740993");
        assert_eq!(facts["data"]["head"]["commitSha"], native["head"]["sha"]);
        assert_eq!(facts["acquisition"]["limits"]["maxAttempts"], 1);
        if let Some(root) = root {
            reconstruct_with_limits(&mut process, &root, json!({"bytes":1024}), &bytes);
        }
        assert_eq!(
            server.recorded_requests().len(),
            1,
            "recovery cannot acquire again"
        );
        let denied = process.call_read("pr://other/repo/7/facts");
        assert_tool_error(&denied, "permission_denied");
        assert_eq!(server.recorded_requests().len(), 1);
        assert!(!bytes.contains("facts-stdio-secret"));
        for (field, hard) in [
            ("maxAttempts", 10_u64),
            ("timeoutMs", 30000),
            ("maxResponseBytes", 8388608),
            ("maxAcceptedBodyBytes", 16777216),
            ("maxRepresentationBytes", 16777216),
        ] {
            for invalid in [
                Value::Null,
                json!(0),
                json!(-1),
                json!(1.5),
                json!("1"),
                json!(hard + 1),
                json!(u64::MAX),
            ] {
                let mut acquisition = json!({});
                acquisition[field] = invalid;
                let rejected = process.request("tools/call", json!({"name":"rfs_read","arguments":{"path":"pr://owner/repo/7/facts","acquisition":acquisition}}));
                assert_eq!(rejected["error"]["code"], -32602);
                assert_eq!(
                    server.recorded_requests().len(),
                    1,
                    "invalid controls cannot reach provider"
                );
            }
        }
        let tools = process.request("tools/list", json!({}));
        let read_tool = tools["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .find(|tool| tool["name"] == "rfs_read")
            .expect("read tool");
        assert!(
            read_tool["inputSchema"]["properties"]
                .get("acquisition")
                .is_some()
        );
        let catalog = process.call_read("rfs://");
        assert_tool_success(&catalog);
        assert!(
            catalog["structuredContent"]["content"]
                .as_str()
                .expect("source catalog content")
                .contains("facts")
        );
        process.finish();
        let requests = server.recorded_requests();
        assert_eq!(requests[0].method, "GET");
        assert_eq!(requests[0].path, "/repos/owner/repo/pulls/7");
        for (key, value) in [
            ("authorization", "Bearer facts-stdio-secret"),
            ("accept", "application/vnd.github+json"),
            ("x-github-api-version", "2022-11-28"),
        ] {
            assert!(
                requests[0]
                    .headers
                    .iter()
                    .any(|(name, actual)| name == key && actual == value)
            );
        }
    }
}

#[cfg(feature = "test-support")]
#[test]
fn github_facts_stdio_cancelled_http_read_cannot_publish_and_session_recovers() {
    let fixture = WorkspaceFixture::new();
    let (server, arrived, release) = profile_tls::ProfileTlsServer::start_blocked_native(
        include_str!("../../../.rfs-0n97/oracles/pr-native.json").to_owned(),
    );
    let profile = fixture.root.join("cancel-facts.json");
    fs::write(&profile, json!({"schemaVersion":1,"sources":[{
        "kind":"github","id":"github","required":true,"apiBaseUrl":server.base_url(),
        "webOrigin":"https://github.example","allowPrivateNetwork":true,
        "credential":{"kind":"environment","name":"RFS_GITHUB_TEST_TOKEN"},"repositories":[{"name":"owner/repo"}]
    }]}).to_string()).expect("profile");
    let mut process = McpProcess::start_profile_with_https_root_and_env(
        &profile,
        &fixture.root,
        &[("RFS_GITHUB_TEST_TOKEN", "cancel-test-secret")],
    );
    process.initialize(VERSION_2026);
    let cancelled = process.send_request(
        "tools/call",
        json!({"name":"rfs_read","arguments":{"path":"pr://owner/repo/7/facts"}}),
    );
    arrived
        .recv_timeout(Duration::from_secs(5))
        .expect("real GitHub HTTP request entered barrier");
    process.notify_cancelled(cancelled);
    // A subsequent serialized protocol response is the ordering barrier, not a sleep.
    let barrier = process.send_request("tools/list", json!({}));
    process.receive_response_until(barrier, cancelled, Duration::from_secs(5));
    release.notify_one();
    let next = process.send_request(
        "tools/call",
        json!({"name":"rfs_read","arguments":{"path":"pr://owner/repo/7/facts"}}),
    );
    let recovered = process.receive_response_until(next, cancelled, Duration::from_secs(10));
    assert_eq!(recovered["result"]["isError"], false, "{recovered}");
    let facts: Value = serde_json::from_str(
        recovered["result"]["structuredContent"]["content"]
            .as_str()
            .expect("small full facts"),
    )
    .expect("JSON");
    assert_eq!(facts["data"]["id"], "9007199254740993");
    assert_eq!(server.recorded_requests().len(), 2);
    process.finish();
}

#[cfg(feature = "test-support")]
#[test]
fn github_comment_facts_recover_without_reacquisition() {
    let fixture = WorkspaceFixture::new();
    let comment: Value = serde_json::from_str(include_str!(
        "../../../.rfs-bfwa/oracles/comment-native.json"
    ))
    .expect("native comment fixture");
    let mut records = Vec::new();
    for id in 9_001..9_009_u64 {
        let mut record = comment.clone();
        record["id"] = json!(id);
        record["url"] = json!(format!("@API@repos/owner/repo/issues/comments/{id}"));
        record["issue_url"] = json!("@API@repos/owner/repo/issues/7");
        record["html_url"] = json!(format!(
            "https://github.example/owner/repo/pull/7#issuecomment-{id}"
        ));
        record["body"] = json!("雪 \"escaped\"\n".repeat(300));
        records.push(record);
    }
    let server = profile_tls::ProfileTlsServer::start_native(vec![
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/pulls/7".into(),
            body: include_str!("../../../.rfs-0n97/oracles/pr-native.json").into(),
            headers: Vec::new(),
        },
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/issues/7/comments?per_page=100&page=1".into(),
            body: serde_json::to_string(&records).expect("page body"),
            headers: Vec::new(),
        },
    ]);
    let profile = fixture.root.join("github-comment-facts.json");
    fs::write(
        &profile,
        json!({"schemaVersion":1,"sources":[{
            "kind":"github","id":"github","required":true,"apiBaseUrl":server.base_url(),
            "webOrigin":"https://github.example","allowPrivateNetwork":true,
            "credential":{"kind":"environment","name":"RFS_GITHUB_TEST_TOKEN"},
            "repositories":[{"name":"owner/repo"}],"acquisition":{"maxAttempts":2}
        }]})
        .to_string(),
    )
    .expect("profile");
    let mut process = McpProcess::start_profile_with_https_root_and_env(
        &profile,
        &fixture.root,
        &[("RFS_GITHUB_TEST_TOKEN", "comment-facts-stdio-secret")],
    );
    process.initialize(VERSION_2026);
    let result = process.call_read_arguments(json!({
        "path":"pr://owner/repo/7/comments/facts","limits":{"bytes":4096}
    }));
    let root = result["structuredContent"]["recoveryReference"]
        .as_str()
        .expect("an oversized collection must name its recovery root")
        .to_owned();
    let (facts, bytes, source_cursor) =
        recover_artifact_document(&mut process, result, json!({"bytes": 1024}));
    assert!(
        source_cursor.is_none(),
        "the fixture has no source cursor to recover"
    );
    assert!(
        bytes.len() > 4_096,
        "the oversized collection must span artifact pages"
    );
    assert!(
        server.recorded_requests().len() == 2,
        "artifact recovery cannot acquire again"
    );
    assert_eq!(facts["kind"], "github.conversation_comment_collection");
    assert_eq!(facts["collection"]["acceptedCount"], 8);
    assert_eq!(facts["data"]["records"][0]["id"], "9001");
    assert_eq!(facts["data"]["records"][7]["id"], "9008");
    assert_eq!(
        facts["data"]["records"][0]["parent"]["number"], "7",
        "verified parent identity survives recovery"
    );
    reconstruct_with_limits(&mut process, &root, json!({"bytes": 1024}), &bytes);
    assert_eq!(
        server.recorded_requests().len(),
        2,
        "one acquisition per endpoint, recovery adds none"
    );
    assert!(!bytes.contains("comment-facts-stdio-secret"));
}

#[cfg(feature = "test-support")]
fn comment_fixture_record(template: &Value, id: u64, body: &str) -> Value {
    let mut record = template.clone();
    record["id"] = json!(id);
    record["url"] = json!(format!("@API@repos/owner/repo/issues/comments/{id}"));
    record["issue_url"] = json!("@API@repos/owner/repo/issues/7");
    record["html_url"] = json!(format!(
        "https://github.example/owner/repo/pull/7#issuecomment-{id}"
    ));
    record["body"] = json!(body);
    record
}

#[cfg(feature = "test-support")]
fn comment_facts_profile(
    fixture: &WorkspaceFixture,
    name: &str,
    server: &profile_tls::ProfileTlsServer,
    max_representation_bytes: usize,
) -> PathBuf {
    let profile = fixture.root.join(format!("{name}.json"));
    fs::write(
        &profile,
        json!({"schemaVersion":1,"sources":[{
            "kind":"github","id":"github","required":true,"apiBaseUrl":server.base_url(),
            "webOrigin":"https://github.example","allowPrivateNetwork":true,
            "credential":{"kind":"environment","name":"RFS_GITHUB_TEST_TOKEN"},
            "repositories":[{"name":"owner/repo"}],
            "acquisition":{"maxAttempts":2,"maxRepresentationBytes":max_representation_bytes}
        }]})
        .to_string(),
    )
    .expect("comment facts profile");
    profile
}

#[cfg(feature = "test-support")]
#[test]
fn github_comment_facts_inline_partial_keeps_source_cursor_separate() {
    let fixture = WorkspaceFixture::new();
    let template: Value = serde_json::from_str(include_str!(
        "../../../.rfs-bfwa/oracles/comment-native.json"
    ))
    .expect("native comment fixture");
    let first = comment_fixture_record(&template, 9_001, "first page");
    let tail = comment_fixture_record(&template, 9_002, "tail page");
    let first_page_body = serde_json::to_string(&vec![first]).expect("first page");
    let tail_page_body = serde_json::to_string(&vec![tail]).expect("tail page");
    let server = profile_tls::ProfileTlsServer::start_native(vec![
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/pulls/7".into(),
            body: include_str!("../../../.rfs-0n97/oracles/pr-native.json").into(),
            headers: Vec::new(),
        },
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/issues/7/comments?per_page=100&page=1".into(),
            body: first_page_body.clone(),
            headers: vec![(
                "Link".into(),
                "<@API@repos/owner/repo/issues/7/comments?per_page=100&page=2>; rel=\"next\""
                    .into(),
            )],
        },
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/pulls/7".into(),
            body: include_str!("../../../.rfs-0n97/oracles/pr-native.json").into(),
            headers: Vec::new(),
        },
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/issues/7/comments?per_page=100&page=2".into(),
            body: tail_page_body,
            headers: Vec::new(),
        },
    ]);
    let profile = comment_facts_profile(&fixture, "inline-partial", &server, 16_000_000);
    let mut process = McpProcess::start_profile_with_https_root_and_env(
        &profile,
        &fixture.root,
        &[("RFS_GITHUB_TEST_TOKEN", "inline-partial-secret")],
    );
    process.initialize(VERSION_2026);
    let first_page = process.call_read_arguments(json!({
        "path":"pr://owner/repo/7/comments/facts","limits":{"bytes":49152,"lines":3000}
    }));
    let structured = assert_tool_success(&first_page);
    assert!(structured.get("recoveryReference").is_none());
    let source_cursor = structured["continuationReference"]
        .as_str()
        .expect("inline partial source cursor")
        .to_owned();
    assert!(!source_cursor.starts_with("artifact://"));
    let (document, _, recovered_source_cursor) = recover_artifact_document(
        &mut process,
        first_page,
        json!({"bytes": 49_152, "lines": 3_000}),
    );
    assert_eq!(
        recovered_source_cursor.as_deref(),
        Some(source_cursor.as_str())
    );
    assert_eq!(document["collection"]["state"], "incomplete");
    assert_eq!(
        document["data"]["records"]
            .as_array()
            .expect("records")
            .len(),
        1
    );
    assert_eq!(server.recorded_requests().len(), 2);

    let next = process.call_read(&source_cursor);
    let (next_document, _, next_source_cursor) =
        recover_artifact_document(&mut process, next, json!({"bytes": 49_152}));
    assert!(next_source_cursor.is_none());
    assert_eq!(
        next_document["data"]["records"][0]["id"], "9002",
        "the source cursor is independently consumable"
    );
    assert_eq!(server.recorded_requests().len(), 4);
    process.finish();
}

#[cfg(feature = "test-support")]
#[test]
fn github_comment_facts_spilled_partial_recovers_one_document_and_exposes_source_cursor() {
    let fixture = WorkspaceFixture::new();
    let template: Value = serde_json::from_str(include_str!(
        "../../../.rfs-bfwa/oracles/comment-native.json"
    ))
    .expect("native comment fixture");
    let first_records = (9_001..=9_008)
        .map(|id| comment_fixture_record(&template, id, &"x".repeat(6_000)))
        .collect::<Vec<_>>();
    let tail = comment_fixture_record(&template, 9_009, &"z".repeat(80_000));
    let first_page_body = serde_json::to_string(&first_records).expect("first page");
    let tail_page_body = serde_json::to_string(&vec![tail]).expect("tail page");
    let server = profile_tls::ProfileTlsServer::start_native(vec![
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/pulls/7".into(),
            body: include_str!("../../../.rfs-0n97/oracles/pr-native.json").into(),
            headers: Vec::new(),
        },
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/issues/7/comments?per_page=100&page=1".into(),
            body: first_page_body,
            headers: vec![(
                "Link".into(),
                "<@API@repos/owner/repo/issues/7/comments?per_page=100&page=2>; rel=\"next\""
                    .into(),
            )],
        },
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/pulls/7".into(),
            body: include_str!("../../../.rfs-0n97/oracles/pr-native.json").into(),
            headers: Vec::new(),
        },
        profile_tls::NativeResponse {
            path: "/repos/owner/repo/issues/7/comments?per_page=100&page=2".into(),
            body: tail_page_body,
            headers: Vec::new(),
        },
    ]);
    let profile = comment_facts_profile(&fixture, "spilled-partial", &server, 16_000_000);
    let mut process = McpProcess::start_profile_with_https_root_and_env(
        &profile,
        &fixture.root,
        &[("RFS_GITHUB_TEST_TOKEN", "spilled-partial-secret")],
    );
    process.initialize(VERSION_2026);
    let first_page = process.call_read_arguments(json!({
        "path":"pr://owner/repo/7/comments/facts","limits":{"bytes":49152,"lines":3000}
    }));
    let structured = assert_tool_success(&first_page);
    let source_cursor = structured["sourceContinuationReference"]
        .as_str()
        .expect("spilled partial source cursor")
        .to_owned();
    let recovery = structured["recoveryReference"]
        .as_str()
        .expect("spilled partial recovery root")
        .to_owned();
    assert!(
        structured["continuationReference"]
            .as_str()
            .is_some_and(|reference| reference.starts_with("artifact://"))
    );
    let requests_after_acquisition = server.recorded_requests().len();
    assert_eq!(requests_after_acquisition, 2);
    let (document, bytes, recovered_source_cursor) =
        recover_artifact_document(&mut process, first_page, json!({"bytes": 4_096}));
    assert_eq!(
        recovered_source_cursor.as_deref(),
        Some(source_cursor.as_str())
    );
    assert!(bytes.len() > 49_152);
    assert_eq!(document["collection"]["state"], "incomplete");
    assert_eq!(
        document["data"]["records"]
            .as_array()
            .expect("records")
            .len(),
        8
    );
    assert_eq!(
        document["data"]["records"][0]["id"], "9001",
        "artifact recovery retains the first source segment"
    );
    assert_eq!(
        server.recorded_requests().len(),
        requests_after_acquisition,
        "artifact pages never reacquire the source"
    );
    let next = process.call_read(&source_cursor);
    let (next_document, _, _) =
        recover_artifact_document(&mut process, next, json!({"bytes": 4_096}));
    assert_eq!(next_document["data"]["records"][0]["id"], "9009");
    assert_eq!(server.recorded_requests().len(), 4);
    assert!(!bytes.contains("spilled-partial-secret"));
    assert!(recovery.starts_with("artifact://"));
    process.finish();
}
#[cfg(feature = "test-support")]
#[test]
fn catalog_advertises_github_repository_and_comment_facts_routes() {
    let fixture = WorkspaceFixture::new();
    let server = profile_tls::ProfileTlsServer::start_native(vec![]);
    let profile = fixture.root.join("github-catalog.json");

    fs::write(
        &profile,
        json!({"schemaVersion":1,"sources":[{
            "kind":"github","id":"github","required":true,"apiBaseUrl":server.base_url(),
            "webOrigin":"https://github.example","allowPrivateNetwork":true,
            "credential":{"kind":"environment","name":"RFS_GITHUB_TEST_TOKEN"},
            "repositories":[{"name":"owner/repo"}]
        }]})
        .to_string(),
    )
    .expect("profile");
    let mut process = McpProcess::start_profile_with_https_root_and_env(
        &profile,
        &fixture.root,
        &[("RFS_GITHUB_TEST_TOKEN", "catalog-secret")],
    );
    process.initialize(VERSION_2026);
    let result = process.call_read_arguments(json!({"path":"rfs://"}));
    assert_eq!(result["isError"], false, "{result}");
    let catalog = result["structuredContent"]["content"]
        .as_str()
        .expect("catalog text");
    let pull_request_route = catalog
        .lines()
        .find(|line| line.starts_with("pr://"))
        .expect("GitHub pull-request catalog route");
    assert!(
        pull_request_route.contains("pr://<owner>/<repository>[/<number>"),
        "repository listing is discoverable without requiring a number: {pull_request_route}"
    );
    assert!(
        pull_request_route.contains("comments[/facts"),
        "conversation-comment collection facts route is discoverable: {pull_request_route}"
    );
    assert!(
        pull_request_route.contains("/<id>/facts"),
        "singular comment facts route is discoverable: {pull_request_route}"
    );
    process.finish();
}
