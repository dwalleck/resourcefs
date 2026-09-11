#!/usr/bin/env python3
"""Read-only GitHub immutable-object probe; Git plumbing is the independent oracle."""
import base64
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SHA = "635ab170ab57542c18272921298d575da2f8b08a"
REPO = "dwalleck/resourcefs"
REQUESTS = []


def git(*args):
    return subprocess.check_output(["git", "-C", str(ROOT), *args])


def api(path):
    REQUESTS.append({"method": "GET", "path": path, "restApiVersion": "2022-11-28"})
    return json.loads(subprocess.check_output([
        "gh", "api", "--method", "GET", "-H", "X-GitHub-Api-Version: 2022-11-28",
        "-H", "Accept: application/vnd.github+json", f"repos/{REPO}/{path}"
    ]))


commit = api(f"commits/{SHA}")
raw_commit = git("cat-file", "commit", SHA)
headers, message = raw_commit.split(b"\n\n", 1)
header_lines = headers.splitlines()
tree_sha = next(x[5:].decode() for x in header_lines if x.startswith(b"tree "))
parents = [x[7:].decode() for x in header_lines if x.startswith(b"parent ")]
assert commit["sha"] == SHA
assert commit["commit"]["tree"]["sha"] == tree_sha
assert [x["sha"] for x in commit["parents"]] == parents
# GitHub strips the final message newline. Record that provider value, do not
# treat it as exact raw commit-object bytes or silently add/remove a newline.
assert commit["commit"]["message"].rstrip("\n") == message.decode().rstrip("\n")
comparisons = [{"premise": "P1", "case": "commit", "sha": SHA, "tree": tree_sha,
                "parents": parents, "messageTrailingNewlines": {
                    "provider": len(commit["commit"]["message"]) - len(commit["commit"]["message"].rstrip("\n")),
                    "git": len(message) - len(message.rstrip(b"\n"))}, "result": "PASS"}]
# Non-recursive trees are the proposed acquisition seam. Every sibling entry is
# compared to Git; the blob selection is independent of provider list order.
current = tree_sha
for prefix, segment in [("", "crates"), ("crates/", "resourcefs-core"),
                        ("crates/resourcefs-core/", "Cargo.toml")]:
    tree = api(f"git/trees/{current}")
    assert tree["sha"] == current and tree["truncated"] is False
    expected = []
    for entry in git("ls-tree", "-z", current).split(b"\0"):
        if not entry:
            continue
        metadata, name = entry.split(b"\t", 1)
        mode, kind, oid = metadata.decode().split()
        expected.append((name.decode(), mode, kind, oid))
    actual = [(e["path"], e["mode"], e["type"], e["sha"]) for e in tree["tree"]]
    assert sorted(actual) == sorted(expected)
    selected = next(e for e in tree["tree"] if e["path"] == segment)
    comparisons.append({"premise": "P1", "case": "tree", "path": prefix,
                        "sha": current, "entries": len(actual), "result": "PASS"})
    current = selected["sha"]
blob = api(f"git/blobs/{current}")
assert blob["sha"] == current and blob["encoding"] == "base64"
encoded = "".join(blob["content"].splitlines())
content = base64.b64decode(encoded, validate=True)
expected_bytes = git("cat-file", "blob", current)
assert content == expected_bytes and blob["size"] == len(expected_bytes)
comparisons.append({"premise": "P1", "case": "blob", "sha": current,
                    "size": len(content), "sha256": hashlib.sha256(content).hexdigest(), "result": "PASS"})
result = {"sourceRevision": SHA, "repository": REPO,
          "oracle": "git cat-file / git ls-tree against local object database",
          "requests": REQUESTS, "comparisons": comparisons,
          "scope": "Pinned existing ASCII nested path only; no new feature enforcement, exotic paths, binary blob, symlink or submodule provider proof."}
print(json.dumps(result, indent=2))
