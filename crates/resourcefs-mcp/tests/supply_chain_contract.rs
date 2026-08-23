//! Fences for the workspace supply-chain gate (rfs-q12y).
//!
//! Every expectation here is transcribed literally from `.rfs-q12y/spec.md`
//! (signed 2026-08-23, Revision 1) rather than derived from `deny.toml`, so
//! the config and its policy cannot drift together silently.
//!
//! `deny.toml` is parsed line-wise on purpose: no TOML crate is a workspace
//! dependency and this change adds none, matching the token-scanning approach
//! `architecture_contract.rs` already uses.

use std::path::{Path, PathBuf};
use std::process::Command;

use cargo_metadata::MetadataCommand;

/// The twelve license identifiers approved in `spec.md`'s Decisions table.
const APPROVED_LICENSES: &[&str] = &[
    "0BSD",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "CDLA-Permissive-2.0",
    "ISC",
    "MIT",
    "MPL-2.0",
    "Unicode-3.0",
    "Unlicense",
    "Zlib",
];

fn workspace_root() -> PathBuf {
    MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("workspace cargo metadata")
        .workspace_root
        .into_std_path_buf()
}

fn deny_config(root: &Path) -> String {
    let path = root.join("deny.toml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// Strips `#` comments so prose mentioning a rejected value (for example the
/// note explaining why the scope is not `"transitive"`) cannot be mistaken for
/// configuration.
fn without_comments(config: &str) -> String {
    config
        .lines()
        .map(|line| line.split('#').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the double-quoted strings inside `key = [ ... ]`, searched only
/// within `section`.
fn array_entries(config: &str, section: &str, key: &str) -> Vec<String> {
    let body = without_comments(config);
    let mut in_section = false;
    let mut collecting = false;
    let mut entries = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !collecting {
            in_section = trimmed == section;
            continue;
        }
        if !in_section {
            continue;
        }
        if !collecting {
            let Some(rest) = trimmed.strip_prefix(key) else {
                continue;
            };
            let Some(rest) = rest.trim_start().strip_prefix('=') else {
                continue;
            };
            let Some(rest) = rest.trim_start().strip_prefix('[') else {
                continue;
            };
            collecting = true;
            entries.extend(quoted(rest));
            if rest.contains(']') {
                return entries;
            }
            continue;
        }
        entries.extend(quoted(trimmed));
        if trimmed.contains(']') {
            return entries;
        }
    }
    entries
}

fn quoted(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        found.push(after[..close].to_owned());
        rest = &after[close + 1..];
    }
    found
}

/// Returns the value of `key = "value"` within `section`.
fn scalar(config: &str, section: &str, key: &str) -> Option<String> {
    let body = without_comments(config);
    let mut in_section = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed == section;
            continue;
        }
        if !in_section {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix(key) else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        return quoted(rest).into_iter().next();
    }
    None
}

/// C1 — the allow-list is exactly the approved set and the committed baseline
/// carries no exceptions and no ignored advisories, so it is provably clean
/// rather than clean by assertion.
#[test]
fn deny_config_matches_signed_policy() {
    let config = deny_config(&workspace_root());

    let allowed = array_entries(&config, "[licenses]", "allow");
    assert_eq!(
        allowed,
        APPROVED_LICENSES
            .iter()
            .map(|entry| (*entry).to_owned())
            .collect::<Vec<_>>(),
        "deny.toml's license allow-list must match the identifiers signed in .rfs-q12y/spec.md"
    );

    let exceptions = without_comments(&config)
        .matches("[[licenses.exceptions]]")
        .count();
    assert_eq!(
        exceptions, 0,
        "the signed policy is a blanket allow-list with zero per-package license exceptions"
    );

    let ignored = array_entries(&config, "[advisories]", "ignore");
    assert!(
        ignored.is_empty(),
        "the committed advisory baseline must carry zero ignore entries so the first use of \
         that escape hatch is a visible change; found: {ignored:?}"
    );
}

/// C4 — the advisory policy is the signed one, and the scope is never weakened
/// to a value that would let an unmaintained dependency land silently.
#[test]
fn advisory_policy_matches_signed_policy() {
    let config = deny_config(&workspace_root());

    assert_eq!(
        scalar(&config, "[advisories]", "yanked").as_deref(),
        Some("deny"),
        "yanked crates must deny per the signed policy"
    );

    let unmaintained = scalar(&config, "[advisories]", "unmaintained");
    assert_eq!(
        unmaintained.as_deref(),
        Some("all"),
        "the signed policy examines every crate for unmaintained advisories"
    );
    // Stated explicitly because these are the two values the signed decision
    // forbids: an unmaintained advisory is cleared only by a documented
    // `ignore` entry, never by narrowing what is examined.
    assert_ne!(unmaintained.as_deref(), Some("transitive"));
    assert_ne!(unmaintained.as_deref(), Some("none"));
}

/// C1 — the whole gate passes against the real workspace tree.
#[test]
fn workspace_passes_the_gate() {
    let root = workspace_root();
    let output = Command::new("cargo")
        .arg("deny")
        .arg("check")
        .current_dir(&root)
        .output()
        .expect("run cargo-deny; it is required to vet this workspace");

    assert!(
        output.status.success(),
        "cargo deny check must pass for the committed tree.\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The four gate commands, transcribed from `.rfs-q12y/spec.md`'s Behavior
/// section — the same commands a maintainer runs locally.
const GATE_COMMANDS: &[&str] = &[
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo test --workspace --all-features",
    "cargo deny check",
];

/// C5 — CI runs the local gates and cannot drift from them.
///
/// The workflow has never executed (no git remote), so this asserts only what
/// is checkable in-repo: the gate commands, the triggers, and the absence of
/// tabs. Whether it actually runs is deferred verification, tracked at
/// rfs-58r1.
#[test]
fn ci_workflow_mirrors_local_gates() {
    let path = workspace_root().join(".github/workflows/ci.yml");
    let workflow = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

    for command in GATE_COMMANDS {
        let occurrences = workflow.matches(command).count();
        assert_eq!(
            occurrences, 1,
            "the CI workflow must run `{command}` exactly once, byte-for-byte identical to the \
             local gate; found {occurrences} occurrences. A reworded command means CI and local \
             practice have diverged; a second occurrence usually means a weaker duplicate was \
             added alongside the real gate."
        );
    }

    for trigger in ["push:", "pull_request:"] {
        assert!(
            workflow.lines().any(|line| line.trim() == trigger),
            "the CI workflow must declare a `{trigger}` trigger"
        );
    }

    assert!(
        !workflow.contains('\t'),
        "the CI workflow must not contain tab characters: tabs are invalid YAML indentation and \
         are the most likely way a hand-written workflow fails the first time it runs"
    );
}

/// Writes `path` and every parent directory it needs.
fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|error| {
            panic!("create {}: {error}", parent.display());
        });
    }
    std::fs::write(path, contents)
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

/// Runs one cargo-deny check in `dir` against the *committed* `deny.toml`, so
/// these fixtures exercise the real policy rather than a copy that could drift
/// from it.
/// `--config` belongs to the `check` subcommand, not to `cargo deny` itself:
/// placing it first is a usage error that exits non-zero, which would let a
/// "did it fail?" assertion pass without the gate ever running.
fn deny_check_in(dir: &Path, which: &str) -> std::process::Output {
    Command::new("cargo")
        .arg("deny")
        .arg("check")
        .arg("--config")
        .arg(workspace_root().join("deny.toml"))
        .arg(which)
        .current_dir(dir)
        .output()
        .expect("run cargo-deny against the fixture graph")
}

/// Asserts cargo-deny rejected the fixture *for the reason under test*.
///
/// Checking only the exit status is not enough: a malformed invocation, an
/// unresolvable manifest, or a missing config all exit non-zero too, so a
/// bare `!success` assertion can pass while the gate never ran. Requiring the
/// specific `<check> FAILED` diagnostic pins the failure to the real cause.
fn assert_rejected_by(output: &std::process::Output, which: &str, why: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        !output.status.success(),
        "{why}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains(&format!("{which} FAILED")),
        "expected cargo-deny to report `{which} FAILED`; a non-zero exit without it means the \
         fixture failed for some other reason (usage error, unresolvable manifest) and the gate \
         was never exercised.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// C2 — the license gate bites. A disallowed license anywhere in the graph
/// must fail, including on a transitive package while the root is compliant:
/// a root-only check would pass this fixture.
#[test]
fn license_gate_rejects_copyleft() {
    let fixture = tempfile::tempdir().expect("temp dir for the license fixture");
    let root = fixture.path();

    write(
        &root.join("Cargo.toml"),
        "[package]\n\
         name = \"license-fixture-root\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         license = \"MIT\"\n\
         \n\
         [dependencies]\n\
         copyleft-dep = { path = \"copyleft-dep\" }\n",
    );
    write(&root.join("src/lib.rs"), "");
    write(
        &root.join("copyleft-dep/Cargo.toml"),
        "[package]\n\
         name = \"copyleft-dep\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         license = \"GPL-3.0\"\n",
    );
    write(&root.join("copyleft-dep/src/lib.rs"), "");

    let output = deny_check_in(root, "licenses");
    assert_rejected_by(
        &output,
        "licenses",
        "a GPL-3.0 package must fail the license gate, otherwise the allow-list is decorative",
    );
}

/// C3 — the source gate bites. Uses a local repository reached by a `file://`
/// URL so the fixture is a genuine cargo-resolved git source while needing no
/// network.
#[test]
fn source_gate_rejects_git() {
    let fixture = tempfile::tempdir().expect("temp dir for the source fixture");
    let root = fixture.path();
    let dependency = root.join("git-dep");

    write(
        &dependency.join("Cargo.toml"),
        "[package]\n\
         name = \"git-dep\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         license = \"MIT\"\n",
    );
    write(&dependency.join("src/lib.rs"), "");

    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&dependency)
            .status()
            .expect("git is required to build the source fixture");
        assert!(status.success(), "git {args:?} failed");
    };
    git(&["init", "--quiet", "."]);
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=fixture@example.invalid",
        "-c",
        "user.name=fixture",
        "commit",
        "--quiet",
        "-m",
        "fixture",
    ]);

    write(
        &root.join("Cargo.toml"),
        &format!(
            "[package]\n\
             name = \"source-fixture-root\"\n\
             version = \"0.1.0\"\n\
             edition = \"2021\"\n\
             license = \"MIT\"\n\
             \n\
             [dependencies]\n\
             git-dep = {{ git = \"{}\" }}\n",
            file_url(&dependency)
        ),
    );
    write(&root.join("src/lib.rs"), "");

    let output = deny_check_in(root, "sources");
    assert_rejected_by(
        &output,
        "sources",
        "a git-sourced dependency must fail the source gate, otherwise the crates.io pin is \
         decorative",
    );
}

/// Renders a path as a `file://` URL Cargo can resolve on every platform.
///
/// `format!("file://{}", path.display())` is not portable: on Windows it
/// produces `file://C:\Users\…`, which has backslash separators and is missing
/// the third slash, so Cargo fails to resolve the dependency at all. The gate
/// test would then exit non-zero for a manifest error rather than a source
/// policy violation — passing for the wrong reason on the very check whose
/// point is that the gate bites. `Url::from_file_path` handles the drive
/// letter and separators.
fn file_url(path: &std::path::Path) -> String {
    url::Url::from_file_path(path)
        .expect("fixture path is absolute")
        .to_string()
}
