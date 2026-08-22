#!/usr/bin/env python3
"""Use stopped heartbeat growth as an independent process-tree termination oracle."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path


def ignore_term(_signum: int, _frame: object) -> None:
    return


def write_heartbeat(path: Path) -> None:
    while True:
        with path.open("ab", buffering=0) as stream:
            stream.write(b"x")
        time.sleep(0.02)


def heartbeat(path: Path) -> None:
    signal.signal(signal.SIGTERM, ignore_term)
    write_heartbeat(path)


def child(directory: Path) -> None:
    signal.signal(signal.SIGTERM, ignore_term)
    child_thread = threading.Thread(
        target=write_heartbeat,
        args=(directory / "child.beat",),
        daemon=True,
    )
    child_thread.start()
    subprocess.Popen([sys.executable, __file__, "--heartbeat", str(directory / "grandchild.beat")])
    while True:
        time.sleep(1)


def parent() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-posix-tree-oracle-") as temp_dir:
        directory = Path(temp_dir)
        process = subprocess.Popen(
            [sys.executable, __file__, "--child", str(directory)],
            start_new_session=True,
        )
        paths = [directory / "child.beat", directory / "grandchild.beat"]
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if all(path.exists() and path.stat().st_size >= 3 for path in paths):
                break
            time.sleep(0.02)
        os.killpg(process.pid, signal.SIGTERM)
        time.sleep(1)
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=1)
        sizes_after_kill = [path.stat().st_size for path in paths]
        time.sleep(0.3)
        sizes_after_observation = [path.stat().st_size for path in paths]
        print(
            json.dumps(
                {
                    "platform": sys.platform,
                    "sizesAfterKill": sizes_after_kill,
                    "sizesAfterObservation": sizes_after_observation,
                    "heartbeatsStopped": sizes_after_kill == sizes_after_observation,
                },
                indent=2,
                sort_keys=True,
            )
        )


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--heartbeat":
        heartbeat(Path(sys.argv[2]))
    elif len(sys.argv) > 1 and sys.argv[1] == "--child":
        child(Path(sys.argv[2]))
    else:
        parent()
