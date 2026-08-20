#!/usr/bin/env python3
"""Independent sequential oracle for the rfs-34pz Path Session/TTL contracts."""

from __future__ import annotations

import json
from dataclasses import dataclass

OBJECT_BYTES = 64 * 1024 * 1024
SESSION_BYTES = 256 * 1024 * 1024
MAX_RECORDS = 1_000
TTL_SECONDS = 86_400


@dataclass(frozen=True)
class Content:
    identity: str
    size: int
    digest_bucket: str


class SessionModel:
    def __init__(self, token: str) -> None:
        self.token = token
        self.active = True
        self.next_id = 1
        self.used_bytes = 0
        self.records: list[tuple[int, Content]] = []
        self.store_calls = 0

    def retain(self, content: Content) -> tuple[str, int | None]:
        if content.size > OBJECT_BYTES:
            return ("limit_exceeded", None)
        if not self.active:
            return ("source_unavailable", None)
        for object_id, existing in self.records:
            if (
                existing.digest_bucket == content.digest_bucket
                and existing.identity == content.identity
                and existing.size == content.size
            ):
                return ("ok", object_id)
        if len(self.records) >= MAX_RECORDS:
            return ("limit_exceeded", None)
        if self.used_bytes + content.size > SESSION_BYTES:
            return ("limit_exceeded", None)
        object_id = self.next_id
        self.next_id += 1
        self.store_calls += 1
        self.records.append((object_id, content))
        self.used_bytes += content.size
        return ("ok", object_id)

    def lookup(self, token: str, object_id: int) -> str:
        if not self.active or token != self.token:
            return "not_found"
        return "ok" if any(item_id == object_id for item_id, _ in self.records) else "not_found"


def cleanup_outcome(*, locked: bool, age_seconds: int, has_marker: bool = True) -> str:
    if locked:
        return "live"
    if not has_marker or age_seconds < TTL_SECONDS:
        return "fresh"
    return "removed"


def main() -> None:
    collision = SessionModel("a" * 32)
    collision_ids = [
        collision.retain(Content("alpha", 5, "forced-collision"))[1],
        collision.retain(Content("beta", 4, "forced-collision"))[1],
        collision.retain(Content("alpha", 5, "forced-collision"))[1],
    ]
    assert collision_ids == [1, 2, 1]
    assert collision.used_bytes == 9
    assert collision.store_calls == 2

    quota = SessionModel("b" * 32)
    exact_ids = [
        quota.retain(Content(f"object-{index}", OBJECT_BYTES, f"digest-{index}"))[1]
        for index in range(4)
    ]
    one_over = quota.retain(Content("one-over", 1, "one-over"))[0]
    assert exact_ids == [1, 2, 3, 4]
    assert quota.used_bytes == SESSION_BYTES
    assert quota.store_calls == 4
    assert one_over == "limit_exceeded"

    object_limit = SessionModel("c" * 32)
    assert object_limit.retain(Content("too-large", OBJECT_BYTES + 1, "large"))[0] == "limit_exceeded"
    assert object_limit.store_calls == 0

    lookup = SessionModel("d" * 32)
    _, known_id = lookup.retain(Content("known", 5, "known"))
    assert known_id == 1
    lookup_outcomes = {
        "known": lookup.lookup("d" * 32, 1),
        "foreign": lookup.lookup("e" * 32, 1),
        "unknown": lookup.lookup("d" * 32, 999),
    }
    lookup.active = False
    lookup_outcomes["inactive"] = lookup.lookup("d" * 32, 1)
    assert lookup_outcomes == {
        "known": "ok",
        "foreign": "not_found",
        "unknown": "not_found",
        "inactive": "not_found",
    }

    cleanup = {
        "ttlMinusOne": cleanup_outcome(locked=False, age_seconds=TTL_SECONDS - 1),
        "ttlExact": cleanup_outcome(locked=False, age_seconds=TTL_SECONDS),
        "ttlPlusOne": cleanup_outcome(locked=False, age_seconds=TTL_SECONDS + 1),
        "lockedOld": cleanup_outcome(locked=True, age_seconds=TTL_SECONDS + 1),
        "lockedMissingMarker": cleanup_outcome(
            locked=True, age_seconds=0, has_marker=False
        ),
    }
    assert cleanup == {
        "ttlMinusOne": "fresh",
        "ttlExact": "removed",
        "ttlPlusOne": "removed",
        "lockedOld": "live",
        "lockedMissingMarker": "live",
    }

    print(
        json.dumps(
            {
                "cleanup": cleanup,
                "collisionIds": collision_ids,
                "collisionStoreCalls": collision.store_calls,
                "collisionUsedBytes": collision.used_bytes,
                "exactQuotaIds": exact_ids,
                "exactQuotaUsedBytes": quota.used_bytes,
                "lookup": lookup_outcomes,
                "objectOneOverStoreCalls": object_limit.store_calls,
                "sessionOneOver": one_over,
                "status": "ok",
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
