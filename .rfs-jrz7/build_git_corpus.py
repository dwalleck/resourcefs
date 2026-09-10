#!/usr/bin/env python3
"""Build offline native-wire fixtures from an independent temporary Git database."""
import base64
import json
import os
from pathlib import Path
import subprocess
import tempfile
from urllib.parse import quote

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "crates/resourcefs-sources/tests/fixtures/github_immutable.json"
API = "@API@repos/owner/repo/"
WEB = "https://github.example/owner/repo"
MESSAGE = "Immutable source corpus\n"


def build():
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull)
    with tempfile.TemporaryDirectory(prefix="rfs-jrz7-git-oracle-") as directory:
        database = Path(directory)

        def git(*args, data=None):
            return subprocess.run(
                ["git", *args], cwd=database, env=environment, input=data,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True,
            ).stdout

        git("init", "--bare", "--quiet")
        blobs = {}
        tree_ids = set()

        def blob(content):
            oid = git("hash-object", "-t", "blob", "-w", "--stdin", data=content).decode().strip()
            blobs[oid] = content
            return oid

        def tree(entries):
            data = b"".join(
                f"{mode} {kind} {oid}\t".encode() + name.encode("utf-8") + b"\0"
                for mode, kind, oid, name in entries
            )
            oid = git("mktree", "-z", "--missing", data=data).decode().strip()
            tree_ids.add(oid)
            return oid

        def commit(oid):
            raw = (
                f"tree {oid}\n"
                "author Fixture Author <author@example.test> 0 +0000\n"
                "committer Fixture Committer <committer@example.test> 0 +0000\n"
                f"\n{MESSAGE}"
            ).encode()
            return git("hash-object", "-t", "commit", "-w", "--stdin", data=raw).decode().strip()

        binary = blob(b"\x00\xff\x80line\r\nend\x00")
        empty = blob(b"")
        crlf = blob(b"one\r\ntwo\r\n")
        mixed = blob(b"one\rtwo\nthree\r\n")
        executable = blob(b"#!/bin/sh\nprintf 'fixture\\n'\n")
        pointer = blob(("version https://git-lfs.github.com/spec/v1\noid sha256:" + "1" * 64 + "\nsize 987654321\n").encode())
        link = blob("docs/λ space%2F:raw.bin".encode())
        empty_tree = tree([])
        submodule = commit(empty_tree)
        docs = tree([("100644", "blob", binary, "λ space%2F:raw.bin")])
        foo = tree([("100644", "blob", binary, "leaf")])
        next_tree = tree([("100644", "blob", binary, "payload")])
        deep = tree([("100644", "blob", binary, "payload"), ("040000", "tree", next_tree, "next")])
        for index in range(6, 0, -1):
            deep = tree([("040000", "tree", deep, f"d{index}")])
        templates = {}
        for count in (1000, 1001):
            oid = tree([("100644", "blob", binary, f"entry{index:04}") for index in range(count)])
            templates[oid] = {"entryCount": count, "entryPrefix": "entry", "objectSha": binary, "sizeBytes": len(blobs[binary])}
        wide, too_wide = templates
        root = tree([
            ("040000", "tree", docs, "docs"),
            ("100644", "blob", empty, "empty"),
            ("100755", "blob", executable, "run.sh"),
            ("100644", "blob", crlf, "crlf.txt"),
            ("100644", "blob", mixed, "mixed.txt"),
            ("100644", "blob", pointer, "pointer.lfs"),
            ("120000", "blob", link, "link"),
            ("160000", "commit", submodule, "submodule"),
            ("100644", "blob", binary, "odd-mode"),
            ("040000", "tree", foo, "foo"),
            ("100644", "blob", binary, "foo.bar"),
            ("100644", "blob", binary, "facts"),
            ("100644", "blob", binary, "line\nbreak"),
            ("040000", "tree", wide, "wide"),
            ("040000", "tree", too_wide, "too-wide"),
            ("040000", "tree", deep, "deep"),
        ])
        # mktree canonicalizes regular modes. Change exactly one native mode in
        # its actual object bytes, then let Git name/store the unusual object.
        raw_root = git("cat-file", "tree", root)
        assert raw_root.count(b"100644 odd-mode\0") == 1
        unusual_root = raw_root.replace(b"100644 odd-mode\0", b"100664 odd-mode\0")
        root = git("hash-object", "-t", "tree", "--literally", "-w", "--stdin", data=unusual_root).decode().strip()
        assert git("cat-file", "tree", root) == unusual_root
        tree_ids.add(root)
        commit_sha = commit(root)

        def entries(oid):
            result = []
            for record in git("ls-tree", "-z", oid).split(b"\0"):
                if not record:
                    continue
                metadata, name = record.split(b"\t", 1)
                mode, kind, object_id = metadata.decode().split()
                # ls-tree canonicalizes this intentionally unusual mode; the
                # verified raw cat-file object, not that display, is authority.
                if oid == root and name == b"odd-mode":
                    mode = "100664"
                row = {"path": name.decode("utf-8"), "mode": mode, "type": kind, "sha": object_id}
                row["url"] = API + {"blob": "git/blobs/", "tree": "git/trees/", "commit": "git/commits/"}[kind] + object_id
                if kind == "blob":
                    row["size"] = len(blobs[object_id])
                result.append(row)
            return result

        assert next(row for row in entries(root) if row["path"] == "odd-mode")["mode"] == "100664"
        native_trees = {
            oid: {"sha": oid, "url": API + "git/trees/" + oid, "truncated": False, "tree": list(reversed(entries(oid)))}
            for oid in sorted(tree_ids) if oid not in templates
        }

        def native_commit(sha, tree_sha):
            return {
                "sha": sha, "url": API + "commits/" + sha,
                "html_url": WEB + "/commit/" + sha,
                "comments_url": API + "commits/" + sha + "/comments",
                "commit": {
                    "url": API + "git/commits/" + sha,
                    "tree": {"sha": tree_sha, "url": API + "git/trees/" + tree_sha},
                    "message": MESSAGE,
                    "author": {"name": "Fixture Author", "email": "author@example.test", "date": "1970-01-01T00:00:00Z"},
                    "committer": {"name": "Fixture Committer", "email": "committer@example.test", "date": "1970-01-01T00:00:00Z"},
                },
                "author": None, "committer": None, "parents": [],
            }

        cases = {}

        def case(name, path, state="available", reason=None):
            containing = root
            parts = path.split("/")
            for index, part in enumerate(parts):
                if containing in templates:
                    template = templates[containing]
                    assert part.startswith(template["entryPrefix"])
                    selected = {"sha": template["objectSha"], "mode": "100644", "type": "blob"}
                else:
                    selected = next(row for row in entries(containing) if row["path"] == part)
                if index != len(parts) - 1:
                    assert selected["type"] == "tree"
                    containing = selected["sha"]
            value = {
                "path": path, "encodedPath": "/".join(quote(part, safe="-._~") for part in parts),
                "objectSha": selected["sha"], "containingTreeSha": containing,
                "mode": selected["mode"], "objectType": selected["type"], "expectedState": state,
            }
            if reason:
                value["reason"] = reason
            if state == "available":
                content = git("cat-file", "blob", selected["sha"])
                value.update(bytesBase64=base64.b64encode(content).decode(), decodedSizeBytes=len(content))
            cases[name] = value

        for name, path in [
            ("binary", "docs/λ space%2F:raw.bin"), ("empty", "empty"),
            ("executable", "run.sh"), ("crlf", "crlf.txt"), ("mixedNewlines", "mixed.txt"),
            ("lfs", "pointer.lfs"), ("ordering", "foo/leaf"), ("factsName", "facts"),
            ("controlName", "line\nbreak"), ("wide", "wide/entry0999"),
        ]:
            case(name, path)
        for name, path in [("symlink", "link"), ("submodule", "submodule"), ("directory", "docs")]:
            case(name, path, "unsupported", "non_regular_object")
        case("unsupportedMode", "odd-mode", "unsupported", "unsupported_object_mode")
        case("tooWide", "too-wide/entry1000", "error", "limit_exceeded")
        case("deepBlobLimit", "deep/d1/d2/d3/d4/d5/d6/payload", "unavailable", "acquisition_failed")
        case("deepBeforeTerminalLimit", "deep/d1/d2/d3/d4/d5/d6/next/payload", "error", "limit_exceeded")

        # Large fixtures remain recipes plus independent Git identities, not
        # multi-megabyte committed payloads or a second production codec.
        boundaries = {}
        for name, size in [("exact", 4194304), ("over", 4194305)]:
            data = b"x" * size
            oid = git("hash-object", "-t", "blob", "-w", "--stdin", data=data).decode().strip()
            tree_sha = tree([("100644", "blob", oid, "payload")])
            sha = commit(tree_sha)
            boundaries[name] = {
                "commitSha": sha, "treeSha": tree_sha, "objectSha": oid,
                "decodedSizeBytes": size, "byteValue": 120,
                "encodedLength": len(base64.b64encode(data)),
                "commit": native_commit(sha, tree_sha),
            }
        assert boundaries["exact"]["encodedLength"] == boundaries["over"]["encodedLength"]
        return {
            "oracle": "Temporary native Git object database: hash-object, mktree, ls-tree, cat-file; no gix or ResourceFS helpers.",
            "commitSha": commit_sha, "treeSha": root,
            "repository": {
                "id": 1, "name": "repo", "full_name": "owner/repo",
                "owner": {"id": 2, "login": "owner", "url": "@API@users/owner", "html_url": "https://github.example/owner"},
                "url": API.rstrip("/"), "html_url": WEB,
            },
            "commit": native_commit(commit_sha, root),
            "trees": native_trees,
            "treeTemplates": templates,
            "blobs": {
                oid: {"sha": oid, "url": API + "git/blobs/" + oid, "size": len(data), "encoding": "base64", "content": base64.encodebytes(data).decode()}
                for oid, data in sorted(blobs.items())
            },
            "cases": cases,
            "decodedBoundary": boundaries,
        }


if __name__ == "__main__":
    corpus = build()
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    # One small native object per line keeps the oracle inspectable without
    # checking in thousands of identical template entries.
    members = []
    for key, value in corpus.items():
        if key in {"trees", "blobs", "cases", "treeTemplates", "decodedBoundary"}:
            encoded = "{\n" + ",\n".join(
                "    " + json.dumps(name) + ": " + json.dumps(row, ensure_ascii=False, sort_keys=True)
                for name, row in value.items()
            ) + "\n  }"
        else:
            encoded = json.dumps(value, ensure_ascii=False, sort_keys=True)
        members.append("  " + json.dumps(key) + ": " + encoded)
    text = "{\n" + ",\n".join(members) + "\n}\n"
    OUTPUT.write_text(text, encoding="utf-8")
    print(json.dumps({"output": str(OUTPUT), "commitSha": corpus["commitSha"], "treeSha": corpus["treeSha"], "cases": len(corpus["cases"]), "treeTemplates": [value["entryCount"] for value in corpus["treeTemplates"].values()], "boundaryEncodedLength": corpus["decodedBoundary"]["exact"]["encodedLength"]}, indent=2))
