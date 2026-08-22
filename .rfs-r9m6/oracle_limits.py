#!/usr/bin/env python3
"""Independent literal oracle for ResourceFS profile limit acceptance."""

from __future__ import annotations

import json

MAXIMA = {
    "text.bytes": 49_152,
    "text.lines": 3_000,
    "text.columns": 512,
    "discovery.searchMatches": 1_000,
    "discovery.globEntries": 1_000,
    "discovery.listingEntries": 1_000,
    "imageBytes": 5_242_880,
    "storage.objectBytes": 67_108_864,
    "storage.sessionBytes": 268_435_456,
}


def accepts(field: str, value: int) -> bool:
    return 1 <= value <= MAXIMA[field]


def main() -> None:
    rows: list[dict[str, object]] = []
    for field, maximum in MAXIMA.items():
        for label, value in (
            ("negative", -1),
            ("zero", 0),
            ("exact", maximum),
            ("oneOver", maximum + 1),
        ):
            rows.append(
                {
                    "field": field,
                    "case": label,
                    "value": value,
                    "accepted": accepts(field, value),
                }
            )
    for object_bytes, session_bytes in ((1, 2), (2, 2), (2, 1)):
        rows.append(
            {
                "field": "storage",
                "case": "relation",
                "objectBytes": object_bytes,
                "sessionBytes": session_bytes,
                "accepted": object_bytes <= session_bytes,
            }
        )
    print(
        json.dumps(
            {
                "omittedUsesMaxima": True,
                "emptyGroupsUseMaxima": True,
                "maxima": MAXIMA,
                "rows": rows,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
