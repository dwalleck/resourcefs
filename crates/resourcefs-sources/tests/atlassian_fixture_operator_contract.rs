#![cfg(unix)]

use serde_json::{Value, json};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

const SITE: &str = "https://example.test";
const PROVISIONER_EMAIL: &str = "provisioner-canary@example.test";
const PROVISIONER_TOKEN: &str = "provisioner-token-canary";
const READER_EMAIL: &str = "reader-canary@example.test";
const READER_TOKEN: &str = "reader-token-canary";
const FIXTURE: &[u8] = include_bytes!("fixtures/atlassian_fixture_operator/fake_curl.py");
const HELPER_AUDIT: &[u8] = br#"#!/usr/bin/env python3
import json
import os
import pathlib
import sys

name = pathlib.Path(sys.argv[0]).name
argv_text = "\x00".join(sys.argv[1:])
credential_names = (
    "ATLASSIAN_PROVISIONER_EMAIL",
    "ATLASSIAN_PROVISIONER_API_TOKEN",
    "ATLASSIAN_READER_EMAIL",
    "ATLASSIAN_READER_API_TOKEN",
)
row = {
    "kind": "helper",
    "helper": name,
    "argv_canary": "canary" in argv_text.lower(),
    "credential_env_present": any(
        os.environ.get(key) for key in credential_names
    ),
}
audit = os.environ.get("FAKE_CURL_AUDIT")
if audit:
    with open(audit, "a", encoding="utf-8") as stream:
        stream.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")
real_path = None
for directory in os.environ.get("FAKE_CURL_REAL_PATH", "").split(os.pathsep):
    candidate = pathlib.Path(directory) / name
    if candidate.is_file() and os.access(candidate, os.X_OK):
        real_path = str(candidate)
        break
if real_path is None:
    raise SystemExit("real helper not found")
if name == "mv" and os.environ.get("FAKE_CURL_DENY_STATE") == sys.argv[-1]:
    raise SystemExit(1)
os.execv(real_path, [name, *sys.argv[1:]])
"#;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("resourcefs-sources is nested under crates/")
        .canonicalize()
        .expect("repository root must canonicalize")
}

fn script_path() -> PathBuf {
    repository_root()
        .join("scripts/atlassian-fixture-bootstrap.sh")
        .canonicalize()
        .expect("operator script must canonicalize")
}

fn checked_in_manifest() -> PathBuf {
    repository_root()
        .join("fixtures/atlassian-live/manifest.json")
        .canonicalize()
        .expect("checked-in manifest must canonicalize")
}

fn write_executable(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("write fake curl");
    let mut permissions = fs::metadata(path)
        .expect("read fake curl metadata")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).expect("make fake curl executable");
}
struct Harness {
    temp: TempDir,
    fake_bin: PathBuf,
    store: PathBuf,
    log: PathBuf,
    audit: PathBuf,
    state: PathBuf,
    audit_enabled: Cell<bool>,
}

impl Harness {
    fn new() -> Self {
        // Normal state parents must not inherit a symlink from the host's TMPDIR.
        let parent = std::env::temp_dir()
            .canonicalize()
            .expect("canonical harness parent");
        let temp = tempfile::tempdir_in(parent).expect("create harness directory");
        let fake_bin = temp.path().join("bin");
        fs::create_dir(&fake_bin).expect("create fake bin directory");
        // Keep the invoking environment's Bash even when a test isolates PATH.
        let bash = Command::new("bash")
            .args(["-c", "printf '%s' \"$BASH\""])
            .output()
            .expect("locate selected Bash interpreter");
        assert_success(&bash);
        let bash = PathBuf::from(OsString::from_vec(bash.stdout))
            .canonicalize()
            .expect("selected Bash interpreter must canonicalize");
        symlink(bash, fake_bin.join("bash")).expect("retain selected Bash in isolated PATH");
        write_executable(&fake_bin.join("curl"), FIXTURE);
        Self {
            store: temp.path().join("store.json"),
            log: temp.path().join("requests.ndjson"),
            audit: temp.path().join("helper-audit.ndjson"),
            state: temp.path().join("state.json"),
            temp,
            fake_bin,
            audit_enabled: Cell::new(false),
        }
    }

    fn enable_audit(&self) {
        for helper in [
            "chmod", "dirname", "env", "jq", "mkdir", "mktemp", "mv", "rm", "rmdir", "sleep",
            "stat", "wc",
        ] {
            write_executable(&self.fake_bin.join(helper), HELPER_AUDIT);
        }
        self.audit_enabled.set(true);
    }

    fn command(&self, mode: &str) -> Output {
        self.command_with(mode, SITE, None, None, "normal")
    }

    fn command_with(
        &self,
        mode: &str,
        site: &str,
        manifest: Option<&Path>,
        state: Option<&Path>,
        scenario: &str,
    ) -> Output {
        let original_path =
            std::env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
        let real_path = original_path.clone();
        let mut path = OsString::from(self.fake_bin.as_os_str());
        path.push(":");
        path.push(original_path);
        let manifest = manifest.map_or_else(checked_in_manifest, Path::to_path_buf);
        let state = state.map_or_else(|| self.state.clone(), Path::to_path_buf);
        let mut command = Command::new(script_path());
        command
            .current_dir(repository_root())
            .args([mode, "--site", site, "--manifest"])
            .arg(manifest)
            .arg("--state")
            .arg(&state)
            .env("PATH", path)
            .env("FAKE_CURL_STORE", &self.store)
            .env("FAKE_CURL_LOG", &self.log)
            .env("FAKE_CURL_SCENARIO", scenario)
            .env("ATLASSIAN_PROVISIONER_EMAIL", PROVISIONER_EMAIL)
            .env("ATLASSIAN_PROVISIONER_API_TOKEN", PROVISIONER_TOKEN)
            .env("ATLASSIAN_READER_EMAIL", READER_EMAIL)
            .env("ATLASSIAN_READER_API_TOKEN", READER_TOKEN);
        if scenario == "state_commit_denied" {
            write_executable(&self.fake_bin.join("mv"), HELPER_AUDIT);
            command
                .env("FAKE_CURL_DENY_STATE", &state)
                .env("FAKE_CURL_REAL_PATH", &real_path);
        }
        if self.audit_enabled.get() {
            command
                .env("FAKE_CURL_AUDIT", &self.audit)
                .env("FAKE_CURL_TEMP_ROOT", self.temp.path())
                .env("FAKE_CURL_REAL_PATH", real_path);
        }
        command.output().expect("invoke fixture operator")
    }

    fn direct_fake_request(&self, url: &str) -> Output {
        let user = serde_json::to_string(&format!("{PROVISIONER_EMAIL}:{PROVISIONER_TOKEN}"))
            .expect("encode fake user");
        let encoded_url = serde_json::to_string(url).expect("encode fake URL");
        let response = self.temp.path().join("direct-response");
        let encoded_response =
            serde_json::to_string(&response.to_string_lossy()).expect("encode response path");
        let mut child = Command::new("python3")
            .arg(self.fake_bin.join("curl"))
            .args(["--config", "-"])
            .env("FAKE_CURL_STORE", &self.store)
            .env("FAKE_CURL_LOG", &self.log)
            .env("FAKE_CURL_SCENARIO", "normal")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn direct fake curl");
        let config = format!(
            "request = \"GET\"\nurl = {encoded_url}\nuser = {user}\noutput = {encoded_response}\nwrite-out = \"%{{http_code}}\"\n"
        );
        child
            .stdin
            .take()
            .expect("fake stdin")
            .write_all(config.as_bytes())
            .expect("write fake config");
        child.wait_with_output().expect("wait for direct fake curl")
    }

    fn command_with_empty_state(&self, mode: &str) -> Output {
        let original_path =
            std::env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
        let mut path = OsString::from(self.fake_bin.as_os_str());
        path.push(":");
        path.push(original_path);
        Command::new(script_path())
            .current_dir(self.temp.path())
            .args([mode, "--site", SITE, "--manifest"])
            .arg(checked_in_manifest())
            .args(["--state", ""])
            .env("PATH", path)
            .env("FAKE_CURL_STORE", &self.store)
            .env("FAKE_CURL_LOG", &self.log)
            .env("FAKE_CURL_SCENARIO", "normal")
            .env("ATLASSIAN_PROVISIONER_EMAIL", PROVISIONER_EMAIL)
            .env("ATLASSIAN_PROVISIONER_API_TOKEN", PROVISIONER_TOKEN)
            .env("ATLASSIAN_READER_EMAIL", READER_EMAIL)
            .env("ATLASSIAN_READER_API_TOKEN", READER_TOKEN)
            .output()
            .expect("invoke empty-state operator")
    }

    fn read_log(&self) -> Vec<Value> {
        let Ok(bytes) = fs::read(&self.log) else {
            return Vec::new();
        };
        String::from_utf8(bytes)
            .expect("request log is UTF-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("request log row is JSON"))
            .collect()
    }
    fn read_audit(&self) -> Vec<Value> {
        let Ok(bytes) = fs::read(&self.audit) else {
            return Vec::new();
        };
        String::from_utf8(bytes)
            .expect("helper audit is UTF-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("helper audit row is JSON"))
            .collect()
    }

    fn read_store(&self) -> Value {
        serde_json::from_slice(&fs::read(&self.store).expect("fake store exists"))
            .expect("fake store is JSON")
    }
    fn read_state(&self) -> Value {
        serde_json::from_slice(&fs::read(&self.state).expect("state exists"))
            .expect("state is JSON")
    }

    fn write_store(&self, store: &Value) {
        fs::write(
            &self.store,
            serde_json::to_vec_pretty(store).expect("serialize fake store"),
        )
        .expect("seed fake store");
    }

    fn write_manifest(&self, value: &Value) -> PathBuf {
        let path = self.temp.path().join("manifest.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(value).expect("serialize manifest"),
        )
        .expect("write test manifest");
        path.canonicalize()
            .expect("manifest path must canonicalize")
    }

    fn clear_log(&self) {
        if self.log.exists() {
            fs::remove_file(&self.log).expect("clear request log");
        }
    }

    fn pending_path(&self) -> PathBuf {
        PathBuf::from(format!("{}.pending", self.state.display()))
    }
}

fn manifest() -> Value {
    serde_json::from_slice(&fs::read(checked_in_manifest()).expect("read checked-in manifest"))
        .expect("checked-in manifest is JSON")
}

fn operation_rows<'a>(rows: &'a [Value], operation: &str) -> Vec<&'a Value> {
    rows.iter()
        .filter(|row| row.get("operation").and_then(Value::as_str) == Some(operation))
        .collect()
}
fn assert_no_new_mutations(before: &[Value], after: &[Value]) {
    for operation in [
        "project_create",
        "issue_create",
        "comment_create",
        "space_create",
        "page_create",
        "owner_property_create",
        "jira_comment_delete",
        "jira_issue_delete",
        "jira_project_delete",
        "confluence_comment_delete",
        "confluence_page_delete",
        "space_delete",
    ] {
        assert_eq!(
            operation_rows(after, operation).len(),
            operation_rows(before, operation).len(),
            "repeated cleanup performed {operation}"
        );
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={:?} stderr={:?}",
        output.stdout,
        output.stderr
    );
}

fn assert_failure(output: &Output, token: &str) {
    assert!(
        !output.status.success(),
        "unexpected success: {:?}",
        output.stdout
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(token), "stderr {stderr:?} omitted {token}");
}

fn assert_no_secrets(bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for secret in [
        PROVISIONER_EMAIL,
        PROVISIONER_TOKEN,
        READER_EMAIL,
        READER_TOKEN,
    ] {
        assert!(
            !text.contains(secret),
            "secret escaped execution boundary: {secret}"
        );
    }
}

fn contains_null_id(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            object.get("id").is_some_and(Value::is_null) || object.values().any(contains_null_id)
        }
        Value::Array(values) => values.iter().any(contains_null_id),
        _ => false,
    }
}

fn create_count(rows: &[Value]) -> usize {
    [
        "project_create",
        "issue_create",
        "comment_create",
        "space_create",
        "page_create",
    ]
    .iter()
    .map(|operation| operation_rows(rows, operation).len())
    .sum()
}

fn assert_state_shape(state: &Value) {
    let object = state.as_object().expect("state object");
    let keys: BTreeSet<_> = object.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        BTreeSet::from(["confluence", "jira", "manifest", "site", "version"])
    );
    assert_eq!(state["version"], 1);
    assert_eq!(state["site"]["id"], "atlassian-live");
    assert_eq!(state["site"]["origin"], SITE);
    for (product, collections) in [
        ("jira", ["projects", "issues", "comments"]),
        ("confluence", ["spaces", "pages", "comments"]),
    ] {
        for collection in collections {
            for row in state[product][collection]
                .as_array()
                .expect("state collection")
            {
                let row_keys: BTreeSet<_> = row
                    .as_object()
                    .expect("state row")
                    .keys()
                    .map(String::as_str)
                    .collect();
                let expected = match (product, collection) {
                    ("jira", "projects") | ("confluence", "spaces") => {
                        BTreeSet::from(["id", "key", "logical_id"])
                    }
                    ("jira", "issues") => BTreeSet::from(["id", "key", "logical_id", "reference"]),
                    ("jira", "comments") => BTreeSet::from(["id", "issue_id", "logical_id"]),
                    ("confluence", "pages") => BTreeSet::from(["id", "logical_id", "reference"]),
                    ("confluence", "comments") => BTreeSet::from(["id", "logical_id", "page_id"]),
                    _ => unreachable!("known state collection"),
                };
                assert_eq!(row_keys, expected);
                assert!(
                    row["id"]
                        .as_str()
                        .is_some_and(|id| id.chars().all(|ch| ch.is_ascii_digit()))
                );
                if product == "jira" && collection == "issues" {
                    let id = row["id"].as_str().expect("issue id");
                    assert_eq!(
                        row["reference"],
                        format!("jira://atlassian-live/issues/{id}")
                    );
                }
                if product == "confluence" && collection == "pages" {
                    let id = row["id"].as_str().expect("page id");
                    assert_eq!(
                        row["reference"],
                        format!("confluence://atlassian-live/pages/{id}")
                    );
                }
            }
        }
    }
    let serialized = serde_json::to_string(state).expect("serialize state");
    for forbidden in [
        "email",
        "token",
        "Authorization",
        "timestamp",
        "nextPageToken",
        "cursor",
        "body",
        "summary",
        "title",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "mutable or secret state member {forbidden}"
        );
    }
}

#[test]
fn fake_rejects_non_https_example_origins_at_its_boundary() {
    for origin in [
        "http://example.test",
        "https://evil.example.test",
        "https://user:pass@example.test",
        "https://example.test:443",
        "https://example.test:8443",
    ] {
        let harness = Harness::new();
        let output = harness.direct_fake_request(&format!("{origin}/rest/api/3/myself"));
        assert!(
            !output.status.success(),
            "fake accepted invalid origin {origin}: stdout={:?} stderr={:?}",
            output.stdout,
            output.stderr
        );
        assert!(
            harness.read_log().is_empty(),
            "invalid origin {origin} reached fake request handler"
        );
    }
}

#[test]
fn operator_interface_is_strict_and_independent() {
    let harness = Harness::new();
    let strict_path = format!("{}:/usr/bin:/bin", harness.fake_bin.display());
    let help = Command::new(script_path())
        .current_dir(repository_root())
        .arg("--help")
        .env("PATH", &strict_path)
        .output()
        .expect("invoke help");
    assert_success(&help);
    let help_text = String::from_utf8_lossy(&help.stdout);
    for mode in ["bootstrap", "verify", "cleanup"] {
        assert!(help_text.contains(mode), "help omitted {mode}");
    }
    let invalid = Command::new(script_path())
        .current_dir(repository_root())
        .args(["apply", "--site", SITE])
        .env("PATH", &strict_path)
        .output()
        .expect("invoke invalid mode");
    assert_failure(&invalid, "Usage: scripts/atlassian-fixture-bootstrap.sh");
    assert!(harness.read_log().is_empty());
}

#[test]
fn invalid_manifest_never_reaches_egress() {
    for label in [
        "malformed",
        "empty",
        "singleton-project",
        "singleton-space",
        "duplicate-key",
        "bad-parent",
    ] {
        let harness = Harness::new();
        let path = if label == "malformed" {
            let path = harness.temp.path().join("malformed.json");
            fs::write(&path, b"{not-json").expect("write malformed manifest");
            path.canonicalize().expect("malformed path canonicalizes")
        } else {
            let mut value = manifest();
            match label {
                "empty" => value = json!({"version": 1, "jira": {}, "confluence": {}}),
                "singleton-project" => {
                    let first_project = value["jira"]["projects"][0].clone();
                    value["jira"]["projects"] = json!([first_project]);
                }
                "singleton-space" => {
                    let first_space = value["confluence"]["spaces"][0].clone();
                    value["confluence"]["spaces"] = json!([first_space]);
                }
                "duplicate-key" => {
                    value["jira"]["projects"][1]["key"] =
                        value["jira"]["projects"][0]["key"].clone();
                }
                "bad-parent" => value["jira"]["issues"][1]["parent"] = json!("missing-parent"),
                _ => unreachable!("malformed case handled above"),
            }
            harness.write_manifest(&value)
        };
        let output =
            harness.command_with("bootstrap", SITE, Some(&path), None, "transport_failure");
        assert_failure(&output, "invalid_manifest");
        assert!(harness.read_log().is_empty(), "{label} reached fake curl");
    }
}

#[test]
fn credentials_never_escape_execution_boundary() {
    let harness = Harness::new();
    harness.enable_audit();
    let bootstrap = harness.command("bootstrap");
    assert_success(&bootstrap);
    let verify = harness.command("verify");
    assert_success(&verify);
    let cleanup = harness.command("cleanup");
    assert_success(&cleanup);
    assert_no_secrets(&bootstrap.stdout);
    assert_no_secrets(&bootstrap.stderr);
    assert_no_secrets(&verify.stdout);
    assert_no_secrets(&verify.stderr);
    assert_no_secrets(&cleanup.stdout);
    assert_no_secrets(&cleanup.stderr);
    assert_no_secrets(&fs::read(&harness.log).expect("request log"));
    assert_no_secrets(&fs::read(&harness.store).expect("fake store"));
    assert_no_secrets(&fs::read(&harness.audit).expect("helper audit"));
    let audit = harness.read_audit();
    assert!(!audit.is_empty(), "helper audit observed no subprocesses");
    let mut helpers = BTreeSet::new();
    for row in &audit {
        assert_eq!(row["argv_canary"], false, "secret reached helper argv");
        assert_eq!(
            row["credential_env_present"], false,
            "secret reached helper environment"
        );
        if row["kind"] == "helper" {
            helpers.insert(row["helper"].as_str().expect("helper name"));
        } else {
            assert_eq!(row["kind"], "curl");
            assert_eq!(row["config_user_present"], true);
            assert_eq!(row["config_canary"], true);
            assert_eq!(row["credential_pair_valid"], true);
            assert_eq!(row["temp_canary"], false, "secret reached a temp file");
        }
    }
    for helper in [
        "chmod", "dirname", "env", "jq", "mkdir", "mktemp", "mv", "rm", "rmdir", "sleep", "stat",
        "wc",
    ] {
        assert!(helpers.contains(helper), "helper audit omitted {helper}");
    }
    for row in harness.read_log() {
        assert_eq!(row["config_user_present"], true);
        assert_eq!(row["argv_safe"], true);
        assert_eq!(row["credential_env_absent"], true);
        assert_eq!(row["credential_pair_valid"], true);
        assert!(
            row.get("user").is_none(),
            "credential-bearing config was logged"
        );
        assert!(
            row.get("argv").is_none(),
            "credential-bearing argv was logged"
        );
    }
}

#[test]
fn empty_state_argument_fails_before_any_request_in_every_mode() {
    for mode in ["bootstrap", "verify", "cleanup"] {
        let harness = Harness::new();
        let output = harness.command_with_empty_state(mode);
        assert!(
            !output.status.success(),
            "{mode} unexpectedly accepted empty --state: stdout={:?} stderr={:?}",
            output.stdout,
            output.stderr
        );
        assert!(
            harness.read_log().is_empty(),
            "{mode} reached egress with empty --state"
        );
    }
}

#[test]
fn bootstrap_is_idempotent_and_collision_safe() {
    let harness = Harness::new();
    assert_success(&harness.command("bootstrap"));
    let first = harness.read_log();
    assert_success(&harness.command("bootstrap"));
    let second = harness.read_log();
    assert_no_new_mutations(&first, &second);

    let foreign = Harness::new();
    foreign.write_store(&json!({
        "projects": [{
            "id": "9000000001",
            "key": "RFSFIX",
            "name": "Foreign",
            "description": "tenant-owned foreign object"
        }]
    }));
    assert_failure(&foreign.command("bootstrap"), "foreign_collision");
    assert!(operation_rows(&foreign.read_log(), "project_create").is_empty());

    let duplicate = Harness::new();
    duplicate.write_store(&json!({
        "projects": [{
            "id": "9000000003",
            "key": "OTHERKEY",
            "name": "Foreign",
            "description": "ResourceFS disposable fixture / rfs-bcym / jira primary"
        }]
    }));
    assert_failure(&duplicate.command("bootstrap"), "foreign_collision");
    assert!(operation_rows(&duplicate.read_log(), "project_create").is_empty());

    let drift = Harness::new();
    assert_success(&drift.command("bootstrap"));
    let before = drift.read_store()["projects"][0].clone();
    assert_success(&drift.command_with("bootstrap", SITE, None, None, "drift"));
    let store = drift.read_store();
    let after = store["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .find(|row| row["key"] == before["key"])
        .expect("reconciled project");
    assert_ne!(after["id"], before["id"]);
    assert_eq!(after["name"], before["name"]);
    assert_eq!(after["description"], before["description"]);

    let embedded_marker = Harness::new();
    assert_success(&embedded_marker.command_with(
        "bootstrap",
        SITE,
        None,
        None,
        "embedded_marker_foreign_issue",
    ));
    assert!(
        embedded_marker.read_store()["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .any(|issue| issue["id"] == "9000000004"),
        "ADF metadata containing an ownership token was deleted"
    );
    assert!(
        operation_rows(&embedded_marker.read_log(), "jira_issue_delete").is_empty(),
        "ADF metadata was treated as fixture ownership"
    );
    assert!(
        embedded_marker.read_store()["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .any(|page| page["id"] == "9000000005"),
        "non-leading HTML ownership token was treated as fixture ownership"
    );
    assert!(
        operation_rows(&embedded_marker.read_log(), "confluence_page_delete").is_empty(),
        "non-leading HTML ownership token caused deletion"
    );

    let foreign_page = Harness::new();
    assert_failure(
        &foreign_page.command_with("bootstrap", SITE, None, None, "foreign_page_property"),
        "foreign_collision",
    );
    assert!(operation_rows(&foreign_page.read_log(), "page_create").is_empty());
    assert!(
        foreign_page.read_store()["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .any(|page| page["id"] == "9000000005"),
        "foreign page with mismatched owner property was deleted"
    );

    let unowned_page = Harness::new();
    assert_failure(
        &unowned_page.command_with("bootstrap", SITE, None, None, "page_missing_property"),
        "foreign_collision",
    );
    assert!(operation_rows(&unowned_page.read_log(), "page_create").is_empty());

    let mismatched_identity = Harness::new();
    assert_failure(
        &mismatched_identity.command_with(
            "bootstrap",
            SITE,
            None,
            None,
            "mismatched_create_identity",
        ),
        "upstream_failure",
    );
    assert!(!mismatched_identity.state.exists());
}

#[test]
fn state_commit_is_atomic_and_minimal() {
    let harness = Harness::new();
    fs::write(&harness.state, br#"{"version":1,"sentinel":"keep"}"#).expect("write sentinel state");
    let mut permissions = fs::metadata(&harness.state)
        .expect("state metadata")
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&harness.state, permissions).expect("protect sentinel state");
    let failed = harness.command_with("bootstrap", SITE, None, None, "raw400");
    assert_failure(&failed, "upstream_failure");
    assert_eq!(
        fs::read_to_string(&harness.state).expect("preserved state"),
        r#"{"version":1,"sentinel":"keep"}"#
    );
    assert_eq!(
        fs::metadata(&harness.state)
            .expect("state metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        !fs::read_dir(harness.temp.path())
            .expect("harness directory")
            .any(|entry| {
                entry
                    .expect("temporary directory entry")
                    .file_name()
                    .to_string_lossy()
                    .contains("state.tmp")
            })
    );
    let denied = Harness::new();
    fs::write(&denied.state, br#"{"version":1,"sentinel":"keep"}"#)
        .expect("write commit-failure sentinel");
    let denied_output = denied.command_with("bootstrap", SITE, None, None, "state_commit_denied");
    assert_failure(&denied_output, "state_commit");
    assert_eq!(
        fs::read_to_string(&denied.state).expect("preserved commit-failure state"),
        r#"{"version":1,"sentinel":"keep"}"#
    );

    assert_success(&harness.command("bootstrap"));
    let state = harness.read_state();
    assert_state_shape(&state);
    assert_eq!(
        fs::metadata(&harness.state)
            .expect("state metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let symlinked = Harness::new();
    let target = symlinked.temp.path().join("target-state.json");
    fs::write(&target, b"keep").expect("write symlink target");
    symlink(&target, &symlinked.state).expect("create state symlink");
    assert_failure(&symlinked.command("bootstrap"), "state_symlink");
    assert!(symlinked.read_log().is_empty());
    let ancestor = Harness::new();
    let target_directory = ancestor.temp.path().join("state-target");
    fs::create_dir(&target_directory).expect("create symlink target directory");
    let linked_directory = ancestor.temp.path().join("state-link");
    symlink(&target_directory, &linked_directory).expect("create ancestor symlink");
    let nested_state = linked_directory.join("nested/state.json");
    assert_failure(
        &ancestor.command_with("bootstrap", SITE, None, Some(&nested_state), "normal"),
        "state_parent_symlink",
    );
    assert!(!target_directory.join("nested").exists());
    assert!(ancestor.read_log().is_empty());

    let locked = Harness::new();
    fs::create_dir(locked.state.with_extension("json.lock")).expect("create lock directory");
    assert_failure(&locked.command("bootstrap"), "lock_busy");
    assert!(locked.read_log().is_empty());
}

#[test]
fn verify_enforces_reader_visibility_boundary() {
    let private_visible = Harness::new();
    assert_success(&private_visible.command("bootstrap"));
    assert_failure(
        &private_visible.command_with("verify", SITE, None, None, "reader_private_visible"),
        "reader_visibility",
    );

    let public_hidden = Harness::new();
    assert_success(&public_hidden.command("bootstrap"));
    assert_failure(
        &public_hidden.command_with("verify", SITE, None, None, "reader_public_hidden"),
        "upstream_failure",
    );
}

#[test]
fn cleanup_waits_for_owned_space_deletion() {
    let harness = Harness::new();
    assert_success(&harness.command("bootstrap"));
    assert_success(&harness.command("cleanup"));
    let after_first = harness.read_log();
    assert!(
        operation_rows(&after_first, "space_delete")
            .iter()
            .any(|row| row["status"] == 202)
    );
    assert!(
        operation_rows(&after_first, "space_delete_poll")
            .iter()
            .any(|row| row["status"] == 200)
    );
    let store = harness.read_store();
    for collection in [
        "projects",
        "issues",
        "jira_comments",
        "spaces",
        "pages",
        "confluence_comments",
    ] {
        assert!(
            store[collection]
                .as_array()
                .expect("store collection")
                .is_empty(),
            "owned graph remained in {collection}"
        );
    }
    assert!(!harness.state.exists());
    assert_success(&harness.command("cleanup"));
    assert_no_new_mutations(&after_first, &harness.read_log());

    // Some project templates deny DELETE_ISSUES to the project lead; cleanup
    // must delegate those deletes to the project cascade and still converge.
    let denied = Harness::new();
    assert_success(&denied.command("bootstrap"));
    assert_success(&denied.command_with("cleanup", SITE, None, None, "jira_delete_denied"));
    let denied_store = denied.read_store();
    assert!(
        denied_store["issues"]
            .as_array()
            .expect("issues")
            .is_empty(),
        "denied issue deletes left residue after project cascade"
    );
    assert!(
        denied_store["projects"]
            .as_array()
            .expect("projects")
            .is_empty()
    );
}

#[test]
fn verify_forces_each_pagination_family() {
    let harness = Harness::new();
    assert_success(&harness.command("bootstrap"));
    assert_success(&harness.command("verify"));
    let rows = harness.read_log();
    for operation in [
        "project_search",
        "issue_search",
        "comment_list",
        "space_list",
        "page_list",
    ] {
        assert!(
            operation_rows(&rows, operation).len() >= 2,
            "{operation} did not cross a page boundary"
        );
    }
    assert!(
        rows.iter()
            .any(|row| row["query"]["startAt"] == json!(["1"]))
    );
    assert!(
        rows.iter()
            .any(|row| row["body"]["nextPageToken"].is_string())
    );
    assert!(rows.iter().any(|row| row["query"]["cursor"].is_array()));

    let malformed = Harness::new();
    assert_success(&malformed.command("bootstrap"));
    assert_failure(
        &malformed.command_with("verify", SITE, None, None, "malformed_paging"),
        "invalid_pagination",
    );
}

#[test]
fn failures_are_bounded_and_redacted() {
    for scenario in ["raw400", "partial_once"] {
        let harness = Harness::new();
        let output = harness.command_with("bootstrap", SITE, None, None, scenario);
        assert_failure(&output, "upstream_failure");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.len() + stderr.len() < 512,
            "diagnostic is not bounded for {scenario}"
        );
        for canary in ["RAW400_FAILURE_CANARY", "FAILURE_RESPONSE_CANARY"] {
            assert!(!stdout.contains(canary), "stdout leaked {canary}");
            assert!(!stderr.contains(canary), "stderr leaked {canary}");
        }
        assert_no_secrets(&output.stdout);
        assert_no_secrets(&output.stderr);
    }

    let harness = Harness::new();
    let invalid_site =
        harness.command_with("bootstrap", "http://example.test", None, None, "normal");
    assert_failure(&invalid_site, "invalid_site");
    assert!(harness.read_log().is_empty());
}

#[test]
fn owner_property_write_failures_recover_without_duplicate_objects() {
    for scenario in [
        "owner_property_failure_before",
        "owner_property_failure_after",
    ] {
        let harness = Harness::new();
        let failed = harness.command_with("bootstrap", SITE, None, None, scenario);
        assert_failure(&failed, "upstream_failure");
        let pending = harness.pending_path();
        assert!(
            pending.exists(),
            "owner-property failure lost recovery receipt"
        );
        assert_no_secrets(&fs::read(&pending).expect("read recovery receipt"));
        let first = harness.read_log();
        assert_eq!(operation_rows(&first, "page_create").len(), 1);

        assert_success(&harness.command("bootstrap"));
        assert!(!pending.exists(), "successful recovery retained receipt");
        let recovered = harness.read_log();
        assert_eq!(operation_rows(&recovered, "page_create").len(), 3);
        assert_success(&harness.command("cleanup"));
        assert!(!harness.state.exists());
        assert!(!pending.exists());
    }
}

#[test]
fn cleanup_recovers_known_owner_property_receipt_after_failed_bootstrap() {
    let harness = Harness::new();
    let failed = harness.command_with(
        "bootstrap",
        SITE,
        None,
        None,
        "owner_property_failure_before",
    );
    assert_failure(&failed, "upstream_failure");
    assert!(harness.pending_path().exists());
    assert_success(&harness.command("cleanup"));
    assert!(!harness.state.exists());
    assert!(!harness.pending_path().exists());
    let store = harness.read_store();
    for collection in [
        "projects",
        "issues",
        "jira_comments",
        "spaces",
        "pages",
        "confluence_comments",
    ] {
        assert!(
            store[collection]
                .as_array()
                .expect("store collection")
                .is_empty()
        );
    }
}

#[test]
fn unknown_page_create_preserves_null_receipt_and_blocks_new_creates() {
    let harness = Harness::new();
    let failed = harness.command_with("bootstrap", SITE, None, None, "unknown_page_create");
    assert!(!failed.status.success());
    let pending = harness.pending_path();
    assert!(pending.exists(), "unknown create lost pending receipt");
    let receipt: Value =
        serde_json::from_slice(&fs::read(&pending).expect("read receipt")).expect("receipt JSON");
    assert!(
        contains_null_id(&receipt),
        "unknown create receipt had no null id"
    );
    let first = harness.read_log();
    let first_creates = create_count(&first);

    let retry = harness.command("bootstrap");
    assert!(!retry.status.success());
    let after_retry = harness.read_log();
    assert_eq!(create_count(&after_retry), first_creates);
    let cleanup = harness.command("cleanup");
    assert!(!cleanup.status.success());
    assert_eq!(create_count(&harness.read_log()), first_creates);
    assert!(harness.pending_path().exists());
}

#[test]
fn nonroot_reply_property_receipt_resumes_without_duplicate_comment() {
    let harness = Harness::new();
    let mut custom = manifest();
    custom["confluence"]["comments"] = json!([
        {
            "id": "reply-root",
            "page": "public-root-page",
            "marker": "<!--rfs-owner:rfs-bcym:confluence-comment:reply-root-->",
            "body": {"representation": "storage", "value": "<p>Reply root.</p>"}
        },
        {
            "id": "reply-child",
            "page": "public-root-page",
            "parent": "reply-root",
            "marker": "<!--rfs-owner:rfs-bcym:confluence-comment:reply-child-->",
            "body": {"representation": "storage", "value": "<p>Reply child.</p>"}
        }
    ]);
    let manifest_path = harness.write_manifest(&custom);
    let failed = harness.command_with(
        "bootstrap",
        SITE,
        Some(&manifest_path),
        None,
        "owner_property_failure_reply_before",
    );
    assert_failure(&failed, "upstream_failure");
    assert!(harness.pending_path().exists());
    let before = harness.read_log();
    assert_eq!(
        harness.read_store()["confluence_comments"]
            .as_array()
            .expect("comments")
            .len(),
        2
    );

    assert_success(&harness.command_with("bootstrap", SITE, Some(&manifest_path), None, "normal"));
    assert_eq!(
        operation_rows(&harness.read_log(), "comment_create").len(),
        operation_rows(&before, "comment_create").len(),
        "reply recovery created a duplicate comment"
    );
    assert!(!harness.pending_path().exists());
}

#[test]
fn tampered_receipt_cannot_claim_foreign_content() {
    let harness = Harness::new();
    let failed = harness.command_with(
        "bootstrap",
        SITE,
        None,
        None,
        "owner_property_failure_before",
    );
    assert_failure(&failed, "upstream_failure");
    let pending = harness.pending_path();
    let original = fs::read(&pending).expect("read recovery receipt");
    let mut receipt: Value = serde_json::from_slice(&original).expect("receipt JSON");
    let mut store = harness.read_store();
    let mut foreign_page = store["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .find(|page| page["id"] == receipt["id"])
        .expect("created page")
        .clone();
    foreign_page["id"] = json!("9100000010");
    foreign_page["space"] = json!("9100000009");
    foreign_page["spaceId"] = json!("9100000009");
    foreign_page["parentId"] = json!("9100000011");
    store["spaces"].as_array_mut().expect("spaces").push(json!({
        "id": "9100000009",
        "key": "FOREIGNSPACE",
        "name": "Foreign space",
        "description": "foreign container",
        "private": false,
        "homepageId": 9_100_000_011_u64
    }));
    store["pages"]
        .as_array_mut()
        .expect("pages")
        .push(foreign_page.clone());
    harness.write_store(&store);
    receipt["id"] = foreign_page["id"].clone();
    receipt["container_id"] = foreign_page["spaceId"].clone();
    let tampered = serde_json::to_vec(&receipt).expect("serialize receipt");
    fs::write(&pending, tampered).expect("tamper recovery receipt");
    harness.clear_log();

    let retry = harness.command("bootstrap");
    assert!(
        !retry.status.success(),
        "tampered receipt unexpectedly resumed"
    );
    assert_no_new_mutations(&[], &harness.read_log());
    let after = harness.read_store();
    assert_eq!(
        after["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .find(|page| page["id"] == foreign_page["id"]),
        Some(&foreign_page),
        "recovery changed foreign content"
    );
    assert!(harness.pending_path().exists());
    assert_eq!(
        operation_rows(&harness.read_log(), "owner_property_create").len(),
        0
    );
}

#[test]
fn dangling_pending_receipt_symlink_is_rejected_before_egress() {
    let harness = Harness::new();
    let failed = harness.command_with(
        "bootstrap",
        SITE,
        None,
        None,
        "owner_property_failure_before",
    );
    assert_failure(&failed, "upstream_failure");
    let pending = harness.pending_path();
    fs::remove_file(&pending).expect("remove original receipt");
    let target = harness.temp.path().join("missing-receipt-target");
    symlink(&target, &pending).expect("create dangling receipt symlink");
    harness.clear_log();

    let retry = harness.command("bootstrap");
    assert!(!retry.status.success());
    assert!(harness.read_log().is_empty());
    assert!(
        fs::symlink_metadata(&pending)
            .expect("pending symlink")
            .file_type()
            .is_symlink()
    );
    assert!(!target.exists());
}

#[test]
fn cleanup_purges_owned_tombstones_and_preserves_foreign_projects() {
    let harness = Harness::new();
    let mut projects: Vec<_> = manifest()["jira"]["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .enumerate()
        .map(|(index, row)| json!({
            "id": (9_200_000_000_u64 + u64::try_from(index).expect("fixture index")).to_string(),
            "key": row["key"], "name": row["name"],
            "description": row["marker"], "deleted": true
        }))
        .collect();
    let foreign = json!({
        "id": "9300000000", "key": "FOREIGN", "name": "Foreign project",
        "description": "tenant-owned foreign object", "deleted": true
    });
    projects.push(foreign.clone());
    harness.write_store(&json!({"projects": projects}));
    assert_success(&harness.command("cleanup"));
    assert_eq!(harness.read_store()["projects"], json!([foreign]));
    assert_success(&harness.command("bootstrap"));
    assert_success(&harness.command("cleanup"));

    let changed = Harness::new();
    assert_success(&changed.command("bootstrap"));
    let before = changed.read_log();
    let mut store = changed.read_store();
    store["projects"][0]["key"] = json!("FOREIGN");
    store["projects"][0]["description"] = json!("tenant-owned foreign object");
    store["projects"][0]["deleted"] = json!(true);
    changed.write_store(&store);
    assert_failure(&changed.command("cleanup"), "foreign_collision");
    assert_no_new_mutations(&before, &changed.read_log());
    assert!(changed.state.exists());
}

#[test]
fn bootstrap_refuses_changed_project_ownership() {
    let harness = Harness::new();
    assert_success(&harness.command("bootstrap"));
    let before = harness.read_log();
    assert_failure(
        &harness.command_with("bootstrap", SITE, None, None, "foreign_project_readback"),
        "foreign_collision",
    );
    assert_no_new_mutations(&before, &harness.read_log());
    assert_eq!(
        harness.read_store()["projects"][0]["description"],
        "tenant-owned foreign object"
    );
}

#[test]
fn cleanup_revalidates_saved_ids_before_deletion() {
    let moved = Harness::new();
    assert_success(&moved.command("bootstrap"));
    let state = moved.read_state();
    let issue_id = state["jira"]["issues"][0]["id"].clone();
    let page_id = state["confluence"]["pages"][0]["id"].clone();
    let mut store = moved.read_store();
    store["projects"]
        .as_array_mut()
        .expect("projects")
        .push(json!({
            "id": "9100000001",
            "key": "FOREIGN",
            "name": "Foreign project",
            "description": "foreign container"
        }));
    store["spaces"].as_array_mut().expect("spaces").push(json!({
        "id": "9100000002",
        "key": "FOREIGNSPACE",
        "name": "Foreign space",
        "description": "foreign container",
        "private": false,
        "homepageId": 9_100_000_003_u64
    }));
    for issue in store["issues"].as_array_mut().expect("issues") {
        if issue["id"] == issue_id {
            issue["project"] = json!("FOREIGN");
            issue["fields"]["project"]["key"] = json!("FOREIGN");
        }
    }
    for page in store["pages"].as_array_mut().expect("pages") {
        if page["id"] == page_id {
            page["space"] = json!("9100000002");
            page["spaceId"] = json!("9100000002");
            page["parentId"] = json!("9100000003");
        }
    }
    moved.write_store(&store);
    assert_success(&moved.command("cleanup"));
    assert!(!moved.state.exists(), "successful cleanup retained state");
    let residue = moved.read_store();
    assert!(
        residue["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .all(|row| row["id"] != issue_id),
        "moved issue survived successful cleanup"
    );
    assert!(
        residue["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .all(|row| row["id"] != page_id),
        "moved page survived successful cleanup"
    );
    assert!(
        residue["projects"]
            .as_array()
            .expect("projects")
            .iter()
            .any(|row| row["id"] == "9100000001")
    );
    assert!(
        residue["spaces"]
            .as_array()
            .expect("spaces")
            .iter()
            .any(|row| row["id"] == "9100000002")
    );

    let changed = Harness::new();
    assert_success(&changed.command("bootstrap"));
    let changed_state = changed.read_state();
    let changed_page_id = changed_state["confluence"]["pages"][0]["id"]
        .as_str()
        .expect("page id");
    let mut changed_store = changed.read_store();
    changed_store["properties"][changed_page_id]["rfs-owner"] = json!("changed-marker");
    changed.write_store(&changed_store);
    let changed_failure = changed.command("cleanup");
    assert!(!changed_failure.status.success());
    assert!(
        changed.state.exists(),
        "changed marker was treated as owned"
    );
    assert!(
        changed.read_store()["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .any(|row| { row["id"].as_str() == Some(changed_page_id) })
    );
}

#[test]
fn footer_comment_replies_use_children_pagination_and_converge() {
    let harness = Harness::new();
    let mut custom = manifest();
    custom["confluence"]["comments"] = json!([
        {
            "id": "reply-root",
            "page": "public-root-page",
            "marker": "<!--rfs-owner:rfs-bcym:confluence-comment:reply-root-->",
            "body": {"representation": "storage", "value": "<p>Reply root.</p>"}
        },
        {
            "id": "reply-child-one",
            "page": "public-root-page",
            "parent": "reply-root",
            "marker": "<!--rfs-owner:rfs-bcym:confluence-comment:reply-child-one-->",
            "body": {"representation": "storage", "value": "<p>Reply one.</p>"}
        },
        {
            "id": "reply-child-two",
            "page": "public-root-page",
            "parent": "reply-root",
            "marker": "<!--rfs-owner:rfs-bcym:confluence-comment:reply-child-two-->",
            "body": {"representation": "storage", "value": "<p>Reply two.</p>"}
        }
    ]);
    let manifest_path = harness.write_manifest(&custom);
    assert_success(&harness.command_with("bootstrap", SITE, Some(&manifest_path), None, "normal"));
    let first = harness.read_log();
    assert_eq!(
        harness.read_store()["confluence_comments"]
            .as_array()
            .expect("comments")
            .len(),
        3
    );
    assert_success(&harness.command_with("verify", SITE, Some(&manifest_path), None, "normal"));
    assert_success(&harness.command_with("bootstrap", SITE, Some(&manifest_path), None, "normal"));
    let second = harness.read_log();
    assert_no_new_mutations(&first, &second);
    assert!(
        second.iter().any(|row| row["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("/children"))
            && row["query"]["cursor"]
                .as_array()
                .is_some_and(|values| !values.is_empty())),
        "reply enumeration did not request a continuation"
    );
    assert_success(&harness.command_with("cleanup", SITE, Some(&manifest_path), None, "normal"));
    assert!(
        harness.read_store()["confluence_comments"]
            .as_array()
            .expect("comments")
            .is_empty()
    );
}

#[test]
fn operator_contract_covers_lifecycle_matrix() {
    let harness = Harness::new();
    assert_success(&harness.command("bootstrap"));
    let first = harness.read_log();
    assert_success(&harness.command("bootstrap"));
    assert_success(&harness.command("verify"));
    assert_success(&harness.command("cleanup"));
    let after_cleanup = harness.read_log();
    assert_success(&harness.command_with("cleanup", SITE, None, None, "second_cleanup"));
    let after_second_cleanup = harness.read_log();
    assert!(
        after_second_cleanup
            .iter()
            .any(|row| row["scenario"] == "second_cleanup"),
        "lifecycle omitted second cleanup"
    );
    assert_no_new_mutations(&after_cleanup, &after_second_cleanup);
    let operations: BTreeSet<_> = first
        .iter()
        .filter_map(|row| row["operation"].as_str())
        .collect();
    for operation in [
        "account_get",
        "issue_types",
        "project_create",
        "issue_create",
        "comment_create",
        "space_create",
        "page_create",
    ] {
        assert!(
            operations.contains(operation),
            "lifecycle omitted {operation}"
        );
    }
    assert!(operation_rows(&first, "issue_types").iter().all(|row| {
        row["path"]
            .as_str()
            .is_some_and(|path| path.contains("/createmeta/") && path.contains("issuetypes"))
    }));
    assert!(
        first
            .iter()
            .filter(|row| row["operation"] == "project_create")
            .all(|row| {
                row["body"]["projectTypeKey"] == "business"
                    && row["body"]["projectTemplateKey"]
                        == "com.atlassian.jira-core-project-templates:jira-core-simplified-project-management"
                    && row["body"]["leadAccountId"] == "fixture-provisioner-account"
            })
    );
    assert!(
        first
            .iter()
            .filter(|row| row["operation"] == "space_create")
            .all(|row| {
                row["body"]["description"]["plain"]["representation"] == "plain"
                    && row["body"]["description"]["plain"]["value"].is_string()
            })
    );
    for row in first
        .iter()
        .filter(|row| row["operation"] == "issue_create")
    {
        let expected = if row["body"]["fields"]["parent"]["id"].is_string() {
            "10002"
        } else {
            "10001"
        };
        assert_eq!(row["body"]["fields"]["issuetype"]["id"], expected);
    }
    assert!(
        first
            .iter()
            .any(|row| row["body"]["fields"]["parent"]["id"].is_string())
    );
    assert!(first.iter().any(|row| row["body"]["parentId"].is_string()));
    assert!(
        first
            .iter()
            .any(|row| row["path"] == "/wiki/rest/api/space/_private")
    );
    let property_rows = operation_rows(&first, "owner_property_create");
    assert_eq!(
        property_rows.len(),
        5,
        "ownership properties not written for every page and comment"
    );
    assert!(
        property_rows.iter().all(|row| {
            row["path"].as_str().is_some_and(|path| {
                path.starts_with("/wiki/rest/api/content/") && path.ends_with("/property")
            }) && row["body"]["key"] == "rfs-owner"
                && row["body"]["value"]
                    .as_str()
                    .is_some_and(|value| value.starts_with("<!--rfs-owner:"))
        }),
        "ownership properties must ride in the rfs-owner content property"
    );

    let partial = Harness::new();
    assert_failure(
        &partial.command_with("bootstrap", SITE, None, None, "partial_once"),
        "upstream_failure",
    );
    assert!(!partial.state.exists());
    assert_success(&partial.command_with("bootstrap", SITE, None, None, "partial_once"));

    let foreign_space = Harness::new();
    assert_failure(
        &foreign_space.command_with("bootstrap", SITE, None, None, "foreign_space_collision"),
        "foreign_collision",
    );
    assert!(operation_rows(&foreign_space.read_log(), "space_create").is_empty());

    let malformed_authority = Harness::new();
    for site in [
        "https://user:pass@example.test",
        "https://example.test/path",
        "https://example..test",
        "https://π.example",
    ] {
        assert_failure(
            &malformed_authority.command_with("bootstrap", site, None, None, "normal"),
            "invalid_site",
        );
    }
    assert!(malformed_authority.read_log().is_empty());
}

fn assert_graph_absent(harness: &Harness) {
    let store = harness.read_store();
    for collection in [
        "projects",
        "issues",
        "jira_comments",
        "spaces",
        "pages",
        "confluence_comments",
    ] {
        assert_eq!(store[collection], json!([]), "residue in {collection}");
    }
    assert!(!harness.state.exists(), "complete absence retained receipt");
}

fn assert_cleanup_refused(harness: &Harness) {
    let receipt = fs::read(&harness.state).expect("receipt before cleanup");
    let output = harness.command("cleanup");
    assert!(
        !output.status.success(),
        "uncertain cleanup succeeded: {:?}",
        output.stdout
    );
    assert_eq!(
        fs::read(&harness.state).expect("uncertain cleanup retained receipt"),
        receipt,
        "uncertain cleanup changed receipt"
    );
    assert_no_secrets(&output.stdout);
    assert_no_secrets(&output.stderr);
}

#[test]
fn cleanup_refuses_unproven_trashed_page_ownership() {
    for mode in [
        "foreign_newline",
        "foreign_nul",
        "missing",
        "foreign",
        "value_object",
        "malformed",
        "scalar",
        "duplicate",
        "duplicate_later",
        "wrong_key",
        "404",
        "loop",
        "untrusted",
        "limit",
    ] {
        let harness = Harness::new();
        assert_success(&harness.command("bootstrap"));
        let mut store = harness.read_store();
        let id = store["pages"][0]["id"]
            .as_str()
            .expect("page id")
            .to_owned();
        let marker = store["properties"][&id]["rfs-owner"].clone();
        store["pages"][0]["status"] = json!("trashed");
        match mode {
            "missing" => {
                store["properties"][&id]
                    .as_object_mut()
                    .expect("properties")
                    .remove("rfs-owner");
            }
            "foreign" => store["properties"][&id]["rfs-owner"] = json!("foreign"),
            "foreign_newline" => {
                store["properties"][&id]["rfs-owner"] =
                    json!(format!("{}\n", marker.as_str().expect("marker")));
            }
            "foreign_nul" => {
                store["properties"][&id]["rfs-owner"] =
                    json!(format!("{}\0", marker.as_str().expect("marker")));
            }
            "value_object" => store["properties"][&id]["rfs-owner"] = json!({"marker": marker}),
            _ => store["property_fault"] = json!({"id": id, "mode": mode}),
        }
        harness.write_store(&store);
        harness.clear_log();
        assert_cleanup_refused(&harness);
        let residue = harness.read_store();
        assert!(
            residue["pages"]
                .as_array()
                .expect("pages")
                .iter()
                .any(|row| row["id"] == id),
            "{mode} authorized page deletion"
        );
        assert!(
            harness
                .read_log()
                .iter()
                .all(|row| row["method"] != "DELETE")
        );
        assert!(harness.read_log().len() <= 512, "request budget exceeded");
        if mode == "limit" {
            assert!(operation_rows(&harness.read_log(), "page_properties").len() <= 10);
        }
        // Same identity and provider graph, with only ownership evidence repaired.
        let mut repaired = residue;
        repaired
            .as_object_mut()
            .expect("store")
            .remove("property_fault");
        repaired["properties"][&id]["rfs-owner"] = marker;
        harness.write_store(&repaired);
        assert_success(&harness.command("cleanup"));
        assert_graph_absent(&harness);
    }
}

#[test]
fn cleanup_resumes_trashed_pages_without_deleting_foreign_space() {
    for status in ["current", "trashed"] {
        let harness = Harness::new();
        assert_success(&harness.command("bootstrap"));
        let mut store = harness.read_store();
        let foreign = json!({
            "id": "9100000002", "key": "FOREIGNSPACE", "name": "Foreign",
            "description": "foreign container", "private": false,
            "homepageId": 9_100_000_003_u64
        });
        store["spaces"]
            .as_array_mut()
            .expect("spaces")
            .push(foreign.clone());
        store["pages"][0]["space"] = foreign["id"].clone();
        store["pages"][0]["spaceId"] = foreign["id"].clone();
        store["pages"][0]["status"] = json!(status);
        let id = store["pages"][0]["id"].clone();
        // A valid marker followed by an empty terminal property page is owned.
        store["property_fault"] = json!({"id": id, "mode": "continued"});
        harness.write_store(&store);
        harness.clear_log();
        assert_success(&harness.command("cleanup"));
        let residue = harness.read_store();
        assert_eq!(residue["pages"], json!([]), "{status} page was not purged");
        assert_eq!(residue["spaces"], json!([foreign]));
        assert!(!harness.state.exists());
        assert!(harness.read_log().len() <= 512);
    }
}

#[test]
fn cleanup_resumes_after_space_delete_interruption() {
    let harness = Harness::new();
    assert_success(&harness.command("bootstrap"));
    let mut store = harness.read_store();
    store["interrupt"] = json!("page_trash");
    harness.write_store(&store);
    assert_cleanup_refused(&harness);
    let mut trashed = harness.read_store();
    assert_eq!(trashed["faults"]["page_trash"], true);
    assert!(
        trashed["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .any(|row| row["status"] == "trashed")
    );
    trashed["interrupt"] = json!("space_complete");
    harness.write_store(&trashed);
    assert_cleanup_refused(&harness);
    let interrupted = harness.read_store();
    assert_eq!(interrupted["faults"]["space_complete"], true);
    assert_eq!(interrupted["spaces"].as_array().expect("spaces").len(), 1);
    assert_success(&harness.command("cleanup"));
    assert_graph_absent(&harness);
}

#[test]
fn cleanup_distinguishes_absent_parent_from_collection_failure() {
    for family in ["pages", "footer-comments", "children"] {
        for absent in [false, true] {
            let harness = Harness::new();
            let mut custom = manifest();
            custom["confluence"]["comments"][1]["parent"] =
                custom["confluence"]["comments"][0]["id"].clone();
            let manifest_path = harness.write_manifest(&custom);
            let command =
                |mode| harness.command_with(mode, SITE, Some(&manifest_path), None, "normal");
            assert_success(&command("bootstrap"));
            let mut store = harness.read_store();
            let (collection, id, path) = match family {
                "pages" => {
                    let id = store["spaces"][0]["id"].as_str().expect("space").to_owned();
                    (
                        "spaces",
                        id.clone(),
                        format!("/wiki/api/v2/spaces/{id}/pages"),
                    )
                }
                "footer-comments" => {
                    let id = store["pages"][0]["id"].as_str().expect("page").to_owned();
                    (
                        "pages",
                        id.clone(),
                        format!("/wiki/api/v2/pages/{id}/footer-comments"),
                    )
                }
                _ => {
                    let id = store["confluence_comments"][0]["id"]
                        .as_str()
                        .expect("comment")
                        .to_owned();
                    (
                        "confluence_comments",
                        id.clone(),
                        format!("/wiki/api/v2/footer-comments/{id}/children"),
                    )
                }
            };
            store["collection_fault"] = json!({"path": path});
            if absent {
                // Parent disappears between the precheck and collection read;
                // saved children remain independently addressable for cleanup.
                store["collection_fault"]["remove_parent"] = json!([collection, id]);
            }
            harness.write_store(&store);
            if absent {
                assert_success(&command("cleanup"));
                assert_graph_absent(&harness);
            } else {
                harness.clear_log();
                let receipt = fs::read(&harness.state).expect("receipt");
                assert_failure(&command("cleanup"), "upstream_failure");
                assert_eq!(fs::read(&harness.state).expect("retained receipt"), receipt);
                assert!(
                    harness.read_store()[collection]
                        .as_array()
                        .expect("parents")
                        .iter()
                        .any(|row| row["id"] == id)
                );
                assert!(
                    harness
                        .read_log()
                        .iter()
                        .all(|row| row["method"] != "DELETE")
                );
                let mut repaired = harness.read_store();
                repaired
                    .as_object_mut()
                    .expect("store")
                    .remove("collection_fault");
                harness.write_store(&repaired);
                assert_success(&command("cleanup"));
                assert_graph_absent(&harness);
            }
        }
    }
}

#[test]
fn cleanup_preserves_receipt_for_uncertain_space_task() {
    // Synthetic fault envelopes cover distinct parser and transition boundaries.
    for envelope in [
        json!({"status": "FINISH_SUCCESS\n"}),
        json!({"status": "FINISH_SUCCESS\u{0}"}),
        json!({"status": "UNKNOWN"}),
        json!({}),
        json!({"status": null}),
        json!({"status": 42}),
        json!({"status": "FAILED"}),
        json!({"status": "FINISH_SUCCESS", "state": "RUNNING"}),
        json!({"status": "FINISH_SUCCESS", "finished": false}),
        json!({"status": "FINISH_SUCCESS", "successful": false}),
        json!({"status": "FINISH_SUCCESS", "finished": "true"}),
        json!({"status": "FINISH_SUCCESS", "successful": 1}),
        json!({"status": "FINISH_SUCCESS", "errors": ["synthetic failure"]}),
        json!({"status": "FINISH_SUCCESS", "errors": {}}),
        json!({"status": "RUNNING", "finished": true, "successful": true}),
        json!({"status": "RUNNING", "finished": false, "successful": false, "errors": []}),
        json!("transport"),
    ] {
        let harness = Harness::new();
        assert_success(&harness.command("bootstrap"));
        // Only the test clock is replaced; the real bounded poll loop runs.
        write_executable(&harness.fake_bin.join("sleep"), b"#!/bin/sh\nexit 0\n");
        let mut store = harness.read_store();
        let second_space = store["spaces"][1].clone();
        store["task_fault"] = envelope.clone();
        harness.write_store(&store);
        harness.clear_log();
        assert_cleanup_refused(&harness);
        let residue = harness.read_store();
        assert!(
            residue["spaces"]
                .as_array()
                .expect("spaces")
                .contains(&second_space),
            "advanced past uncertain task {envelope}"
        );
        assert!(
            residue["tasks"]
                .as_object()
                .expect("tasks")
                .values()
                .all(|task| task["space"] != second_space["id"])
        );
        assert!(
            !residue["tasks"].as_object().expect("tasks").is_empty(),
            "never reached accepted DELETE"
        );
        let polls = operation_rows(&harness.read_log(), "space_delete_poll").len();
        assert!(polls <= 30, "poll ceiling exceeded");
        if envelope["status"] == "RUNNING" && envelope["finished"] == false {
            assert_eq!(
                polls, 30,
                "known progress did not exhaust the bounded poll budget"
            );
        }
        let mut repaired = residue;
        repaired
            .as_object_mut()
            .expect("store")
            .remove("task_fault");
        harness.write_store(&repaired);
        assert_success(&harness.command("cleanup"));
        assert_graph_absent(&harness);
    }
}

#[test]
fn cleanup_revalidates_each_destructive_target() {
    for (collection, index, after) in [
        ("confluence_comments", 0, "confluence_comment_delete"),
        ("pages", 0, "confluence_comment_delete"),
        ("spaces", 0, "confluence_page_delete"),
        ("jira_comments", 1, "jira_comment_delete"),
        ("issues", 0, "jira_comment_delete"),
        ("projects", 0, "jira_issue_delete"),
    ] {
        let harness = Harness::new();
        assert_success(&harness.command("bootstrap"));
        let mut store = harness.read_store();
        let original = store[collection][index].clone();
        let id = original["id"].clone();
        let mut switch = json!({"after": after, "target": [collection, id]});
        match collection {
            "pages" | "confluence_comments" => switch["marker"] = json!("foreign"),
            "spaces" | "projects" => switch["fields"] = json!({"description": "foreign"}),
            "issues" => {
                let mut fields = original["fields"].clone();
                fields["description"] = json!({"type": "doc", "version": 1, "content": []});
                switch["fields"] = json!({"fields": fields});
            }
            _ => switch["fields"] = json!({"body": {"type": "doc", "version": 1, "content": []}}),
        }
        store["late_switch"] = switch;
        harness.write_store(&store);
        assert_cleanup_refused(&harness);
        let mut residue = harness.read_store();
        assert_eq!(
            residue["faults"]["late_switch"], true,
            "late seam not reached for {collection}"
        );
        let target = residue[collection]
            .as_array_mut()
            .expect("targets")
            .iter_mut()
            .find(|row| row["id"] == id)
            .expect("foreign target survived");
        *target = original;
        if let Some(marker) = store["properties"][id.as_str().expect("id")].get("rfs-owner") {
            residue["properties"][id.as_str().expect("id")]["rfs-owner"] = marker.clone();
        }
        harness.write_store(&residue);
        assert_success(&harness.command("cleanup"));
        assert_graph_absent(&harness);
    }
}

#[test]
fn cleanup_revalidates_trashed_marker_before_purge() {
    let harness = Harness::new();
    assert_success(&harness.command("bootstrap"));
    let mut store = harness.read_store();
    let page = store["pages"]
        .as_array()
        .expect("pages")
        .last()
        .expect("last page");
    let id = page["id"].as_str().expect("id").to_owned();
    let marker = store["properties"][&id]["rfs-owner"].clone();
    store["late_switch"] = json!({
        "after": "confluence_page_delete", "target": ["pages", id], "marker": "foreign"
    });
    harness.write_store(&store);
    assert_cleanup_refused(&harness);
    let mut residue = harness.read_store();
    assert_eq!(residue["faults"]["late_switch"], true);
    assert!(
        residue["pages"]
            .as_array()
            .expect("pages")
            .iter()
            .any(|row| row["id"] == id && row["status"] == "trashed")
    );
    residue["properties"][&id]["rfs-owner"] = marker;
    harness.write_store(&residue);
    assert_success(&harness.command("cleanup"));
    assert_graph_absent(&harness);
}

#[test]
fn cleanup_skips_absent_space_id_with_reused_key() {
    let harness = Harness::new();
    assert_success(&harness.command("bootstrap"));
    let mut store = harness.read_store();
    let id = store["spaces"][0]["id"].clone();
    let mut replacement = store["spaces"][0].clone();
    replacement["id"] = json!("9100000002");
    replacement["description"] = json!("foreign replacement");
    store["late_switch"] = json!({
        "after": "confluence_page_delete", "target": ["spaces", id],
        "replacement": replacement
    });
    harness.write_store(&store);
    assert_success(&harness.command("cleanup"));
    let residue = harness.read_store();
    assert_eq!(residue["faults"]["late_switch"], true);
    assert_eq!(residue["spaces"], json!([replacement]));
    assert_eq!(residue["pages"], json!([]));
    assert!(!harness.state.exists());
}

#[test]
fn cleanup_retains_receipt_until_all_targets_absent() {
    for collection in ["pages", "spaces", "projects"] {
        let harness = Harness::new();
        assert_success(&harness.command("bootstrap"));
        let mut store = harness.read_store();
        let mut row = store[collection][0].clone();
        if collection == "projects" {
            row["deleted"] = json!(true);
        }
        store["linger"] = json!({"collection": collection, "row": row});
        harness.write_store(&store);
        assert_cleanup_refused(&harness);
        let residue = harness.read_store();
        assert_eq!(
            residue["faults"]["linger"], true,
            "did not reach final task"
        );
        assert!(
            residue[collection]
                .as_array()
                .expect("targets")
                .contains(&row)
        );
        // Independent provider completion, with the same retained receipt.
        let mut completed = residue;
        for name in [
            "projects",
            "issues",
            "jira_comments",
            "spaces",
            "pages",
            "confluence_comments",
        ] {
            completed[name] = json!([]);
        }
        harness.write_store(&completed);
        assert_success(&harness.command("cleanup"));
        assert_graph_absent(&harness);
    }
}

#[test]
fn cleanup_resumes_all_already_absent_receipt() {
    let harness = Harness::new();
    assert_success(&harness.command("bootstrap"));
    let receipt = fs::read(&harness.state).expect("receipt");
    let permissions = fs::metadata(&harness.state)
        .expect("receipt metadata")
        .permissions();
    assert_success(&harness.command("cleanup"));
    assert_graph_absent(&harness);
    fs::write(&harness.state, receipt).expect("restore receipt after external completion");
    fs::set_permissions(&harness.state, permissions).expect("restore private receipt permissions");
    harness.clear_log();
    assert_success(&harness.command("cleanup"));
    assert_graph_absent(&harness);
    assert!(
        harness
            .read_log()
            .iter()
            .all(|row| row["method"] != "DELETE")
    );
}

#[test]
fn cleanup_refuses_unknown_page_status() {
    for status in [json!("archived"), json!(null), json!(42), json!({})] {
        let harness = Harness::new();
        assert_success(&harness.command("bootstrap"));
        let mut store = harness.read_store();
        let id = store["pages"][0]["id"].clone();
        store["pages"][0]["status"] = status;
        harness.write_store(&store);
        assert_cleanup_refused(&harness);
        let mut residue = harness.read_store();
        let page = residue["pages"]
            .as_array_mut()
            .expect("pages")
            .iter_mut()
            .find(|row| row["id"] == id)
            .expect("unknown-status page survives");
        page["status"] = json!("current");
        harness.write_store(&residue);
        assert_success(&harness.command("cleanup"));
        assert_graph_absent(&harness);
    }
}
