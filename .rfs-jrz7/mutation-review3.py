#!/usr/bin/env python3
"""A/B the reviewed placement gate against the current one on one injected row.

Each case writes a fresh ledger from the pristine copy of its side, injects
exactly one policy row, runs both gates over the same source tree and prints
only the failures that name that row's symbols or paths. The reviewed side is
`scripts/module_shape*.py` plus `scripts/module-ledger.json` at the commit that
introduced the mechanism under test; the repaired side is the working tree's.

    python3 .rfs-jrz7/mutation-review3.py [--reviewed 6e1c55c] [--repository PATH]

The source cases mutate a disposable copy of `crates/`, never the repository.
"""
import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE_FILES = ("module_shape.py", "module_shape_base.py")

CASES = {
    "M1 pure move of a policy-tracked helper": (
        ("fetch_bounded_attempts", "http/read.rs"), {
        "relocated_symbols": {"immutable-grammar": {
            "fetch_bounded_attempts": {
                "name": "fetch_bounded_attempts",
                "owner": "crates/resourcefs-sources/src/http/read.rs"}}}}),
    "M2 pure move to an owner outside the watch list": (
        ("canonical_identity",), {
        "relocated_symbols": {"immutable-grammar": {
            "canonical_identity": {
                "name": "canonical_identity",
                "owner": "crates/resourcefs-core/src/discovery.rs"}}}}),
    "M3 pure move of a symbol the policy does not track": (
        ("parse_pull_request_reference",), {
        "relocated_symbols": {"immutable-grammar": {
            "parse_pull_request_reference": {
                "name": "parse_pull_request_reference",
                "owner": "crates/resourcefs-core/src/reference/pull.rs"}}}}),
    "M4 retired name still declared": (
        ("encode_rfc3986_segment",), {
        "relocated_symbols": {"immutable-grammar": {
            "encode_rfc3986_segment": {
                "name": "encode_rfc3986_segment_renamed",
                "owner": "crates/resourcefs-core/src/reference.rs"}}}}),
    "M5 owner does not declare the moved symbol": (
        ("parse_jira_address",), {
        "relocated_symbols": {"immutable-grammar": {
            "parse_jira_address": {
                "name": "parse_jira_address",
                "owner": "crates/resourcefs-core/src/discovery.rs"}}}}),
    "M6 protected parent present now, absent at its pin": (
        ("reference/github.rs",), {
        "protected_parents": {"crates/resourcefs-core/src/reference/github.rs": {
            "baseline": "635ab170ab57542c18272921298d575da2f8b08a",
            "stage": "immutable-grammar",
            "changes": {}}}}),
}
SOURCE_CASES = {
    "M7 inline cfg(test) copy in an owner file": (
        "parse_pull_request_reference",
        ("crates/resourcefs-core/src/reference/pull.rs",
         "\n#[cfg(test)]\nmod zz_probe {\n    fn parse_pull_request_reference() {}\n}\n")),
    "M8 divergent same-name copy in the owner's directory": (
        "parse_pull_request_reference",
        ("crates/resourcefs-core/src/reference/source_page.rs",
         "\nfn parse_pull_request_reference() -> u8 { 7 }\n")),
    "M9 trait bearing an owned name in the owner file": (
        "GithubCommitId",
        ("crates/resourcefs-core/src/reference/github.rs",
         "\nmod zz_probe {\n    trait GithubCommitId {}\n}\n")),
}


def show(repository, revision, path):
    return subprocess.run(["git", "-C", str(repository), "show", f"{revision}:{path}"],
                          capture_output=True, text=True, check=True).stdout


def stage(repository, reviewed, scratch):
    """Pristine gate copies: the reviewed revision and the working tree."""
    for side in ("reviewed", "current"):
        (scratch / side).mkdir(parents=True)
    for name in GATE_FILES:
        (scratch / "reviewed" / name).write_text(show(repository, reviewed, f"scripts/{name}"))
        shutil.copy2(repository / "scripts" / name, scratch / "current" / name)
    (scratch / "reviewed" / "module-ledger.json").write_text(
        show(repository, reviewed, "scripts/module-ledger.json"))
    shutil.copy2(repository / "scripts" / "module-ledger.json",
                 scratch / "current" / "module-ledger.json")


def fresh(scratch, side, rows):
    """A per-case copy of one side with exactly the case's rows injected."""
    target = scratch / f"{side}-case"
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(scratch / side, target)
    data = json.loads((target / "module-ledger.json").read_text())
    for stage_name, row in rows.get("relocated_symbols", {}).items():
        data.setdefault("relocated_symbols", {}).setdefault(stage_name, {}).update(row)
    for path, row in rows.get("protected_parents", {}).items():
        data.setdefault("protected_parents", {})[path] = row
    (target / "module-ledger.json").write_text(json.dumps(data, indent=2) + "\n")
    return target


def run(gate, repository, root, needles):
    result = subprocess.run(
        [sys.executable, str(gate / "module_shape.py"),
         "--root", str(root), "--repository", str(repository)],
        capture_output=True, text=True)
    text = result.stdout + result.stderr
    lines = [line for line in text.splitlines()
             if " FAIL " in line and any(needle in line for needle in needles)]
    return ("PASS" if result.returncode == 0 else "FAIL"), lines


def report(name, needles, reviewed, current):
    print(f"### {name}")
    for label, (verdict, lines) in (("reviewed", reviewed), ("repaired", current)):
        print(f"  {label}: {verdict}")
        for line in lines:
            print(f"       {line}")
    print()


def source_copy(repository, scratch):
    copy = scratch / "source"
    if copy.exists():
        shutil.rmtree(copy)
    (copy / "crates").mkdir(parents=True)
    for crate in sorted((repository / "crates").glob("*/src")):
        shutil.copytree(crate.parent, copy / "crates" / crate.parent.name)
    return copy


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, default=ROOT,
                        help="worktree holding the repaired gate and the source tree")
    parser.add_argument("--reviewed", default="6e1c55c",
                        help="revision whose gate is the reviewed side")
    parser.add_argument("--only", help="run one case by its number prefix")
    args = parser.parse_args()
    repository = args.repository.resolve()
    with tempfile.TemporaryDirectory(prefix="module-shape-ab-") as temporary:
        scratch = Path(temporary)
        stage(repository, args.reviewed, scratch)
        for name, (needles, rows) in CASES.items():
            if args.only and not name.startswith(args.only):
                continue
            report(name, needles,
                   run(fresh(scratch, "reviewed", rows), repository, repository, needles),
                   run(fresh(scratch, "current", rows), repository, repository, needles))
        copy = source_copy(repository, scratch)
        for name, (needles, (target, addition)) in SOURCE_CASES.items():
            if args.only and not name.startswith(args.only):
                continue
            path = copy / target
            original = path.read_text()
            try:
                path.write_text(original + addition)
                report(name, (needles,),
                       run(fresh(scratch, "reviewed", {}), repository, copy, (needles,)),
                       run(fresh(scratch, "current", {}), repository, copy, (needles,)))
            finally:
                path.write_text(original)


if __name__ == "__main__":
    main()
