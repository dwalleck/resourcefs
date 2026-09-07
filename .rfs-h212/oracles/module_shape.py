#!/usr/bin/env python3
"""C16: enforce the approved h212 module ledger, not cosmetic file splitting."""

import argparse
from pathlib import Path
import re
import sys

# Captured from the upstream discovered by routing, before feature branching.
BASELINE = "0b81d3118fde18fb4df5679ebb5830d67a124a64"
CORE = "crates/resourcefs-core/src/"
SOURCES = "crates/resourcefs-sources/src/"
PARENTS = {
    CORE + "reference.rs": (2089, 1989),
    CORE + "discovery.rs": (1342, 1367),
    SOURCES + "atlassian/jira.rs": (467, 317),
    SOURCES + "atlassian/wire.rs": (571, 601),
    SOURCES + "atlassian/render.rs": (480, 488),
    SOURCES + "http/mod.rs": (1759, 1679),
}
CHILD_LIMITS = {
    CORE + "reference/jira.rs": 650,
    CORE + "reference/source_page.rs": 160,
    SOURCES + "http/read.rs": 350,
    SOURCES + "atlassian/jira/transport.rs": 400,
    SOURCES + "atlassian/jira/browse.rs": 600,
    SOURCES + "atlassian/jira/cursor.rs": 250,
    SOURCES + "atlassian/wire/collections.rs": 600,
    SOURCES + "atlassian/render/collections.rs": 350,
}
REQUIRED = {
    "extraction": (
        CORE + "reference/jira.rs",
        SOURCES + "http/read.rs",
        SOURCES + "atlassian/jira/transport.rs",
    ),
    "projects": (
        CORE + "reference/source_page.rs",
        SOURCES + "atlassian/jira/browse.rs",
        SOURCES + "atlassian/wire/collections.rs",
        SOURCES + "atlassian/render/collections.rs",
    ),
    "issues": (SOURCES + "atlassian/jira/cursor.rs",),
}
OWNERS = {
    "parse_jira_address": CORE + "reference/jira.rs",
    "encode_jira_segment": CORE + "reference/jira.rs",
    "fetch_bounded_attempts": SOURCES + "http/read.rs",
    "decode_project_page": SOURCES + "atlassian/wire/collections.rs",
    "decode_project": SOURCES + "atlassian/wire/collections.rs",
    "decode_issue_page": SOURCES + "atlassian/wire/collections.rs",
    "render_project": SOURCES + "atlassian/render/collections.rs",
    "render_projects": SOURCES + "atlassian/render/collections.rs",
    "render_issues": SOURCES + "atlassian/render/collections.rs",
    "encode_cursor": SOURCES + "atlassian/jira/cursor.rs",
    "decode_cursor": SOURCES + "atlassian/jira/cursor.rs",
}
JIRA_TRANSPORT = {
    "fetch_cached", "fetch_uncached", "cache_namespace", "cache_key",
    "classify_status", "sanitize_fetch_error",
}
DECLARATION = re.compile(
    r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_]\w*)\b"
)
INFRASTRUCTURE = re.compile(r"\b(?:serde_json|reqwest|rmcp)\b")


def check(root, stage):
    failures = []
    observations = []

    def fail(path, predicate):
        failures.append(f"C16 FAIL {path}: {predicate}")

    stages = ("extraction", "projects", "issues")
    required = {
        path
        for name in stages[: stages.index(stage) + 1]
        for path in REQUIRED[name]
    }
    for relative in sorted(required):
        if not (root / relative).is_file():
            fail(relative, "required published owner is missing")

    for relative, (before, maximum) in PARENTS.items():
        path = root / relative
        if not path.is_file():
            fail(relative, "protected parent is missing")
            continue
        lines = len(path.read_text(encoding="utf-8").splitlines())
        observations.append(f"{relative}: {before} -> {lines}, maximum {maximum}")
        if lines > maximum:
            fail(relative, f"{lines} lines exceeds approved {maximum}; placement review required")

    watched = (
        root / CORE / "reference",
        root / SOURCES / "atlassian",
        root / SOURCES / "http",
    )
    paths = {path for directory in watched for path in directory.rglob("*.rs")}
    paths.add(root / CORE / "reference.rs")
    for path in sorted(paths):
        relative = path.relative_to(root).as_posix()
        source = path.read_text(encoding="utf-8")
        if relative not in PARENTS:
            maximum = CHILD_LIMITS.get(relative, 650)
            lines = len(source.splitlines())
            if lines > maximum:
                fail(relative, f"{lines} lines exceeds child tripwire {maximum}")
        for name in DECLARATION.findall(source):
            owner = OWNERS.get(name)
            if relative.startswith(SOURCES + "atlassian/jira") and name in JIRA_TRANSPORT:
                owner = SOURCES + "atlassian/jira/transport.rs"
            if owner is not None and relative != owner:
                fail(relative, f"{name} belongs in {owner}")
        if relative.startswith(CORE + "reference/"):
            if INFRASTRUCTURE.search(source):
                fail(relative, "provider/protocol infrastructure reached core reference grammar")
        if relative != SOURCES + "http/mod.rs" and re.search(r"\breqwest\b", source):
            fail(relative, "HTTP client must remain in the existing substrate module")

    for path in (root / "crates/resourcefs-mcp/src").rglob("*.rs"):
        source = path.read_text(encoding="utf-8")
        if re.search(r"\b(?:with_fixture_browse_limits|FixtureBrowseLimits|with_jira_browse_limits_for_test|BrowseLimits)\b", source):
            fail(path.relative_to(root).as_posix(), "test-only browse limits reached production protocol code")
    return failures, observations


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", choices=tuple(REQUIRED), required=True)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    try:
        failures, observations = check(root, args.stage)
    except (OSError, UnicodeError) as error:
        print(f"C16 FAIL oracle input: {type(error).__name__}", file=sys.stderr)
        return 1
    for observation in observations:
        print(observation)
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"C16 PASS stage={args.stage} baseline={BASELINE}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
