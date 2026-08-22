#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pywinrm==0.5.0"]
# ///
"""Upload and run the native Windows probe and independent oracle over local WinRM."""

from __future__ import annotations

import base64
import json
from pathlib import Path

import winrm


def credentials() -> dict[str, str]:
    path = Path.home() / ".config" / "resourcefs" / "windows-vm" / "credentials.env"
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        name, value = line.split("=", 1)
        values[name] = value
    return values


def upload(session: winrm.Session, source: Path, destination: str) -> None:
    encoded = base64.b64encode(source.read_bytes()).decode("ascii")
    commands = [f"[IO.File]::WriteAllBytes('{destination}',[byte[]]@())"]
    commands.extend(
        "$bytes=[Convert]::FromBase64String('"
        + encoded[offset : offset + 2_000]
        + "');$stream=[IO.File]::Open('"
        + destination
        + "',[IO.FileMode]::Append);try{$stream.Write($bytes,0,$bytes.Length)}"
        + "finally{$stream.Dispose()}"
        for offset in range(0, len(encoded), 2_000)
    )
    for command in commands:
        result = session.run_ps(command)
        if result.status_code != 0:
            raise RuntimeError(result.std_err.decode("utf-8", errors="replace"))


def run_script(session: winrm.Session, path: str) -> dict[str, object]:
    result = session.run_cmd(
        "powershell.exe",
        ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path],
    )
    if result.status_code != 0:
        raise RuntimeError(result.std_err.decode("utf-8", errors="replace"))
    return json.loads(result.std_out.decode("utf-8"))


def main() -> None:
    values = credentials()
    session = winrm.Session(
        "http://127.0.0.1:55985/wsman",
        auth=(values["RFS_WINDOWS_USER"], values["RFS_WINDOWS_PASSWORD"]),
        transport="basic",
        server_cert_validation="ignore",
    )
    root = Path(__file__).parent
    remote_probe = r"C:\Windows\Temp\resourcefs-probe-windows.ps1"
    remote_oracle = r"C:\Windows\Temp\resourcefs-oracle-windows.ps1"
    upload(session, root / "probe_windows.ps1", remote_probe)
    upload(session, root / "oracle_windows.ps1", remote_oracle)
    try:
        result = {
            "probe": run_script(session, remote_probe),
            "oracle": run_script(session, remote_oracle),
        }
    finally:
        session.run_ps(
            f"Remove-Item -LiteralPath '{remote_probe}','{remote_oracle}' -Force "
            "-ErrorAction SilentlyContinue"
        )
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
