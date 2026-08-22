#!/usr/bin/env python3
"""Independent literal oracle for ResourceFS logging boundaries and rotation."""

from __future__ import annotations

import json

MAX_ROTATION_BYTES = 10_485_760
MAX_RETAINED_FILES = 10


def main() -> None:
    size_rows = [
        {"bytes": value, "accepted": 0 <= value <= MAX_ROTATION_BYTES}
        for value in (-1, 0, MAX_ROTATION_BYTES, MAX_ROTATION_BYTES + 1)
    ]
    retention_rows = [
        {"files": value, "accepted": 1 <= value <= MAX_RETAINED_FILES}
        for value in (-1, 0, 1, MAX_RETAINED_FILES, MAX_RETAINED_FILES + 1)
    ]
    rotation_rows = [
        {
            "currentBytes": current,
            "recordBytes": record,
            "rotationBytes": limit,
            "rotate": current > 0 and (limit == 0 or current + record > limit),
        }
        for current, record, limit in (
            (0, 16, 0),
            (16, 16, 0),
            (16, 16, 32),
            (17, 16, 32),
            (MAX_ROTATION_BYTES, 1, MAX_ROTATION_BYTES),
        )
    ]
    print(
        json.dumps(
            {
                "familyForThree": [
                    "resourcefs.log",
                    "resourcefs.log.1",
                    "resourcefs.log.2",
                ],
                "retentionRows": retention_rows,
                "rotationRows": rotation_rows,
                "sizeRows": size_rows,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
