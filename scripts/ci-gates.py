#!/usr/bin/env python3
"""Run the local/CI gates: python scripts/ci-gates.py (Python 3.9+, cargo-deny).

Ignored tests are inventoried from compiled release harnesses, not source text.
Unknown or missing ignored rows fail closed; `live_*` smokes (run by
scripts/live-smoke.sh) and the child-server hosts are excluded, never run here.

Release tests run one target at a time, serial by default, so that a wall-clock
assertion anywhere in a target keeps a quiet machine. Targets cleared to run
parallel are named, with their clearing inspection, in PARALLEL_RELEASE_TARGETS.
"""

import argparse
import json
import os
import re
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
# Test targets this run skips, as a comma-separated RFS_SKIP_TEST_TARGETS. Empty
# by default, so the local gate and every `main` build run everything.
#
# CI sets it on macOS for pull requests only. That platform's 60-minute leg was
# the matrix's critical path and `atlassian_fixture_operator_contract` -- 78
# tests driving a create/verify/cleanup lifecycle through a real subprocess --
# was most of it. Skipping it on pull requests buys the whole gap back on the
# path that gates a review.
#
# It still runs on macOS when `main` builds, and that is deliberate rather than
# leftover: the workflow installs brew bash on macOS *for this contract*,
# because `scripts/atlassian-fixture-bootstrap.sh` needs Bash 4.4+ and the
# system ships 3.2. This target is the only thing proving that script runs on
# macOS at all, so it is moved off the critical path, not deleted.
SKIP_TEST_TARGETS = {
    name.strip()
    for name in os.environ.get("RFS_SKIP_TEST_TARGETS", "").split(",")
    if name.strip()
}
# Targets this run skips in the RELEASE leg only, as a comma-separated
# RFS_SKIP_RELEASE_TEST_TARGETS. Empty by default, and a superset relationship
# is not implied: a name here still runs in the functional leg, which is the
# difference from RFS_SKIP_TEST_TARGETS above.
#
# The release leg exists to catch what a debug `cargo test` cannot -- assertions
# compiled out, integer overflow wrapping instead of panicking, and the feature
# set a dev-dependency-free build actually resolves. A target earns its place
# there by being able to behave differently under those. One that spends its
# time in subprocesses, sockets and the filesystem behaves identically in both
# profiles, so running it twice buys a second copy of the same evidence.
#
# CI names `atlassian_fixture_operator_contract` here for pull requests. It is
# 551 s of a 21-minute release leg -- 42% of it, measured on run 34722270821 --
# and it is a fixture-lifecycle contract: create, verify, clean up, over a real
# subprocess and a TLS handshake. It still runs in full in the functional leg on
# every platform that can (see the Bash 4.4 note above), and the release leg
# still runs it on `main`, so nothing stops being covered before a merge.
SKIP_RELEASE_TEST_TARGETS = {
    name.strip()
    for name in os.environ.get("RFS_SKIP_RELEASE_TEST_TARGETS", "").split(",")
    if name.strip()
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
    if name.startswith("measure_") or "::measure_" in name:
        return "report-only measurement harness, run by hand with RFS_MEASURE=1"
    return None


def run(command, *, capture=False):
    print("+ " + " ".join(map(str, command)), flush=True)
    return subprocess.run(
        command, cwd=ROOT, env=ENV, check=False,
        stdout=subprocess.PIPE if capture else None, text=True, encoding="utf-8",
    )


def test_targets(profile_flags, *, require_no_debug_assertions):
    """Every compiled test target, as (package_id, name, kind, executable).

    Enumerated from cargo's own artifacts rather than from source text, so the
    inventory is what is built and runnable. Returns None when the build failed.
    `profile_flags` selects the profile, so the release leg and the debug leg
    share one enumeration instead of repeating it.
    """
    result = run([
        "cargo", "test", *profile_flags, "--workspace", "--all-features",
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
        if require_no_debug_assertions and artifact["profile"]["debug_assertions"]:
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


def functional_tests():
    """The debug test leg, run under nextest.

    nextest rather than `cargo test` because it runs each test in its own
    process. Four test binaries install a counting `#[global_allocator]` and
    assert a peak-heap ceiling, and a process-global counter only means
    anything when one test owns the process; under a shared binary a
    concurrent test's allocations land in this one's measurement. That is
    rfs-1e6h; with isolation in place the raised allocation ceiling is back at
    its original value, and those rows assert it only under nextest.
    (The `#[ignore]`d budgets were never affected -- ignored_budgets() already
    runs each one alone with `--exact`.)

    Changing runners also collapses the skip path. `cargo test --workspace` has
    no way to exclude a single target, so RFS_SKIP_TEST_TARGETS used to force a
    full target enumeration and one `cargo test` invocation per binary --
    giving up the shared build and the batching to drop one target. nextest
    takes a filterset, so the skip is one more argument to the same single
    command.

    `--no-fail-fast` is absent because it is not a flag here: the `ci` profile
    inherits `fail-fast = false` from `[profile.default]` in
    `.config/nextest.toml`.

    Doctests are not compiled test targets, so nextest does not see them and
    they get their own pass.
    """
    command = ["cargo", "nextest", "run", "-P", "ci", "--workspace", "--all-features"]
    if SKIP_TEST_TARGETS:
        skipped = sorted(SKIP_TEST_TARGETS)
        for name in skipped:
            print(f"Skipped functional target {name}: RFS_SKIP_TEST_TARGETS", flush=True)
        # `=` asks for an exact match. A bare `binary(x)` means the same thing
        # today -- exact is nextest's default for this matcher -- but the
        # substring form `binary(~x)` is one character away and would silently
        # take more than the caller named: `~atlassian` drops 90 tests where
        # the exact name drops 78. Spelling the operator keeps the strict
        # reading pinned.
        #
        # A name that matches no binary is a hard error from nextest
        # ("operator didn't match any binary names"), so the gate fails loudly
        # rather than quietly skipping nothing. That is a change from the
        # enumerate-and-loop this replaced, where an unknown name was a no-op;
        # a stale RFS_SKIP_TEST_TARGETS now gets reported instead of silently
        # putting the target back on the critical path.
        excluded = " | ".join(f"binary(={name})" for name in skipped)
        command += ["-E", f"not ({excluded})"]
    passed = run(command).returncode == 0
    doctests = run(["cargo", "test", "--workspace", "--all-features", "--doc"])
    return doctests.returncode == 0 and passed


def release_workspace():
    """Release tests, serial except for the targets cleared to run parallel.

    Serial is the default: a target runs with `--test-threads=1` unless it is
    named in PARALLEL_RELEASE_TARGETS. Doctests are not compiler artifacts, so
    they run in their own serial pass rather than being lost.
    """
    targets = test_targets(RELEASE, require_no_debug_assertions=True)
    if targets is None:
        return False
    passed = True
    for package_id, name, kind, _executable in targets:
        if name in SKIP_TEST_TARGETS:
            print(f"Skipped release target {name}: RFS_SKIP_TEST_TARGETS", flush=True)
            continue
        if name in SKIP_RELEASE_TEST_TARGETS:
            print(
                f"Skipped release target {name}: RFS_SKIP_RELEASE_TEST_TARGETS"
                " (still ran in the functional leg)",
                flush=True,
            )
            continue
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
    targets = test_targets(RELEASE, require_no_debug_assertions=True)
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


# The phase each gate belongs to, for `--phase`. CI runs the two phases as
# separate jobs on the same runner image so their wall-clock cost is the larger
# of the two rather than the sum: on Ubuntu the debug phase is about 16 minutes
# and the release phase about 22, so one job spent 38 minutes doing what two
# spend 22 doing. Locally, `--phase` is not passed and every gate runs in one
# process exactly as before.
#
# `release` holds the two gates that need release artifacts -- they share the
# `cargo test --release --no-run` enumeration, which is 285 s by itself, so
# separating them would pay for it twice. Everything else is `debug`.
#
# PHASES is the whole vocabulary. main() refuses a gate carrying anything else,
# which is what stops a gate added later from belonging to no job and silently
# never running in CI.
PHASES = ("debug", "release")

# Wall-clock budgets (rfs-cn1r). The policy is defined once, in Rust: the
# policy names, the unset default, and report mode's hard-ceiling factor are
# read from these declarations rather than restated here, so the banner and
# the validation below cannot drift from what the tests do.
BUDGET_POLICY_SOURCE = ROOT / "crates/resourcefs-core/src/test_support.rs"
# Each measurement is appended to the file this names. nextest discards a
# passing test's output and the release leg runs libtest captured, so without
# the file a report-mode number never reaches the log.
BUDGET_REPORT_VARIABLE = "RFS_BUDGET_REPORT"


def budget_policy_facts():
    """(report, enforce, factor) as declared in the Rust policy module."""
    source = BUDGET_POLICY_SOURCE.read_text(encoding="utf-8")

    def declared(pattern, what):
        found = re.search(pattern, source)
        if found is None:
            raise ValueError(f"{BUDGET_POLICY_SOURCE.relative_to(ROOT)} no longer declares {what}")
        return found.group(1)

    report = declared(r'pub const REPORT_POLICY: &str = "([a-z]+)";', "REPORT_POLICY")
    enforce = declared(r'pub const ENFORCE_POLICY: &str = "([a-z]+)";', "ENFORCE_POLICY")
    factor = int(declared(
        r"pub const REPORT_HARD_CEILING_FACTOR: u32 = ([0-9_]+);",
        "REPORT_HARD_CEILING_FACTOR as an integer literal",
    ).replace("_", ""))
    return report, enforce, factor


def budget_policy_banner():
    """The effective policy, or an error message, from RFS_BUDGETS.

    The test helper panics on an unknown value, but only inside the first test
    that measures a budget, so it is checked here before a minute of
    compilation is spent on a typo. Returns (message, policy), with policy
    None when the value is rejected and message then saying why.
    """
    report, enforce, factor = budget_policy_facts()
    value = os.environ.get("RFS_BUDGETS", report)
    if value == report:
        return (
            f"Wall-clock budgets: {report} (RFS_BUDGETS unset or {report}). An "
            f"over-budget row passes unless it reaches {factor}x its budget; "
            f"RFS_BUDGETS={enforce} fails at 1x.",
            report,
        )
    if value == enforce:
        return f"Wall-clock budgets: {enforce}. Any over-budget row fails.", enforce
    return (
        f"RFS_BUDGETS must be {report}, {enforce}, or unset, not {value!r}",
        None,
    )


def budget_report_dir(phases):
    """The target directory's budget-report folder, emptied for `phases` only.

    A `--phase` run clears only its own phase's file, so the other phase's
    report from an earlier run in the same target directory survives.
    """
    result = run(["cargo", "metadata", "--format-version", "1", "--no-deps"], capture=True)
    if result.returncode:
        raise ValueError("cargo metadata failed; cannot place the budget report")
    folder = Path(json.loads(result.stdout)["target_directory"]) / "budget-report"
    folder.mkdir(parents=True, exist_ok=True)
    for phase in phases:
        (folder / f"{phase}.log").unlink(missing_ok=True)
    return folder


def print_budget_report(phase, path, policy):
    """Prints one phase's collected measurements, over-budget lines first.

    Flushed before returning: the gate summary that follows goes to stderr,
    and a cancelled job must not lose buffered lines.
    """
    print(f"\n=== Wall-clock budgets, {phase} phase ===")
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except FileNotFoundError:
        lines = []
    if not lines:
        print("No wall-clock budget was measured in this phase.", flush=True)
        return
    over = [line for line in lines if "(OVER" in line]
    within = [line for line in lines if "(OVER" not in line]
    print(f"{len(lines)} measured, {len(over)} over budget, policy {policy}.")
    for line in over + within:
        print(line)
    sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser(description="Run the repository gates.")
    parser.add_argument(
        "--phase", choices=PHASES, default=None,
        help="run only this phase; omit to run every gate, as a local run does",
    )
    arguments = parser.parse_args()
    try:
        banner, policy = budget_policy_banner()
    except (OSError, ValueError) as error:
        print(f"Budget policy: {error}", file=sys.stderr)
        return 1
    if policy is None:
        print(banner, file=sys.stderr)
        return 2
    print(banner, flush=True)
    gates = [
        ("debug", "Formatting", ["cargo", "fmt", "--all", "--", "--check"]),
        ("debug", "Lints", ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"]),
        # These three are callables, not argument lists: each enumerates or
        # filters its own targets. See functional_tests() for why the debug leg
        # runs under nextest, and why doctests ride inside it rather than
        # taking a gate of their own.
        ("debug", "Functional tests", functional_tests),
        ("release", "Release workspace", release_workspace),
        ("release", "Ignored production budgets", ignored_budgets),
        # `fuzz/` declares its own `[workspace]`, so `--workspace` above cannot
        # reach it and nothing else type-checks it. Without this the targets rot
        # silently against the APIs they exercise. Running the fuzzers is a
        # separate, longer job.
        #
        # `check`, never `build`. A libFuzzer target is `#![no_main]` and takes
        # its entry point from the sanitizer runtime that `cargo fuzz` wires in
        # with `-Zsanitizer=fuzzer`. A plain `cargo build` omits those flags: on
        # Linux `libfuzzer-sys`'s prebuilt archive happens to supply a `main`
        # anyway, so it links, but on Windows MSVC nothing does and every target
        # fails with `LNK1561: entry point must be defined`. Type-checking never
        # links, so it catches the API rot this gate exists for on every
        # platform. Verified against `x86_64-pc-windows-msvc`.
        ("debug", "Fuzz targets", ["cargo", "fmt", "--manifest-path", "fuzz/Cargo.toml", "--", "--check"]),
        ("debug", "Fuzz targets check", ["cargo", "check", "--manifest-path", "fuzz/Cargo.toml"]),
        ("debug", "Dependency vetting", ["cargo", "deny", "check"]),
    ]
    if sys.platform == "linux":
        gates.insert(0, (
            "debug",
            "Module placement",
            [sys.executable, "scripts/module_shape.py"],
        ))
    unknown = sorted({phase for phase, _, _ in gates} - set(PHASES))
    if unknown:
        print(f"Gates declare unknown phases: {unknown}", file=sys.stderr)
        return 1
    if arguments.phase is not None:
        gates = [gate for gate in gates if gate[0] == arguments.phase]
        if not gates:
            print(f"No gates in phase {arguments.phase}", file=sys.stderr)
            return 1
    failed = []
    try:
        report_dir = budget_report_dir({phase for phase, _, _ in gates})
    except (OSError, ValueError, KeyError) as error:
        print(f"Budget report: {error}", file=sys.stderr)
        return 1
    last_gate_of_phase = {phase: index for index, (phase, _, _) in enumerate(gates)}
    for index, (phase, name, command) in enumerate(gates):
        report = report_dir / f"{phase}.log"
        ENV[BUDGET_REPORT_VARIABLE] = str(report)
        print(f"\n=== {name} ===", flush=True)
        try:
            passed = command() if callable(command) else run(command).returncode == 0
        except (OSError, ValueError, KeyError) as error:
            print(f"{name}: {error}", file=sys.stderr)
            passed = False
        if not passed:
            failed.append(name)
        if last_gate_of_phase[phase] == index:
            print_budget_report(phase, report, policy)
    if failed:
        print(f"Failed gates: {', '.join(failed)}", file=sys.stderr)
        return 1
    scope = "repository gates" if arguments.phase is None else f"{arguments.phase}-phase gates"
    print(f"All {scope} passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
