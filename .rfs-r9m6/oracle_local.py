#!/usr/bin/env python3
"""Independent canonical-path, extension, and permission oracle for Slice 4."""

from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path

SAFE_EXTENSION_BYTES = frozenset(b"abcdefghijklmnopqrstuvwxyz0123456789_+-")


def extension(value: str) -> bool:
    encoded = value.encode("utf-8")
    return 0 < len(encoded) <= 64 and all(byte in SAFE_EXTENSION_BYTES for byte in encoded)


def disjoint_extensions(groups: list[list[str]]) -> bool:
    claims: set[str] = set()
    for group in groups:
        if not group:
            return False
        for value in group:
            if not extension(value) or value in claims:
                return False
            claims.add(value)
    return bool(groups)


def contained(base: Path, candidate: Path, kind: str) -> bool:
    try:
        canonical_base = base.resolve(strict=True)
        resolved = candidate if candidate.is_absolute() else canonical_base / candidate
        canonical_target = resolved.resolve(strict=True)
    except (FileNotFoundError, OSError):
        return False
    try:
        canonical_target.relative_to(canonical_base)
    except ValueError:
        return False
    return (kind == "file" and canonical_target.is_file()) or (
        kind == "directory" and canonical_target.is_dir()
    ) or (kind == "either" and (canonical_target.is_file() or canonical_target.is_dir()))


def subset(parent: tuple[bool, bool, bool], child: tuple[bool, bool, bool]) -> bool:
    return all(not nested or source for source, nested in zip(parent, child, strict=True))


def observe(name: str, observed: bool, expected: bool) -> dict[str, object]:
    return {"name": name, "observed": observed, "expected": expected, "agree": observed == expected}


def main() -> None:
    observations: list[dict[str, object]] = [
        observe("extension-single", disjoint_extensions([["md"]]), True),
        observe("extension-case-collision", disjoint_extensions([["MD"], ["md"]]), False),
        observe("extension-empty", disjoint_extensions([[]]), False),
        observe("extension-leading-dot", disjoint_extensions([[".md"]]), False),
        observe("extension-path", disjoint_extensions([["a/b"]]), False),
        observe("extension-uppercase", disjoint_extensions([["MD"]]), False),
        observe("vault-subset", subset((True, True, False), (True, False, False)), True),
        observe("vault-superset", subset((True, False, False), (False, True, False)), False),
    ]

    with tempfile.TemporaryDirectory(prefix="resourcefs-local-oracle-") as temporary:
        base = Path(temporary) / "profile"
        sibling = Path(temporary) / "sibling"
        base.mkdir()
        sibling.mkdir()
        (base / "dir with spaces").mkdir()
        (base / "unicodé.txt").write_text("content", encoding="utf-8")
        (sibling / "outside.txt").write_text("outside", encoding="utf-8")
        (base / "alias.txt").symlink_to(base / "unicodé.txt")
        (base / "escape.txt").symlink_to(sibling / "outside.txt")

        observations.extend(
            [
                observe("contained-directory", contained(base, Path("dir with spaces"), "directory"), True),
                observe("contained-file", contained(base, Path("unicodé.txt"), "file"), True),
                observe("missing", contained(base, Path("missing"), "either"), False),
                observe("wrong-kind", contained(base, Path("unicodé.txt"), "directory"), False),
                observe("symlink-alias", contained(base, Path("alias.txt"), "file"), True),
                observe("symlink-escape", contained(base, Path("escape.txt"), "file"), False),
                observe("parent-escape", contained(base, Path("../sibling/outside.txt"), "file"), False),
                observe(
                    "alias-identity",
                    (base / "alias.txt").resolve(strict=True) == (base / "unicodé.txt").resolve(strict=True),
                    True,
                ),
                observe("sibling-unchanged", (sibling / "outside.txt").read_text(encoding="utf-8") == "outside", True),
            ]
        )

    print(
        json.dumps(
            {
                "allAgree": all(row["agree"] for row in observations),
                "engine": "python pathlib.resolve plus literal extension and permission tables",
                "platform": os.name,
                "observations": observations,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
