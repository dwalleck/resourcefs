use std::{
    fs,
    net::TcpListener,
    path::Path,
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;

#[test]
fn static_check_is_offline_and_deterministic() {
    let temporary = TempDir::new().expect("check fixture directory");
    let listener = TcpListener::bind("127.0.0.1:0").expect("denied-network listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let endpoint = format!(
        "https://127.0.0.1:{}/",
        listener.local_addr().expect("listener address").port()
    );
    let helper_name = helper_name();
    create_executable(&temporary.path().join(helper_name));
    let helper_marker = temporary.path().join("helper-ran");
    let profile = json!({
        "schemaVersion": 1,
        "sources": [
            {
                "kind": "https",
                "id": "web",
                "required": false,
                "origins": [{
                    "baseUrl": endpoint,
                    "allowPrivateNetwork": true,
                    "credential": {
                        "header": "Authorization",
                        "secret": {
                            "kind": "command",
                            "command": {
                                "argv": [format!("./{helper_name}")],
                                "environment": {
                                    "RFS_CHECK_MARKER": {
                                        "kind": "literal",
                                        "value": helper_marker.to_string_lossy()
                                    }
                                }
                            }
                        }
                    }
                }]
            },
            {
                "kind": "documents",
                "id": "documents",
                "required": true,
                "converters": [{
                    "extensions": ["md"],
                    "input": "stdin",
                    "command": {
                        "argv": [format!("./{helper_name}")],
                        "environment": {
                            "RFS_CHECK_MARKER": {
                                "kind": "literal",
                                "value": helper_marker.to_string_lossy()
                            }
                        }
                    }
                }]
            }
        ]
    });
    let profile_path = write_profile(temporary.path(), "valid.json", &profile);
    let expected = concat!(
        "{\"ok\":true,\"schemaVersion\":1,\"probe\":false,\"sources\":[",
        "{\"id\":\"web\",\"kind\":\"https\",\"required\":false,\"state\":\"notProbed\"},",
        "{\"id\":\"documents\",\"kind\":\"documents\",\"required\":true,\"state\":\"notProbed\"}",
        "]}\n"
    );

    let first = run_check(&profile_path, false, &[]);
    let second = run_check(&profile_path, false, &[]);
    for output in [&first, &second] {
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, expected.as_bytes());
        assert!(output.stderr.is_empty());
    }
    assert_eq!(first.stdout, second.stdout);
    assert_no_connection(&listener);
    assert!(!helper_marker.exists(), "static check executed a helper");

    let command_directory = temporary.path().join("command-bin");
    fs::create_dir(&command_directory).expect("command directory");
    create_executable(&command_directory.join(helper_name));
    let bare_command = json!({
        "schemaVersion": 1,
        "sources": [{
            "kind": "documents",
            "id": "bare-command",
            "required": false,
            "converters": [{
                "extensions": ["rst"],
                "input": "stdin",
                "command": {
                    "argv": [helper_name],
                    "environment": {
                        "PATH": {
                            "kind": "literal",
                            "value": "command-bin"
                        }
                    }
                }
            }]
        }]
    });
    let bare_command_path = write_profile(temporary.path(), "bare-command.json", &bare_command);
    let bare_output = run_check(&bare_command_path, false, &[]);
    assert_eq!(bare_output.status.code(), Some(0));
    assert_eq!(
        bare_output.stdout,
        b"{\"ok\":true,\"schemaVersion\":1,\"probe\":false,\"sources\":[{\"id\":\"bare-command\",\"kind\":\"documents\",\"required\":false,\"state\":\"notProbed\"}]}\n"
    );
    assert!(bare_output.stderr.is_empty());

    let empty_profile_path =
        write_profile(temporary.path(), "empty.json", &json!({"schemaVersion": 1}));
    let empty_output = run_check(&empty_profile_path, false, &[]);
    assert_eq!(empty_output.status.code(), Some(0));
    assert_eq!(
        empty_output.stdout,
        b"{\"ok\":true,\"schemaVersion\":1,\"probe\":false,\"sources\":[]}\n"
    );
    assert!(empty_output.stderr.is_empty());

    let missing_environment = json!({
        "schemaVersion": 1,
        "sources": [{
            "kind": "github",
            "id": "github",
            "required": true,
            "allowPrivateNetwork": false,
            "credential": {"kind": "environment", "name": "RFS_CHECK_MISSING_SECRET"},
            "repositories": [{"name": "owner/repository"}]
        }]
    });
    let missing_environment_path = write_profile(
        temporary.path(),
        "missing-environment.json",
        &missing_environment,
    );
    assert_static_failure(
        run_check(
            &missing_environment_path,
            false,
            &[("RFS_CHECK_MISSING_SECRET", None)],
        ),
        "sources[0].credential",
    );

    let missing_executable = json!({
        "schemaVersion": 1,
        "sources": [{
            "kind": "documents",
            "id": "documents",
            "required": false,
            "converters": [{
                "extensions": ["txt"],
                "input": "path",
                "command": {"argv": ["./missing-helper"]}
            }]
        }]
    });
    let missing_executable_path = write_profile(
        temporary.path(),
        "missing-executable.json",
        &missing_executable,
    );
    assert_static_failure(
        run_check(&missing_executable_path, false, &[]),
        "sources[0].converters[0].command.argv[0]",
    );

    let missing_local_path = json!({
        "schemaVersion": 1,
        "sources": [{
            "kind": "skills",
            "id": "skills",
            "required": false,
            "roots": ["missing-skills"]
        }]
    });
    let missing_local_path_file = write_profile(
        temporary.path(),
        "missing-local-path.json",
        &missing_local_path,
    );
    assert_static_failure(
        run_check(&missing_local_path_file, false, &[]),
        "sources[0]",
    );

    let invalid_grant = json!({
        "schemaVersion": 1,
        "sources": [{
            "kind": "https",
            "id": "invalid-grant",
            "required": false,
            "grants": {"create": true},
            "origins": [{
                "baseUrl": endpoint,
                "allowPrivateNetwork": true
            }]
        }]
    });
    let invalid_grant_path = write_profile(temporary.path(), "invalid-grant.json", &invalid_grant);
    assert_static_failure(run_check(&invalid_grant_path, false, &[]), "source grants");

    let invalid_environment_name = json!({
        "schemaVersion": 1,
        "sources": [{
            "kind": "ssh",
            "id": "invalid-environment",
            "required": false,
            "command": {
                "argv": [format!("./{helper_name}")],
                "environment": {
                    "BAD=NAME": {"kind": "literal", "value": "x"}
                }
            },
            "hosts": [{"alias": "host", "remoteRoots": ["/srv"]}]
        }]
    });
    let invalid_environment_path = write_profile(
        temporary.path(),
        "invalid-environment.json",
        &invalid_environment_name,
    );
    assert_static_failure(
        run_check(&invalid_environment_path, false, &[]),
        "environment variable names",
    );

    let claim_collision = json!({
        "schemaVersion": 1,
        "sources": [{
            "kind": "downstreamMcp",
            "id": "claim-collision",
            "required": false,
            "servers": [
                {
                    "id": "first",
                    "schemes": ["Docs"],
                    "transport": {
                        "kind": "stdio",
                        "command": {"argv": [format!("./{helper_name}")]}
                    }
                },
                {
                    "id": "second",
                    "schemes": ["docs"],
                    "transport": {
                        "kind": "stdio",
                        "command": {"argv": [format!("./{helper_name}")]}
                    }
                }
            ]
        }]
    });
    let claim_collision_path =
        write_profile(temporary.path(), "claim-collision.json", &claim_collision);
    assert_static_failure(run_check(&claim_collision_path, false, &[]), "sources[0]");
    assert_no_connection(&listener);
}

fn run_check(path: &Path, probe: bool, environment: &[(&str, Option<&str>)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_resourcefs"));
    command.args(["check", "--config"]).arg(path);
    if probe {
        command.arg("--probe");
    }
    for (name, value) in environment {
        match value {
            Some(value) => {
                command.env(name, value);
            }
            None => {
                command.env_remove(name);
            }
        }
    }
    command.output().expect("resourcefs check")
}

fn assert_static_failure(output: Output, field: &str) {
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.len() <= 4_128);
    let diagnostic = String::from_utf8(output.stderr).expect("UTF-8 diagnostic");
    assert!(diagnostic.contains(field), "diagnostic: {diagnostic}");
    assert!(!diagnostic.contains("private-secret-sentinel"));
}

fn write_profile(directory: &Path, name: &str, profile: &Value) -> std::path::PathBuf {
    let path = directory.join(name);
    let bytes = serde_json::to_vec(profile).expect("profile JSON");
    fs::write(&path, bytes).expect("write profile");
    path
}

fn assert_no_connection(listener: &TcpListener) {
    match listener.accept() {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok(_) => panic!("static check performed network I/O"),
        Err(error) => panic!("listener failed: {error}"),
    }
}

#[cfg(unix)]
fn helper_name() -> &'static str {
    "check-helper"
}

#[cfg(windows)]
fn helper_name() -> &'static str {
    "check-helper.exe"
}

#[cfg(unix)]
fn create_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(
        path,
        b"#!/bin/sh\n: > \"$RFS_CHECK_MARKER\"\nprintf fixture-secret\n",
    )
    .expect("write helper");
    let mut permissions = fs::metadata(path).expect("helper metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("helper permissions");
}

#[cfg(windows)]
fn create_executable(path: &Path) {
    fs::write(path, b"fixture").expect("write helper");
}
