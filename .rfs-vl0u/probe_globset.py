#!/usr/bin/env python3
"""Probe for globset 0.4.20 matching semantics (rfs-vl0u prove-it evidence).

Generates a throwaway Cargo crate in a temporary directory, pins
globset = "=0.4.20", invokes Cargo, runs the crate against a fixed corpus of
slash paths, and emits one deterministic JSON object on stdout.

Adopted syntax (matches the rfs-vl0u spec: "Globset-compatible path syntax
using `/` as the canonical separator; `*`, `?`, and character classes stay
within one component; `**` crosses directories; `{a,b}` provides alternation"):

  * GlobBuilder defaults, plus literal_separator(true) for every case, so
    `*` and `?` never match `/` (globset's default lets them match `/`).
  * `*`      any run of characters inside one path component.
  * `?`      exactly one character inside one path component.
  * `**`     zero or more whole path components (full component only).
  * `[..]`   character class; `^` negates; `-` makes ranges.
  * `{a,b}`  alternation (no empty alternatives).
  * Whole path must match (fully anchored); no filesystem walking.

case_insensitive is enabled per case via GlobBuilder::case_insensitive(true).
The corpus is pure ASCII, so Unicode and ASCII case folding coincide.

This script never touches production Rust, manifests, tests, spec.md,
route.md, or evidence.md.  On subprocess failure it prints the captured
stderr to stderr and exits nonzero; it never fabricates a plausible result.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

CRATE = "globset"
VERSION = "0.4.20"

# Shared visible corpus: slash-relative ASCII paths (files and directories,
# no trailing slash, no empty components).  Must stay in sync with
# oracle_globset.py.
CORPUS: list[str] = [
    "a.rs",
    "b.rs",
    "ab.rs",
    "main.rs",
    "a.txt",
    "b.txt",
    "c.txt",
    "ab.txt",
    "x1.txt",
    "x2.txt",
    "xy.txt",
    "literal*.txt",
    "src/main.rs",
    "src/lib.rs",
    "src/a.rs",
    "src/deep/leaf.rs",
    "src/deep/leaf.txt",
    "notes/2024/report.md",
    "notes/2024/01/report.md",
    "notes/todo.md",
    "data/a",
    "data/b",
    "data/ab",
    "data/a/b",
    "data/a/b/c",
    "data/ab/c",
    "README.md",
    "readme.md",
    "Cargo.toml",
    "cargo.toml",
    "img/logo.png",
    "img/logo.PNG",
    "img/logo.jpg",
    "src",
    "src/deep",
    "data",
    "notes",
    "img",
    "a/b",
]

# Named pattern cases: (name, pattern, case_insensitive).  Same order in both
# scripts; output lists cases in this order.
CASES: list[tuple[str, str, bool]] = [
    ("star_component", "*.rs", False),
    ("star_nested_component", "src/*.rs", False),
    ("qmark_single_char", "x?.txt", False),
    ("qmark_component", "data/?", False),
    ("qmark_no_separator", "a?b", False),
    ("class_single", "[abc].txt", False),
    ("class_negated", "[^a].txt", False),
    ("class_range", "x[0-9].txt", False),
    ("recursive_suffix", "src/**", False),
    ("recursive_suffix_data", "data/**", False),
    ("recursive_prefix", "**/*.rs", False),
    ("recursive_middle", "src/**/*.rs", False),
    ("recursive_middle_zero", "notes/**/report.md", False),
    ("alternation", "{a,b}.rs", False),
    ("alternation_components", "{src,data}/*", False),
    ("separator_non_crossing_star", "data/*/c", False),
    ("separator_non_crossing_qmark", "data/?/b/c", False),
    ("escaped_star", "literal\\*.txt", False),
    ("case_insensitive_literal", "readme.md", True),
    ("case_insensitive_star", "img/*.png", True),
]

CARGO_TOML = """\
[package]
name = "globset_probe"
version = "0.1.0"
edition = "2021"

[dependencies]
globset = "=0.4.20"

[profile.dev]
debug = 0
"""

# Verified against globset 0.4.20; generated code must not contain bare
# unwraps.  /*__CORPUS__*/ and /*__CASES__*/ are replaced by the renderer.
RUST_SRC_TEMPLATE = r'''use globset::{GlobBuilder, GlobSetBuilder};

struct Case {
    name: &'static str,
    pattern: &'static str,
    case_insensitive: bool,
}

fn main() {
    let corpus: &[&str] = &[
/*__CORPUS__*/
    ];
    let cases: &[Case] = &[
/*__CASES__*/
    ];

    let mut out = String::new();
    out.push_str("{\"crate\":\"globset\",\"version\":\"0.4.20\",\"cases\":[");
    for (i, case) in cases.iter().enumerate() {
        let glob = GlobBuilder::new(case.pattern)
            .case_insensitive(case.case_insensitive)
            .literal_separator(true)
            .backslash_escape(true)
            .build()
            .unwrap_or_else(|e| panic!("globset rejected pattern {:?}: {}", case.pattern, e));
        let mut set_builder = GlobSetBuilder::new();
        set_builder.add(glob);
        let set = set_builder
            .build()
            .expect("building a set from one valid glob cannot fail");
        let mut matches: Vec<&str> = corpus
            .iter()
            .copied()
            .filter(|path| set.is_match(path))
            .collect();
        matches.sort_unstable();
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        out.push_str(&json_string(case.name));
        out.push_str(",\"pattern\":");
        out.push_str(&json_string(case.pattern));
        out.push_str(",\"case_insensitive\":");
        out.push_str(if case.case_insensitive { "true" } else { "false" });
        out.push_str(",\"matches\":[");
        for (j, m) in matches.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&json_string(m));
        }
        out.push_str("]}");
    }
    out.push_str("]}");
    println!("{}", out);
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
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
    out.push('"');
    out
}
'''


def rust_literal(value: str) -> str:
    """Render an ASCII string as a Rust string literal (quote/backslash only)."""
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def render_main_rs() -> str:
    corpus_block = "\n".join("        " + rust_literal(p) + "," for p in CORPUS)
    cases_block = "\n".join(
        "        Case { name: "
        + rust_literal(name)
        + ", pattern: "
        + rust_literal(pattern)
        + ", case_insensitive: "
        + ("true" if ci else "false")
        + " },"
        for (name, pattern, ci) in CASES
    )
    return (
        RUST_SRC_TEMPLATE.replace("/*__CORPUS__*/", corpus_block).replace(
            "/*__CASES__*/", cases_block
        )
    )


def validate_corpus() -> None:
    if len(CORPUS) != len(set(CORPUS)):
        raise AssertionError("corpus contains duplicate paths")
    for path in CORPUS:
        if not path or path.startswith("/") or path.endswith("/") or "//" in path:
            raise AssertionError(f"corpus path not canonical slash-relative: {path!r}")
        if any(ord(ch) > 127 for ch in path):
            raise AssertionError(f"corpus path must be ASCII: {path!r}")
    for name, pattern, _ci in CASES:
        if any(ord(ch) > 127 for ch in name + pattern):
            raise AssertionError(f"case name/pattern must be ASCII: {name!r}")


def run_probe(tmpdir: Path) -> dict:
    (tmpdir / "src").mkdir()
    (tmpdir / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
    (tmpdir / "src" / "main.rs").write_text(render_main_rs(), encoding="utf-8")
    env = dict(os.environ)
    env["CARGO_HOME"] = str(tmpdir / "cargo-home")
    env["CARGO_TARGET_DIR"] = str(tmpdir / "target")
    try:
        proc = subprocess.run(
            ["cargo", "run", "--quiet", "--manifest-path", str(tmpdir / "Cargo.toml")],
            capture_output=True,
            text=True,
            env=env,
            timeout=1800,
        )
    except subprocess.TimeoutExpired as exc:
        sys.stderr.write("probe_globset: cargo run timed out\n")
        if exc.stderr:
            sys.stderr.write(str(exc.stderr))
        sys.exit(1)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        if not proc.stderr.endswith("\n"):
            sys.stderr.write("\n")
        sys.exit(1)
    try:
        payload = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        sys.stderr.write(f"probe_globset: cargo program emitted invalid JSON: {exc}\n")
        sys.stderr.write(proc.stdout)
        sys.exit(1)
    return payload


def main() -> None:
    validate_corpus()
    corpus_set = set(CORPUS)
    with tempfile.TemporaryDirectory(prefix="rfs-vl0u-globset-") as tmp:
        payload = run_probe(Path(tmp))
    # Validate shape and content before reporting anything.
    if payload.get("crate") != CRATE or payload.get("version") != VERSION:
        sys.stderr.write(
            f"probe_globset: unexpected crate/version in payload: {payload!r}\n"
        )
        sys.exit(1)
    cases = payload.get("cases")
    if not isinstance(cases, list) or len(cases) != len(CASES):
        sys.stderr.write(f"probe_globset: unexpected case count: {cases!r}\n")
        sys.exit(1)
    for expected, got in zip(CASES, cases):
        name, pattern, ci = expected
        if (
            not isinstance(got, dict)
            or got.get("name") != name
            or got.get("pattern") != pattern
            or got.get("case_insensitive") is not ci
            or not isinstance(got.get("matches"), list)
        ):
            sys.stderr.write(
                f"probe_globset: case mismatch for {name!r}: {got!r}\n"
            )
            sys.exit(1)
        matches = got["matches"]
        if any(not isinstance(m, str) for m in matches):
            sys.stderr.write(f"probe_globset: non-string match in {name!r}\n")
            sys.exit(1)
        if matches != sorted(matches):
            sys.stderr.write(f"probe_globset: matches not sorted in {name!r}\n")
            sys.exit(1)
        if len(matches) != len(set(matches)):
            sys.stderr.write(f"probe_globset: duplicate matches in {name!r}\n")
            sys.exit(1)
        if any(m not in corpus_set for m in matches):
            sys.stderr.write(f"probe_globset: match outside corpus in {name!r}\n")
            sys.exit(1)
    out = {
        "crate": CRATE,
        "version": VERSION,
        "cases": [
            {
                "name": name,
                "pattern": pattern,
                "case_insensitive": ci,
                "matches": got["matches"],
            }
            for (name, pattern, ci), got in zip(CASES, cases)
        ],
    }
    print(json.dumps(out, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
