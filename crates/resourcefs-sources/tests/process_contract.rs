#[cfg(unix)]
use std::time::Instant as StdInstant;
use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{self, Command},
    sync::Arc,
    thread,
    time::Duration,
};

use resourcefs_core::OperationGuard;
use resourcefs_sources::{
    ChildEnvironment, CommandErrorKind, CommandExecutor, CommandInput, CommandLimits, CommandRole,
    CommandSpec, EnvironmentValue, SECRET_HELPER_STREAM_BYTES, SecretReference,
};
use tempfile::TempDir;
use tokio::{task::JoinHandle, time::sleep};

const FIXTURE_MODE: &str = "RFS_PROCESS_FIXTURE";
const INSPECT_BEGIN: &str = "RFS_INSPECT_BEGIN";
const INSPECT_END: &str = "RFS_INSPECT_END";

#[test]
fn fixture_process_child() {
    let Some(mode) = env::var_os(FIXTURE_MODE) else {
        return;
    };
    let result = match mode.to_string_lossy().as_ref() {
        "inspect" => fixture_inspect(),
        "stream" => fixture_stream(),
        "stdin" => fixture_stdin(),
        "marker" => fixture_marker(),
        "tree-parent" => fixture_tree_parent(),
        "tree-grandchild" => fixture_tree_grandchild(),
        _ => Err(io::Error::other("unknown process fixture mode")),
    };
    process::exit(if result.is_ok() { 0 } else { 97 });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_argv_and_environment() {
    if env::var_os("RFS_NESTED_DIRECT_TEST").is_none() {
        let output = Command::new(env::current_exe().expect("current test executable"))
            .args([
                "direct_argv_and_environment",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("RFS_NESTED_DIRECT_TEST", "1")
            .env("RFS_PARENT_SECRET_SENTINEL", "must-not-leak")
            .env("RFS_INHERIT_SOURCE", "forwarded")
            .output()
            .expect("nested direct environment contract");
        assert!(
            output.status.success(),
            "nested direct environment contract failed"
        );
        return;
    }

    let temporary = TempDir::new().expect("command base");
    let executor = CommandExecutor::new(1, temporary.path()).expect("executor");
    let literal = "literal;$HOME&|<>()";
    let unicode = "argument with spaces — 雪";
    let mut extra = BTreeMap::new();
    extra.insert(
        "RFS_EXPLICIT".to_owned(),
        EnvironmentValue::literal(literal).expect("literal environment"),
    );
    extra.insert(
        "RFS_RENAMED".to_owned(),
        EnvironmentValue::Inherit("RFS_INHERIT_SOURCE".to_owned()),
    );
    let spec = fixture_spec_with_args("inspect", &["--skip", literal, "--skip", unicode], extra);
    let output = executor
        .run(
            &spec,
            CommandRole::OneShot,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect("direct inspect command");
    let rendered = String::from_utf8(output.stdout().to_vec()).expect("UTF-8 fixture output");
    let inspection = rendered
        .split_once(INSPECT_BEGIN)
        .and_then(|(_, tail)| tail.split_once(INSPECT_END).map(|(body, _)| body))
        .expect("bounded inspection body");

    assert!(inspection.contains(&format!("ARG={literal}\n")));
    assert!(inspection.contains(&format!("ARG={unicode}\n")));
    assert!(inspection.contains(&format!("ENV=RFS_EXPLICIT={literal}\n")));
    assert!(inspection.contains("ENV=RFS_RENAMED=forwarded\n"));
    assert!(inspection.lines().any(|line| line.starts_with("ENV=PATH=")));
    #[cfg(windows)]
    assert!(
        inspection
            .lines()
            .any(|line| line.to_ascii_uppercase().starts_with("ENV=SYSTEMROOT="))
    );
    assert!(!inspection.contains("RFS_PARENT_SECRET_SENTINEL"));
    assert!(!inspection.contains("RFS_INHERIT_SOURCE"));
    let path = temporary.path().join("input with spaces.txt");
    fs::write(&path, "path input").expect("path fixture");
    let canonical = path.canonicalize().expect("canonical path fixture");
    let path_output = executor
        .run(
            &fixture_spec("inspect", BTreeMap::new()),
            CommandRole::OneShot,
            CommandInput::Path(&canonical),
            &OperationGuard::new(),
        )
        .await
        .expect("canonical path input");
    let path_inspection =
        String::from_utf8(path_output.stdout().to_vec()).expect("path inspection UTF-8");
    assert!(path_inspection.contains(&format!("ARG={}\n", canonical.display())));

    let mut recursive = BTreeMap::new();
    recursive.insert(
        "RFS_RECURSIVE".to_owned(),
        EnvironmentValue::Secret(
            SecretReference::environment("RFS_PARENT_SECRET_SENTINEL").expect("secret reference"),
        ),
    );
    let recursive_spec = fixture_spec("inspect", recursive);
    let error = executor
        .run(
            &recursive_spec,
            CommandRole::SecretHelper,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect_err("secret helper recursion must fail");
    assert_eq!(error.kind(), CommandErrorKind::InvalidEnvironment);
    assert_eq!(error.stdout_observed(), 0);
    assert_eq!(error.stderr_observed(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stream_boundaries() {
    let temporary = TempDir::new().expect("command base");
    let boundary = SECRET_HELPER_STREAM_BYTES;
    let limits =
        CommandLimits::new(Duration::from_secs(2), boundary, boundary).expect("stream test limits");
    let executor = CommandExecutor::new(1, temporary.path())
        .expect("executor")
        .with_role_limits(CommandRole::SecretHelper, limits)
        .expect("lower helper limits");

    let baseline = executor
        .run(
            &stream_spec(0, 0, "stdout-first"),
            CommandRole::SecretHelper,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect("stream baseline");
    assert!(baseline.stdout().len() < boundary);
    assert!(baseline.stderr().is_empty());
    let fixture_stdout = boundary - baseline.stdout().len();

    let exact = executor
        .run(
            &stream_spec(fixture_stdout, boundary, "stdout-first"),
            CommandRole::SecretHelper,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect("exact dual-stream boundary");
    assert_eq!(exact.stdout().len(), boundary);
    assert_eq!(exact.stderr().len(), boundary);

    let stdout_over = executor
        .run(
            &stream_spec(fixture_stdout + 1, boundary, "stderr-first"),
            CommandRole::SecretHelper,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect_err("stdout one-over must fail");
    assert_eq!(stdout_over.kind(), CommandErrorKind::LimitExceeded);
    assert_eq!(stdout_over.stdout_observed(), boundary + 1);
    assert_eq!(stdout_over.stderr_observed(), boundary);

    let stderr_over = executor
        .run(
            &stream_spec(fixture_stdout, boundary + 1, "stdout-first"),
            CommandRole::SecretHelper,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect_err("stderr one-over must fail");
    assert_eq!(stderr_over.kind(), CommandErrorKind::LimitExceeded);
    assert_eq!(stderr_over.stdout_observed(), boundary);
    assert_eq!(stderr_over.stderr_observed(), boundary + 1);

    let input_executor = CommandExecutor::new(1, temporary.path())
        .expect("input executor")
        .with_role_limits(
            CommandRole::OneShot,
            CommandLimits::new(Duration::from_secs(2), boundary, boundary).expect("input limits"),
        )
        .expect("lower input limits");
    let exact_input = vec![b'I'; boundary];
    let input_output = input_executor
        .run(
            &fixture_spec("stdin", BTreeMap::new()),
            CommandRole::OneShot,
            CommandInput::Stdin(&exact_input),
            &OperationGuard::new(),
        )
        .await
        .expect("exact stdin boundary");
    assert!(
        String::from_utf8_lossy(input_output.stdout()).contains(&format!("RFS_STDIN={boundary}")),
        "fixture did not receive the complete bounded stdin"
    );
    let over_input = vec![b'I'; boundary + 1];
    let input_error = input_executor
        .run(
            &fixture_spec("stdin", BTreeMap::new()),
            CommandRole::OneShot,
            CommandInput::Stdin(&over_input),
            &OperationGuard::new(),
        )
        .await
        .expect_err("stdin one-over must fail before spawn");
    assert_eq!(input_error.kind(), CommandErrorKind::LimitExceeded);
    assert_eq!(input_error.stdout_observed(), 0);
    assert_eq!(input_error.stderr_observed(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn process_admission() {
    admission_case(1).await;
    admission_case(32).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unix_tree_cleanup() {
    tree_cleanup().await;
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_tree_cleanup() {
    tree_cleanup().await;
}

async fn admission_case(ceiling: usize) {
    let temporary = TempDir::new().expect("command base");
    let executor = Arc::new(
        CommandExecutor::new(ceiling, temporary.path())
            .expect("executor")
            .with_role_limits(
                CommandRole::DownstreamCall,
                CommandLimits::new(Duration::from_secs(3), 8_192, 8_192).expect("holder limits"),
            )
            .expect("lower holder limits")
            .with_role_limits(
                CommandRole::SecretHelper,
                CommandLimits::new(Duration::from_millis(150), 8_192, 8_192)
                    .expect("queued limits"),
            )
            .expect("lower queued limits"),
    );

    let mut holders = Vec::new();
    for index in 0..ceiling {
        let marker = temporary.path().join(format!("holder-{index}"));
        let guard = OperationGuard::new();
        let task = spawn_run(
            Arc::clone(&executor),
            marker_spec(&marker, 5_000),
            CommandRole::DownstreamCall,
            guard.clone(),
        );
        holders.push((marker, guard, task));
    }
    for (marker, _, _) in &holders {
        wait_for_path(marker).await;
    }

    let queued_marker = temporary.path().join("queued-timeout");
    let queued_error = executor
        .run(
            &marker_spec(&queued_marker, 0),
            CommandRole::SecretHelper,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect_err("one-over admission must time out");
    assert_eq!(queued_error.kind(), CommandErrorKind::LimitExceeded);
    assert!(!queued_marker.exists(), "queued command must not start");

    if ceiling == 1 {
        let cancelled_marker = temporary.path().join("queued-cancelled");
        let cancelled_guard = OperationGuard::new();
        let queued = spawn_run(
            Arc::clone(&executor),
            marker_spec(&cancelled_marker, 0),
            CommandRole::DownstreamCall,
            cancelled_guard.clone(),
        );
        sleep(Duration::from_millis(50)).await;
        cancelled_guard.cancel();
        let cancelled = queued
            .await
            .expect("queued task")
            .expect_err("queued cancellation");
        assert_eq!(cancelled.kind(), CommandErrorKind::Cancelled);
        assert!(!cancelled_marker.exists());
    }

    for (_, guard, _) in &holders {
        guard.cancel();
    }
    for (_, _, task) in holders {
        let error = task
            .await
            .expect("holder task")
            .expect_err("holder cancellation");
        assert_eq!(error.kind(), CommandErrorKind::Cancelled);
    }

    let released_marker = temporary.path().join("released");
    executor
        .run(
            &marker_spec(&released_marker, 0),
            CommandRole::DownstreamCall,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect("permit released after cleanup");
    assert!(released_marker.exists());
}

async fn tree_cleanup() {
    let temporary = TempDir::new().expect("tree fixture");
    let report = temporary.path().join("pids");
    let heartbeat = temporary.path().join("heartbeat");
    let mut environment = BTreeMap::new();
    environment.insert(
        "RFS_TREE_REPORT".to_owned(),
        EnvironmentValue::literal(report.to_string_lossy()).expect("report path"),
    );
    environment.insert(
        "RFS_TREE_HEARTBEAT".to_owned(),
        EnvironmentValue::literal(heartbeat.to_string_lossy()).expect("heartbeat path"),
    );
    let spec = fixture_spec("tree-parent", environment);
    let executor = CommandExecutor::new(1, temporary.path())
        .expect("executor")
        .with_role_limits(
            CommandRole::OneShot,
            CommandLimits::new(Duration::from_millis(350), 8_192, 8_192).expect("tree limits"),
        )
        .expect("lower tree limits");
    let error = executor
        .run(
            &spec,
            CommandRole::OneShot,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect_err("resistant process tree must time out");
    assert_eq!(error.kind(), CommandErrorKind::LimitExceeded);

    let pids = fs::read_to_string(&report).expect("tree PID report");
    let pids = pids
        .lines()
        .map(|line| line.parse::<u32>().expect("reported PID"))
        .collect::<Vec<_>>();
    assert_eq!(pids.len(), 2);
    for (index, pid) in pids.into_iter().enumerate() {
        assert!(
            process_absent_or_zombie(pid),
            "owned PID {pid} at report index {index} survived cleanup"
        );
    }
    let first = fs::read_to_string(&heartbeat).expect("first heartbeat");
    sleep(Duration::from_millis(300)).await;
    let second = fs::read_to_string(&heartbeat).expect("second heartbeat");
    assert_eq!(first, second, "grandchild heartbeat continued after return");
}

fn spawn_run(
    executor: Arc<CommandExecutor>,
    spec: CommandSpec,
    role: CommandRole,
    guard: OperationGuard,
) -> JoinHandle<Result<resourcefs_sources::CommandOutput, resourcefs_sources::CommandError>> {
    tokio::spawn(async move { executor.run(&spec, role, CommandInput::None, &guard).await })
}

async fn wait_for_path(path: &Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "fixture did not start"
        );
        sleep(Duration::from_millis(10)).await;
    }
}

fn fixture_spec(mode: &str, extra: BTreeMap<String, EnvironmentValue>) -> CommandSpec {
    fixture_spec_with_args(mode, &[], extra)
}

fn fixture_spec_with_args(
    mode: &str,
    extra_args: &[&str],
    mut environment: BTreeMap<String, EnvironmentValue>,
) -> CommandSpec {
    environment.insert(
        FIXTURE_MODE.to_owned(),
        EnvironmentValue::literal(mode).expect("fixture mode"),
    );
    let mut argv = vec![
        env::current_exe()
            .expect("current test executable")
            .to_string_lossy()
            .into_owned(),
        "fixture_process_child".to_owned(),
        "--exact".to_owned(),
        "--nocapture".to_owned(),
        "--test-threads=1".to_owned(),
    ];
    argv.extend(extra_args.iter().map(|argument| (*argument).to_owned()));
    CommandSpec::new(
        argv,
        ChildEnvironment::new(environment).expect("fixture environment"),
    )
    .expect("fixture command")
}

fn stream_spec(stdout: usize, stderr: usize, order: &str) -> CommandSpec {
    let environment = [
        ("RFS_FIXTURE_STDOUT", stdout.to_string()),
        ("RFS_FIXTURE_STDERR", stderr.to_string()),
        ("RFS_FIXTURE_ORDER", order.to_owned()),
    ]
    .into_iter()
    .map(|(name, value)| {
        (
            name.to_owned(),
            EnvironmentValue::literal(value).expect("stream value"),
        )
    })
    .collect();
    fixture_spec("stream", environment)
}

fn marker_spec(marker: &Path, sleep_millis: u64) -> CommandSpec {
    let environment = [
        ("RFS_MARKER", marker.to_string_lossy().into_owned()),
        ("RFS_MARKER_SLEEP", sleep_millis.to_string()),
    ]
    .into_iter()
    .map(|(name, value)| {
        (
            name.to_owned(),
            EnvironmentValue::literal(value).expect("marker value"),
        )
    })
    .collect();
    fixture_spec("marker", environment)
}

fn fixture_inspect() -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{INSPECT_BEGIN}")?;
    for argument in env::args().skip(1) {
        writeln!(stdout, "ARG={argument}")?;
    }
    let mut environment = env::vars_os()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>();
    environment.sort_unstable();
    for (name, value) in environment {
        writeln!(stdout, "ENV={name}={value}")?;
    }
    writeln!(stdout, "{INSPECT_END}")?;
    stdout.flush()
}

fn fixture_stdin() -> io::Result<()> {
    let mut input = Vec::new();
    io::stdin().lock().read_to_end(&mut input)?;
    writeln!(io::stdout().lock(), "RFS_STDIN={}", input.len())
}

fn fixture_stream() -> io::Result<()> {
    let stdout_bytes = parse_fixture_usize("RFS_FIXTURE_STDOUT")?;
    let stderr_bytes = parse_fixture_usize("RFS_FIXTURE_STDERR")?;
    let stderr_first = env::var("RFS_FIXTURE_ORDER").as_deref() == Ok("stderr-first");
    if stderr_first {
        write_repeated(io::stderr().lock(), b'E', stderr_bytes)?;
        write_repeated(io::stdout().lock(), b'O', stdout_bytes)?;
    } else {
        write_repeated(io::stdout().lock(), b'O', stdout_bytes)?;
        write_repeated(io::stderr().lock(), b'E', stderr_bytes)?;
    }
    Ok(())
}

fn write_repeated(mut destination: impl Write, byte: u8, count: usize) -> io::Result<()> {
    let chunk = [byte; 1024];
    let mut remaining = count;
    while remaining != 0 {
        let length = remaining.min(chunk.len());
        destination.write_all(&chunk[..length])?;
        remaining -= length;
    }
    destination.flush()
}

fn fixture_marker() -> io::Result<()> {
    let marker = required_fixture_path("RFS_MARKER")?;
    fs::write(marker, process::id().to_string())?;
    thread::sleep(Duration::from_millis(parse_fixture_u64(
        "RFS_MARKER_SLEEP",
    )?));
    Ok(())
}

fn fixture_tree_parent() -> io::Result<()> {
    ignore_termination_signal();
    let report = required_fixture_path("RFS_TREE_REPORT")?;
    let heartbeat = required_fixture_path("RFS_TREE_HEARTBEAT")?;
    let mut child = Command::new(env::current_exe()?)
        .args([
            "fixture_process_child",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(FIXTURE_MODE, "tree-grandchild")
        .env("RFS_TREE_HEARTBEAT", &heartbeat)
        .spawn()?;
    fs::write(&report, format!("{}\n{}\n", process::id(), child.id()))?;
    loop {
        thread::sleep(Duration::from_secs(1));
        let _ = child.try_wait();
    }
}

fn fixture_tree_grandchild() -> io::Result<()> {
    ignore_termination_signal();
    let heartbeat = required_fixture_path("RFS_TREE_HEARTBEAT")?;
    let mut count = 0_u64;
    loop {
        count = count.wrapping_add(1);
        fs::write(&heartbeat, count.to_string())?;
        thread::sleep(Duration::from_millis(20));
    }
}

fn parse_fixture_usize(name: &str) -> io::Result<usize> {
    env::var(name)
        .map_err(|_| io::Error::other("missing numeric fixture value"))?
        .parse()
        .map_err(|_| io::Error::other("invalid numeric fixture value"))
}

fn parse_fixture_u64(name: &str) -> io::Result<u64> {
    env::var(name)
        .map_err(|_| io::Error::other("missing numeric fixture value"))?
        .parse()
        .map_err(|_| io::Error::other("invalid numeric fixture value"))
}

fn required_fixture_path(name: &str) -> io::Result<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("missing fixture path"))
}

#[cfg(unix)]
fn ignore_termination_signal() {
    unsafe extern "C" {
        fn signal(signal: i32, handler: usize) -> usize;
    }
    // SAFETY: POSIX `SIG_IGN` is the sentinel value 1, and rustix supplies SIGTERM's number.
    let _ = unsafe { signal(rustix::process::Signal::TERM.as_raw(), 1) };
}

#[cfg(windows)]
fn ignore_termination_signal() {}

#[cfg(target_os = "linux")]
fn process_absent_or_zombie(pid: u32) -> bool {
    let path = PathBuf::from(format!("/proc/{pid}/stat"));
    let deadline = StdInstant::now() + Duration::from_secs(1);
    loop {
        match fs::read_to_string(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return true,
            Ok(stat)
                if stat
                    .rsplit_once(") ")
                    .is_some_and(|(_, rest)| rest.starts_with('Z')) =>
            {
                return true;
            }
            Ok(_) | Err(_) if StdInstant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(_) | Err(_) => return false,
        }
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_absent_or_zombie(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return true;
    };
    let deadline = StdInstant::now() + Duration::from_secs(1);
    loop {
        match rustix::process::test_kill_process(pid) {
            Err(rustix::io::Errno::SRCH) => return true,
            Ok(()) | Err(_) if StdInstant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(()) | Err(_) => return false,
        }
    }
}

#[cfg(windows)]
fn process_absent_or_zombie(pid: u32) -> bool {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::WAIT_OBJECT_0,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };

    // SAFETY: OpenProcess returns a new owned handle or null.
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if raw.is_null() {
        return true;
    }
    // SAFETY: The checked non-null handle is uniquely owned by this scope.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    // SAFETY: The process handle remains live for this zero-time wait.
    unsafe { WaitForSingleObject(handle.as_raw_handle().cast(), 0) == WAIT_OBJECT_0 }
}

/// rfs-7r1w — a profile-declared relative `PATH` is resolved against the
/// configuration base, never against the directory the server was launched
/// from.
///
/// The profile checker validates a declared `PATH` by joining each
/// non-absolute entry onto the configuration base, so the directory it
/// accepted must be the directory the child actually searches. Forwarding the
/// value verbatim left that to the child's inherited working directory, which
/// is how one profile could pass `rfs check` from anywhere and then fail to
/// find its credential helper under `rfs serve`.
///
/// The cargo working directory is never the temporary command base, so a child
/// that received the relative value unchanged would search somewhere else
/// entirely — the assertion below cannot pass by coincidence of layout.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_relative_path_resolves_against_the_command_base() {
    let temporary = TempDir::new().expect("command base");
    let base = temporary
        .path()
        .canonicalize()
        .expect("canonical command base");
    assert_ne!(
        env::current_dir().expect("test working directory"),
        base,
        "this fence is only meaningful when the launch directory differs from the base"
    );

    let executor = CommandExecutor::new(1, temporary.path()).expect("executor");
    let mut extra = BTreeMap::new();
    extra.insert(
        "PATH".to_owned(),
        EnvironmentValue::literal("command-bin").expect("relative PATH literal"),
    );
    let output = executor
        .run(
            &fixture_spec("inspect", extra),
            CommandRole::OneShot,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect("inspect command");
    let rendered = String::from_utf8(output.stdout().to_vec()).expect("UTF-8 fixture output");
    let inspection = rendered
        .split_once(INSPECT_BEGIN)
        .and_then(|(_, tail)| tail.split_once(INSPECT_END).map(|(body, _)| body))
        .expect("bounded inspection body");
    let observed = inspection
        .lines()
        .find_map(|line| line.strip_prefix("ENV=PATH="))
        .expect("the child reports the PATH it was handed");
    assert_eq!(
        observed,
        base.join("command-bin").to_string_lossy(),
        "the child must search the configuration base, not the launch directory"
    );
}

/// The same rule, proven by an actual lookup rather than by the value handed
/// to the child: a bare `argv[0]` resolves through a declared relative `PATH`.
///
/// Unix only because a two-line shell script is the cheapest helper that is
/// unambiguously executable; the portable half of the rule is fenced above.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_relative_path_resolves_a_bare_program() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TempDir::new().expect("command base");
    let base = temporary
        .path()
        .canonicalize()
        .expect("canonical command base");
    let directory = base.join("command-bin");
    fs::create_dir(&directory).expect("command directory");
    let marker = base.join("helper-ran");
    let helper = directory.join("rfs-fixture-helper");
    fs::write(&helper, "#!/bin/sh\n: > \"$RFS_MARKER\"\n").expect("helper script");
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).expect("helper mode");

    let mut environment = BTreeMap::new();
    environment.insert(
        "PATH".to_owned(),
        EnvironmentValue::literal("command-bin").expect("relative PATH literal"),
    );
    environment.insert(
        "RFS_MARKER".to_owned(),
        EnvironmentValue::literal(marker.to_string_lossy().into_owned()).expect("marker path"),
    );
    let spec = CommandSpec::new(
        vec!["rfs-fixture-helper".to_owned()],
        ChildEnvironment::new(environment).expect("child environment"),
    )
    .expect("bare-program command");

    CommandExecutor::new(1, temporary.path())
        .expect("executor")
        .run(
            &spec,
            CommandRole::OneShot,
            CommandInput::None,
            &OperationGuard::new(),
        )
        .await
        .expect("a bare program resolves through the declared relative PATH");
    assert!(
        marker.exists(),
        "the helper under the configuration base ran"
    );
}
