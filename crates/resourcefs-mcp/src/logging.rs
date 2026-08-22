#![allow(
    dead_code,
    reason = "the standalone sink is wired by the approved Slice 11 launch plan"
)]

use std::{
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
};

use resourcefs_core::Redactor;
use tokio::{io::AsyncWriteExt, sync::Mutex};

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

pub(crate) struct LogSink {
    level: LogLevel,
    redactor: Redactor,
    destination: Mutex<LogDestination>,
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
        Ok(Self {
            level: config.level,
            redactor,
            destination: Mutex::new(destination),
        })
    }

    pub(crate) async fn write(&self, level: LogLevel, message: &str) -> Result<bool, LogError> {
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
