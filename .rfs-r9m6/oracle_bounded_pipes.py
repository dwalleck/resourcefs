#!/usr/bin/env python3
"""Independently bound two subprocess pipes with Python reader threads."""

from __future__ import annotations

import json
import subprocess
import sys
import threading
import time

LIMIT = 65_536
CHUNK = b"x" * 4_096


def child() -> None:
    def write(stream: object) -> None:
        while True:
            try:
                stream.write(CHUNK)  # type: ignore[attr-defined]
                stream.flush()  # type: ignore[attr-defined]
            except BrokenPipeError:
                return

    stdout = threading.Thread(target=write, args=(sys.stdout.buffer,))
    stderr = threading.Thread(target=write, args=(sys.stderr.buffer,))
    stdout.start()
    stderr.start()
    stdout.join()
    stderr.join()


def bounded(stream: object, result: dict[str, int], key: str) -> None:
    total = 0
    while total <= LIMIT:
        chunk = stream.read(min(8_192, LIMIT + 1 - total))  # type: ignore[attr-defined]
        if not chunk:
            break
        total += len(chunk)
    result[key] = total


def parent() -> None:
    started = time.monotonic()
    process = subprocess.Popen(
        [sys.executable, __file__, "--child"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert process.stdout is not None and process.stderr is not None
    counts: dict[str, int] = {}
    stdout = threading.Thread(target=bounded, args=(process.stdout, counts, "stdout"))
    stderr = threading.Thread(target=bounded, args=(process.stderr, counts, "stderr"))
    stdout.start()
    stderr.start()
    stdout.join(timeout=5)
    stderr.join(timeout=5)
    no_deadlock = not stdout.is_alive() and not stderr.is_alive()
    process.kill()
    process.wait(timeout=1)
    print(
        json.dumps(
            {
                "runtime": "python subprocess/threading",
                "stdoutObserved": counts.get("stdout"),
                "stderrObserved": counts.get("stderr"),
                "stdoutOverLimit": counts.get("stdout", 0) > LIMIT,
                "stderrOverLimit": counts.get("stderr", 0) > LIMIT,
                "noDeadlock": no_deadlock,
                "reaped": process.poll() is not None,
                "elapsedMs": int((time.monotonic() - started) * 1_000),
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--child":
        child()
    else:
        parent()
