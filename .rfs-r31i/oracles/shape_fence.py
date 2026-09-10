#!/usr/bin/env python3
"""Module-shape fence for rfs-r31i (design claims C16 and C29).

Checks the approved module ledger in .rfs-r31i/design.md against the real
source tree and diff. Reports the claim ID and the exact path/symbol/delta it
rejects, so a buggy placement mutation turns this red with a localized reason.
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GITHUB = ROOT / "crates/resourcefs-sources/src/github"
FACTS = GITHUB / "facts"
FACTS_RS = GITHUB / "facts.rs"
CORE_TESTS = ROOT / "crates/resourcefs-core/tests/github_reference_contract.rs"
SOURCE_TESTS = ROOT / "crates/resourcefs-sources/tests/github_facts_contract.rs"

REQUIRED = [
    FACTS / "family.rs",
    FACTS / "parent.rs",
    FACTS / "review.rs",
    FACTS / "inline.rs",
]

# (claim, path, forbidden regex, why)
FORBIDDEN = [
    (
        "C16",
        FACTS / "collection.rs",
        r"\b(comment|review|inline)::",
        "the collection engine must reach families only through `family::`",
    ),
    (
        "C16",
        FACTS / "comment.rs",
        r"\bstruct ParentFacts\b|\bfn fetch_parent\b|\bfn parent_facts\b",
        "parent acquisition and projection belong to facts/parent.rs",
    ),
    (
        "C16",
        FACTS / "review.rs",
        r"\bCollectionStop\b|\bMAX_COLLECTION_RECORDS\b|\bCollectionFamily\b|\bfacts_page\b",
        "traversal, admission and coverage belong to facts/collection.rs",
    ),
    (
        "C16",
        FACTS / "inline.rs",
        r"\bCollectionStop\b|\bMAX_COLLECTION_RECORDS\b|\bCollectionFamily\b|\bfacts_page\b",
        "traversal, admission and coverage belong to facts/collection.rs",
    ),
    (
        "C29",
        FACTS_RS,
        r"\bNativeReview\b|\bNativeInlineComment\b|\bfn validate_record\b",
        "facts.rs is a protected parent: declarations and dispatch arms only",
    ),
]

# Required test locations: each fence the plan names must exist.
REQUIRED_TESTS = [
    (CORE_TESTS, "review_and_inline_facts_routes_and_cursor_scope", "C2/C3"),
    (SOURCE_TESTS, "review_facts_collection_and_item_preserve_native_facts", "C5/C6"),
    (SOURCE_TESTS, "inline_facts_preserve_native_anchor_presence", "C7-C12"),
    (
        SOURCE_TESTS,
        "review_and_inline_identity_contradictions_reject_the_component",
        "C13-C15",
    ),
]

LEDGER_PATHS = {
    "crates/resourcefs-core/src/reference.rs",
    "crates/resourcefs-core/src/reference/pull.rs",
    "crates/resourcefs-sources/src/github/facts.rs",
    "crates/resourcefs-sources/src/github/facts/family.rs",
    "crates/resourcefs-sources/src/github/facts/parent.rs",
    "crates/resourcefs-sources/src/github/facts/collection.rs",
    "crates/resourcefs-sources/src/github/facts/comment.rs",
    "crates/resourcefs-sources/src/github/facts/review.rs",
    "crates/resourcefs-sources/src/github/facts/inline.rs",
    "crates/resourcefs-sources/src/github/facts/continuation.rs",
    "crates/resourcefs-sources/src/github/facts/identity.rs",
    "crates/resourcefs-sources/src/github/mod.rs",
}

failures = []


def fail(claim, detail):
    failures.append(f"{claim}: {detail}")


def git(*args):
    return subprocess.run(
        ["git", "-C", str(ROOT), *args], capture_output=True, text=True, check=True
    ).stdout


def change_base():
    """The revision this change started from.

    Discovered from the commit that first added the family dispatch module, so
    the fence never hard-codes a branch and survives rebases. Before the first
    commit it is HEAD, which makes the working tree the delta under test.
    """
    added = git(
        "log", "--diff-filter=A", "--format=%H", "--",
        "crates/resourcefs-sources/src/github/facts/family.rs",
    ).strip().splitlines()
    if not added:
        return git("rev-parse", "HEAD").strip()
    return git("rev-parse", f"{added[-1]}^").strip()


for path in REQUIRED:
    if not path.exists():
        fail("C29", f"{path.relative_to(ROOT)} is missing from the approved ledger")

for claim, path, pattern, why in FORBIDDEN:
    if not path.exists():
        fail(claim, f"{path.relative_to(ROOT)} is missing")
        continue
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if re.search(pattern, line):
            fail(claim, f"{path.relative_to(ROOT)}:{number} matches {pattern!r}: {why}")

facts_rs = FACTS_RS
for module in ("family", "parent", "review", "inline"):
    if not re.search(rf"^mod {module};$", facts_rs.read_text(), re.MULTILINE):
        fail("C29", f"facts.rs must declare `mod {module};` privately")

for path, name, claims in REQUIRED_TESTS:
    if name not in path.read_text():
        fail(claims, f"{path.relative_to(ROOT)} lacks fence `{name}`")

# Protected parent: github/mod.rs production delta is the catalog grammar line.
merge_base = change_base()
for line in git("diff", "--unified=0", merge_base, "--", "crates/resourcefs-sources/src/github/mod.rs").splitlines():
    if line.startswith(("+++", "---", "@@")):
        continue
    if line.startswith(("+", "-")) and "pr://<owner>/<repository>" not in line:
        fail("C29", f"github/mod.rs protected-parent delta is not the grammar line: {line.strip()!r}")

# No production file outside the approved ledger.
changed = [
    line
    for line in git("diff", "--name-only", merge_base, "--", "crates").splitlines()
    if line.endswith(".rs")
]
for path in changed:
    if path not in LEDGER_PATHS and "/tests/" not in path:
        fail("C29", f"{path} is production code outside the approved module ledger")

if failures:
    print("shape_fence: FAIL")
    for failure in failures:
        print(f"  {failure}")
    sys.exit(1)
print("shape_fence: PASS")
