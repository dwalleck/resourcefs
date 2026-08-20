#!/usr/bin/env python3
"""Independent oracle for globset 0.4.20 matching semantics (rfs-vl0u prove-it).

Reports a manually enumerated expected table for the shared corpus and named
pattern cases.  It imports no glob/regex/fnmatch machinery and computes no
matches: the table below was derived by hand from the adopted globset 0.4.20
syntax (component `*`, `?`, character classes, recursive `**`, `{a,b}`
alternation, separator non-crossing via literal_separator(true), ASCII
case-insensitive mode via case_insensitive(true), fully anchored whole-path
matching).  The corpus is pure ASCII, so Unicode and ASCII case folding
coincide.

Assertions guard the table: every expected path belongs to the shared corpus,
every named case is present, and lists are sorted and duplicate-free.
Output JSON is byte-identical to probe_globset.py's when globset honors the
adopted syntax.
"""

from __future__ import annotations

import json
import sys

CRATE = "globset"
VERSION = "0.4.20"

# Shared visible corpus: slash-relative ASCII paths.  Must stay in sync with
# probe_globset.py.
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

# Manually enumerated expected matches per named case (sorted ascending,
# bytewise, which equals Python string order for ASCII).  NOT computed by any
# matching machinery.
EXPECTED: dict[str, list[str]] = {
    # `*` matches any run of characters inside one component: root-level .rs
    # files only; src/... is excluded because `*` never matches `/`.
    "star_component": ["a.rs", "ab.rs", "b.rs", "main.rs"],
    # `*` inside src/: src/deep/leaf.rs needs a second component, no match.
    "star_nested_component": ["src/a.rs", "src/lib.rs", "src/main.rs"],
    # `?` matches exactly one character: x1.txt, x2.txt, and xy.txt.
    "qmark_single_char": ["x1.txt", "x2.txt", "xy.txt"],
    # One-character entries directly under data/.
    "qmark_component": ["data/a", "data/b"],
    # `?` never matches `/`: a/b must not match a?b.
    "qmark_no_separator": [],
    # Class of one of a, b, c, then .txt: exactly one character before .txt.
    "class_single": ["a.txt", "b.txt", "c.txt"],
    # Negated class: one character that is not a, then .txt.
    "class_negated": ["b.txt", "c.txt"],
    # Range class [0-9]: x1.txt, x2.txt; xy.txt has a non-digit.
    "class_range": ["x1.txt", "x2.txt"],
    # `**` crosses separators and matches directories too, but never the
    # bare base `src` itself (a trailing `/**` needs the separator).
    "recursive_suffix": [
        "src/a.rs",
        "src/deep",
        "src/deep/leaf.rs",
        "src/deep/leaf.txt",
        "src/lib.rs",
        "src/main.rs",
    ],
    "recursive_suffix_data": [
        "data/a",
        "data/a/b",
        "data/a/b/c",
        "data/ab",
        "data/ab/c",
        "data/b",
    ],
    # Leading `**/` matches zero or more components, so root-level .rs files
    # match as well.
    "recursive_prefix": [
        "a.rs",
        "ab.rs",
        "b.rs",
        "main.rs",
        "src/a.rs",
        "src/deep/leaf.rs",
        "src/lib.rs",
        "src/main.rs",
    ],
    # `**` between src/ and *.rs: all .rs files under src/.
    "recursive_middle": ["src/a.rs", "src/deep/leaf.rs", "src/lib.rs", "src/main.rs"],
    # `**` matches zero or more middle components: report.md at both depths.
    "recursive_middle_zero": ["notes/2024/01/report.md", "notes/2024/report.md"],
    # {a,b} alternation: exactly a.rs or b.rs, not ab.rs.
    "alternation": ["a.rs", "b.rs"],
    # {src,data}/`*`: one component under src/ or data/; data/a/b and
    # src/deep/leaf.rs need an extra component, so they do not match.
    "alternation_components": [
        "data/a",
        "data/ab",
        "data/b",
        "src/a.rs",
        "src/deep",
        "src/lib.rs",
        "src/main.rs",
    ],
    # `*` never crosses `/`: only data/ab/c has exactly one component
    # between data/ and c.
    "separator_non_crossing_star": ["data/ab/c"],
    # `?` never crosses `/` and matches exactly one character: only a.
    "separator_non_crossing_qmark": ["data/a/b/c"],
    # Backslash escapes `*` on every platform.
    "escaped_star": ["literal*.txt"],
    # case_insensitive(true) folds ASCII literals: both README.md spellings.
    "case_insensitive_literal": ["README.md", "readme.md"],
    # case_insensitive(true) folds ASCII in `*`-matched content: logo.PNG.
    "case_insensitive_star": ["img/logo.PNG", "img/logo.png"],
}


def build_output() -> dict:
    """Canonical output object; identical shape to probe_globset.py."""
    return {
        "crate": CRATE,
        "version": VERSION,
        "cases": [
            {
                "name": name,
                "pattern": pattern,
                "case_insensitive": ci,
                "matches": list(EXPECTED[name]),
            }
            for (name, pattern, ci) in CASES
        ],
    }


def main() -> None:
    errors: list[str] = []

    if len(CORPUS) != len(set(CORPUS)):
        errors.append("corpus contains duplicate paths")
    for path in CORPUS:
        if not path or path.startswith("/") or path.endswith("/") or "//" in path:
            errors.append(f"corpus path not canonical slash-relative: {path!r}")
        if any(ord(ch) > 127 for ch in path):
            errors.append(f"corpus path must be ASCII: {path!r}")
    for name, pattern, _ci in CASES:
        if any(ord(ch) > 127 for ch in name + pattern):
            errors.append(f"case name/pattern must be ASCII: {name!r}")

    corpus_set = set(CORPUS)

    # Assertion: every expected path belongs to the shared corpus.
    for name, paths in EXPECTED.items():
        for path in paths:
            if path not in corpus_set:
                errors.append(f"{name}: expected path {path!r} is not in the corpus")

    # Assertion: expected lists are sorted and duplicate-free.
    for name, paths in EXPECTED.items():
        if len(paths) != len(set(paths)):
            errors.append(f"{name}: duplicate expected paths")
        if paths != sorted(paths):
            errors.append(f"{name}: expected paths not sorted ascending")

    # Assertion: every named case is present, and nothing extra.
    case_names = [name for (name, _pattern, _ci) in CASES]
    if set(EXPECTED) != set(case_names):
        errors.append(
            "case name mismatch: "
            f"expected={sorted(EXPECTED)} cases={sorted(case_names)}"
        )

    if errors:
        for error in errors:
            sys.stderr.write(f"oracle_globset: {error}\n")
        sys.exit(1)

    print(json.dumps(build_output(), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
