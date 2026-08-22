use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

#[path = "../src/logging.rs"]
mod logging;

use logging::{
    DEFAULT_RETAINED_FILES, LogConfig, LogDestinationKind, LogErrorKind, LogLevel, LogSink,
    MAX_LOG_MESSAGE_BYTES, MAX_RETAINED_FILES, MAX_ROTATION_BYTES,
};
use resourcefs_core::{Redactor, Secret};
use tempfile::TempDir;

fn empty_redactor() -> Redactor {
    Redactor::new(std::iter::empty::<&Secret>()).expect("empty redactor")
}

fn redactor_for(value: &str) -> Redactor {
    let secret = Secret::new(value.to_owned()).expect("secret");
    Redactor::new([&secret]).expect("redactor")
}

fn family_path(path: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.{index}", path.as_os_str().to_string_lossy()))
}

fn family_bytes(path: &Path, retained_files: usize) -> Vec<Vec<u8>> {
    let mut bytes = vec![fs::read(path).expect("active log")];
    for index in 1..retained_files {
        let backup = family_path(path, index);
        if backup.exists() {
            bytes.push(fs::read(backup).expect("backup log"));
        }
    }
    bytes
}

#[test]
fn configuration_defaults_and_signed_boundaries_are_exact() {
    let default = LogConfig::default();
    assert_eq!(default.level(), LogLevel::Info);
    assert_eq!(default.destination_kind(), LogDestinationKind::Stderr);
    assert_eq!(default.path(), None);
    assert_eq!(default.rotation_bytes(), None);
    assert_eq!(default.retained_files(), None);

    for level in [
        LogLevel::Error,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
    ] {
        assert_eq!(LogConfig::stderr(level).level(), level);
    }

    for (bytes, accepted) in [
        (-1, false),
        (0, true),
        (MAX_ROTATION_BYTES as i64, true),
        (MAX_ROTATION_BYTES as i64 + 1, false),
    ] {
        let result = LogConfig::file(LogLevel::Info, "resourcefs.log", bytes, 3);
        assert_eq!(result.is_ok(), accepted, "rotationBytes={bytes}");
        if !accepted {
            assert_eq!(
                result.expect_err("invalid rotation size").kind(),
                LogErrorKind::LimitExceeded
            );
        }
    }

    for retained_files in -1..=11 {
        let accepted = (1..=MAX_RETAINED_FILES as i64).contains(&retained_files);
        let result = LogConfig::file(
            LogLevel::Info,
            "resourcefs.log",
            MAX_ROTATION_BYTES as i64,
            retained_files,
        );
        assert_eq!(result.is_ok(), accepted, "retainFiles={retained_files}");
        if !accepted {
            assert_eq!(
                result.expect_err("invalid retention count").kind(),
                LogErrorKind::LimitExceeded
            );
        }
    }

    assert_eq!(
        LogConfig::file(LogLevel::Info, "", 1, DEFAULT_RETAINED_FILES as i64)
            .expect_err("empty path")
            .kind(),
        LogErrorKind::InvalidConfig
    );

    let relative = LogConfig::file(LogLevel::Debug, "logs/resourcefs.log", 7, 2)
        .expect("relative file config");
    assert_eq!(relative.destination_kind(), LogDestinationKind::File);
    assert_eq!(relative.path(), Some(Path::new("logs/resourcefs.log")));
    assert_eq!(relative.rotation_bytes(), Some(7));
    assert_eq!(relative.retained_files(), Some(2));

    let absolute_path = std::env::current_dir()
        .expect("current directory")
        .join("resourcefs-absolute.log");
    let absolute = LogConfig::file(LogLevel::Error, absolute_path.clone(), 11, 1)
        .expect("absolute file config");
    assert_eq!(absolute.path(), Some(absolute_path.as_path()));
}

#[tokio::test]
async fn severity_floor_filters_before_writing() {
    let temporary = TempDir::new().expect("temporary log directory");
    let path = temporary.path().join("severity.log");
    let config =
        LogConfig::file(LogLevel::Warn, &path, MAX_ROTATION_BYTES as i64, 3).expect("log config");
    let sink = LogSink::new(config, empty_redactor())
        .await
        .expect("log sink");

    assert!(!sink.write(LogLevel::Debug, "debug").await.expect("debug"));
    assert!(!sink.write(LogLevel::Info, "info").await.expect("info"));
    assert!(sink.write(LogLevel::Warn, "warn").await.expect("warn"));
    assert!(sink.write(LogLevel::Error, "error").await.expect("error"));

    assert_eq!(
        fs::read(path).expect("severity log"),
        b"[warn] warn\n[error] error\n"
    );
}

#[tokio::test]
async fn rotation_occurs_before_the_next_record_crosses_the_threshold() {
    let temporary = TempDir::new().expect("temporary log directory");
    let path = temporary.path().join("exact.log");
    let first_record = b"[info] alpha\n";
    let config =
        LogConfig::file(LogLevel::Info, &path, first_record.len() as i64, 3).expect("log config");
    let sink = LogSink::new(config, empty_redactor())
        .await
        .expect("log sink");

    sink.write(LogLevel::Info, "alpha")
        .await
        .expect("exact write");
    assert_eq!(
        fs::metadata(&path).expect("active metadata").len(),
        first_record.len() as u64
    );
    assert!(!family_path(&path, 1).exists());

    sink.write(LogLevel::Info, "beta")
        .await
        .expect("rotating write");
    assert_eq!(
        fs::read(family_path(&path, 1)).expect("first backup"),
        first_record
    );
    assert_eq!(fs::read(&path).expect("active log"), b"[info] beta\n");
}

#[tokio::test]
async fn every_retention_count_keeps_exactly_its_configured_log_family() {
    for retained_files in 1..=MAX_RETAINED_FILES {
        let temporary = TempDir::new().expect("temporary log directory");
        let path = temporary.path().join("retained.log");
        let config = LogConfig::file(LogLevel::Info, &path, 0, retained_files as i64)
            .expect("zero-threshold log config");
        let sink = LogSink::new(config, empty_redactor())
            .await
            .expect("log sink");

        for index in 0..retained_files + 2 {
            sink.write(LogLevel::Info, &format!("record-{index}"))
                .await
                .expect("rotating record");
        }

        assert!(path.is_file(), "retainFiles={retained_files} active");
        for index in 1..retained_files {
            assert!(
                family_path(&path, index).is_file(),
                "retainFiles={retained_files} backup={index}"
            );
        }
        assert!(
            !family_path(&path, retained_files).exists(),
            "retainFiles={retained_files} must not retain an extra backup"
        );
        assert_eq!(family_bytes(&path, retained_files).len(), retained_files);
    }
}

#[tokio::test]
async fn concurrent_messages_are_serialized_without_loss_or_interleaving() {
    let temporary = TempDir::new().expect("temporary log directory");
    let path = temporary.path().join("concurrent.log");
    let config =
        LogConfig::file(LogLevel::Info, &path, MAX_ROTATION_BYTES as i64, 3).expect("log config");
    let sink = Arc::new(
        LogSink::new(config, empty_redactor())
            .await
            .expect("log sink"),
    );
    let mut tasks = Vec::new();
    for index in 0..128 {
        let sink = Arc::clone(&sink);
        tasks.push(tokio::spawn(async move {
            sink.write(LogLevel::Info, &format!("record-{index:03}"))
                .await
                .expect("concurrent write");
        }));
    }
    for task in tasks {
        task.await.expect("writer task");
    }

    let text = fs::read_to_string(path).expect("concurrent log");
    let mut actual = text.lines().map(str::to_owned).collect::<Vec<_>>();
    actual.sort_unstable();
    let expected = (0..128)
        .map(|index| format!("[info] record-{index:03}"))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn redaction_precedes_every_file_write_and_rotation_is_contained() {
    const SENTINEL: &str = "RFS_LOG_SECRET_7aa8c2e1";
    let temporary = TempDir::new().expect("temporary log directory");
    let path = temporary.path().join("resourcefs.log");
    let sibling = temporary.path().join("resourcefs.log.sibling");
    fs::write(&sibling, b"operator-owned\n").expect("sibling sentinel");
    let sibling_before = fs::read(&sibling).expect("sibling before");
    let config = LogConfig::file(LogLevel::Debug, &path, 48, 3).expect("log config");
    let sink = LogSink::new(config, redactor_for(SENTINEL))
        .await
        .expect("log sink");

    for index in 0..12 {
        sink.write(LogLevel::Debug, &format!("diagnostic-{index}-{SENTINEL}"))
            .await
            .expect("secret-bearing write");
    }

    let family = family_bytes(&path, 3);
    assert_eq!(family.len(), 3);
    assert_eq!(fs::read(&sibling).expect("sibling after"), sibling_before);
    assert!(family.iter().all(|bytes| {
        !bytes
            .windows(SENTINEL.len())
            .any(|window| window == SENTINEL.as_bytes())
    }));
    assert!(family.iter().any(|bytes| {
        bytes
            .windows(b"<redacted>".len())
            .any(|window| window == b"<redacted>")
    }));
}

#[tokio::test]
async fn exact_message_and_file_ceilings_succeed_and_one_over_fails() {
    let temporary = TempDir::new().expect("temporary log directory");
    let path = temporary.path().join("maximum.log");
    let config = LogConfig::file(LogLevel::Info, &path, MAX_ROTATION_BYTES as i64, 2)
        .expect("maximum log config");
    let sink = LogSink::new(config, empty_redactor())
        .await
        .expect("log sink");

    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open active log")
        .set_len(MAX_ROTATION_BYTES)
        .expect("set sparse exact rotation length");
    sink.write(LogLevel::Info, "after-exact-file")
        .await
        .expect("rotate exact file");
    assert_eq!(
        fs::metadata(family_path(&path, 1))
            .expect("exact-size backup")
            .len(),
        MAX_ROTATION_BYTES
    );

    let exact_message = "x".repeat(MAX_LOG_MESSAGE_BYTES);
    sink.write(LogLevel::Info, &exact_message)
        .await
        .expect("exact message");
    let before = family_bytes(&path, 2);
    let error = sink
        .write(LogLevel::Info, &"x".repeat(MAX_LOG_MESSAGE_BYTES + 1))
        .await
        .expect_err("one-over message");
    assert_eq!(error.kind(), LogErrorKind::LimitExceeded);
    assert_eq!(family_bytes(&path, 2), before);
}

#[tokio::test]
async fn invalid_file_destination_fails_without_fallback() {
    let temporary = TempDir::new().expect("temporary log directory");
    let config = LogConfig::file(LogLevel::Info, temporary.path(), 1, 1)
        .expect("directory-shaped file config");
    let error = LogSink::new(config, empty_redactor())
        .await
        .err()
        .expect("directory destination must fail");
    assert_eq!(error.kind(), LogErrorKind::Io);
}

#[cfg(unix)]
#[tokio::test]
async fn file_destination_permission_denial_is_reported() {
    use std::os::unix::fs::PermissionsExt;

    let temporary = TempDir::new().expect("temporary log directory");
    let denied = temporary.path().join("denied");
    fs::create_dir(&denied).expect("denied directory");
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o500)).expect("deny writes");
    let config = LogConfig::file(LogLevel::Info, denied.join("resourcefs.log"), 1, 1)
        .expect("denied log config");
    let result = LogSink::new(config, empty_redactor()).await;
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o700)).expect("restore writes");

    let error = result.err().expect("permission denial must fail");
    assert_eq!(error.kind(), LogErrorKind::Io);
}
