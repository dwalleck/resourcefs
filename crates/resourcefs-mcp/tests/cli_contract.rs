use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio},
};

use serde_json::{Value, json};
use tempfile::TempDir;

fn binary() -> PathBuf {
    std::env::var_os("RESOURCEFS_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_resourcefs")))
}

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
    let mut command = Command::new(binary());
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

#[test]
fn selects_exactly_one_launch_authority() {
    let temporary = TempDir::new().expect("launch authority fixture");
    let root = temporary.path().join("cli-root");
    fs::create_dir(&root).expect("CLI root");
    let profile = write_profile(
        temporary.path(),
        "scratch.json",
        &scratch_profile("scratch-cache"),
    );
    let root_argument = format!("workspace={}", root.display());

    for arguments in [
        vec!["serve".to_owned()],
        vec![
            "serve".to_owned(),
            "--config".to_owned(),
            profile.display().to_string(),
            "--root".to_owned(),
            root_argument.clone(),
        ],
        vec![
            "serve".to_owned(),
            "--config".to_owned(),
            profile.display().to_string(),
            "--primary-root".to_owned(),
            "workspace".to_owned(),
        ],
    ] {
        let output = run_binary(&arguments, temporary.path(), &[]);
        assert_eq!(output.status.code(), Some(2), "arguments: {arguments:?}");
        assert!(output.stdout.is_empty(), "arguments: {arguments:?}");
        assert!(!output.stderr.is_empty(), "arguments: {arguments:?}");
    }

    let profile_only = run_initialized_serve(
        &[
            "serve".to_owned(),
            "--config".to_owned(),
            profile.display().to_string(),
        ],
        temporary.path(),
        &[],
    );
    assert_clean_protocol_serve(profile_only);

    let roots_only = run_initialized_serve(
        &[
            "serve".to_owned(),
            "--root".to_owned(),
            root_argument,
            "--primary-root".to_owned(),
            "workspace".to_owned(),
        ],
        temporary.path(),
        &[],
    );
    assert_clean_protocol_serve(roots_only);

    let help = run_binary(&["--help".to_owned()], temporary.path(), &[]);
    assert_eq!(help.status.code(), Some(0));
    assert!(help.stderr.is_empty());
    let help = String::from_utf8(help.stdout).expect("UTF-8 help");
    for command in ["serve", "check", "schema"] {
        assert!(help.contains(command), "missing {command} command: {help}");
    }
    for absent in ["config-manager", "profile", "telemetry"] {
        assert!(
            !help.contains(absent),
            "unexpected command {absent}: {help}"
        );
    }
}

#[test]
fn exit_and_channel_matrix() {
    let temporary = TempDir::new().expect("exit matrix fixture");
    let root = temporary.path().join("workspace");
    fs::create_dir(&root).expect("workspace");

    let success = run_binary(&["schema".to_owned()], temporary.path(), &[]);
    assert_eq!(success.status.code(), Some(0));
    assert!(success.stderr.is_empty());
    assert!(success.stdout.starts_with(b"{"));
    assert_eq!(success.stdout.last(), Some(&b'\n'));

    let missing = run_binary(
        &[
            "serve".to_owned(),
            "--config".to_owned(),
            temporary.path().join("missing.json").display().to_string(),
        ],
        temporary.path(),
        &[],
    );
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("could not read Server Profile"));

    // A configured kind with no compiled adapter still stops startup at exit 3.
    // `memory` stands in for that class now that `https` mounts; the assertion
    // is unchanged, only the kind that exercises it.
    let memory_root = temporary.path().join("memory-root");
    fs::create_dir(&memory_root).expect("memory root");
    let unsupported_profile = write_profile(
        temporary.path(),
        "unsupported.json",
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"unsupported-cache"},
            "sources":[{
                "kind":"memory",
                "id":"notes",
                "required":true,
                "roots":[{"name":"notes","path":"memory-root"}]
            }]
        }),
    );
    let unsupported = run_serve_profile(&unsupported_profile, temporary.path(), &[]);
    assert_eq!(unsupported.status.code(), Some(3));
    assert!(unsupported.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("not supported"));

    let log_directory = temporary.path().join("log-directory");
    fs::create_dir(&log_directory).expect("log directory");
    let internal_profile = write_profile(
        temporary.path(),
        "internal.json",
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"internal-cache"},
            "logging":{"destination":{"kind":"file","path":"log-directory"}}
        }),
    );
    let internal = run_serve_profile(&internal_profile, temporary.path(), &[]);
    assert_eq!(internal.status.code(), Some(1));
    assert!(internal.stdout.is_empty());
    assert!(!internal.stderr.is_empty());
    let clean = run_initialized_serve(
        &[
            "serve".to_owned(),
            "--root".to_owned(),
            format!("workspace={}", root.display()),
        ],
        temporary.path(),
        &[],
    );
    assert_clean_protocol_serve(clean);
}

#[test]
fn profile_serve_matrix() {
    let temporary = TempDir::new().expect("profile serve fixture");
    let scratch = write_profile(
        temporary.path(),
        "scratch.json",
        &scratch_profile("scratch-cache"),
    );
    assert_clean_protocol_serve(run_initialized_serve(
        &[
            "serve".to_owned(),
            "--config".to_owned(),
            scratch.display().to_string(),
        ],
        temporary.path(),
        &[],
    ));

    let helper = temporary.path().join(helper_name());
    create_executable(&helper);
    for directory in ["skills", "notes", "vault"] {
        fs::create_dir(temporary.path().join(directory)).expect("source directory");
    }
    fs::write(temporary.path().join("rules.json"), b"{}").expect("rules manifest");
    fs::write(temporary.path().join("agents.json"), b"{}").expect("agent manifest");

    // `https` is absent because this binary now mounts it; the kinds below are
    // the ones that remain declared-but-uncompiled. Its startup behaviour is
    // covered by `required_https_fails_startup` and the degradation fences.
    for kind in [
        "github",
        "ssh",
        "documents",
        "skills",
        "rules",
        "memory",
        "vault",
        "agentExport",
        "downstreamMcp",
    ] {
        let source = unsupported_source(kind, &helper);
        let profile = write_profile(
            temporary.path(),
            &format!("{kind}.json"),
            &json!({
                "schemaVersion":1,
                "session":{"cacheDirectory":format!("{kind}-cache")},
                "sources":[source]
            }),
        );
        let output = run_serve_profile(
            &profile,
            temporary.path(),
            &[("RFS_SERVE_SECRET", Some("serve-secret-sentinel"))],
        );
        assert_eq!(output.status.code(), Some(3), "source kind {kind}");
        assert!(output.stdout.is_empty(), "source kind {kind}");
        let diagnostic = String::from_utf8(output.stderr).expect("UTF-8 unavailable diagnostic");
        assert!(
            diagnostic.contains(kind),
            "source kind {kind}: {diagnostic}"
        );
        assert!(
            !diagnostic.contains("serve-secret-sentinel"),
            "source kind {kind}: {diagnostic}"
        );
    }
}

#[test]
fn secret_never_reaches_observable_channels() {
    const STATIC_SECRET: &str = "RFS-static-credential-sentinel";
    const PROBE_SECRET: &str = "RFS-probe-credential-sentinel";
    const LOG_SECRET: &str = "RFS-log-ambient-sentinel";

    let temporary = TempDir::new().expect("secret sink fixture");
    #[cfg(feature = "test-support")]
    const STDERR_REDACTION_SECRET: &str = "RFS-stderr-redaction-sentinel";
    #[cfg(feature = "test-support")]
    const FILE_REDACTION_SECRET: &str = "RFS-file-redaction-sentinel";
    let static_profile = write_profile(
        temporary.path(),
        "secret-static.json",
        &json!({
            "schemaVersion":1,
            "sources":[{
                "kind":"github",
                "id":"github",
                "required":false,
                "allowPrivateNetwork":false,
                "credential":{"kind":"environment","name":"RFS_SECRET_STATIC"},
                "repositories":[{"name":"owner/repository"}]
            }]
        }),
    );
    let static_output = run_check(
        &static_profile,
        false,
        &[("RFS_SECRET_STATIC", Some(STATIC_SECRET))],
    );
    assert_eq!(static_output.status.code(), Some(0));
    assert_bytes_exclude(&static_output.stdout, STATIC_SECRET);
    assert_bytes_exclude(&static_output.stderr, STATIC_SECRET);

    let listener = TcpListener::bind("127.0.0.1:0").expect("probe listener");
    let probe_profile = write_profile(
        temporary.path(),
        "secret-probe.json",
        &json!({
            "schemaVersion":1,
            "sources":[{
                "kind":"https",
                "id":"web",
                "required":false,
                "origins":[{
                    "baseUrl":format!(
                        "https://127.0.0.1:{}/",
                        listener.local_addr().expect("probe listener address").port()
                    ),
                    "allowPrivateNetwork":true,
                    "credential":{
                        "header":"Authorization",
                        "secret":{"kind":"environment","name":"RFS_SECRET_PROBE"}
                    }
                }]
            }]
        }),
    );
    let probe_output = run_check(
        &probe_profile,
        true,
        &[("RFS_SECRET_PROBE", Some(PROBE_SECRET))],
    );
    assert!(matches!(probe_output.status.code(), Some(0) | Some(3)));
    assert_bytes_exclude(&probe_output.stdout, PROBE_SECRET);
    assert_bytes_exclude(&probe_output.stderr, PROBE_SECRET);

    let cache_file = temporary.path().join("not-a-cache-directory");

    #[cfg(feature = "test-support")]
    {
        let stderr_profile = write_profile(
            temporary.path(),
            "redaction-stderr.json",
            &scratch_profile("redaction-stderr-cache"),
        );
        let stderr_redaction = run_initialized_serve(
            &[
                "serve".to_owned(),
                "--config".to_owned(),
                stderr_profile.display().to_string(),
            ],
            temporary.path(),
            &[
                (
                    "RESOURCEFS_TEST_REDACTION_SECRET",
                    Some(STDERR_REDACTION_SECRET),
                ),
                ("RESOURCEFS_TEST_LOG_MESSAGE", Some(STDERR_REDACTION_SECRET)),
            ],
        );
        assert_bytes_exclude(&stderr_redaction.stdout, STDERR_REDACTION_SECRET);
        assert_bytes_exclude(&stderr_redaction.stderr, STDERR_REDACTION_SECRET);
        let redacted_stderr = assert_protocol_serve(stderr_redaction);
        assert!(redacted_stderr.contains("<redacted>"));

        let redaction_log = temporary.path().join("redaction.log");
        let file_profile = write_profile(
            temporary.path(),
            "redaction-file.json",
            &json!({
                "schemaVersion":1,
                "session":{
                    "cacheDirectory":"redaction-file-cache",
                    "retentionTtlSeconds":0
                },
                "logging":{"destination":{
                    "kind":"file",
                    "path":"redaction.log",
                    "rotationBytes":1024,
                    "retainFiles":2
                }}
            }),
        );
        let file_redaction = run_initialized_serve(
            &[
                "serve".to_owned(),
                "--config".to_owned(),
                file_profile.display().to_string(),
            ],
            temporary.path(),
            &[
                (
                    "RESOURCEFS_TEST_REDACTION_SECRET",
                    Some(FILE_REDACTION_SECRET),
                ),
                ("RESOURCEFS_TEST_LOG_MESSAGE", Some(FILE_REDACTION_SECRET)),
            ],
        );
        assert_bytes_exclude(&file_redaction.stdout, FILE_REDACTION_SECRET);
        assert_bytes_exclude(&file_redaction.stderr, FILE_REDACTION_SECRET);
        assert_clean_protocol_serve(file_redaction);
        let log_bytes = fs::read(redaction_log).expect("redacted file log");
        assert_bytes_exclude(&log_bytes, FILE_REDACTION_SECRET);
        assert!(String::from_utf8_lossy(&log_bytes).contains("<redacted>"));
    }
    fs::write(&cache_file, b"fixture").expect("cache file");
    let log_path = temporary.path().join("resourcefs.log");
    let logging_profile = write_profile(
        temporary.path(),
        "secret-log.json",
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"not-a-cache-directory"},
            "logging":{"destination":{
                "kind":"file",
                "path":"resourcefs.log",
                "rotationBytes":1024,
                "retainFiles":2
            }}
        }),
    );
    let logged = run_serve_profile(
        &logging_profile,
        temporary.path(),
        &[("RFS_LOG_AMBIENT", Some(LOG_SECRET))],
    );
    assert_eq!(logged.status.code(), Some(1));
    assert!(logged.stdout.is_empty());
    assert!(logged.stderr.is_empty());
    assert_bytes_exclude(&fs::read(log_path).expect("file diagnostic"), LOG_SECRET);

    let unavailable = run_serve_profile(
        &static_profile,
        temporary.path(),
        &[("RFS_SECRET_STATIC", Some(STATIC_SECRET))],
    );
    assert_eq!(unavailable.status.code(), Some(3));
    assert_bytes_exclude(&unavailable.stdout, STATIC_SECRET);
    assert_bytes_exclude(&unavailable.stderr, STATIC_SECRET);
}

#[test]
fn profile_workspace_authority() {
    let temporary = TempDir::new().expect("profile workspace fixture");
    let profile_directory = temporary.path().join("profile");
    let launch_directory = temporary.path().join("launch");
    let profile_root = profile_directory.join("profile-root");
    let client_root = temporary.path().join("client-root");
    fs::create_dir_all(&profile_root).expect("profile root");
    fs::create_dir(&launch_directory).expect("launch directory");
    fs::create_dir(&client_root).expect("client root");
    fs::write(profile_root.join("shared.txt"), "profile").expect("profile sentinel");
    fs::write(launch_directory.join("shared.txt"), "launch").expect("launch sentinel");
    fs::write(client_root.join("shared.txt"), "client").expect("client sentinel");
    let profile = write_profile(
        &profile_directory,
        "server.json",
        &workspace_profile("profile-root", "profile-cache"),
    );

    let mut process = ProfileMcpProcess::start(&profile, &launch_directory);
    process.initialize_with_roots(Vec::new());
    let profile_read = process.call_read("shared.txt");
    assert_eq!(profile_read["structuredContent"]["content"], "profile");
    assert_eq!(
        profile_read["structuredContent"]["canonicalReference"],
        "rfs://workspace/profile/shared.txt"
    );
    assert!(
        profile_read["structuredContent"]
            .get("backingFileUri")
            .is_none()
    );

    process.change_roots(vec![mcp_root(&client_root, "client")]);
    let client_read = process.call_read_until_content("shared.txt", "client");
    assert_eq!(client_read["structuredContent"]["content"], "client");
    assert_ne!(
        client_read["structuredContent"]["canonicalReference"],
        "rfs://workspace/profile/shared.txt"
    );
    let old_profile = process.call_read("rfs://workspace/profile/shared.txt");
    assert_eq!(old_profile["isError"], true);

    process.change_roots(Vec::new());
    process.call_read_until_content("shared.txt", "profile");
    assert!(process.finish().is_empty());

    let alternate_root = temporary.path().join("alternate-root");
    fs::create_dir(&alternate_root).expect("alternate client root");
    fs::write(alternate_root.join("shared.txt"), "alternate").expect("alternate sentinel");
    let mut deferred_profile = scratch_profile("deferred-primary-cache");
    deferred_profile
        .as_object_mut()
        .expect("profile object")
        .insert("workspace".to_owned(), json!({"primaryRoot":"client"}));
    let deferred_profile = write_profile(
        &profile_directory,
        "deferred-primary.json",
        &deferred_profile,
    );
    let mut deferred = ProfileMcpProcess::start(&deferred_profile, &launch_directory);
    deferred.initialize_with_roots(vec![
        mcp_root(&alternate_root, "alternate"),
        mcp_root(&client_root, "client"),
    ]);
    deferred.call_read_until_content("shared.txt", "client");
    assert!(deferred.finish().is_empty());
}

#[test]
fn profile_is_immutable_per_process() {
    let temporary = TempDir::new().expect("immutable profile fixture");
    let first_root = temporary.path().join("first");
    let second_root = temporary.path().join("second");
    let launch_directory = temporary.path().join("launch");
    fs::create_dir(&first_root).expect("first root");
    fs::create_dir(&second_root).expect("second root");
    fs::create_dir(&launch_directory).expect("launch directory");
    fs::write(first_root.join("version.txt"), "version-one").expect("first version");
    fs::write(second_root.join("version.txt"), "version-two").expect("second version");
    let profile_path = write_profile(
        temporary.path(),
        "server.json",
        &workspace_profile("first", "immutable-cache"),
    );

    let mut first = ProfileMcpProcess::start(&profile_path, &launch_directory);
    first.initialize_with_roots(Vec::new());
    assert_eq!(
        first.call_read("version.txt")["structuredContent"]["content"],
        "version-one"
    );
    write_profile(
        temporary.path(),
        "server.json",
        &workspace_profile("second", "immutable-cache"),
    );
    assert_eq!(
        first.call_read("version.txt")["structuredContent"]["content"],
        "version-one"
    );
    assert!(first.finish().is_empty());

    let mut second = ProfileMcpProcess::start(&profile_path, &launch_directory);
    second.initialize_with_roots(Vec::new());
    assert_eq!(
        second.call_read("version.txt")["structuredContent"]["content"],
        "version-two"
    );
    fs::remove_file(&profile_path).expect("remove profile");
    assert_eq!(
        second.call_read("version.txt")["structuredContent"]["content"],
        "version-two"
    );
    assert!(second.finish().is_empty());

    let missing = run_serve_profile(&profile_path, &launch_directory, &[]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
}

fn workspace_profile(root: &str, cache: &str) -> Value {
    json!({
        "schemaVersion":1,
        "workspace":{
            "roots":[{"id":"profile","path":root}],
            "primaryRoot":"profile"
        },
        "session":{"cacheDirectory":cache,"retentionTtlSeconds":0}
    })
}

fn mcp_root(path: &Path, name: &str) -> Value {
    json!({
        "uri":url::Url::from_directory_path(path)
            .expect("directory file URI")
            .to_string(),
        "name":name
    })
}

struct ProfileMcpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    roots: Vec<Value>,
    next_id: u64,
}

impl ProfileMcpProcess {
    fn start(profile: &Path, current_directory: &Path) -> Self {
        let mut child = Command::new(binary())
            .args(["serve", "--config"])
            .arg(profile)
            .current_dir(current_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start profile ResourceFS");
        let stdin = child.stdin.take().expect("profile ResourceFS stdin");
        let stdout = BufReader::new(child.stdout.take().expect("profile ResourceFS stdout"));
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            roots: Vec::new(),
            next_id: 1,
        }
    }

    fn initialize_with_roots(&mut self, roots: Vec<Value>) {
        self.roots = roots;
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion":"2026-07-28",
                "capabilities":{"roots":{"listChanged":true}},
                "clientInfo":{"name":"profile-cli-contract","version":"1.0.0"}
            }),
        );
        assert!(response.get("error").is_none(), "initialize: {response}");
        self.write(&json!({
            "jsonrpc":"2.0",
            "method":"notifications/initialized"
        }));
    }
    fn call_read(&mut self, path: &str) -> Value {
        let response = self.request(
            "tools/call",
            json!({"name":"rfs_read","arguments":{"path":path}}),
        );
        assert!(response.get("error").is_none(), "read response: {response}");
        response["result"].clone()
    }

    fn call_read_until_content(&mut self, path: &str, expected: &str) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let result = self.call_read(path);
            if result["structuredContent"]["content"].as_str() == Some(expected) {
                return result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "read {path:?} did not converge to {expected:?}: {result}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn change_roots(&mut self, roots: Vec<Value>) {
        self.roots = roots;
        self.write(&json!({
            "jsonrpc":"2.0",
            "method":"notifications/roots/list_changed"
        }));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({
            "jsonrpc":"2.0",
            "id":id,
            "method":method,
            "params":params
        }));
        loop {
            let message = self.read();
            if message["method"] == "roots/list" {
                let request_id = message["id"].clone();
                self.write(&json!({
                    "jsonrpc":"2.0",
                    "id":request_id,
                    "result":{"roots":self.roots}
                }));
                continue;
            }
            if message["id"] == id {
                return message;
            }
        }
    }

    fn write(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("open profile ResourceFS stdin");
        serde_json::to_writer(&mut *stdin, message).expect("serialize MCP message");
        writeln!(stdin).expect("MCP message delimiter");
        stdin.flush().expect("flush MCP message");
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        let bytes = self.stdout.read_line(&mut line).expect("read MCP message");
        assert_ne!(bytes, 0, "profile ResourceFS closed protocol stdout");
        serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("invalid MCP response {line:?}: {error}"))
    }

    fn finish(&mut self) -> String {
        self.stdin.take();
        let status = self.child.wait().expect("wait for profile ResourceFS");
        let mut remaining = String::new();
        self.stdout
            .read_to_string(&mut remaining)
            .expect("drain profile stdout");
        assert!(
            remaining.trim().is_empty(),
            "unexpected MCP stdout: {remaining}"
        );
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .expect("profile ResourceFS stderr")
            .read_to_string(&mut stderr)
            .expect("read profile stderr");
        assert!(
            status.success(),
            "profile ResourceFS exited {status}: {stderr}"
        );
        stderr
    }
}

impl Drop for ProfileMcpProcess {
    fn drop(&mut self) {
        if self.child.try_wait().is_ok_and(|status| status.is_none()) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn scratch_profile(cache_directory: &str) -> Value {
    json!({
        "schemaVersion":1,
        "session":{"cacheDirectory":cache_directory,"retentionTtlSeconds":0}
    })
}

fn unsupported_source(kind: &str, helper: &Path) -> Value {
    let command = helper.display().to_string();
    let body = match kind {
        "https" => json!({
            "origins":[{"baseUrl":"https://example.test/","allowPrivateNetwork":false}]
        }),
        "github" => json!({
            "allowPrivateNetwork":false,
            "credential":{"kind":"environment","name":"RFS_SERVE_SECRET"},
            "repositories":[{"name":"owner/repository"}]
        }),
        "ssh" => json!({
            "command":{"argv":[command]},
            "hosts":[{"alias":"host","remoteRoots":["/srv/repository"]}]
        }),
        "documents" => json!({
            "converters":[{
                "extensions":["md"],
                "input":"stdin",
                "command":{"argv":[command]}
            }]
        }),
        "skills" => json!({"roots":["skills"]}),
        "rules" => json!({"manifests":["rules.json"]}),
        "memory" => json!({"roots":[{"name":"notes","path":"notes"}]}),
        "vault" => json!({"vaults":[{"name":"vault","path":"vault"}]}),
        "agentExport" => json!({"manifests":["agents.json"]}),
        "downstreamMcp" => json!({
            "servers":[{
                "id":"server",
                "schemes":["example"],
                "transport":{"kind":"stdio","command":{"argv":[command]}}
            }]
        }),
        _ => panic!("unknown source kind {kind}"),
    };
    let mut source = body.as_object().expect("source body").clone();
    source.insert("kind".to_owned(), Value::String(kind.to_owned()));
    source.insert("id".to_owned(), Value::String(format!("{kind}-source")));
    source.insert("required".to_owned(), Value::Bool(false));
    Value::Object(source)
}

fn run_serve_profile(
    profile: &Path,
    current_directory: &Path,
    environment: &[(&str, Option<&str>)],
) -> Output {
    run_binary(
        &[
            "serve".to_owned(),
            "--config".to_owned(),
            profile.display().to_string(),
        ],
        current_directory,
        environment,
    )
}

fn run_binary(
    arguments: &[String],
    current_directory: &Path,
    environment: &[(&str, Option<&str>)],
) -> Output {
    let mut command = Command::new(binary());
    command
        .args(arguments)
        .current_dir(current_directory)
        .stdin(Stdio::null());
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
    command.output().expect("run resourcefs")
}

fn run_initialized_serve(
    arguments: &[String],
    current_directory: &Path,
    environment: &[(&str, Option<&str>)],
) -> Output {
    use std::io::Write as _;

    let mut command = Command::new(binary());
    command
        .args(arguments)
        .current_dir(current_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
    let mut child = command.spawn().expect("start initialized resourcefs");
    let mut stdin = child.stdin.take().expect("resourcefs stdin");
    serde_json::to_writer(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{
                "protocolVersion":"2026-07-28",
                "capabilities":{},
                "clientInfo":{"name":"cli-contract","version":"1.0.0"}
            }
        }),
    )
    .expect("serialize initialize");
    writeln!(stdin).expect("initialize delimiter");
    serde_json::to_writer(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .expect("serialize initialized");
    writeln!(stdin).expect("initialized delimiter");
    stdin.flush().expect("flush protocol input");
    drop(stdin);
    child.wait_with_output().expect("finish initialized serve")
}

fn assert_protocol_serve(output: Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let messages = String::from_utf8(output.stdout).expect("UTF-8 protocol output");
    let rows = messages.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "protocol messages: {messages}");
    let response: Value = serde_json::from_str(rows[0]).expect("initialize response JSON");
    assert_eq!(response["id"], 1);
    assert!(
        response.get("error").is_none(),
        "initialize response: {response}"
    );
    String::from_utf8(output.stderr).expect("UTF-8 diagnostic output")
}

fn assert_clean_protocol_serve(output: Output) {
    let stderr = assert_protocol_serve(output);
    assert!(stderr.is_empty(), "unexpected diagnostic: {stderr}");
}

fn assert_bytes_exclude(bytes: &[u8], sentinel: &str) {
    const MIN_FRAGMENT_BYTES: usize = 8;
    assert!(sentinel.len() >= MIN_FRAGMENT_BYTES);
    for fragment in sentinel.as_bytes().windows(MIN_FRAGMENT_BYTES) {
        assert!(
            !bytes
                .windows(MIN_FRAGMENT_BYTES)
                .any(|window| window == fragment),
            "observable bytes contained a secret sentinel fragment"
        );
    }
}

/// C14 — a `required` source that cannot be reached stops startup at exit 3,
/// and the diagnostic names the source id so an operator knows which one.
///
/// Both rows below produce the same `Degraded`/`Failed` shape from the probe's
/// point of view, because `connect_policed` collapses its causes into one bool
/// (giving probe failures a real diagnostic is tracked at rfs-0p7j). They are
/// told apart here by a **server-side** observation instead: the policy row
/// keeps a live listener and asserts it accepted nothing, which is only true if
/// the address policy refused before egress. The unreachable row grants
/// private-network access, so policy cannot be the refuser and only the closed
/// port can explain the failure. Neither fence depends on the missing
/// diagnostic.
#[test]
fn required_https_fails_startup() {
    let temporary = TempDir::new().expect("temporary directory");

    // Unreachable: the grant is present, so the closed port is the only cause.
    let closed = TcpListener::bind("127.0.0.1:0").expect("closed-port listener");
    let closed_port = closed.local_addr().expect("closed address").port();
    drop(closed);
    let unreachable = write_profile(
        temporary.path(),
        "required-unreachable.json",
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"unreachable-cache"},
            "sources":[{
                "kind":"https","id":"web","required":true,
                "origins":[{
                    "baseUrl":format!("https://127.0.0.1:{closed_port}/"),
                    "allowPrivateNetwork":true
                }]
            }]
        }),
    );
    let run = run_serve_profile(&unreachable, temporary.path(), &[]);
    assert_eq!(
        run.status.code(),
        Some(3),
        "a required source must stop startup"
    );
    assert!(run.stdout.is_empty(), "MCP stdout carries no diagnostics");
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(
        stderr.contains("web"),
        "the diagnostic must name the source id; got: {stderr}"
    );
    // Without this, the row passes for the wrong reason: before `https` became
    // a compiled kind, *every* https profile also exited 3 — with "not
    // supported", a message that likewise contains the source id. Only the
    // startup probe produces this phrasing.
    assert!(
        stderr.contains("unavailable at startup"),
        "exit 3 must come from the startup probe, not the uncompiled-kind gate; got: {stderr}"
    );

    // Policy: something *is* listening, but the grant is withheld. The listener
    // is the oracle — a refusal that happens before egress accepts nothing.
    let live = TcpListener::bind("127.0.0.1:0").expect("policy-row listener");
    live.set_nonblocking(true).expect("nonblocking listener");
    let live_port = live.local_addr().expect("live address").port();
    let ungranted = write_profile(
        temporary.path(),
        "required-ungranted.json",
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"ungranted-cache"},
            "sources":[{
                "kind":"https","id":"intranet","required":true,
                "origins":[{
                    "baseUrl":format!("https://127.0.0.1:{live_port}/"),
                    "allowPrivateNetwork":false
                }]
            }]
        }),
    );
    let refused = run_serve_profile(&ungranted, temporary.path(), &[]);
    assert_eq!(
        refused.status.code(),
        Some(3),
        "a withheld private-network grant also stops a required source"
    );
    let refused_stderr = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        refused_stderr.contains("intranet"),
        "the diagnostic must name the source id"
    );
    assert!(
        refused_stderr.contains("unavailable at startup"),
        "exit 3 must come from the startup probe; got: {refused_stderr}"
    );
    assert_eq!(
        live.accept()
            .expect_err("policy must refuse before egress")
            .kind(),
        std::io::ErrorKind::WouldBlock,
        "the probe must not connect to an address its origin does not grant"
    );
}

/// C14 — `resourcefs check --probe` reports reachability, and does so without
/// mutating the origin it probes.
///
/// Both rows are optional sources, so both are *acceptable* and both exit 0.
/// Holding the exit code constant is deliberate: it forces the fence onto the
/// reported `state`, which is the thing that actually carries reachability. The
/// two profiles differ in one variable — whether the port is open — so the
/// available/degraded split is attributable to reachability and nothing else.
///
/// Non-mutation is proved server-side rather than asserted. The probe stops at
/// a TCP connect, so the accepted socket must yield end-of-stream with zero
/// bytes; a connection that carries no application bytes cannot have changed
/// anything on the far side under any protocol. Reading the socket is also what
/// confirms the probe genuinely connected, so the reachable row cannot pass by
/// never dialling at all.
#[test]
fn check_probe_reports_reachability_without_mutating() {
    let temporary = TempDir::new().expect("temporary directory");

    let live = TcpListener::bind("127.0.0.1:0").expect("reachable listener");
    let live_port = live.local_addr().expect("live address").port();
    let reachable = write_profile(
        temporary.path(),
        "probe-reachable.json",
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"probe-reachable-cache"},
            "sources":[{
                "kind":"https","id":"web","required":false,
                "origins":[{
                    "baseUrl":format!("https://127.0.0.1:{live_port}/"),
                    "allowPrivateNetwork":true
                }]
            }]
        }),
    );
    let output = run_check(&reachable, true, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a reachable source is acceptable"
    );
    let report = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        report.contains("\"probe\":true") && report.contains("\"state\":\"available\""),
        "a reachable origin must be reported available; got: {report}"
    );

    // Non-blocking: `run_check` has already returned, so the connection is
    // either in the backlog or it never happened. Blocking here would hang
    // instead of failing when the probe stops dialling.
    live.set_nonblocking(true).expect("nonblocking listener");
    let (mut accepted, _) = live
        .accept()
        .expect("the probe must connect to a granted origin");
    accepted
        .set_nonblocking(false)
        .expect("blocking accepted stream");
    accepted
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("read timeout");
    let mut received = Vec::new();
    accepted
        .read_to_end(&mut received)
        .expect("the probe must close its connection");
    assert!(
        received.is_empty(),
        "the probe must not send a request; it sent {} byte(s)",
        received.len()
    );

    // One variable changes: the port is closed. Same shape, same exit code.
    let closed = TcpListener::bind("127.0.0.1:0").expect("closed-port listener");
    let closed_port = closed.local_addr().expect("closed address").port();
    drop(closed);
    let unreachable = write_profile(
        temporary.path(),
        "probe-unreachable.json",
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"probe-unreachable-cache"},
            "sources":[{
                "kind":"https","id":"web","required":false,
                "origins":[{
                    "baseUrl":format!("https://127.0.0.1:{closed_port}/"),
                    "allowPrivateNetwork":true
                }]
            }]
        }),
    );
    let degraded = run_check(&unreachable, true, &[]);
    assert_eq!(
        degraded.status.code(),
        Some(0),
        "an unreachable *optional* source is still acceptable"
    );
    let degraded_report = String::from_utf8_lossy(&degraded.stdout).into_owned();
    assert!(
        degraded_report.contains("\"state\":\"degraded\""),
        "an unreachable origin must be reported degraded; got: {degraded_report}"
    );
}

/// C14 budget — probing is bounded by the shared five-second startup deadline,
/// and one unresponsive origin cannot starve a healthy sibling of its window.
///
/// A *refused* port answers instantly, so every other fence in this file
/// measures the cheap failure. This one builds the expensive one: a listener
/// whose accept queue is saturated drops further SYNs, so a connect neither
/// succeeds nor is refused — it hangs until the probe's own 30-second timeout.
/// That is what a firewalled origin does in production, and it is the only
/// shape that can breach the budget.
///
/// The blackholed source is deliberately **first** in profile order. That makes
/// the starvation assertion real rather than decorative: probing sequentially
/// under one shared budget would spend all five seconds on it and report the
/// healthy sibling unavailable for a failure that was not its own. Asserting
/// the sibling is `available` is therefore what distinguishes a concurrent run
/// under a deadline from a sequential one, and no wall-clock bound alone can
/// tell those two apart.
#[test]
fn probing_is_bounded_and_does_not_starve_a_healthy_source() {
    let temporary = TempDir::new().expect("temporary directory");

    let blackhole = TcpListener::bind("127.0.0.1:0").expect("blackhole listener");
    let blackhole_port = blackhole.local_addr().expect("blackhole address").port();
    // Saturating is delegated to the probe module, the one place the
    // architecture contract permits raw TCP egress. These connections must
    // outlive the run, hence the binding.
    let _saturation = resourcefs_sources::saturate_accept_queue(blackhole_port);
    assert!(
        !_saturation.is_empty(),
        "the blackhole listener must accept some connections before saturating"
    );

    fs::create_dir(temporary.path().join("memory-root")).expect("memory root");

    let profile = write_profile(
        temporary.path(),
        "probe-budget.json",
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"probe-budget-cache"},
            "sources":[
                {
                    "kind":"https","id":"blackholed","required":false,
                    "origins":[{
                        "baseUrl":format!("https://127.0.0.1:{blackhole_port}/"),
                        "allowPrivateNetwork":true
                    }]
                },
                // A different kind, because a profile admits each kind once.
                // `memory` probes locally and instantly, which sharpens rather
                // than weakens the claim: if even the cheapest probe in the
                // system is reported unavailable, it can only be because the
                // run never reached it.
                {
                    "kind":"memory","id":"notes","required":false,
                    "roots":[{"name":"notes","path":"memory-root"}]
                }
            ]
        }),
    );

    let started = std::time::Instant::now();
    let output = run_check(&profile, true, &[]);
    let elapsed = started.elapsed();

    assert_eq!(output.status.code(), Some(0), "both sources are optional");
    // The deadline is 5s and an unbounded run is 30s; 20s separates them with
    // room for a loaded machine without ever admitting the unbounded case.
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "probing must be bounded by the startup deadline; took {elapsed:?}"
    );

    let report: Value =
        serde_json::from_slice(&output.stdout).expect("the probe report must be JSON");
    let state = |id: &str| -> String {
        report["sources"]
            .as_array()
            .expect("sources array")
            .iter()
            .find(|source| source["id"] == id)
            .unwrap_or_else(|| panic!("no report row for source '{id}'"))["state"]
            .as_str()
            .expect("state string")
            .to_owned()
    };
    assert_eq!(
        state("blackholed"),
        "degraded",
        "an origin that never answers is unavailable at startup"
    );
    assert_eq!(
        state("notes"),
        "available",
        "a healthy source must not be starved of the window by an unresponsive sibling"
    );
}

/// C14 — a source whose kind this binary cannot mount is refused *before* it is
/// probed, so startup neither dials its origin nor runs its credential helper.
///
/// Startup probing made this reachable: the launch path now probes every
/// configured source, and probing resolves deferred secrets. Without ordering
/// the kind gate first, `serve` would dial GitHub and execute an operator's
/// credential command on the way to refusing the source outright — work done on
/// behalf of something that can never serve a byte.
///
/// Two server-side oracles, because the two costs are independent and a fix for
/// one does not imply the other. The listener proves no egress: it stays live,
/// so accepting nothing is only possible if nothing dialled. The marker proves
/// no execution: the helper creates it when run, so its absence is positive
/// evidence the credential was never resolved rather than merely unused.
#[test]
fn an_unmountable_kind_is_refused_before_it_is_probed() {
    let temporary = TempDir::new().expect("temporary directory");
    // The helper is resolved through the command's own `PATH`, which is how
    // this profile schema locates credential commands; an absolute argv fails
    // to resolve, and a credential that never resolves would make both oracles
    // below pass for the wrong reason.
    let command_directory = temporary.path().join("command-bin");
    fs::create_dir(&command_directory).expect("command directory");
    create_executable(&command_directory.join("credential-helper"));
    let marker = temporary.path().join("credential-was-executed");

    let listener = TcpListener::bind("127.0.0.1:0").expect("origin listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let port = listener.local_addr().expect("listener address").port();

    let profile = write_profile(
        temporary.path(),
        "unmountable.json",
        &json!({
            "schemaVersion":1,
            "session":{"cacheDirectory":"unmountable-cache"},
            "sources":[{
                "kind":"github","id":"forge","required":false,
                "apiBaseUrl":format!("https://127.0.0.1:{port}/"),
                "allowPrivateNetwork":true,
                "credential":{
                    "kind":"command",
                    "command":{
                        "argv":["credential-helper"],
                        "environment":{
                            "PATH":{"kind":"literal","value":"command-bin"},
                            "RFS_CHECK_MARKER":{
                                "kind":"literal",
                                "value":marker.display().to_string()
                            }
                        }
                    }
                },
                "repositories":[{"name":"owner/repository"}]
            }]
        }),
    );

    let output = run_serve_profile(&profile, temporary.path(), &[]);
    assert_eq!(
        output.status.code(),
        Some(3),
        "an uncompiled kind still stops startup"
    );
    let diagnostic = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        diagnostic.contains("not supported"),
        "the refusal must name the missing adapter, not a reachability verdict; got: {diagnostic}"
    );
    assert_eq!(
        listener
            .accept()
            .expect_err("startup must not dial a source it cannot mount")
            .kind(),
        std::io::ErrorKind::WouldBlock,
        "no connection may be made on behalf of an unmountable source"
    );
    assert!(
        !marker.exists(),
        "the credential helper must not run for a source that cannot mount"
    );
}
