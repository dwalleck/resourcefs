#!/usr/bin/env python3
"""Run the local/CI gates: python scripts/ci-gates.py (Python 3.9+, cargo-deny).

The functional suite runs once, in debug. Release mode runs only the production
budgets: every test a RELEASE_FILTERS substring selects, ignored rows included,
in one build and one serialized invocation so timing assertions measure
optimized code without neighbours. A source oracle fails closed when a test
scales an assertion on `debug_assertions` but no release filter selects it, so
a budget cannot silently drop out of the release pass.

Ignored tests are inventoried from compiled release harnesses, not source text.
Unknown or missing ignored rows fail closed; live tests and the child-server
entry point are never enabled. Add new ignored rows to the appropriate set.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
# libtest substring filters that select the release pass. Every production
# budget, ignored or not, must match one of these; `release_selection` proves it.
RELEASE_FILTERS = ("budget",)
BUDGETS = {
    "reference_parse_budget",
    "operation_journal_budget",
    "profile::model::tests::github_profile_validation_budget",
    "http::tests::retry_policy_budget",
    "github_pagination_cache_budget",
    "github_search_budget",
    "github_single_resource_render_budget",
    "github_cache_namespace_removal_budget",
    "creation_document_budget",
    "github_wire_decode_budget",
    "http_mutation_request_budget",
    "http_metadata_budget",
}
EXCLUDED = {
    "live_stdio_profile_probe_serve_and_tools_hold_up": "live GitHub/stdio smoke",
    "live_github_reads_hold_up": "live GitHub smoke",
    "live_https_reads_hold_up": "live HTTPS smoke",
    "live_jira_issue_read": "live Jira smoke",
    "live_jira_project_browse": "live Jira project smoke",
    "live_jira_browse": "live Jira project and issue browse smoke",
    "live_jira_jql": "live Jira native JQL smoke",
    "server::jira_query_tests::live_jira_query_stdio": "live Jira native JQL stdio smoke",
    "server::jira_query_tests::jira_query_stdio_test_host": "child server, invoked by functional tests",
    "profile_https_test_server": "child server, invoked by functional tests",
}
# Both command-line config and environment are explicit: local Cargo profile
# overrides must not silently select debug-scaled production assertions.
RELEASE = ["--release", "--config", "profile.release.debug-assertions=false"]
ENV = dict(os.environ, CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS="false")
# A test whose bound depends on the build profile. `cfg!(debug_assertions)`,
# `#[cfg(debug_assertions)]`, and `#[cfg(not(debug_assertions))]` all count.
DEBUG_SCALED = re.compile(r"cfg!?\((?:not\()?debug_assertions")
FN = re.compile(r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z0-9_]+)")


def run(command, *, capture=False):
    print("+ " + " ".join(map(str, command)), flush=True)
    return subprocess.run(
        command, cwd=ROOT, env=ENV, check=False,
        stdout=subprocess.PIPE if capture else None, text=True, encoding="utf-8",
    )


def selected(name):
    return any(needle in name for needle in RELEASE_FILTERS)


def release_selection():
    """The release filters select every budget and nothing that must stay off."""
    passed = True
    for name in sorted(BUDGETS):
        if not selected(name):
            print(f"Ignored budget {name} matches no release filter {RELEASE_FILTERS}.", file=sys.stderr)
            passed = False
    for name in sorted(EXCLUDED):
        if selected(name):
            print(f"Excluded row {name} matches a release filter and would run.", file=sys.stderr)
            passed = False
    for path in sorted(ROOT.glob("crates/**/*.rs")):
        enclosing = None
        closing = None
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            match = FN.match(line)
            if match and not line.rstrip().endswith("}"):
                enclosing = match.group(2)
                closing = match.group(1) + "}"
            elif line.rstrip() == closing:
                enclosing = closing = None
            if DEBUG_SCALED.search(line) and not (enclosing and selected(enclosing)):
                owner = f"`{enclosing}`" if enclosing else "a module-level item"
                print(
                    f"{path.relative_to(ROOT)}:{number}: {owner} scales on debug_assertions "
                    f"but no release filter {RELEASE_FILTERS} selects it; name the test after a "
                    "filter or extend RELEASE_FILTERS.",
                    file=sys.stderr,
                )
                passed = False
    return passed


def release_inventory():
    """Classify every ignored row of the compiled release harnesses; run nothing."""
    result = run([
        "cargo", "test", *RELEASE, "--workspace", "--all-features",
        "--no-run", "--message-format=json",
    ], capture=True)
    if result.returncode:
        print(result.stdout, end="")
        return False
    seen = set()
    passed = True
    for line in result.stdout.splitlines():
        artifact = json.loads(line)
        if artifact.get("reason") != "compiler-artifact":
            continue
        executable = artifact.get("executable")
        if not executable or not artifact["profile"]["test"]:
            continue
        if artifact["profile"]["debug_assertions"]:
            print(
                f"Release test target {artifact['target']['name']} has debug assertions enabled; remove the profile override.",
                file=sys.stderr,
            )
            passed = False
            continue
        listing = run([executable, "--ignored", "--list", "--format=terse"], capture=True)
        if listing.returncode:
            print(listing.stdout, end="")
            passed = False
            continue
        names = [line.removesuffix(": test") for line in listing.stdout.splitlines()
                 if line.endswith(": test")]
        for name in names:
            seen.add(name)
            if name in EXCLUDED:
                print(f"Excluded {name}: {EXCLUDED[name]}", flush=True)
            elif name not in BUDGETS:
                print(f"Unclassified ignored test: {name}", file=sys.stderr)
                passed = False
    missing = BUDGETS - seen
    if missing:
        print(f"Missing production budget rows: {sorted(missing)}", file=sys.stderr)
    return passed and not missing


def release_budgets():
    """Every budget, ignored rows included, optimized and serialized, in one build."""
    return run([
        "cargo", "test", *RELEASE, "--workspace", "--all-features", "--no-fail-fast",
        "--", *RELEASE_FILTERS, "--include-ignored", "--test-threads=1",
    ]).returncode == 0


def main():
    gates = [
        ("Formatting", ["cargo", "fmt", "--all", "--", "--check"]),
        ("Release selection", release_selection),
        ("Lints", ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"]),
        ("Functional tests", ["cargo", "test", "--workspace", "--all-features", "--no-fail-fast"]),
        ("Release inventory", release_inventory),
        ("Release budgets", release_budgets),
        ("Dependency vetting", ["cargo", "deny", "check"]),
    ]
    if sys.platform == "linux":
        gates.insert(0, (
            "Module placement",
            [sys.executable, ".rfs-nae2/oracles/module_shape.py", "--stage", "query"],
        ))
    failed = []
    for name, gate in gates:
        print(f"\n=== {name} ===", flush=True)
        started = time.monotonic()
        try:
            passed = gate() if callable(gate) else run(gate).returncode == 0
        except (OSError, ValueError, KeyError) as error:
            print(f"{name}: {error}", file=sys.stderr)
            passed = False
        verdict = "passed" if passed else "FAILED"
        print(f"=== {name} {verdict} in {time.monotonic() - started:.0f}s ===", flush=True)
        if not passed:
            failed.append(name)
    if failed:
        print(f"Failed gates: {', '.join(failed)}", file=sys.stderr)
        return 1
    print("All repository gates passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
