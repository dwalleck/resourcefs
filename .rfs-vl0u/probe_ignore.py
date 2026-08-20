#!/usr/bin/env python3
"""Probe instrument for rfs-vl0u prove-it: ignore 0.4.33.

Builds a corpus tree (authorized root, outside sentinel, escaping directory
symlink under the root, root and nested .gitignore files) and a throwaway
Rust crate pinned to ignore =0.4.33. The crate reports:
  * whether a follow-links WalkBuilder enumerates the outside sentinel, and
  * a normalized path -> ignored map computed with the low-level
    ignore::gitignore matchers for every corpus path.

Emits one deterministic JSON object on stdout. Evidence only: touches only
its temporary directory. On any failure, prints captured stderr to stderr
and exits nonzero; a failure is never turned into a plausible result.
Platforms without symlink capability fail explicitly instead of skipping.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

IGNORE_PIN = "0.4.33"

# Corpus paths relative to the authorized root, each with the is_dir flag used
# for ignore matching. Covers: plain ignore (stray.log, stray.tmp), in-file
# negation (important.log), nested override of a root rule (sub/override.tmp),
# exclusion (docs, docs/notes.txt), and no-match cases. Link containment is
# measured separately from ignore matching.
CORPUS = [
    ("keep.txt", False),
    ("stray.log", False),
    ("stray.tmp", False),
    ("important.log", False),
    ("docs", True),
    ("docs/notes.txt", False),
    ("sub", True),
    ("sub/data.tmp", False),
    ("sub/override.tmp", False),
    ("sub/scratch.bak", False),
    ("scratch.bak", False),
]

ROOT_GITIGNORE = "*.log\n*.tmp\n!important.log\ndocs/\n"
SUB_GITIGNORE = "!override.tmp\n*.bak\n"

CARGO_TOML = """\
[package]
name = "rfs-vl0u-ignore-probe"
version = "0.0.0"
edition = "2021"

[dependencies]
ignore = "=0.4.33"
"""

MAIN_RS = r"""use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::Match;

fn die(message: &str) -> ! {
    eprintln!("rfs-vl0u probe error: {message}");
    process::exit(1);
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ignore,
    Whitelist,
    NoMatch,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Ignore => "ignore",
            Status::Whitelist => "whitelist",
            Status::NoMatch => "no_match",
        }
    }
}

/// Loads the matcher for `<dir>/.gitignore` if that file exists.
fn load_gitignore(dir: &Path) -> Option<Gitignore> {
    let file = dir.join(".gitignore");
    if !file.is_file() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(dir);
    if let Some(error) = builder.add(&file) {
        panic!("corpus .gitignore file must be readable and valid: {error}");
    }
    Some(
        builder
            .build()
            .expect("corpus .gitignore patterns are valid"),
    )
}

/// Matcher directories for a path whose parent directory is `parent_rel`
/// (relative to `root`; empty for root-level paths), shallowest first.
fn matcher_dirs(root: &Path, parent_rel: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut cur: Option<&Path> = Some(parent_rel);
    while let Some(d) = cur {
        if d.as_os_str().is_empty() {
            dirs.push(root.to_path_buf());
            break;
        }
        dirs.push(root.join(d));
        cur = d.parent();
    }
    dirs.reverse();
    dirs
}

/// Last matching status and pattern across all relevant .gitignore files,
/// evaluated shallowest directory first so deeper files override shallower
/// ones (matching git precedence).
fn last_match(
    igs: &BTreeMap<PathBuf, Gitignore>,
    root: &Path,
    path_abs: &Path,
    parent_rel: &Path,
    is_dir: bool,
) -> (Status, Option<String>) {
    let mut last = Status::NoMatch;
    let mut pattern: Option<String> = None;
    for dir in matcher_dirs(root, parent_rel) {
        let gi = match igs.get(&dir) {
            Some(gi) => gi,
            None => continue,
        };
        let rel = match path_abs.strip_prefix(&dir) {
            Ok(rel) => rel,
            Err(_) => die(&format!(
                "corpus path {:?} is not under matcher dir {:?}",
                path_abs, dir
            )),
        };
        match gi.matched(rel, is_dir) {
            Match::Ignore(glob) => {
                last = Status::Ignore;
                pattern = Some(glob.original().to_string());
            }
            Match::Whitelist(glob) => {
                last = Status::Whitelist;
                pattern = Some(glob.original().to_string());
            }
            Match::None => {}
        }
    }
    (last, pattern)
}

/// Whether the directory `dir_rel` (relative to `root`, non-empty) is
/// excluded, honoring git's rule that a path cannot be re-included when a
/// parent directory is excluded.
fn dir_excluded(
    igs: &BTreeMap<PathBuf, Gitignore>,
    root: &Path,
    dir_rel: &Path,
    memo: &mut BTreeMap<PathBuf, bool>,
) -> bool {
    if dir_rel.as_os_str().is_empty() {
        return false;
    }
    if let Some(&known) = memo.get(dir_rel) {
        return known;
    }
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut cur: Option<&Path> = Some(dir_rel);
    while let Some(d) = cur {
        if d.as_os_str().is_empty() {
            break;
        }
        chain.push(d.to_path_buf());
        cur = d.parent();
    }
    chain.reverse();
    let mut prev_excluded = false;
    for d in &chain {
        let parent_rel: &Path = match d.parent() {
            Some(p) => p,
            None => Path::new(""),
        };
        let (last, _) = last_match(igs, root, &root.join(d), parent_rel, true);
        let excluded = last == Status::Ignore || prev_excluded;
        memo.insert(d.clone(), excluded);
        prev_excluded = excluded;
    }
    prev_excluded
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        die("usage: probe_ignore <root> <sentinel> <corpus-spec>");
    }
    let root = PathBuf::from(&args[1]);
    let sentinel = PathBuf::from(&args[2]);
    let spec = &args[3];
    if !root.is_dir() {
        die(&format!("root {root:?} is not a directory"));
    }
    if !sentinel.is_file() {
        die(&format!("sentinel {sentinel:?} is not a file"));
    }

    // Walk the root with link following enabled.
    let mut walk_errors: Vec<String> = Vec::new();
    let mut visited: BTreeMap<String, bool> = BTreeMap::new();
    let walker = ignore::WalkBuilder::new(&root).follow_links(true).build();
    for entry in walker {
        match entry {
            Ok(de) => {
                let path = de.path();
                let rel = match path.strip_prefix(&root) {
                    Ok(rel) => rel,
                    Err(_) => die("walker yielded a path outside the root"),
                };
                let is_dir = match de.file_type() {
                    Some(ft) => ft.is_dir(),
                    None => false,
                };
                visited.insert(rel.to_string_lossy().into_owned(), is_dir);
            }
            Err(err) => walk_errors.push(err.to_string()),
        }
    }

    let sentinel_real = match std::fs::canonicalize(&sentinel) {
        Ok(real) => real,
        Err(err) => die(&format!(
            "cannot canonicalize sentinel {sentinel:?}: {err}"
        )),
    };
    let mut sentinel_visited = false;
    let mut linked_subtree: Vec<String> = Vec::new();
    for rel in visited.keys() {
        if rel.starts_with("linked/") {
            linked_subtree.push(rel.clone());
        }
        let abs = root.join(rel);
        if let Ok(real) = std::fs::canonicalize(&abs) {
            if real == sentinel_real {
                sentinel_visited = true;
            }
        }
    }

    // Corpus spec: one "<rel>\t<is_dir>" line per corpus path.
    let mut corpus: Vec<(String, bool)> = Vec::new();
    for line in spec.lines() {
        let mut parts = line.splitn(2, '\t');
        let rel = match parts.next() {
            Some(rel) => rel,
            None => die("corpus spec line has no path"),
        };
        let flag = match parts.next() {
            Some(flag) => flag,
            None => die("corpus spec line has no is_dir flag"),
        };
        let is_dir = match flag {
            "1" => true,
            "0" => false,
            _ => die(&format!(
                "corpus spec line has invalid is_dir flag {flag:?}"
            )),
        };
        if rel.is_empty() {
            die("corpus spec line has an empty path");
        }
        corpus.push((rel.to_string(), is_dir));
    }

    // Load a matcher for every directory that can matter: the root and all
    // ancestor directories of corpus paths.
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    dirs.insert(root.clone());
    for (rel, _) in &corpus {
        let mut cur: Option<&Path> = Path::new(rel).parent();
        while let Some(d) = cur {
            if d.as_os_str().is_empty() {
                break;
            }
            dirs.insert(root.join(d));
            cur = d.parent();
        }
    }
    let mut igs: BTreeMap<PathBuf, Gitignore> = BTreeMap::new();
    for d in dirs.iter() {
        if let Some(gi) = load_gitignore(d) {
            igs.insert(d.clone(), gi);
        }
    }

    // Normalized path -> ignored map for every corpus path.
    let mut memo: BTreeMap<PathBuf, bool> = BTreeMap::new();
    let mut ignored: Vec<(String, bool, Status, Option<String>, bool)> = Vec::new();
    for (rel, is_dir) in &corpus {
        let path_abs = root.join(rel);
        let parent_rel: &Path = match Path::new(rel).parent() {
            Some(p) => p,
            None => Path::new(""),
        };
        let (last, pattern) = last_match(&igs, &root, &path_abs, parent_rel, *is_dir);
        let parent_excluded = !parent_rel.as_os_str().is_empty()
            && dir_excluded(&igs, &root, parent_rel, &mut memo);
        let excluded = last == Status::Ignore || parent_excluded;
        let status = if excluded { Status::Ignore } else { last };
        ignored.push((rel.clone(), excluded, status, pattern, *is_dir));
    }

    // Emit deterministic JSON.
    let mut out = String::new();
    out.push_str("{\"walk\":{");
    out.push_str(&format!(
        "\"outside_sentinel_visited\":{},",
        if sentinel_visited { "true" } else { "false" }
    ));
    out.push_str(&format!(
        "\"linked_subtree_visited\":{},",
        if linked_subtree.is_empty() { "false" } else { "true" }
    ));
    out.push_str("\"linked_subtree_entries\":[");
    for (i, p) in linked_subtree.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(p));
        out.push('"');
    }
    out.push_str("],");
    out.push_str(&format!("\"total_entries\":{},", visited.len()));
    out.push_str("\"errors\":[");
    for (i, e) in walk_errors.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(e));
        out.push('"');
    }
    out.push_str("]}");
    out.push_str(",\"ignored\":{");
    let mut first = true;
    for (rel, excluded, status, pattern, is_dir) in &ignored {
        if !first {
            out.push(',');
        }
        first = false;
        out.push('"');
        out.push_str(&json_escape(rel));
        out.push_str("\":{\"ignored\":");
        out.push_str(if *excluded { "true" } else { "false" });
        out.push_str(&format!(",\"status\":\"{}\",", status.name()));
        out.push_str("\"pattern\":");
        match pattern {
            Some(p) => {
                out.push('"');
                out.push_str(&json_escape(p));
                out.push('"');
            }
            None => out.push_str("null"),
        }
        out.push_str(&format!(",\"is_dir\":{}}}", if *is_dir { "true" } else { "false" }));
    }
    out.push_str("}}\n");
    print!("{}", out);
}
"""


def fail(message):
    print(f"probe_ignore: {message}", file=sys.stderr)
    sys.exit(1)


def build_tree(base):
    """Create the shared corpus tree; returns (root, sentinel)."""
    root = base / "root"
    outside = base / "outside"
    root.mkdir()
    outside.mkdir()
    (root / ".gitignore").write_text(ROOT_GITIGNORE, encoding="utf-8")
    (root / "keep.txt").write_text("keep\n", encoding="utf-8")
    (root / "stray.log").write_text("stray log\n", encoding="utf-8")
    (root / "stray.tmp").write_text("stray tmp\n", encoding="utf-8")
    (root / "important.log").write_text("important log\n", encoding="utf-8")
    (root / "scratch.bak").write_text("scratch bak\n", encoding="utf-8")
    docs = root / "docs"
    docs.mkdir()
    (docs / "notes.txt").write_text("notes\n", encoding="utf-8")
    sub = root / "sub"
    sub.mkdir()
    (sub / ".gitignore").write_text(SUB_GITIGNORE, encoding="utf-8")
    (sub / "data.tmp").write_text("data\n", encoding="utf-8")
    (sub / "override.tmp").write_text("override\n", encoding="utf-8")
    (sub / "scratch.bak").write_text("sub scratch bak\n", encoding="utf-8")
    sentinel = outside / "secret.txt"
    sentinel.write_text("outside sentinel\n", encoding="utf-8")
    link = root / "linked"
    try:
        os.symlink("../outside", link)
    except OSError as exc:
        fail(f"cannot create symlink {link} -> ../outside: {exc}")
    if not os.path.islink(link):
        fail("symlink creation reported success but the link is not a symlink")
    return root, sentinel


def run_crate(crate_dir, root, sentinel):
    spec = "\n".join(f"{rel}\t{1 if is_dir else 0}" for rel, is_dir in CORPUS)
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    env["CARGO_HOME"] = str(Path(crate_dir) / "cargo-home")
    env["CARGO_TARGET_DIR"] = str(Path(crate_dir) / "target")
    try:
        proc = subprocess.run(
            ["cargo", "run", "--quiet", "--", str(root), str(sentinel), spec],
            cwd=str(crate_dir),
            env=env,
            capture_output=True,
            text=True,
            timeout=1800,
        )
    except OSError as exc:
        fail(f"cannot run cargo: {exc}")
    except subprocess.TimeoutExpired as exc:
        fail(f"cargo run timed out after 1800s: {exc}")
    if proc.returncode != 0:
        print(proc.stderr, file=sys.stderr)
        fail(f"cargo run exited with status {proc.returncode}")
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        print(proc.stdout, file=sys.stderr)
        fail(f"crate emitted invalid JSON: {exc}")


def lock_versions(crate_dir):
    lock = Path(crate_dir) / "Cargo.lock"
    try:
        data = tomllib.loads(lock.read_text(encoding="utf-8"))
    except OSError as exc:
        fail(f"cannot read Cargo.lock: {exc}")
    except tomllib.TOMLDecodeError as exc:
        fail(f"cannot parse Cargo.lock: {exc}")
    versions = {}
    for pkg in data.get("package", []):
        name = pkg.get("name")
        if name in ("ignore", "globset"):
            versions[name] = pkg.get("version")
    resolved = versions.get("ignore")
    if resolved != IGNORE_PIN:
        fail(f"resolved ignore version {resolved!r} != pinned {IGNORE_PIN}")
    return versions


def main():
    tmp = Path(tempfile.mkdtemp(prefix="rfs-vl0u-ignore-probe-"))
    try:
        root, sentinel = build_tree(tmp)
        crate = tmp / "crate"
        crate.mkdir()
        (crate / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
        src = crate / "src"
        src.mkdir()
        (src / "main.rs").write_text(MAIN_RS, encoding="utf-8")
        result = run_crate(crate, root, sentinel)
        versions = lock_versions(crate)
        out = {
            "instrument": "probe",
            "subject": "ignore",
            "subject_version": versions["ignore"],
            "globset_version": versions.get("globset"),
            "walk": result["walk"],
            "ignored": result["ignored"],
        }
        print(json.dumps(out, indent=2, sort_keys=True))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
