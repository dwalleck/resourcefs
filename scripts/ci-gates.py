#!/usr/bin/env python3
"""Run the local/CI gates: python scripts/ci-gates.py (Python 3.9+, cargo-deny).

Ignored tests are inventoried from compiled release harnesses, not source text.
Unknown or missing ignored rows fail closed; `live_*` smokes (run by
scripts/live-smoke.sh) and the child-server hosts are excluded, never run here.

Release tests run one target at a time, serial by default, so that a wall-clock
assertion anywhere in a target keeps a quiet machine. Targets cleared to run
parallel are named, with their clearing inspection, in PARALLEL_RELEASE_TARGETS.
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
# Release-mode test targets that may run with default test threads. The default
# is the other way round: every target not named here runs serial, because one
# wall-clock assertion anywhere in it flakes under contention, and no source scan
# can prove a target free of one (assertions span lines, hide behind helpers, and
# need not spell `elapsed`). Each entry therefore records the inspection that
# cleared it, and a target added later is serial until someone clears it too.
PARALLEL_RELEASE_TARGETS = {
    "atlassian_fixture_operator_contract":
        "fixture-lifecycle contract: 78 tests driving a TLS fixture through create/verify/cleanup, no wall-clock assertion in the target",
}
# Fixed rather than per-machine: each of those tests opens its own fixture
# listener, and pinning the width keeps the local and hosted runs the same shape
# instead of fanning out to whatever the workstation has.
PARALLEL_TEST_THREADS = 4
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


def release_test_targets():
    """Every compiled release test target, as (package_id, name, kind, executable).

    Enumerated from cargo's own artifacts rather than from source text, so the
    inventory is what is built and runnable. Returns None when the build failed.
    """
    result = run([
        "cargo", "test", *RELEASE, "--workspace", "--all-features",
        "--no-run", "--message-format=json",
    ], capture=True)
    if result.returncode:
        print(result.stdout, end="")
        return None
    targets = []
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
            return None
        target = artifact["target"]
        targets.append((artifact["package_id"], target["name"], target["kind"], executable))
    return targets


def target_selector(kind, name):
    """The cargo selector that runs one target, or None when it has none."""
    if "test" in kind:
        return ["--test", name]
    if "lib" in kind:
        return ["--lib"]
    if "bin" in kind:
        return ["--bin", name]
    return None


def release_workspace():
    """Release tests, serial except for the targets cleared to run parallel.

    Serial is the default: a target runs with `--test-threads=1` unless it is
    named in PARALLEL_RELEASE_TARGETS. Doctests are not compiler artifacts, so
    they run in their own serial pass rather than being lost.
    """
    targets = release_test_targets()
    if targets is None:
        return False
    passed = True
    for package_id, name, kind, _executable in targets:
        selector = target_selector(kind, name)
        if selector is None:
            print(f"Unsupported release test target: {name}", file=sys.stderr)
            passed = False
            continue
        if name in PARALLEL_RELEASE_TARGETS:
            threads = ["--", f"--test-threads={PARALLEL_TEST_THREADS}"]
        else:
            threads = ["--", "--test-threads=1"]
        result = run([
            "cargo", "test", *RELEASE, "-p", package_id, "--all-features",
            *selector, "--no-fail-fast", *threads,
        ])
        passed = result.returncode == 0 and passed
    doctests = run([
        "cargo", "test", *RELEASE, "--workspace", "--all-features",
        "--doc", "--", "--test-threads=1",
    ])
    return doctests.returncode == 0 and passed


def ignored_budgets():
    targets = release_test_targets()
    if targets is None:
        return False
    seen = set()
    passed = True
    for package_id, target_name, kind, executable in targets:
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
            selector = target_selector(kind, target_name)
            if selector is None:
                print(f"Unsupported ignored-test target: {target_name}", file=sys.stderr)
                passed = False
                continue
            result = run([
                "cargo", "test", *RELEASE, "-p", package_id,
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
        ("Release workspace", release_workspace),
        ("Ignored production budgets", ignored_budgets),
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
            passed = command() if callable(command) else run(command).returncode == 0
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
