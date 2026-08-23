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

/// The eleven license identifiers approved in `spec.md`'s Decisions table.
const APPROVED_LICENSES: &[&str] = &[
    "0BSD",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
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
        "deny.toml's license allow-list must match the eleven identifiers signed in .rfs-q12y/spec.md"
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
