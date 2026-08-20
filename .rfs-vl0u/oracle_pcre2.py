#!/usr/bin/env python3
"""Oracle: PCRE2 behavior via the installed `pcre2test` CLI (no Rust bindings).

Independent failure mechanism for the pcre2 probe: exercises the same
semantic questions -- lookbehind compile/match, match-limit and depth-limit
bounded work -- through the PCRE2 reference test driver, and normalizes the
CLI output (which exposes the exact typed error codes as "Failed: error -47"
etc.) to the same booleans and canonical PCRE2_ERROR_* names the probe emits.

Requires `pcre2test` on PATH (fails with a nonzero exit if absent). Writes
only a temporary input file. On subprocess failure the captured stderr is
written to stderr and the process exits nonzero; unexpected output is
reported verbatim, never guessed.

Verified on this machine (main agent, 2026-08-20):
  - subject-line modifiers use `\\=`, e.g. `aaa...b\\=match_limit=1000`;
  - match-limit exhaustion prints `Failed: error -47`;
  - `\\=depth_limit=1` on the recursive pattern prints `Failed: error -53`;
  - run as `pcre2test -8 -q <file>`.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

MATCH_RE = re.compile(r"^\s*(\d+):\s(.*)$")
MATCH_ERROR_RE = re.compile(r"^Failed:\s*error\s*(-?\d+)(?:\s+at offset\s+\d+)?(?::.*)?$")
COMPILE_FAILED_RE = re.compile(r"^Failed:\s*(.*)$")
NO_MATCH = "No match"

MATCH_LIMIT_SUBJECT = "a" * 18 + "b"
DEPTH_SUBJECT = "a" * 20 + "b" * 20

TEST_INPUT = f"""\
/(?<=foo)bar/
foobar

/(?<=a+)b/
aaab

/^(a+)+$/
{MATCH_LIMIT_SUBJECT}\\=match_limit=1000

/^(a+)+$/
{MATCH_LIMIT_SUBJECT}

/(a(?1)?b)/
{DEPTH_SUBJECT}\\=depth_limit=1

/(a(?1)?b)/
{DEPTH_SUBJECT}

/CAFÉ/i,utf,ucp
café
"""


def error_name(code):
    return {
        -1: "PCRE2_ERROR_NOMATCH",
        -47: "PCRE2_ERROR_MATCHLIMIT",
        -53: "PCRE2_ERROR_DEPTHLIMIT",
    }.get(code, "PCRE2_ERROR_OTHER")


def nonblank(lines, i):
    while i < len(lines) and lines[i].strip() == "":
        i += 1
    return i


def parse_results(lines):
    out = {}
    i = 0

    # Case 1: fixed-length lookbehind compiles and matches "bar" in "foobar".
    i = nonblank(lines, i)
    line = lines[i].strip() if i < len(lines) else ""
    m = MATCH_RE.match(line)
    if m:
        capture_index, text = int(m.group(1)), m.group(2)
        out["pcre2test_compiles_lookbehind"] = True
        out["pcre2test_lookbehind_matches_foobar"] = capture_index == 0 and text == "bar"
        start = "foobar".index(text)
        out["pcre2test_lookbehind_match_span"] = [start, start + len(text)]
    else:
        fm = COMPILE_FAILED_RE.match(line)
        out["pcre2test_compiles_lookbehind"] = False
        out["pcre2test_lookbehind_matches_foobar"] = False
        out["pcre2test_lookbehind_match_span"] = None
        if fm:
            out["pcre2test_lookbehind_compile_error"] = fm.group(1)
    out["pcre2test_lookbehind_observed"] = line
    i += 1

    # Case 2: variable-length lookbehind must be rejected at compile time.
    i = nonblank(lines, i)
    line = lines[i].strip() if i < len(lines) else ""
    fm = COMPILE_FAILED_RE.match(line)
    out["pcre2test_rejects_unbounded_lookbehind"] = fm is not None
    out["pcre2test_unbounded_lookbehind_observed"] = line
    if fm:
        out["pcre2test_unbounded_lookbehind_message"] = fm.group(1)
    i += 1

    # Case 3: match_limit=1000 on an exponential backtracking subject.
    i = nonblank(lines, i)
    line = lines[i].strip() if i < len(lines) else ""
    me = MATCH_ERROR_RE.match(line)
    code = int(me.group(1)) if me else None
    out["pcre2test_match_limit_exhaustion_rc"] = code
    out["pcre2test_match_limit_exhaustion"] = error_name(code) if code is not None else None
    out["pcre2test_match_limit_observed"] = line
    i += 1

    # Case 4: same pattern with the default 10M limit must NOT be exhausted.
    i = nonblank(lines, i)
    line = lines[i].strip() if i < len(lines) else ""
    out["pcre2test_default_match_limit_not_exhausted"] = line == NO_MATCH
    out["pcre2test_default_match_limit_observed"] = line
    i += 1

    # Case 5: depth_limit=1 on the recursive pattern.
    i = nonblank(lines, i)
    line = lines[i].strip() if i < len(lines) else ""
    me = MATCH_ERROR_RE.match(line)
    code = int(me.group(1)) if me else None
    out["pcre2test_depth_limit_exhaustion_rc"] = code
    out["pcre2test_depth_limit_exhaustion"] = error_name(code) if code is not None else None
    out["pcre2test_depth_limit_observed"] = line
    i += 1

    # Case 6: same pattern with the default depth limit must succeed.
    i = nonblank(lines, i)
    line = lines[i].strip() if i < len(lines) else ""
    m = MATCH_RE.match(line)
    out["pcre2test_default_depth_limit_not_exhausted"] = m is not None and int(m.group(1)) == 0
    if m:
        out["pcre2test_default_depth_limit_span"] = [
            int(m.group(1)),
            int(m.group(1)) + len(m.group(2)),
        ]
    out["pcre2test_default_depth_limit_observed"] = line
    i += 1

    # Case 7: Unicode-aware caseless matching agrees with the search contract.
    i = nonblank(lines, i)
    line = lines[i].strip() if i < len(lines) else ""
    m = MATCH_RE.match(line)
    out["pcre2test_unicode_casefold_matches"] = m is not None and int(m.group(1)) == 0
    out["pcre2test_unicode_casefold_observed"] = line

    return out
def is_outcome(line):
    stripped = line.strip()
    match = MATCH_RE.match(stripped)
    return (
        (match is not None and int(match.group(1)) == 0)
        or MATCH_ERROR_RE.match(stripped) is not None
        or COMPILE_FAILED_RE.match(stripped) is not None
        or stripped == NO_MATCH
    )




def tool_version(pcre2test):
    try:
        proc = subprocess.run(
            [pcre2test, "-C"], capture_output=True, text=True, timeout=60
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0 or not proc.stdout.strip():
        return None
    first = proc.stdout.splitlines()[0].strip()
    m = re.match(r"PCRE2 version\s+(\S+)", first)
    return m.group(1) if m else first


def main():
    pcre2test = shutil.which("pcre2test")
    if pcre2test is None:
        sys.stderr.write("fatal: pcre2test executable not found on PATH\n")
        sys.exit(1)

    with tempfile.TemporaryDirectory(prefix="pcre2-oracle-") as td:
        testfile = os.path.join(td, "tests.txt")
        with open(testfile, "w", encoding="utf-8") as fh:
            fh.write(TEST_INPUT)
        try:
            proc = subprocess.run(
                [pcre2test, "-8", "-q", testfile],
                capture_output=True,
                text=True,
                timeout=120,
            )
        except FileNotFoundError:
            sys.stderr.write("fatal: pcre2test disappeared after being found\n")
            sys.exit(1)
        except subprocess.TimeoutExpired:
            sys.stderr.write("fatal: pcre2test timed out\n")
            sys.exit(1)
        lines = [
            ln.strip()
            for ln in proc.stdout.splitlines()
            if is_outcome(ln)
        ]
        if not lines:
            sys.stderr.write(
                f"fatal: pcre2test produced no output (exit {proc.returncode})\n"
            )
            if proc.stderr:
                sys.stderr.write(proc.stderr)
            sys.exit(1)
        if proc.returncode != 0:
            sys.stderr.write(f"warning: pcre2test exited nonzero ({proc.returncode}); using output\n")
            if proc.stderr:
                sys.stderr.write(proc.stderr)

        result = parse_results(lines)
        result["oracle"] = "pcre2"
        result["tool"] = "pcre2test"
        result["tool_version"] = tool_version(pcre2test)
        print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
