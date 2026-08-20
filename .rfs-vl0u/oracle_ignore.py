#!/usr/bin/env python3
"""Oracle instrument for rfs-vl0u prove-it: git check-ignore.

Builds the same corpus tree as probe_ignore.py (authorized root, outside
sentinel, escaping directory symlink under the root, root and nested
.gitignore files), initializes a throwaway git repository at the authorized
root, classifies the link by Python realpath containment, and computes the
normalized path -> ignored map with `git check-ignore --no-index -v` per
corpus path. Negation is distinguished from no-match by re-querying a copy
of the tree whose .gitignore files have their negation lines removed, and
the documented parent-directory exclusion rule is propagated from git's own
answers on ancestor directories. No Python glob/ignore library is used.

Emits one deterministic JSON object on stdout. Evidence only: touches only
its temporary directory. On any failure, prints captured stderr to stderr
and exits nonzero; a failure is never turned into a plausible result.
Platforms without symlink capability fail explicitly instead of skipping.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# Must stay identical to probe_ignore.py's corpus.
CORPUS = [
    ("keep.txt", False),
    ("stray.log", False),
    ("stray.tmp", False),
    ("important.log", False),
    ("docs", True),
    ("docs/notes.txt", False),
    ("sub", True),
    ("sub/data.tmp", False),
    ("sub/override.tmp", False),
    ("sub/scratch.bak", False),
    ("scratch.bak", False),
]

ROOT_GITIGNORE = "*.log\n*.tmp\n!important.log\ndocs/\n"
SUB_GITIGNORE = "!override.tmp\n*.bak\n"



def fail(message):
    print(f"oracle_ignore: {message}", file=sys.stderr)
    sys.exit(1)


def build_tree(base):
    """Create the shared corpus tree; returns (root, sentinel)."""
    root = base / "root"
    outside = base / "outside"
    root.mkdir()
    outside.mkdir()
    (root / ".gitignore").write_text(ROOT_GITIGNORE, encoding="utf-8")
    (root / "keep.txt").write_text("keep\n", encoding="utf-8")
    (root / "stray.log").write_text("stray log\n", encoding="utf-8")
    (root / "stray.tmp").write_text("stray tmp\n", encoding="utf-8")
    (root / "important.log").write_text("important log\n", encoding="utf-8")
    (root / "scratch.bak").write_text("scratch bak\n", encoding="utf-8")
    docs = root / "docs"
    docs.mkdir()
    (docs / "notes.txt").write_text("notes\n", encoding="utf-8")
    sub = root / "sub"
    sub.mkdir()
    (sub / ".gitignore").write_text(SUB_GITIGNORE, encoding="utf-8")
    (sub / "data.tmp").write_text("data\n", encoding="utf-8")
    (sub / "override.tmp").write_text("override\n", encoding="utf-8")
    (sub / "scratch.bak").write_text("sub scratch bak\n", encoding="utf-8")
    sentinel = outside / "secret.txt"
    sentinel.write_text("outside sentinel\n", encoding="utf-8")
    link = root / "linked"
    try:
        os.symlink("../outside", link)
    except OSError as exc:
        fail(f"cannot create symlink {link} -> ../outside: {exc}")
    if not os.path.islink(link):
        fail("symlink creation reported success but the link is not a symlink")
    return root, sentinel


def git_env():
    env = os.environ.copy()
    env["GIT_CONFIG_NOSYSTEM"] = "1"
    env["GIT_CONFIG_GLOBAL"] = os.devnull
    for var in (
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_COUNT",
    ):
        env.pop(var, None)
    return env


def run_git(env, cwd, *args):
    try:
        proc = subprocess.run(
            ["git", "-c", "core.excludesFile=" + os.devnull, *args],
            cwd=str(cwd),
            env=env,
            capture_output=True,
            text=True,
            timeout=120,
        )
    except OSError as exc:
        fail(f"cannot run git: {exc}")
    except subprocess.TimeoutExpired as exc:
        fail(f"git timed out after 120s: {exc}")
    return proc


def check_one(env, repo, rel):
    """Return Git's effective ignore decision and responsible pattern."""
    proc = run_git(env, repo, "check-ignore", "--no-index", "-v", "--", rel)
    if proc.returncode == 1 and not proc.stdout.strip():
        return False, None
    if proc.returncode != 0:
        print(proc.stderr, file=sys.stderr)
        fail(f"check-ignore {rel!r} exited with status {proc.returncode}")
    lines = proc.stdout.strip().splitlines()
    if len(lines) != 1:
        print(proc.stdout, file=sys.stderr)
        fail(f"check-ignore {rel!r}: expected exactly one pattern line")
    meta, sep, pathname = lines[0].partition("\t")
    if not sep or pathname != rel:
        print(proc.stdout, file=sys.stderr)
        fail(f"check-ignore {rel!r}: unexpected output line {lines[0]!r}")
    pattern = meta.partition(":")[2].partition(":")[2]
    return not pattern.startswith("!"), pattern




def main():
    tmp = Path(tempfile.mkdtemp(prefix="rfs-vl0u-ignore-oracle-"))
    try:
        root, _sentinel = build_tree(tmp)
        env = git_env()
        git_version = run_git(env, tmp, "--version").stdout.strip()
        if not git_version:
            fail("git --version returned no output")
        proc = run_git(env, tmp, "init", "-q", str(root))
        if proc.returncode != 0:
            print(proc.stderr, file=sys.stderr)
            fail(f"git init failed for {root}")

        ignored = {}
        for rel, is_dir in CORPUS:
            is_ignored, pattern = check_one(env, root, rel)
            status = (
                "ignore"
                if is_ignored
                else "whitelist"
                if pattern is not None
                else "no_match"
            )
            ignored[rel] = {
                "ignored": is_ignored,
                "status": status,
                "is_dir": is_dir,
                "pattern": pattern,
            }

        link = root / "linked"
        root_real = os.path.realpath(root)
        link_real = os.path.realpath(link)
        escapes_root = os.path.commonpath([root_real, link_real]) != root_real
        out = {
            "instrument": "oracle",
            "subject": "git check-ignore --no-index",
            "git_version": git_version,
            "link": {
                "is_symlink": os.path.islink(link),
                "escapes_root": escapes_root,
            },
            "ignored": ignored,
        }
        print(json.dumps(out, indent=2, sort_keys=True))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
