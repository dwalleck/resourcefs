#![allow(
    dead_code,
    reason = "the standalone sink is wired by the approved Slice 11 launch plan"
)]

use std::{
    ffi::OsString,
    fmt::{self, Write as _},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex, Once, OnceLock,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    thread,
};

use resourcefs_core::Redactor;
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex, mpsc},
};
use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    layer::{Context, Layer, SubscriberExt as _},
    registry::LookupSpan,
};

pub(crate) const MAX_LOG_MESSAGE_BYTES: usize = 65_536;
pub(crate) const MAX_ROTATION_BYTES: u64 = 10_485_760;
pub(crate) const MAX_RETAINED_FILES: usize = 10;
pub(crate) const DEFAULT_RETAINED_FILES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    const fn priority(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warn => 1,
            Self::Info => 2,
            Self::Debug => 3,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }

    const fn admits(self, message_level: Self) -> bool {
        message_level.priority() <= self.priority()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogDestinationKind {
    Stderr,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogConfig {
    level: LogLevel,
    destination: LogDestinationConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LogDestinationConfig {
    Stderr,
    File(FileConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileConfig {
    path: PathBuf,
    rotation_bytes: u64,
    retained_files: usize,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self::stderr(LogLevel::Info)
    }
}

impl LogConfig {
    pub(crate) const fn stderr(level: LogLevel) -> Self {
        Self {
            level,
            destination: LogDestinationConfig::Stderr,
        }
    }

    pub(crate) fn file(
        level: LogLevel,
        path: impl Into<PathBuf>,
        rotation_bytes: i64,
        retained_files: i64,
    ) -> Result<Self, LogError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(LogError::invalid_config(
                "logging.destination.path must not be empty",
            ));
        }
        let rotation_bytes = u64::try_from(rotation_bytes).map_err(|_| {
            LogError::limit_exceeded(format!(
                "logging.destination.rotationBytes must be between 0 and {MAX_ROTATION_BYTES}"
            ))
        })?;
        if rotation_bytes > MAX_ROTATION_BYTES {
            return Err(LogError::limit_exceeded(format!(
                "logging.destination.rotationBytes must be between 0 and {MAX_ROTATION_BYTES}"
            )));
        }
        let retained_files = usize::try_from(retained_files).map_err(|_| {
            LogError::limit_exceeded(format!(
                "logging.destination.retainFiles must be between 1 and {MAX_RETAINED_FILES}"
            ))
        })?;
        if !(1..=MAX_RETAINED_FILES).contains(&retained_files) {
            return Err(LogError::limit_exceeded(format!(
                "logging.destination.retainFiles must be between 1 and {MAX_RETAINED_FILES}"
            )));
        }
        Ok(Self {
            level,
            destination: LogDestinationConfig::File(FileConfig {
                path,
                rotation_bytes,
                retained_files,
            }),
        })
    }

    pub(crate) const fn level(&self) -> LogLevel {
        self.level
    }

    pub(crate) const fn destination_kind(&self) -> LogDestinationKind {
        match self.destination {
            LogDestinationConfig::Stderr => LogDestinationKind::Stderr,
            LogDestinationConfig::File(_) => LogDestinationKind::File,
        }
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        match &self.destination {
            LogDestinationConfig::Stderr => None,
            LogDestinationConfig::File(config) => Some(&config.path),
        }
    }

    pub(crate) const fn rotation_bytes(&self) -> Option<u64> {
        match &self.destination {
            LogDestinationConfig::Stderr => None,
            LogDestinationConfig::File(config) => Some(config.rotation_bytes),
        }
    }

    pub(crate) const fn retained_files(&self) -> Option<usize> {
        match &self.destination {
            LogDestinationConfig::Stderr => None,
            LogDestinationConfig::File(config) => Some(config.retained_files),
        }
    }
}

/// The state a [`LogSink`] handle and the tracing drain task both write through.
struct LogSinkInner {
    level: LogLevel,
    redactor: Redactor,
    destination: Mutex<LogDestination>,
}

/// A handle to one configured diagnostic destination.
///
/// The handle is what callers own and what `serve` holds by value. The state
/// behind it is shared, because the `tracing` drain task writes to the same
/// destination and must not depend on the caller's handle outliving it.
pub(crate) struct LogSink {
    inner: Arc<LogSinkInner>,
}

enum LogDestination {
    Stderr,
    File(FileSink),
}

struct FileSink {
    path: PathBuf,
    rotation_bytes: u64,
    retained_files: usize,
}

impl LogSink {
    pub(crate) async fn new(config: LogConfig, redactor: Redactor) -> Result<Self, LogError> {
        let destination = match config.destination {
            LogDestinationConfig::Stderr => LogDestination::Stderr,
            LogDestinationConfig::File(config) => {
                open_append(&config.path).await?;
                LogDestination::File(FileSink {
                    path: config.path,
                    rotation_bytes: config.rotation_bytes,
                    retained_files: config.retained_files,
                })
            }
        };
        let inner = Arc::new(LogSinkInner {
            level: config.level,
            redactor,
            destination: Mutex::new(destination),
        });
        // Constructing a destination is the moment the process knows where
        // diagnostics go, so it is also where `tracing` events start flowing.
        attach_tracing_bridge(&inner);
        Ok(Self { inner })
    }

    pub(crate) fn level(&self) -> LogLevel {
        self.inner.level
    }

    pub(crate) async fn write(&self, level: LogLevel, message: &str) -> Result<bool, LogError> {
        self.inner.write(level, message).await
    }
}

impl LogSinkInner {
    async fn write(&self, level: LogLevel, message: &str) -> Result<bool, LogError> {
        if message.len() > MAX_LOG_MESSAGE_BYTES {
            return Err(LogError::limit_exceeded(format!(
                "log message must not exceed {MAX_LOG_MESSAGE_BYTES} bytes"
            )));
        }
        if !self.level.admits(level) {
            return Ok(false);
        }
        let scrubbed = self.redactor.scrub(message);
        let record = format!("[{}] {scrubbed}\n", level.as_str());
        let mut destination = self.destination.lock().await;
        destination.write(record.as_bytes()).await?;
        Ok(true)
    }
}

impl LogDestination {
    async fn write(&mut self, record: &[u8]) -> Result<(), LogError> {
        match self {
            Self::Stderr => {
                let mut stderr = tokio::io::stderr();
                stderr
                    .write_all(record)
                    .await
                    .map_err(|error| LogError::io("write stderr log record", error))?;
                stderr
                    .flush()
                    .await
                    .map_err(|error| LogError::io("flush stderr log record", error))
            }
            Self::File(file) => file.write(record).await,
        }
    }
}

impl FileSink {
    async fn write(&mut self, record: &[u8]) -> Result<(), LogError> {
        let current_bytes = file_len(&self.path).await?;
        let record_bytes = u64::try_from(record.len())
            .map_err(|_| LogError::limit_exceeded("encoded log record length exceeds u64"))?;
        if current_bytes > 0
            && (self.rotation_bytes == 0
                || current_bytes.saturating_add(record_bytes) > self.rotation_bytes)
        {
            rotate(&self.path, self.retained_files).await?;
        }
        let mut file = open_append(&self.path).await?;
        file.write_all(record)
            .await
            .map_err(|error| LogError::io("append file log record", error))?;
        file.flush()
            .await
            .map_err(|error| LogError::io("flush file log record", error))
    }
}

async fn open_append(path: &Path) -> Result<tokio::fs::File, LogError> {
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|error| LogError::io("open file log destination", error))
}

async fn file_len(path: &Path) -> Result<u64, LogError> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) if metadata.is_file() => Ok(metadata.len()),
        Ok(_) => Err(LogError::invalid_config(
            "logging.destination.path must identify a file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(LogError::io("inspect file log destination", error)),
    }
}

async fn rotate(path: &Path, retained_files: usize) -> Result<(), LogError> {
    if retained_files == 1 {
        return remove_file_if_exists(path, "remove current file log").await;
    }
    let oldest = family_path(path, retained_files - 1);
    remove_file_if_exists(&oldest, "remove oldest rotated log").await?;
    for index in (1..retained_files).rev() {
        let source = if index == 1 {
            path.to_owned()
        } else {
            family_path(path, index - 1)
        };
        let destination = family_path(path, index);
        rename_if_exists(&source, &destination).await?;
    }
    Ok(())
}

async fn remove_file_if_exists(path: &Path, operation: &str) -> Result<(), LogError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(LogError::io(operation, error)),
    }
}

async fn rename_if_exists(source: &Path, destination: &Path) -> Result<(), LogError> {
    match tokio::fs::rename(source, destination).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(LogError::io("rotate file log", error)),
    }
}

fn family_path(path: &Path, index: usize) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogErrorKind {
    InvalidConfig,
    LimitExceeded,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogError {
    kind: LogErrorKind,
    message: String,
}

impl LogError {
    fn invalid_config(message: impl Into<String>) -> Self {
        Self {
            kind: LogErrorKind::InvalidConfig,
            message: message.into(),
        }
    }

    fn limit_exceeded(message: impl Into<String>) -> Self {
        Self {
            kind: LogErrorKind::LimitExceeded,
            message: message.into(),
        }
    }

    fn io(operation: &str, error: io::Error) -> Self {
        Self {
            kind: LogErrorKind::Io,
            message: format!("{operation} failed ({:?})", error.kind()),
        }
    }

    pub(crate) const fn kind(&self) -> LogErrorKind {
        self.kind
    }
}

impl fmt::Display for LogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LogError {}

/// How many rendered events may wait for the writer.
///
/// `Layer::on_event` is synchronous and runs on whatever thread emitted the
/// event, including runtime workers; `LogSink::write` is asynchronous and takes
/// a lock. A bounded queue keeps the two apart without letting a slow or
/// rotating file sink apply backpressure to traced code.
const TRACING_QUEUE_DEPTH: usize = 1024;

impl LogLevel {
    /// A stable numeric spelling for the atomic the bridge reads per event.
    const fn as_repr(self) -> u8 {
        self.priority()
    }

    const fn from_repr(repr: u8) -> Self {
        match repr {
            0 => Self::Error,
            1 => Self::Warn,
            2 => Self::Info,
            _ => Self::Debug,
        }
    }

    const fn from_tracing(level: Level) -> Self {
        match level {
            Level::ERROR => Self::Error,
            Level::WARN => Self::Warn,
            Level::INFO => Self::Info,
            // This sink's vocabulary stops at `debug`; TRACE would otherwise be
            // silently unrepresentable.
            Level::DEBUG | Level::TRACE => Self::Debug,
        }
    }
}

/// Renders one event's fields into a single line.
///
/// Rendering completes *before* anything is handed to the sink, which is what
/// lets the redactor see a whole record. A `tracing_subscriber::fmt` layer
/// writes fields directly to its writer one at a time, so a secret spanning two
/// fields -- or one split across two `write` calls -- would never match the
/// redactor's automaton. Formatting here and scrubbing there keeps the existing
/// guarantee exactly as `LogSink::write` already states it.
struct EventLine {
    rendered: String,
}

impl Visit for EventLine {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if !self.rendered.is_empty() {
            self.rendered.push(' ');
        }
        // `message` is the event's primary text and carries no useful key.
        if field.name() == "message" {
            let _ = write!(self.rendered, "{value:?}");
        } else {
            let _ = write!(self.rendered, "{}={value:?}", field.name());
        }
    }
}

/// Whether an event from `target` at `level` belongs in the sink.
///
/// `logging.level` is ResourceFS's own diagnostic verbosity. A dependency's
/// scale is its own: `rmcp` narrates ordinary lifecycle at INFO -- "Service
/// initialized as server peer", "received notification", once per request --
/// and forwarding that at our INFO would turn a clean run's empty stderr into
/// per-request chatter. What this bridge exists to recover is the 22 `error!`
/// and 52 `warn!` sites that report faults `rmcp` never returns as a `Result`.
///
/// So foreign targets are admitted from WARN up. The exception is `debug`,
/// where the operator has explicitly asked for everything and a transport
/// problem is the likely reason: there, a dependency's full narration is the
/// point.
pub(crate) fn admits_foreign(configured: LogLevel, target: &str, level: LogLevel) -> bool {
    if matches!(configured, LogLevel::Debug) || target.starts_with("resourcefs") {
        return true;
    }
    matches!(level, LogLevel::Error | LogLevel::Warn)
}

/// Forwards `tracing` events into whichever [`LogSink`] is currently configured.
struct SinkLayer {
    state: &'static BridgeState,
}

impl<S> Layer<S> for SinkLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let level = LogLevel::from_tracing(*metadata.level());
        let configured = LogLevel::from_repr(self.state.configured.load(Ordering::Relaxed));
        if !admits_foreign(configured, metadata.target(), level) {
            return;
        }
        let mut line = EventLine {
            rendered: String::new(),
        };
        event.record(&mut line);
        let record = format!("{}: {}", metadata.target(), line.rendered);

        // A full queue drops this event rather than blocking a runtime worker.
        // The count is not discarded: the drain task reports it, so a dropped
        // diagnostic is a visible gap rather than a silent one.
        if self.state.sender.try_send((level, record)).is_err() {
            self.state.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The process-wide bridge between `tracing` and the configured destination.
///
/// A global subscriber may be installed exactly once per process, but a test
/// binary builds many sinks. So the subscriber is installed once and the
/// destination it writes through is republished by each new sink: the most
/// recently constructed one receives events, which is the only rule that is
/// predictable in both a server process (one sink, forever) and a test binary.
struct BridgeState {
    sender: mpsc::Sender<(LogLevel, String)>,
    dropped: AtomicU64,
    /// The current sink's level, as `LogLevel::as_repr`, read on every event.
    configured: AtomicU8,
    target: StdMutex<Option<Arc<LogSinkInner>>>,
}

static BRIDGE: OnceLock<BridgeState> = OnceLock::new();

/// Points the process's `tracing` bridge at `inner`, installing it on first use.
///
/// Nothing here can reach stdout: a sink writes to stderr or to a file, and
/// stdout carries the MCP protocol stream.
fn attach_tracing_bridge(inner: &Arc<LogSinkInner>) {
    let state = BRIDGE.get_or_init(|| {
        let (sender, mut receiver) = mpsc::channel::<(LogLevel, String)>(TRACING_QUEUE_DEPTH);
        // The drain owns its runtime on a dedicated thread rather than riding
        // `tokio::spawn`. A spawned task lives on whichever runtime happened to
        // be current when the first sink was built, and dies with it: in a test
        // binary that is the first `#[tokio::test]`'s runtime, so every later
        // test's events would queue with nobody reading. Production is
        // accidentally safe -- one sink, one long-lived runtime -- but coupling
        // the drain's lifetime to an arbitrary caller's runtime is a trap, and
        // the release gate found it.
        thread::Builder::new()
            .name("resourcefs-log-drain".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    // Nothing can be reported through the sink if the drain
                    // cannot start, so failing loudly beats a silent gap.
                    Err(error) => panic!("log drain runtime: {error}"),
                };
                runtime.block_on(async move {
            while let Some((level, record)) = receiver.recv().await {
                let Some(target) = current_target() else {
                    continue;
                };
                let missed = current_state().dropped.swap(0, Ordering::Relaxed);
                if missed > 0 {
                    // Best effort by construction: if this write fails there is
                    // no second channel on which to report it.
                    drop(
                        target
                            .write(
                                LogLevel::Warn,
                                &format!(
                                    "resourcefs: {missed} diagnostic events were dropped; the log queue was full"
                                ),
                            )
                            .await,
                    );
                }
                drop(target.write(level, &record).await);
            }
                });
            })
            .expect("spawn the log drain thread");
        BridgeState {
            sender,
            dropped: AtomicU64::new(0),
            configured: AtomicU8::new(inner.level.as_repr()),
            target: StdMutex::new(None),
        }
    });

    state
        .configured
        .store(inner.level.as_repr(), Ordering::Relaxed);
    if let Ok(mut target) = state.target.lock() {
        *target = Some(Arc::clone(inner));
    }

    // Installing the global subscriber is a one-time step guarded by the same
    // `OnceLock`, so a second sink re-targets the existing bridge rather than
    // racing to install another.
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        // A failure here means something outside this module already installed
        // a global subscriber. That is not a reason to refuse to serve, and it
        // is not silent: the sink itself reports it.
        if tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(SinkLayer { state }),
        )
        .is_err()
        {
            let reporting = Arc::clone(inner);
            tokio::spawn(async move {
                drop(
                    reporting
                        .write(
                            LogLevel::Warn,
                            "resourcefs: a tracing subscriber was already installed; \
                             dependency diagnostics will not reach this destination",
                        )
                        .await,
                );
            });
        }
    });
}

fn current_state() -> &'static BridgeState {
    BRIDGE
        .get()
        .expect("the bridge is initialized before it drains")
}

fn current_target() -> Option<Arc<LogSinkInner>> {
    current_state().target.lock().ok()?.clone()
}
