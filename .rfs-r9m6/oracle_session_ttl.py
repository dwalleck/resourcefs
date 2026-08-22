#!/usr/bin/env python3
"""Independent literal oracle for retained-session TTL decisions."""

from __future__ import annotations

import json

MAX_TTL_SECONDS = 86_400


def main() -> None:
    boundary_rows = [
        {"seconds": seconds, "accepted": 0 <= seconds <= MAX_TTL_SECONDS}
        for seconds in (-1, 0, MAX_TTL_SECONDS, MAX_TTL_SECONDS + 1)
    ]
    elapsed_rows = [
        {
            "ttlSeconds": ttl,
            "elapsedSeconds": elapsed,
            "expired": elapsed >= ttl,
        }
        for ttl, elapsed in (
            (0, 0),
            (MAX_TTL_SECONDS, MAX_TTL_SECONDS - 1),
            (MAX_TTL_SECONDS, MAX_TTL_SECONDS),
            (MAX_TTL_SECONDS, MAX_TTL_SECONDS + 1),
        )
    ]
    print(
        json.dumps(
            {
                "boundaryRows": boundary_rows,
                "elapsedRows": elapsed_rows,
                "namespaceChild": "resourcefs",
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
