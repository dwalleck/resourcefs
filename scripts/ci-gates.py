#!/usr/bin/env python3
"""Run the local/CI gates: python scripts/ci-gates.py (Python 3.9+, cargo-deny).

Ignored tests are inventoried from compiled release harnesses, not source text.
Unknown or missing ignored rows fail closed; `live_*` smokes (run by
scripts/live-smoke.sh) and the child-server hosts are excluded, never run here.
"""

import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
# Budgets stay a literal inventory: below, `missing = BUDGETS - seen` is what
# proves every row is still compiled and executed, and no naming convention can
# do that. Live smokes instead follow the `live_*` convention AGENTS.md
# mandates, so a new smoke row needs no edit here.
BUDGETS = {
    "reference_parse_budget",
    "immutable_reference_parse_budget",
    "immutable_commit_production_budget",
    "immutable_source_production_budget",
    "operation_journal_budget",
    "profile::model::tests::github_profile_validation_budget",
    "http::tests::retry_policy_budget",
    "github_pagination_cache_budget",
    "github_search_budget",
    "github_single_resource_render_budget",
    "github_cache_namespace_removal_budget",
    "github_facts_production_budget",
    "github_collection_production_budget",
    "creation_document_budget",
    "github_wire_decode_budget",
    "http_mutation_request_budget",
    "http_metadata_budget",
}
# The only ignored rows outside the live_* convention: child servers that
# functional tests spawn and drive themselves.
CHILD_SERVERS = {
    "server::jira_query_tests::jira_query_stdio_test_host": "child server, invoked by functional tests",
    "profile_https_test_server": "child server, invoked by functional tests",
}
# Both command-line config and environment are explicit: local Cargo profile
# overrides must not silently select debug-scaled production assertions.
RELEASE = ["--release", "--config", "profile.release.debug-assertions=false"]
ENV = dict(os.environ, CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS="false")


def exclusion_reason(name):
    """Why `name` is skipped instead of run, or None when it must be run.

    Budget membership is checked first: the literal set this replaced was
    consulted before `BUDGETS`, so a name appearing in both was skipped
    silently and never executed.
    """
    if name in BUDGETS:
        return None
    if name in CHILD_SERVERS:
        return CHILD_SERVERS[name]
    if name.startswith("live_") or "::live_" in name:
        return "live smoke, run by scripts/live-smoke.sh outside the gates"
    return None


def run(command, *, capture=False):
    print("+ " + " ".join(map(str, command)), flush=True)
    return subprocess.run(
        command, cwd=ROOT, env=ENV, check=False,
        stdout=subprocess.PIPE if capture else None, text=True, encoding="utf-8",
    )


def ignored_budgets():
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
            reason = exclusion_reason(name)
            if reason is not None:
                print(f"Excluded {name}: {reason}", flush=True)
                continue
            if name not in BUDGETS:
                print(f"Unclassified ignored test: {name}", file=sys.stderr)
                passed = False
                continue
            target = artifact["target"]
            kind = target["kind"]
            if "test" in kind:
                selector = ["--test", target["name"]]
            elif "lib" in kind:
                selector = ["--lib"]
            elif "bin" in kind:
                selector = ["--bin", target["name"]]
            else:
                print(f"Unsupported ignored-test target: {target}", file=sys.stderr)
                passed = False
                continue
            result = run([
                "cargo", "test", *RELEASE, "-p", artifact["package_id"],
                "--all-features", *selector, "--", "--ignored", "--exact", name,
                "--test-threads=1",
            ])
            passed = result.returncode == 0 and passed
    missing = BUDGETS - seen
    if missing:
        print(f"Missing production budget rows: {sorted(missing)}", file=sys.stderr)
    return passed and not missing


def main():
    gates = [
        ("Formatting", ["cargo", "fmt", "--all", "--", "--check"]),
        ("Lints", ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"]),
        ("Functional tests", ["cargo", "test", "--workspace", "--all-features", "--no-fail-fast"]),
        ("Release workspace", ["cargo", "test", *RELEASE, "--workspace", "--all-features", "--no-fail-fast", "--", "--test-threads=1"]),
        ("Ignored production budgets", None),
        ("Dependency vetting", ["cargo", "deny", "check"]),
    ]
    if sys.platform == "linux":
        gates.insert(0, (
            "Module placement",
            [sys.executable, "scripts/module_shape.py"],
        ))
    failed = []
    for name, command in gates:
        print(f"\n=== {name} ===", flush=True)
        try:
            passed = ignored_budgets() if command is None else run(command).returncode == 0
        except (OSError, ValueError, KeyError) as error:
            print(f"{name}: {error}", file=sys.stderr)
            passed = False
        if not passed:
            failed.append(name)
    if failed:
        print(f"Failed gates: {', '.join(failed)}", file=sys.stderr)
        return 1
    print("All repository gates passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
