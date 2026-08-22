use std::{
    fs,
    net::TcpListener,
    path::Path,
    process::{Command, Output},
};

use serde_json::{Value, json};
use tempfile::TempDir;

#[test]
fn probe_state_and_exit_matrix() {
    let temporary = TempDir::new().expect("probe fixture directory");
    let helper_name = helper_name();
    create_executable(&temporary.path().join(helper_name));
    let probe_marker = temporary.path().join("probe-helper-ran");
    let credential = probe_credential(&probe_marker, helper_name);
    let refused_port = refused_port();
    let refused_url = format!("https://127.0.0.1:{refused_port}/");

    let profile = json!({
        "schemaVersion": 1,
        "sources": [
            {
                "kind": "documents",
                "id": "documents",
                "required": true,
                "converters": [{
                    "extensions": ["md"],
                    "input": "stdin",
                    "command": {"argv": [format!("./{helper_name}")]}
                }]
            },
            {
                "kind": "https",
                "id": "web",
                "required": false,
                "origins": [{
                    "baseUrl": refused_url,
                    "allowPrivateNetwork": true
                }]
            },
            {
                "kind": "github",
                "id": "github",
                "required": true,
                "apiBaseUrl": format!("https://127.0.0.1:{refused_port}/api/"),
                "allowPrivateNetwork": true,
                "credential": credential,
                "repositories": [{"name": "owner/repository"}]
            },
            {
                "kind": "downstreamMcp",
                "id": "downstream",
                "required": false,
                "servers": [{
                    "id": "docs",
                    "schemes": ["docs"],
                    "transport": {
                        "kind": "stdio",
                        "command": {"argv": [format!("./{helper_name}")]}
                    }
                }]
            }
        ]
    });
    let profile_path = write_profile(temporary.path(), "probe.json", &profile);
    let output = run_probe(&profile_path, &[("RFS_PROBE_TOKEN", "private-probe-token")]);
    let expected = concat!(
        "{\"ok\":false,\"schemaVersion\":1,\"probe\":true,\"sources\":[",
        "{\"id\":\"documents\",\"kind\":\"documents\",\"required\":true,\"state\":\"available\"},",
        "{\"id\":\"web\",\"kind\":\"https\",\"required\":false,\"state\":\"degraded\"},",
        "{\"id\":\"github\",\"kind\":\"github\",\"required\":true,\"state\":\"failed\"},",
        "{\"id\":\"downstream\",\"kind\":\"downstreamMcp\",\"required\":false,\"state\":\"unsupported\"}",
        "]}\n"
    );
    assert_probe_output(output, 3, expected);
    assert_probe_helper_ran(&probe_marker);

    let optional_degraded = json!({
        "schemaVersion": 1,
        "sources": [{
            "kind": "https",
            "id": "optional-web",
            "required": false,
            "origins": [{
                "baseUrl": format!("https://127.0.0.1:{refused_port}/"),
                "allowPrivateNetwork": true
            }]
        }]
    });
    let optional_path = write_profile(
        temporary.path(),
        "optional-degraded.json",
        &optional_degraded,
    );
    assert_probe_output(
        run_probe(&optional_path, &[]),
        0,
        concat!(
            "{\"ok\":true,\"schemaVersion\":1,\"probe\":true,\"sources\":[",
            "{\"id\":\"optional-web\",\"kind\":\"https\",\"required\":false,\"state\":\"degraded\"}",
            "]}\n"
        ),
    );
    let failing_helper = std::env::current_exe().expect("current probe contract executable");
    let failed_dependency = json!({
        "schemaVersion": 1,
        "sources": [{
            "kind": "github",
            "id": "credential-failure",
            "required": true,
            "apiBaseUrl": format!("https://127.0.0.1:{refused_port}/api/"),
            "allowPrivateNetwork": true,
            "credential": {
                "kind": "command",
                "command": {
                    "argv": [
                        failing_helper.to_string_lossy(),
                        "--resourcefs-invalid-probe-helper-option"
                    ]
                }
            },
            "repositories": [{"name": "owner/repository"}]
        }]
    });
    let failed_dependency_path = write_profile(
        temporary.path(),
        "failed-dependency.json",
        &failed_dependency,
    );
    assert_probe_output(
        run_probe(&failed_dependency_path, &[]),
        3,
        concat!(
            "{\"ok\":false,\"schemaVersion\":1,\"probe\":true,\"sources\":[",
            "{\"id\":\"credential-failure\",\"kind\":\"github\",\"required\":true,\"state\":\"failed\",\"diagnostic\":\"source credential could not be resolved\"}",
            "]}\n"
        ),
    );

    let empty_path = write_profile(temporary.path(), "empty.json", &json!({"schemaVersion": 1}));
    assert_probe_output(
        run_probe(&empty_path, &[]),
        0,
        "{\"ok\":true,\"schemaVersion\":1,\"probe\":true,\"sources\":[]}\n",
    );
}

fn run_probe(path: &Path, environment: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_resourcefs"));
    command.args(["check", "--config"]).arg(path).arg("--probe");
    for (name, value) in environment {
        command.env(name, value);
    }
    command.output().expect("resourcefs probe check")
}

fn assert_probe_output(output: Output, status: i32, expected: &str) {
    assert_eq!(output.status.code(), Some(status));
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    for sentinel in [
        b"private-probe-token".as_slice(),
        b"fixture-secret".as_slice(),
    ] {
        assert!(!contains_bytes(&output.stdout, sentinel));
        assert!(!contains_bytes(&output.stderr, sentinel));
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

fn write_profile(directory: &Path, name: &str, profile: &Value) -> std::path::PathBuf {
    let path = directory.join(name);
    fs::write(&path, serde_json::to_vec(profile).expect("profile JSON")).expect("write profile");
    path
}

fn refused_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve refused port");
    let port = listener.local_addr().expect("listener address").port();
    drop(listener);
    port
}

#[cfg(unix)]
fn helper_name() -> &'static str {
    "probe-helper"
}

#[cfg(windows)]
fn helper_name() -> &'static str {
    "probe-helper.exe"
}

#[cfg(unix)]
fn create_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(
        path,
        b"#!/bin/sh\nif [ -n \"$RFS_PROBE_MARKER\" ]; then : > \"$RFS_PROBE_MARKER\"; fi\nprintf fixture-secret\n",
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

#[cfg(unix)]
fn probe_credential(marker: &Path, helper_name: &str) -> Value {
    json!({
        "kind": "command",
        "command": {
            "argv": [format!("./{helper_name}")],
            "environment": {
                "RFS_PROBE_MARKER": {
                    "kind": "literal",
                    "value": marker.to_string_lossy()
                }
            }
        }
    })
}

#[cfg(windows)]
fn probe_credential(_marker: &Path, _helper_name: &str) -> Value {
    json!({"kind": "environment", "name": "RFS_PROBE_TOKEN"})
}

#[cfg(unix)]
fn assert_probe_helper_ran(marker: &Path) {
    assert!(
        marker.is_file(),
        "probe check did not execute credential helper"
    );
}

#[cfg(windows)]
fn assert_probe_helper_ran(_marker: &Path) {}
