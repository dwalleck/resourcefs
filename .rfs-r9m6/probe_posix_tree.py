#!/usr/bin/env python3
"""Probe whole-process-group termination and force-kill on the native POSIX host."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def ignore_term(_signum: int, _frame: object) -> None:
    return


def grandchild(pid_file: Path) -> None:
    signal.signal(signal.SIGTERM, ignore_term)
    pid_file.write_text(str(os.getpid()), encoding="utf-8")
    while True:
        time.sleep(1)


def child(directory: Path) -> None:
    signal.signal(signal.SIGTERM, ignore_term)
    (directory / "child.pid").write_text(str(os.getpid()), encoding="utf-8")
    subprocess.Popen([sys.executable, __file__, "--grandchild", str(directory / "grandchild.pid")])
    while True:
        time.sleep(1)


def process_state(pid: int) -> str:
    stat_path = Path(f"/proc/{pid}/stat")
    if not stat_path.exists():
        return "absent"
    fields = stat_path.read_text(encoding="utf-8").split()
    return fields[2]


def parent() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-posix-tree-probe-") as temp_dir:
        directory = Path(temp_dir)
        process = subprocess.Popen(
            [sys.executable, __file__, "--child", str(directory)],
            start_new_session=True,
        )
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if (directory / "child.pid").exists() and (directory / "grandchild.pid").exists():
                break
            time.sleep(0.01)
        child_pid = int((directory / "child.pid").read_text(encoding="utf-8"))
        grandchild_pid = int((directory / "grandchild.pid").read_text(encoding="utf-8"))
        started = time.monotonic()
        os.killpg(process.pid, signal.SIGTERM)
        time.sleep(1)
        graceful_states = {
            "child": process_state(child_pid),
            "grandchild": process_state(grandchild_pid),
        }
        if any(state not in {"absent", "Z"} for state in graceful_states.values()):
            os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=1)
        deadline = time.monotonic() + 1
        final_states = {
            "child": process_state(child_pid),
            "grandchild": process_state(grandchild_pid),
        }
        while time.monotonic() < deadline and any(
            state not in {"absent", "Z"} for state in final_states.values()
        ):
            time.sleep(0.01)
            final_states = {
                "child": process_state(child_pid),
                "grandchild": process_state(grandchild_pid),
            }
        print(
            json.dumps(
                {
                    "platform": sys.platform,
                    "processGroup": process.pid,
                    "childPid": child_pid,
                    "grandchildPid": grandchild_pid,
                    "gracefulStates": graceful_states,
                    "finalStates": final_states,
                    "noLiveDescendants": all(
                        state in {"absent", "Z"} for state in final_states.values()
                    ),
                    "elapsedMs": int((time.monotonic() - started) * 1_000),
                },
                indent=2,
                sort_keys=True,
            )
        )


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--grandchild":
        grandchild(Path(sys.argv[2]))
    elif len(sys.argv) > 1 and sys.argv[1] == "--child":
        child(Path(sys.argv[2]))
    else:
        parent()
