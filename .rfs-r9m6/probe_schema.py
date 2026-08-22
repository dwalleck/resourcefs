#!/usr/bin/env python3
"""Emit the product schema and a hand-authored profile-shape observation table."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

EXPECTED = {
    "minimal": True,
    "valid_https": True,
    "unknown_top_level": False,
    "null_optional_object": False,
    "missing_source_required": False,
    "unknown_source_kind": False,
    "wrong_kind_field": False,
    "extra_source_field": False,
    "unsupported_version": False,
}

CASES = [
    ("minimal", {"schemaVersion": 1}),
    (
        "valid_https",
        {
            "schemaVersion": 1,
            "sources": [
                {
                    "kind": "https",
                    "id": "web",
                    "required": False,
                    "origins": [
                        {
                            "baseUrl": "https://example.com/",
                            "allowPrivateNetwork": False,
                        }
                    ],
                }
            ],
        },
    ),
    ("unknown_top_level", {"schemaVersion": 1, "typo": True}),
    ("null_optional_object", {"schemaVersion": 1, "workspace": None}),
    (
        "missing_source_required",
        {
            "schemaVersion": 1,
            "sources": [{"kind": "https", "id": "web", "origins": []}],
        },
    ),
    (
        "unknown_source_kind",
        {
            "schemaVersion": 1,
            "sources": [{"kind": "ftp", "id": "ftp", "required": False}],
        },
    ),
    (
        "wrong_kind_field",
        {
            "schemaVersion": 1,
            "sources": [
                {
                    "kind": "https",
                    "id": "web",
                    "required": False,
                    "repositories": ["owner/repo"],
                }
            ],
        },
    ),
    (
        "extra_source_field",
        {
            "schemaVersion": 1,
            "sources": [
                {
                    "kind": "github",
                    "id": "github",
                    "required": True,
                    "allowPrivateNetwork": False,
                    "credential": {"kind": "environment", "name": "GITHUB_TOKEN"},
                    "repositories": [{"name": "owner/repo"}],
                    "extra": 1,
                }
            ],
        },
    ),
    ("unsupported_version", {"schemaVersion": 2}),
]


def main() -> None:
    repository = Path(__file__).resolve().parent.parent
    corpus = json.loads(
        (
            repository
            / "crates/resourcefs-mcp/tests/fixtures/profile_corpus.json"
        ).read_text(encoding="utf-8")
    )
    completed = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "resourcefs-mcp", "--", "schema"],
        cwd=repository,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    print(
        json.dumps(
            {
                "schema": json.loads(completed.stdout),
                "observations": [
                    {
                        "name": name,
                        "value": value,
                        "expected": EXPECTED[name],
                    }
                    for name, value in CASES
                ]
                + [
                    {
                        "name": f"corpus::{row['name']}",
                        "value": row["profile"],
                        "expected": row["accepted"],
                    }
                    for row in corpus
                ],
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
