use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    io::{self, Write},
    process, thread,
    time::Duration,
};

use resourcefs_core::{OperationGuard, Secret};
use resourcefs_sources::{
    ChildEnvironment, CommandExecutor, CommandLimits, CommandRole, CommandSpec, EnvironmentValue,
    SECRET_HELPER_STREAM_BYTES, SecretReference, SecretResolutionError, SecretResolutionErrorKind,
    secret,
};
use tempfile::TempDir;

const FIXTURE_MODE: &str = "RFS_SECRET_FIXTURE";
const AMBIENT_SENTINEL_NAME: &str = "RFS_AMBIENT_SECRET";
const PRIVATE_SENTINEL: &str = "private-secret-sentinel";

fn main() {
    if let Some(mode) = env::var_os(FIXTURE_MODE) {
        let result = fixture(mode.to_string_lossy().as_ref());
        process::exit(if result.is_ok() { 0 } else { 97 });
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("secret contract runtime");
    runtime.block_on(async {
        helper_output_matrix().await;
        environment_matrix().await;
        cancellation_is_typed().await;
        helper_environment_cannot_recurse();
    });
}

async fn helper_output_matrix() {
    let temporary = TempDir::new().expect("command base");
    let executor = CommandExecutor::new(1, temporary.path()).expect("executor");
    let operation = OperationGuard::new();

    let cases = [
        ("value", Expected::Text("value")),
        ("lf", Expected::Text("value")),
        ("crlf", Expected::Text("value")),
        ("double-lf", Expected::Text("value\n")),
        ("embedded-lf", Expected::Text("left\nright")),
        (
            "empty",
            Expected::Error(SecretResolutionErrorKind::InvalidValue),
        ),
        (
            "nul",
            Expected::Error(SecretResolutionErrorKind::InvalidValue),
        ),
        (
            "non-utf8",
            Expected::Error(SecretResolutionErrorKind::InvalidValue),
        ),
        (
            "exact-limit",
            Expected::Repeated(b'X', SECRET_HELPER_STREAM_BYTES),
        ),
        (
            "over-limit",
            Expected::Error(SecretResolutionErrorKind::LimitExceeded),
        ),
        (
            "stderr-over-limit",
            Expected::Error(SecretResolutionErrorKind::LimitExceeded),
        ),
        (
            "nonzero",
            Expected::Error(SecretResolutionErrorKind::HelperFailed),
        ),
    ];

    for (mode, expected) in cases {
        let reference = fixture_reference(mode);
        let result = secret::resolve(&reference, &executor, &operation, |_| None).await;
        match expected {
            Expected::Text(expected) => {
                let resolved = resolved(result, mode);
                assert_eq!(resolved.expose(), expected, "helper case {mode}");
            }
            Expected::Repeated(byte, length) => {
                let resolved = resolved(result, mode);
                assert_eq!(resolved.expose().len(), length, "helper case {mode}");
                assert!(
                    resolved
                        .expose()
                        .as_bytes()
                        .iter()
                        .all(|value| *value == byte),
                    "helper case {mode} returned unexpected bytes"
                );
            }
            Expected::Error(kind) => expect_error(result, kind, mode),
        }
    }

    let short_limits = CommandLimits::new(
        Duration::from_millis(100),
        SECRET_HELPER_STREAM_BYTES,
        SECRET_HELPER_STREAM_BYTES,
    )
    .expect("short helper limits");
    let short_executor = CommandExecutor::new(1, temporary.path())
        .expect("short executor")
        .with_role_limits(CommandRole::SecretHelper, short_limits)
        .expect("lower helper limits");
    let timeout = fixture_reference("timeout");
    expect_error(
        secret::resolve(&timeout, &short_executor, &OperationGuard::new(), |_| None).await,
        SecretResolutionErrorKind::LimitExceeded,
        "timeout",
    );

    // SAFETY: this custom-harness contract is the only test running in this
    // process, and the variable is removed before the next contract row.
    unsafe { env::set_var(AMBIENT_SENTINEL_NAME, PRIVATE_SENTINEL) };
    let ambient = fixture_reference("ambient");
    let ambient_result =
        secret::resolve(&ambient, &executor, &OperationGuard::new(), |_| None).await;
    // SAFETY: paired with the isolated mutation above.
    unsafe { env::remove_var(AMBIENT_SENTINEL_NAME) };
    expect_error(
        ambient_result,
        SecretResolutionErrorKind::InvalidValue,
        "ambient environment",
    );
}

async fn environment_matrix() {
    let temporary = TempDir::new().expect("command base");
    let executor = CommandExecutor::new(1, temporary.path()).expect("executor");
    let reference = SecretReference::environment("RFS_TEST_SECRET").expect("environment reference");

    let exact = secret::resolve(&reference, &executor, &OperationGuard::new(), |_| {
        Some(OsString::from("environment-value\n"))
    })
    .await;
    assert_eq!(
        resolved(exact, "environment value").expose(),
        "environment-value\n",
        "environment values must not receive helper newline normalization"
    );

    expect_error(
        secret::resolve(&reference, &executor, &OperationGuard::new(), |_| None).await,
        SecretResolutionErrorKind::Unavailable,
        "absent environment",
    );
    expect_error(
        secret::resolve(&reference, &executor, &OperationGuard::new(), |_| {
            Some(OsString::new())
        })
        .await,
        SecretResolutionErrorKind::InvalidValue,
        "empty environment",
    );
    expect_error(
        secret::resolve(&reference, &executor, &OperationGuard::new(), |_| {
            Some(OsString::from("environment-secret\0suffix"))
        })
        .await,
        SecretResolutionErrorKind::InvalidValue,
        "NUL environment",
    );
    expect_error(
        secret::resolve(&reference, &executor, &OperationGuard::new(), |_| {
            Some(non_unicode_os_string())
        })
        .await,
        SecretResolutionErrorKind::InvalidValue,
        "non-Unicode environment",
    );
}

async fn cancellation_is_typed() {
    let temporary = TempDir::new().expect("command base");
    let executor = CommandExecutor::new(1, temporary.path()).expect("executor");
    let operation = OperationGuard::new();
    operation.cancel();
    let reference = fixture_reference("value");

    expect_error(
        secret::resolve(&reference, &executor, &operation, |_| None).await,
        SecretResolutionErrorKind::Cancelled,
        "cancelled helper",
    );
}

fn helper_environment_cannot_recurse() {
    let nested = SecretReference::environment("RFS_NESTED_SECRET").expect("nested reference");
    let environment = ChildEnvironment::new(BTreeMap::from([(
        "RFS_DESTINATION".to_owned(),
        EnvironmentValue::secret(nested),
    )]))
    .expect("child environment");
    let command = CommandSpec::new(vec!["helper".to_owned()], environment).expect("command");
    let error = SecretReference::command(command).expect_err("recursive helper must be rejected");

    assert_eq!(
        error.to_string(),
        "secret helper environments cannot contain secret references"
    );
    assert!(!error.to_string().contains(PRIVATE_SENTINEL));
}

fn fixture_reference(mode: &str) -> SecretReference {
    let environment = ChildEnvironment::new(BTreeMap::from([(
        FIXTURE_MODE.to_owned(),
        EnvironmentValue::literal(mode).expect("fixture mode"),
    )]))
    .expect("fixture environment");
    let command = CommandSpec::new(
        vec![
            env::current_exe()
                .expect("current contract executable")
                .to_string_lossy()
                .into_owned(),
        ],
        environment,
    )
    .expect("fixture command");
    SecretReference::command(command).expect("fixture reference")
}

fn resolved(result: Result<Secret, SecretResolutionError>, case: &str) -> Secret {
    match result {
        Ok(secret) => secret,
        Err(error) => panic!("helper case {case} failed: {error}"),
    }
}

fn expect_error(
    result: Result<Secret, SecretResolutionError>,
    expected: SecretResolutionErrorKind,
    case: &str,
) {
    let error = match result {
        Ok(_) => panic!("secret case {case} unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), expected, "secret case {case}");
    let rendered = error.to_string();
    assert!(
        rendered.len() <= 64,
        "secret case {case} error was unbounded"
    );
    assert!(
        !rendered.contains(PRIVATE_SENTINEL)
            && !rendered.contains("environment-secret")
            && !rendered.contains("helper-secret"),
        "secret case {case} leaked a credential"
    );
}

#[cfg(unix)]
fn non_unicode_os_string() -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(vec![0xff])
}

#[cfg(windows)]
fn non_unicode_os_string() -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&[0xd800])
}

fn fixture(mode: &str) -> io::Result<()> {
    match mode {
        "value" => io::stdout().write_all(b"value"),
        "lf" => io::stdout().write_all(b"value\n"),
        "crlf" => io::stdout().write_all(b"value\r\n"),
        "double-lf" => io::stdout().write_all(b"value\n\n"),
        "embedded-lf" => io::stdout().write_all(b"left\nright"),
        "empty" => Ok(()),
        "nul" => io::stdout().write_all(b"helper-secret\0suffix"),
        "non-utf8" => io::stdout().write_all(&[0xff]),
        "exact-limit" => write_repeated(io::stdout().lock(), b'X', SECRET_HELPER_STREAM_BYTES),
        "over-limit" => write_repeated(io::stdout().lock(), b'X', SECRET_HELPER_STREAM_BYTES + 1),
        "stderr-over-limit" => {
            io::stdout().write_all(b"helper-secret")?;
            write_repeated(io::stderr().lock(), b'E', SECRET_HELPER_STREAM_BYTES + 1)
        }
        "nonzero" => {
            io::stdout().write_all(b"helper-secret")?;
            io::stderr().write_all(PRIVATE_SENTINEL.as_bytes())?;
            process::exit(31);
        }
        "timeout" => {
            io::stdout().write_all(b"helper-secret")?;
            io::stdout().flush()?;
            thread::sleep(Duration::from_secs(2));
            Ok(())
        }
        "ambient" => {
            if let Some(value) = env::var_os(AMBIENT_SENTINEL_NAME) {
                io::stdout().write_all(value.to_string_lossy().as_bytes())?;
            }
            Ok(())
        }
        _ => Err(io::Error::other("unknown secret fixture mode")),
    }
}

fn write_repeated(mut destination: impl Write, byte: u8, count: usize) -> io::Result<()> {
    let chunk = [byte; 1_024];
    let mut remaining = count;
    while remaining != 0 {
        let length = remaining.min(chunk.len());
        destination.write_all(&chunk[..length])?;
        remaining -= length;
    }
    destination.flush()
}

enum Expected {
    Text(&'static str),
    Repeated(u8, usize),
    Error(SecretResolutionErrorKind),
}
