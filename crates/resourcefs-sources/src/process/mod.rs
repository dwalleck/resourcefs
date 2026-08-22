//! Bounded direct-argv command execution with whole-tree ownership.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use self::unix as platform;
#[cfg(windows)]
use self::windows as platform;

use std::{
    collections::{BTreeMap, HashSet},
    env,
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::Arc,
    time::Duration,
};

use resourcefs_core::OperationGuard;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
    sync::Semaphore,
    task::JoinHandle,
    time::{Instant, sleep, sleep_until, timeout},
};

use crate::{CommandSpec, EnvironmentValue};

/// Maximum live child-process trees owned by one ResourceFS process.
pub const MAX_LIVE_COMMAND_TREES: usize = 32;
/// Secret-helper wall-time ceiling.
pub const SECRET_HELPER_TIMEOUT: Duration = Duration::from_millis(5_000);
/// Secret-helper stdout and stderr ceiling.
pub const SECRET_HELPER_STREAM_BYTES: usize = 65_536;
/// Converter and SSH one-shot wall-time ceiling.
pub const ONE_SHOT_TIMEOUT: Duration = Duration::from_millis(30_000);
/// Converter and SSH stdout ceiling.
pub const ONE_SHOT_STDOUT_BYTES: usize = 64 * 1024 * 1024;
/// Converter and SSH stderr ceiling.
pub const ONE_SHOT_STDERR_BYTES: usize = 1024 * 1024;
/// Downstream MCP startup wall-time ceiling.
pub const DOWNSTREAM_STARTUP_TIMEOUT: Duration = Duration::from_millis(10_000);
/// Downstream MCP per-call wall-time ceiling.
pub const DOWNSTREAM_CALL_TIMEOUT: Duration = Duration::from_millis(30_000);
/// Downstream MCP protocol-frame ceiling.
pub const DOWNSTREAM_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// Downstream MCP stderr-line ceiling.
pub const DOWNSTREAM_STDERR_LINE_BYTES: usize = 65_536;

const TERMINATION_GRACE: Duration = Duration::from_millis(1_000);
const TERMINATION_POLL: Duration = Duration::from_millis(10);
const PIPE_JOIN_GRACE: Duration = Duration::from_millis(1_000);

/// Execution role selecting the command's hard time and output ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandRole {
    SecretHelper,
    OneShot,
    DownstreamStartup,
    DownstreamCall,
}

impl CommandRole {
    const fn index(self) -> usize {
        match self {
            Self::SecretHelper => 0,
            Self::OneShot => 1,
            Self::DownstreamStartup => 2,
            Self::DownstreamCall => 3,
        }
    }

    const fn hard_limits(self) -> CommandLimits {
        match self {
            Self::SecretHelper => CommandLimits {
                timeout: SECRET_HELPER_TIMEOUT,
                stdout_bytes: SECRET_HELPER_STREAM_BYTES,
                stderr_bytes: SECRET_HELPER_STREAM_BYTES,
            },
            Self::OneShot => CommandLimits {
                timeout: ONE_SHOT_TIMEOUT,
                stdout_bytes: ONE_SHOT_STDOUT_BYTES,
                stderr_bytes: ONE_SHOT_STDERR_BYTES,
            },
            Self::DownstreamStartup => CommandLimits {
                timeout: DOWNSTREAM_STARTUP_TIMEOUT,
                stdout_bytes: DOWNSTREAM_FRAME_BYTES,
                stderr_bytes: DOWNSTREAM_STDERR_LINE_BYTES,
            },
            Self::DownstreamCall => CommandLimits {
                timeout: DOWNSTREAM_CALL_TIMEOUT,
                stdout_bytes: DOWNSTREAM_FRAME_BYTES,
                stderr_bytes: DOWNSTREAM_STDERR_LINE_BYTES,
            },
        }
    }
}

/// Positive lower-only ceilings for one command role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandLimits {
    timeout: Duration,
    stdout_bytes: usize,
    stderr_bytes: usize,
}

impl CommandLimits {
    pub fn new(
        timeout: Duration,
        stdout_bytes: usize,
        stderr_bytes: usize,
    ) -> Result<Self, CommandError> {
        if timeout.is_zero() || stdout_bytes == 0 || stderr_bytes == 0 {
            return Err(CommandError::new(
                CommandErrorKind::InvalidConfiguration,
                "command limits must be positive",
            ));
        }
        Ok(Self {
            timeout,
            stdout_bytes,
            stderr_bytes,
        })
    }

    pub const fn timeout(self) -> Duration {
        self.timeout
    }

    pub const fn stdout_bytes(self) -> usize {
        self.stdout_bytes
    }

    pub const fn stderr_bytes(self) -> usize {
        self.stderr_bytes
    }
}

/// One direct command's input delivery mode.
#[derive(Clone, Copy)]
pub enum CommandInput<'a> {
    None,
    Stdin(&'a [u8]),
    /// Appends one already-canonical absolute path to argv.
    Path(&'a Path),
}

/// Stable command-execution failure category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandErrorKind {
    InvalidConfiguration,
    InvalidEnvironment,
    InvalidInput,
    Spawn,
    Io,
    LimitExceeded,
    Cancelled,
}

/// Bounded, output-free command failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError {
    kind: CommandErrorKind,
    message: &'static str,
    stdout_observed: usize,
    stderr_observed: usize,
}

impl CommandError {
    const fn new(kind: CommandErrorKind, message: &'static str) -> Self {
        Self {
            kind,
            message,
            stdout_observed: 0,
            stderr_observed: 0,
        }
    }

    const fn with_observed(mut self, stdout: usize, stderr: usize) -> Self {
        self.stdout_observed = stdout;
        self.stderr_observed = stderr;
        self
    }

    pub const fn kind(&self) -> CommandErrorKind {
        self.kind
    }

    pub const fn stdout_observed(&self) -> usize {
        self.stdout_observed
    }

    pub const fn stderr_observed(&self) -> usize {
        self.stderr_observed
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for CommandError {}

/// Fully collected output from one bounded command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    success: bool,
    code: Option<i32>,
}

impl CommandOutput {
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    pub const fn success(&self) -> bool {
        self.success
    }

    pub const fn code(&self) -> Option<i32> {
        self.code
    }
}

/// Cloneable owner of one process-wide admission semaphore and command base.
#[derive(Clone)]
pub struct CommandExecutor {
    permits: Arc<Semaphore>,
    command_base: Arc<PathBuf>,
    limits: [CommandLimits; 4],
}

impl CommandExecutor {
    /// Creates an executor whose clones share one live-tree ceiling.
    pub fn new(
        max_live_trees: usize,
        command_base: impl AsRef<Path>,
    ) -> Result<Self, CommandError> {
        if !(1..=MAX_LIVE_COMMAND_TREES).contains(&max_live_trees) {
            return Err(CommandError::new(
                CommandErrorKind::InvalidConfiguration,
                "process concurrency must be between 1 and 32",
            ));
        }
        let command_base = std::fs::canonicalize(command_base.as_ref()).map_err(|_| {
            CommandError::new(
                CommandErrorKind::InvalidConfiguration,
                "command base must be an existing directory",
            )
        })?;
        if !command_base.is_dir() {
            return Err(CommandError::new(
                CommandErrorKind::InvalidConfiguration,
                "command base must be an existing directory",
            ));
        }
        Ok(Self {
            permits: Arc::new(Semaphore::new(max_live_trees)),
            command_base: Arc::new(command_base),
            limits: [
                CommandRole::SecretHelper.hard_limits(),
                CommandRole::OneShot.hard_limits(),
                CommandRole::DownstreamStartup.hard_limits(),
                CommandRole::DownstreamCall.hard_limits(),
            ],
        })
    }

    /// Lowers one role's ceilings without changing the shared semaphore.
    pub fn with_role_limits(
        mut self,
        role: CommandRole,
        limits: CommandLimits,
    ) -> Result<Self, CommandError> {
        let hard = role.hard_limits();
        if limits.timeout > hard.timeout
            || limits.stdout_bytes > hard.stdout_bytes
            || limits.stderr_bytes > hard.stderr_bytes
        {
            return Err(CommandError::new(
                CommandErrorKind::InvalidConfiguration,
                "command role limits cannot exceed binary ceilings",
            ));
        }
        self.limits[role.index()] = limits;
        Ok(self)
    }

    /// Executes one validated command and returns only after tree cleanup and reap.
    pub async fn run(
        &self,
        spec: &CommandSpec,
        role: CommandRole,
        input: CommandInput<'_>,
        operation: &OperationGuard,
    ) -> Result<CommandOutput, CommandError> {
        let limits = self.limits[role.index()];
        validate_input(role, input, limits)?;
        let deadline = Instant::now() + limits.timeout;
        let permit = tokio::select! {
            biased;
            () = operation.cancelled() => {
                return Err(CommandError::new(CommandErrorKind::Cancelled, "command operation was cancelled"));
            }
            result = sleep_until(deadline) => {
                let () = result;
                return Err(CommandError::new(CommandErrorKind::LimitExceeded, "command admission exceeded its time ceiling"));
            }
            result = self.permits.clone().acquire_owned() => {
                result.map_err(|_| CommandError::new(CommandErrorKind::Io, "command admission is unavailable"))?
            }
        };

        let result = self
            .run_admitted(spec, role, input, operation, limits, deadline)
            .await;
        drop(permit);
        result
    }

    async fn run_admitted(
        &self,
        spec: &CommandSpec,
        role: CommandRole,
        input: CommandInput<'_>,
        operation: &OperationGuard,
        limits: CommandLimits,
        deadline: Instant,
    ) -> Result<CommandOutput, CommandError> {
        let mut command = build_command(spec, role, input, &self.command_base)?;
        command.kill_on_drop(true);
        platform::configure(command.as_std_mut());
        let mut child = command.spawn().map_err(|_| {
            CommandError::new(
                CommandErrorKind::Spawn,
                "configured command could not be started",
            )
        })?;

        let Some(pid) = child.id() else {
            terminate_unattached(&mut child).await;
            return Err(CommandError::new(
                CommandErrorKind::Io,
                "configured command has no process identity",
            ));
        };
        #[cfg(unix)]
        let mut tree = match platform::ChildTree::attach(pid) {
            Ok(tree) => tree,
            Err(_) => {
                terminate_unattached(&mut child).await;
                return Err(CommandError::new(
                    CommandErrorKind::Io,
                    "configured command could not establish tree ownership",
                ));
            }
        };
        #[cfg(windows)]
        let mut tree = {
            let Some(handle) = child.raw_handle() else {
                terminate_unattached(&mut child).await;
                return Err(CommandError::new(
                    CommandErrorKind::Io,
                    "configured command has no process handle",
                ));
            };
            match platform::ChildTree::attach(pid, handle) {
                Ok(tree) => tree,
                Err(_) => {
                    terminate_unattached(&mut child).await;
                    return Err(CommandError::new(
                        CommandErrorKind::Io,
                        "configured command could not establish tree ownership",
                    ));
                }
            }
        };

        let Some(stdout) = child.stdout.take() else {
            let cleanup = cleanup_tree(&mut child, &mut tree).await;
            return Err(cleanup.err().unwrap_or_else(|| {
                CommandError::new(
                    CommandErrorKind::Io,
                    "configured command stdout pipe is unavailable",
                )
            }));
        };
        let Some(stderr) = child.stderr.take() else {
            let cleanup = cleanup_tree(&mut child, &mut tree).await;
            return Err(cleanup.err().unwrap_or_else(|| {
                CommandError::new(
                    CommandErrorKind::Io,
                    "configured command stderr pipe is unavailable",
                )
            }));
        };
        let stdin = child.stdin.take();

        let mut stdout_task = tokio::spawn(read_bounded(stdout, limits.stdout_bytes));
        let mut stderr_task = tokio::spawn(read_bounded(stderr, limits.stderr_bytes));
        let mut stdin_future = Box::pin(write_input(stdin, input));
        let mut stdout_result: Option<BoundedRead> = None;
        let mut stderr_result: Option<BoundedRead> = None;
        let mut stdin_done = false;
        let mut exit_status = None;

        let stop = loop {
            if let Some(result) = stdout_result.as_ref()
                && result.exceeded
            {
                break Some(ExecutionStop::OutputLimit);
            }
            if let Some(result) = stderr_result.as_ref()
                && result.exceeded
            {
                break Some(ExecutionStop::OutputLimit);
            }
            if stdout_result.is_some()
                && stderr_result.is_some()
                && stdin_done
                && exit_status.is_some()
            {
                break None;
            }

            tokio::select! {
                biased;
                () = operation.cancelled() => break Some(ExecutionStop::Cancelled),
                joined = &mut stdout_task, if stdout_result.is_none() => {
                    match join_reader(joined) {
                        Ok(result) => stdout_result = Some(result),
                        Err(error) => break Some(ExecutionStop::Io(error.message)),
                    }
                }
                joined = &mut stderr_task, if stderr_result.is_none() => {
                    match join_reader(joined) {
                        Ok(result) => stderr_result = Some(result),
                        Err(error) => break Some(ExecutionStop::Io(error.message)),
                    }
                }
                result = &mut stdin_future, if !stdin_done => {
                    match result {
                        Ok(()) => stdin_done = true,
                        Err(_) => break Some(ExecutionStop::Io("configured command stdin write failed")),
                    }
                }
                () = sleep_until(deadline) => break Some(ExecutionStop::Timeout),
                status = child.wait(), if exit_status.is_none() => {
                    match status {
                        Ok(status) => exit_status = Some(status),
                        Err(_) => break Some(ExecutionStop::Io("configured command could not be reaped")),
                    }
                }
            }
        };

        if let Some(stop) = stop {
            drop(stdin_future);
            let cleanup = cleanup_tree(&mut child, &mut tree).await;
            stdout_result = finish_reader(stdout_result, stdout_task).await;
            stderr_result = finish_reader(stderr_result, stderr_task).await;
            let stdout_observed = stdout_result
                .as_ref()
                .map_or(0, |result| result.bytes.len());
            let stderr_observed = stderr_result
                .as_ref()
                .map_or(0, |result| result.bytes.len());
            let error = cleanup.err().unwrap_or_else(|| stop.error());
            return Err(error.with_observed(stdout_observed, stderr_observed));
        }

        let stdout = stdout_result.ok_or_else(|| {
            CommandError::new(CommandErrorKind::Io, "command stdout result is unavailable")
        })?;
        let stderr = stderr_result.ok_or_else(|| {
            CommandError::new(CommandErrorKind::Io, "command stderr result is unavailable")
        })?;
        let status = exit_status.ok_or_else(|| {
            CommandError::new(CommandErrorKind::Io, "command exit status is unavailable")
        })?;
        drop(tree);
        Ok(output(status, stdout.bytes, stderr.bytes))
    }
}

#[derive(Debug, Clone, Copy)]
enum ExecutionStop {
    OutputLimit,
    Timeout,
    Cancelled,
    Io(&'static str),
}

impl ExecutionStop {
    const fn error(self) -> CommandError {
        match self {
            Self::OutputLimit => CommandError::new(
                CommandErrorKind::LimitExceeded,
                "configured command output exceeded its byte ceiling",
            ),
            Self::Timeout => CommandError::new(
                CommandErrorKind::LimitExceeded,
                "configured command exceeded its time ceiling",
            ),
            Self::Cancelled => CommandError::new(
                CommandErrorKind::Cancelled,
                "command operation was cancelled",
            ),
            Self::Io(message) => CommandError::new(CommandErrorKind::Io, message),
        }
    }
}

#[derive(Debug)]
struct BoundedRead {
    bytes: Vec<u8>,
    exceeded: bool,
}

fn validate_input(
    role: CommandRole,
    input: CommandInput<'_>,
    limits: CommandLimits,
) -> Result<(), CommandError> {
    match input {
        CommandInput::None => Ok(()),
        CommandInput::Stdin(bytes) if role == CommandRole::OneShot => {
            if bytes.len() > limits.stdout_bytes {
                Err(CommandError::new(
                    CommandErrorKind::LimitExceeded,
                    "configured command input exceeded its byte ceiling",
                ))
            } else {
                Ok(())
            }
        }
        CommandInput::Path(path) if role == CommandRole::OneShot && path.is_absolute() => Ok(()),
        CommandInput::Path(_) if role == CommandRole::OneShot => Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "command path input must be canonical and absolute",
        )),
        _ => Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "command role does not accept this input mode",
        )),
    }
}

fn build_command(
    spec: &CommandSpec,
    role: CommandRole,
    input: CommandInput<'_>,
    base: &Path,
) -> Result<Command, CommandError> {
    let argv = spec.argv();
    let program = resolve_program(&argv[0], base);
    let mut command = Command::new(program);
    command.args(&argv[1..]);
    if let CommandInput::Path(path) = input {
        let canonical = std::fs::canonicalize(path).map_err(|_| {
            CommandError::new(
                CommandErrorKind::InvalidInput,
                "command path input must name an existing resource",
            )
        })?;
        command.arg(canonical);
    }
    command.env_clear();
    command.envs(resolve_environment(spec, role)?);
    command.stdin(match input {
        CommandInput::Stdin(_) => Stdio::piped(),
        CommandInput::None | CommandInput::Path(_) => Stdio::null(),
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    Ok(command)
}

fn resolve_program(program: &str, base: &Path) -> PathBuf {
    let path = Path::new(program);
    if path.is_absolute() || !contains_path_separator(program) {
        path.to_owned()
    } else {
        base.join(path)
    }
}

#[cfg(unix)]
fn contains_path_separator(value: &str) -> bool {
    value.contains('/')
}

#[cfg(windows)]
fn contains_path_separator(value: &str) -> bool {
    value.contains(['/', '\\'])
}

fn resolve_environment(
    spec: &CommandSpec,
    role: CommandRole,
) -> Result<BTreeMap<OsString, OsString>, CommandError> {
    let entries = spec.environment().entries();
    let explicit = entries
        .keys()
        .map(|name| name.to_uppercase())
        .collect::<HashSet<_>>();
    let mut resolved = BTreeMap::new();
    inherit_automatic(&mut resolved, &explicit, "PATH");
    #[cfg(windows)]
    inherit_automatic(&mut resolved, &explicit, "SystemRoot");

    for (destination, value) in entries {
        let value = match value {
            EnvironmentValue::Literal(value) => OsString::from(value),
            EnvironmentValue::Inherit(source) => env::var_os(source).ok_or_else(|| {
                CommandError::new(
                    CommandErrorKind::InvalidEnvironment,
                    "required inherited environment variable is absent",
                )
            })?,
            EnvironmentValue::Secret(_) if role == CommandRole::SecretHelper => {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidEnvironment,
                    "secret helper environment cannot resolve another secret",
                ));
            }
            EnvironmentValue::Secret(_) => {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidEnvironment,
                    "command environment secret must be resolved before execution",
                ));
            }
        };
        resolved.insert(OsString::from(destination), value);
    }
    Ok(resolved)
}

fn inherit_automatic(
    environment: &mut BTreeMap<OsString, OsString>,
    explicit: &HashSet<String>,
    name: &str,
) {
    if !explicit.contains(&name.to_uppercase())
        && let Some(value) = env::var_os(name)
    {
        environment.insert(OsString::from(name), value);
    }
}

async fn write_input(
    stdin: Option<tokio::process::ChildStdin>,
    input: CommandInput<'_>,
) -> io::Result<()> {
    let CommandInput::Stdin(bytes) = input else {
        return Ok(());
    };
    let mut stdin = stdin.ok_or_else(|| io::Error::other("stdin pipe is unavailable"))?;
    stdin.write_all(bytes).await?;
    stdin.shutdown().await
}

async fn read_bounded<R>(mut reader: R, limit: usize) -> io::Result<BoundedRead>
where
    R: AsyncRead + Unpin,
{
    let target = limit.saturating_add(1);
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    while bytes.len() < target {
        let remaining = target - bytes.len();
        let chunk_limit = remaining.min(chunk.len());
        let read = reader.read(&mut chunk[..chunk_limit]).await?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(BoundedRead {
        exceeded: bytes.len() > limit,
        bytes,
    })
}

fn join_reader(
    joined: Result<io::Result<BoundedRead>, tokio::task::JoinError>,
) -> Result<BoundedRead, CommandError> {
    joined
        .map_err(|_| CommandError::new(CommandErrorKind::Io, "command pipe reader stopped"))?
        .map_err(|_| CommandError::new(CommandErrorKind::Io, "command pipe read failed"))
}

async fn finish_reader(
    current: Option<BoundedRead>,
    mut task: JoinHandle<io::Result<BoundedRead>>,
) -> Option<BoundedRead> {
    if current.is_some() {
        return current;
    }
    match timeout(PIPE_JOIN_GRACE, &mut task).await {
        Ok(Ok(Ok(result))) => Some(result),
        Ok(Ok(Err(_))) | Ok(Err(_)) => None,
        Err(_) => {
            task.abort();
            drop(task.await);
            None
        }
    }
}

async fn cleanup_tree(
    child: &mut Child,
    tree: &mut platform::ChildTree,
) -> Result<(), CommandError> {
    let mut cleanup_failed = false;
    drop(tree.request_termination());
    let deadline = Instant::now() + TERMINATION_GRACE;
    let force_needed = loop {
        if child.try_wait().is_err() {
            cleanup_failed = true;
        }
        match tree.has_live_processes() {
            Ok(false) => break false,
            Ok(true) if Instant::now() < deadline => sleep(TERMINATION_POLL).await,
            Ok(true) | Err(_) => break true,
        }
    };
    if force_needed && tree.force_termination().is_err() {
        cleanup_failed = true;
        if child.start_kill().is_err() {
            cleanup_failed = true;
        }
    }
    match timeout(PIPE_JOIN_GRACE, child.wait()).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => cleanup_failed = true,
        Err(_) => {
            cleanup_failed = true;
            drop(child.start_kill());
            if !matches!(timeout(PIPE_JOIN_GRACE, child.wait()).await, Ok(Ok(_))) {
                cleanup_failed = true;
            }
        }
    }
    if cleanup_failed {
        Err(CommandError::new(
            CommandErrorKind::Io,
            "configured command tree cleanup failed",
        ))
    } else {
        Ok(())
    }
}

async fn terminate_unattached(child: &mut Child) {
    drop(child.kill().await);
    drop(child.wait().await);
}

fn output(status: ExitStatus, stdout: Vec<u8>, stderr: Vec<u8>) -> CommandOutput {
    CommandOutput {
        stdout,
        stderr,
        success: status.success(),
        code: status.code(),
    }
}
