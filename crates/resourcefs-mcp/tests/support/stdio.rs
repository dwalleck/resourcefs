//! Shared test-only stdio framing, process admission, and EOF lifecycle.
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, ChildStdout},
    sync::{Condvar, LazyLock, Mutex},
    thread,
    time::{Duration, Instant},
};

const MAX_CONCURRENT_MCP_PROCESSES: usize = 16;
static MCP_PROCESS_GATE: LazyLock<(Mutex<usize>, Condvar)> =
    LazyLock::new(|| (Mutex::new(0), Condvar::new()));

pub(crate) struct McpProcessPermit;

impl McpProcessPermit {
    pub(crate) fn acquire() -> Self {
        let (active, available) = &*MCP_PROCESS_GATE;
        let mut active = active.lock().expect("MCP process gate");
        while *active >= MAX_CONCURRENT_MCP_PROCESSES {
            active = available.wait(active).expect("wait for MCP process permit");
        }
        *active += 1;
        Self
    }
}

impl Drop for McpProcessPermit {
    fn drop(&mut self) {
        let (active, available) = &*MCP_PROCESS_GATE;
        let mut active = active.lock().expect("MCP process gate");
        assert!(*active > 0, "MCP process permit count underflow");
        *active -= 1;
        available.notify_one();
    }
}

pub(crate) fn read_message_from(
    stdout: &mut BufReader<ChildStdout>,
    allow_harness_output: bool,
) -> Value {
    loop {
        let mut line = String::new();
        let bytes = stdout.read_line(&mut line).expect("read response");
        assert_ne!(bytes, 0, "resourcefs closed stdout before responding");
        let candidate = if allow_harness_output {
            line.find('{').map_or(line.as_str(), |start| &line[start..])
        } else {
            line.as_str()
        };
        match serde_json::from_str::<Value>(candidate) {
            Ok(response) => {
                assert_eq!(response["jsonrpc"], "2.0");
                return response;
            }
            Err(_) if allow_harness_output => continue,
            Err(error) => panic!("stdout was not JSON-RPC: {error}: {line:?}"),
        }
    }
}

pub(crate) fn write_message_to(stdin: &mut Option<ChildStdin>, message: &Value) {
    let stdin = stdin.as_mut().expect("open child stdin");
    serde_json::to_writer(&mut *stdin, message).expect("serialize message");
    writeln!(stdin).expect("write message delimiter");
    stdin.flush().expect("flush message");
}

pub(crate) fn finish_process(
    child: &mut Child,
    stdin: &mut Option<ChildStdin>,
    stdout: &mut BufReader<ChildStdout>,
    allow_harness_output: bool,
) -> String {
    stdin.take();
    // Bounds process teardown, which a loaded Windows CI runner has exceeded
    // at 2 s. Ten seconds still fails a server that genuinely hangs on stdin
    // close rather than one that is merely descheduled. See rfs-1e6h.
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "resourcefs did not exit after stdin closed"
        );
        thread::sleep(Duration::from_millis(10));
    };

    let mut remaining_stdout = String::new();
    stdout
        .read_to_string(&mut remaining_stdout)
        .expect("remaining stdout");
    if !allow_harness_output {
        assert!(
            remaining_stdout.trim().is_empty(),
            "unexpected protocol stdout after final response: {remaining_stdout:?}"
        );
    }

    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("child stderr")
        .read_to_string(&mut stderr)
        .expect("read child stderr");
    assert!(status.success(), "resourcefs exited {status}: {stderr}");
    stderr
}
