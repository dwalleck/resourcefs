#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pywinrm==0.5.0"]
# ///
"""Upload and run a cross-compiled process contract on the local Windows VM."""

from __future__ import annotations

import glob
import json
import sys
import uuid
from pathlib import Path

import winrm


UPLOAD_CHUNK_BYTES = 60_000
TREE_STRESS_RUNS = 20


def credentials() -> dict[str, str]:
    path = Path.home() / ".config" / "resourcefs" / "windows-vm" / "credentials.env"
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        name, value = line.split("=", 1)
        values[name] = value
    return values


def newest_match(pattern: str) -> Path:
    candidates = [Path(path) for path in glob.glob(pattern) if Path(path).is_file()]
    if not candidates:
        raise RuntimeError(f"no Windows test executable matches {pattern!r}")
    newest_mtime = max(path.stat().st_mtime_ns for path in candidates)
    newest = [path for path in candidates if path.stat().st_mtime_ns == newest_mtime]
    if len(newest) != 1:
        rendered = ", ".join(str(path) for path in newest)
        raise RuntimeError(f"multiple equally-new Windows test executables match: {rendered}")
    return newest[0]


def upload(session: winrm.Session, source: Path, destination: str) -> None:
    protocol = session.protocol
    shell_id = protocol.open_shell()
    try:
        command_id = protocol.run_command(
            shell_id,
            "powershell.exe",
            [
                "-NoProfile",
                "-Command",
                "$stream=[IO.File]::Create('"
                + destination
                + "');try{[Console]::OpenStandardInput().CopyTo($stream)}"
                + "finally{$stream.Dispose()}",
            ],
        )
        payload = source.read_bytes()
        for offset in range(0, len(payload), UPLOAD_CHUNK_BYTES):
            chunk = payload[offset : offset + UPLOAD_CHUNK_BYTES]
            protocol.send_command_input(
                shell_id,
                command_id,
                chunk,
                end=offset + len(chunk) == len(payload),
            )
        _, stderr, status = protocol.get_command_output(shell_id, command_id)
        if status != 0:
            raise RuntimeError(stderr.decode("utf-8", errors="replace"))
    finally:
        protocol.close_shell(shell_id)


def run_test(session: winrm.Session, executable: str, arguments: list[str]) -> dict[str, object]:
    result = session.run_cmd(executable, arguments)
    return {
        "status": result.status_code,
        "stdout": result.std_out.decode("utf-8", errors="replace"),
        "stderr": result.std_err.decode("utf-8", errors="replace"),
    }


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} '<test-executable-glob>'")
    executable = newest_match(sys.argv[1])
    values = credentials()
    session = winrm.Session(
        "http://127.0.0.1:55985/wsman",
        auth=(values["RFS_WINDOWS_USER"], values["RFS_WINDOWS_PASSWORD"]),
        transport="basic",
        server_cert_validation="ignore",
    )
    remote = rf"C:\Windows\Temp\resourcefs-process-contract-{uuid.uuid4().hex}.exe"
    upload(session, executable, remote)
    try:
        contract = run_test(session, remote, ["--nocapture", "--test-threads=1"])
        tree_failures = []
        for iteration in range(TREE_STRESS_RUNS):
            result = run_test(
                session,
                remote,
                ["windows_tree_cleanup", "--exact", "--nocapture", "--test-threads=1"],
            )
            if result["status"] != 0:
                result["iteration"] = iteration + 1
                tree_failures.append(result)
        report = {
            "executable": str(executable),
            "contract": contract,
            "treeStress": {
                "iterations": TREE_STRESS_RUNS,
                "failures": tree_failures,
            },
        }
        print(json.dumps(report, indent=2, sort_keys=True))
        if contract["status"] != 0 or tree_failures:
            raise SystemExit(1)
    finally:
        session.run_ps(
            f"Remove-Item -LiteralPath '{remote}' -Force -ErrorAction SilentlyContinue"
        )


if __name__ == "__main__":
    main()
