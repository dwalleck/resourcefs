//! Fences for the workspace supply-chain gate (rfs-q12y).
//!
//! Every expectation here is transcribed literally from `.rfs-q12y/spec.md`
//! (signed 2026-08-23, Revision 1) rather than derived from `deny.toml`, so
//! the config and its policy cannot drift together silently.
//!
//! `deny.toml` is read as TOML through the `toml` crate, a dev-dependency that
//! was already in the lockfile through `trybuild`. The line-wise scanner it
//! replaced returned an empty list for a key that was *absent* exactly as it
//! did for one that was present and empty, so renaming `[advisories]` or
//! misspelling `ignore` left the zero-ignores fence passing while it detected
//! nothing (rfs-kjju). A parsed table keeps `None` and `Some([])` apart, and a
//! `#` inside a quoted value is a character rather than a comment. Only the
//! reading of `deny.toml` changed: `APPROVED_LICENSES` stays a literal
//! transcribed from the signed spec.

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

/// Parses the committed policy so that absence and emptiness stay distinct.
fn policy(config: &str) -> toml::Table {
    config
        .parse()
        .unwrap_or_else(|error| panic!("deny.toml is not valid TOML: {error}"))
}

/// Returns `name` as a table, or `None` when no such section exists.
///
/// A section present with another shape fails outright rather than reading
/// as absent: "missing" and "malformed" are different findings.
fn section<'a>(policy: &'a toml::Table, name: &str) -> Option<&'a toml::Table> {
    let value = policy.get(name)?;
    Some(
        value
            .as_table()
            .unwrap_or_else(|| panic!("deny.toml `{name}` must be a table, found {value}")),
    )
}

/// Returns the strings in `section.key`, or `None` when the section or the
/// key is absent. Any other shape fails rather than reading as empty.
fn string_array(policy: &toml::Table, section_name: &str, key: &str) -> Option<Vec<String>> {
    let value = section(policy, section_name)?.get(key)?;
    let entries = value.as_array().unwrap_or_else(|| {
        panic!("deny.toml `{section_name}.{key}` must be an array, found {value}")
    });
    Some(
        entries
            .iter()
            .map(|entry| {
                entry.as_str().map(str::to_owned).unwrap_or_else(|| {
                    panic!(
                        "deny.toml `{section_name}.{key}` must contain only strings, found {entry}"
                    )
                })
            })
            .collect(),
    )
}

/// Returns the string at `section.key`, or `None` when the section or the
/// key is absent. Any other shape fails rather than reading as absent.
fn string_scalar(policy: &toml::Table, section_name: &str, key: &str) -> Option<String> {
    let value = section(policy, section_name)?.get(key)?;
    Some(value.as_str().map(str::to_owned).unwrap_or_else(|| {
        panic!("deny.toml `{section_name}.{key}` must be a string, found {value}")
    }))
}

/// Returns the tables in the array of tables at `section.key` (the shape
/// `[[licenses.exceptions]]` produces), or `None` when the section or the key
/// is absent. Any other shape fails rather than reading as empty.
fn table_array(policy: &toml::Table, section_name: &str, key: &str) -> Option<Vec<toml::Table>> {
    let value = section(policy, section_name)?.get(key)?;
    let entries = value.as_array().unwrap_or_else(|| {
        panic!("deny.toml `{section_name}.{key}` must be an array of tables, found {value}")
    });
    Some(
        entries
            .iter()
            .map(|entry| {
                entry.as_table().cloned().unwrap_or_else(|| {
                    panic!(
                        "deny.toml `{section_name}.{key}` must contain only tables, found {entry}"
                    )
                })
            })
            .collect(),
    )
}

/// C1 — the allow-list is exactly the approved set and the committed baseline
/// carries no exceptions and no ignored advisories, so it is provably clean
/// rather than clean by assertion.
#[test]
fn deny_config_matches_signed_policy() {
    let policy = policy(&deny_config(&workspace_root()));

    let approved: Vec<String> = APPROVED_LICENSES
        .iter()
        .map(|entry| (*entry).to_owned())
        .collect();
    assert_eq!(
        string_array(&policy, "licenses", "allow"),
        Some(approved),
        "deny.toml's license allow-list must match the identifiers signed in .rfs-q12y/spec.md"
    );

    // An array of tables has no present-and-empty spelling in `[[...]]` form,
    // so absence is the only way to write zero exceptions and is accepted.
    // That is unlike `ignore` below, where the empty list is spelled out on
    // purpose.
    let exceptions =
        table_array(&policy, "licenses", "exceptions").map_or(0, |tables| tables.len());
    assert_eq!(
        exceptions, 0,
        "the signed policy is a blanket allow-list with zero per-package license exceptions"
    );

    // `ignore = []` must be present as well as empty. The committed policy
    // spells the empty list out so the first ignore entry is a diff against
    // an existing line, and the previous scanner could not tell that spelling
    // from a renamed section or a misspelled key (rfs-kjju).
    let Some(ignored) = string_array(&policy, "advisories", "ignore") else {
        panic!(
            "deny.toml must spell out `ignore = []` under [advisories]; the key is absent, so \
             the section was renamed or the escape hatch was removed, and this fence could no \
             longer see its first use"
        );
    };
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
    let policy = policy(&deny_config(&workspace_root()));

    assert_eq!(
        string_scalar(&policy, "advisories", "yanked").as_deref(),
        Some("deny"),
        "yanked crates must deny per the signed policy"
    );

    let unmaintained = string_scalar(&policy, "advisories", "unmaintained");
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

/// The reader keeps an absent key apart from an empty one. This is what makes
/// the zero-ignores assertion falsifiable: the line-wise scanner it replaced
/// returned an empty list for every case below (rfs-kjju).
#[test]
fn reader_reports_an_absent_key_as_none_not_empty() {
    let renamed_section = policy("[advisory]\nignore = [\"RUSTSEC-0000-0000\"]\n");
    assert_eq!(
        string_array(&renamed_section, "advisories", "ignore"),
        None,
        "a renamed section must read as absent, not as an empty ignore list"
    );

    let misspelled_key = policy("[advisories]\nignored = [\"RUSTSEC-0000-0000\"]\n");
    assert_eq!(
        string_array(&misspelled_key, "advisories", "ignore"),
        None,
        "a misspelled key must read as absent, not as an empty ignore list"
    );

    let present_and_empty = policy("[advisories]\nignore = []\n");
    assert_eq!(
        string_array(&present_and_empty, "advisories", "ignore"),
        Some(Vec::new()),
        "the spelled-out empty list is the one shape the baseline accepts"
    );
}

/// The reader returns the entries that would make the zero-ignores fence fail.
#[test]
fn reader_returns_ignore_entries() {
    let policy = policy(
        "[advisories]\n\
         ignore = [\n\
             \"RUSTSEC-0000-0000\", # rationale goes here\n\
             \"RUSTSEC-0000-0001\",\n\
         ]\n",
    );
    assert_eq!(
        string_array(&policy, "advisories", "ignore"),
        Some(vec![
            "RUSTSEC-0000-0000".to_owned(),
            "RUSTSEC-0000-0001".to_owned()
        ])
    );
}

/// A `#` inside a quoted value is part of the value. The replaced scanner cut
/// every line at its first `#` and then found no closing quote.
#[test]
fn reader_keeps_a_hash_inside_a_quoted_value() {
    let policy = policy("[advisories]\nunmaintained = \"all # not a comment\"\n");
    assert_eq!(
        string_scalar(&policy, "advisories", "unmaintained").as_deref(),
        Some("all # not a comment")
    );
}

/// Exceptions are counted as parsed tables, not as occurrences of the header
/// text, and a section with no exceptions reads as absent.
#[test]
fn reader_counts_exception_tables() {
    let with_exception = policy(
        "[licenses]\n\
         allow = [\"MIT\"]\n\
         \n\
         [[licenses.exceptions]]\n\
         name = \"some-crate\"\n\
         allow = [\"GPL-3.0\"]\n",
    );
    let exceptions = table_array(&with_exception, "licenses", "exceptions")
        .expect("an array-of-tables header creates the key");
    assert_eq!(exceptions.len(), 1);
    assert_eq!(
        exceptions[0].get("name").and_then(toml::Value::as_str),
        Some("some-crate")
    );

    let without = policy("[licenses]\nallow = [\"MIT\"]\n");
    assert_eq!(table_array(&without, "licenses", "exceptions"), None);
}

/// A key present with the wrong shape is a finding, never an empty list.
#[test]
#[should_panic(expected = "`advisories.ignore` must be an array")]
fn reader_rejects_a_scalar_where_an_array_is_required() {
    let policy = policy("[advisories]\nignore = \"RUSTSEC-0000-0000\"\n");
    string_array(&policy, "advisories", "ignore");
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
/// these fixtures exercise the real policy rather than a separately maintained
/// copy that could drift from it.
/// Stage the config under its default name: cargo-deny 0.20 moved `--config`
/// from `check` to the root command, but both CLI versions discover `deny.toml`
/// in the fixture directory.
fn deny_check_in(dir: &Path, which: &str) -> std::process::Output {
    write(&dir.join("deny.toml"), &deny_config(&workspace_root()));
    Command::new("cargo")
        .arg("deny")
        .args(["--format", "json"])
        .arg("check")
        .arg(which)
        .current_dir(dir)
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .expect("run cargo-deny against the fixture graph")
}

/// Asserts cargo-deny rejected the fixture *for the reason under test*.
///
/// A nonzero exit alone can mean invocation or manifest failure, or a default
/// config rejecting every license. Require exactly the intended crate's policy
/// error, with no unrelated errors. Cargo's proxy may prefix JSON with warnings;
/// those are not policy evidence, and every failure retains both output streams.
fn assert_rejected_by(
    output: &std::process::Output,
    code: &str,
    crate_name: &str,
    expected_exit: i32,
    why: &str,
) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let fail = |reason: &str| -> ! {
        panic!(
            "{why}\n{reason}\nstatus: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
    };
    // cargo-deny returns a bitmask: licenses = 4, sources = 8.
    if output.status.code() != Some(expected_exit) {
        fail(&format!(
            "expected cargo-deny policy exit mask {expected_exit}"
        ));
    }

    let mut rejected = 0;
    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Human cargo/proxy diagnostics are preserved above, not policy
        // evidence. JSON-shaped output must parse rather than being discarded.
        if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
            continue;
        }
        let entry: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|error| fail(&format!("invalid cargo-deny JSON: {error}")));
        let fields = &entry["fields"];
        if entry["type"] == "log"
            && fields["message"]
                .as_str()
                .is_some_and(|message| message.contains("falling back to default config"))
        {
            fail("cargo-deny did not load the committed policy");
        }
        if entry["type"] == "diagnostic" && fields["severity"] == "error" {
            let graphs = fields["graphs"].as_array();
            if fields["code"] != code
                || !graphs.is_some_and(|graphs| {
                    graphs.len() == 1 && graphs[0]["Krate"]["name"] == crate_name
                })
            {
                fail("cargo-deny reported an unrelated error or rejected another crate");
            }
            rejected += 1;
        }
    }
    if rejected != 1 {
        fail(&format!(
            "expected exactly one `{code}` error for `{crate_name}`, observed {rejected}"
        ));
    }
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
        "rejected",
        "copyleft-dep",
        4,
        "a GPL-3.0 package must fail the license gate, otherwise the allow-list is decorative",
    );

    // A positive control proves that the same policy accepts the permitted root
    // and dependency. An empty/default allow-list must never satisfy this fence.
    let dependency_manifest = root.join("copyleft-dep/Cargo.toml");
    let manifest = std::fs::read_to_string(&dependency_manifest).expect("read fixture manifest");
    write(&dependency_manifest, &manifest.replace("GPL-3.0", "MIT"));
    let permitted = deny_check_in(root, "licenses");
    assert!(
        permitted.status.success(),
        "the committed policy must accept the MIT-only graph.\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        permitted.status,
        String::from_utf8_lossy(&permitted.stdout),
        String::from_utf8_lossy(&permitted.stderr)
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
        "source-not-allowed",
        "git-dep",
        8,
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
