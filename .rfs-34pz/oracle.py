#!/usr/bin/env python3
"""Independent source/documentation oracle for rfs-34pz empirical premises."""

from __future__ import annotations

import glob
import json
import urllib.request
from pathlib import Path


def one(pattern: str) -> Path:
    matches = [Path(path) for path in glob.glob(pattern)]
    if len(matches) != 1:
        raise RuntimeError(f"expected one match for {pattern!r}, found {matches!r}")
    return matches[0]


def fetch(url: str) -> str:
    request = urllib.request.Request(url, headers={"User-Agent": "resourcefs-evidence/1.0"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read().decode("utf-8", errors="replace")


def main() -> None:
    home = Path.home()
    rmcp = one(str(home / ".cargo/registry/src/*/rmcp-3.1.3/src/service.rs")).read_text(
        encoding="utf-8"
    )
    rustix_fd = one(str(home / ".cargo/registry/src/*/rustix-1.1.4/src/fs/fd.rs")).read_text(
        encoding="utf-8"
    )
    windows_fs = one(
        str(
            home
            / ".cargo/registry/src/*/windows-sys-0.61.2/src/Windows/Win32/Storage/FileSystem/mod.rs"
        )
    ).read_text(encoding="utf-8")

    linux_manual = fetch("https://man7.org/linux/man-pages/man2/flock.2.html")
    apple_manual = fetch(
        "https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/flock.2.html"
    )
    windows_manual = fetch(
        "https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex"
    )

    p1 = {
        "eofBreaksClosed": "break QuitReason::Closed" in rmcp,
        "closedDrainIsFiveSeconds": (
            "QuitReason::Closed => Some(Duration::from_secs(5))" in rmcp
        ),
        "drainHasTimeout": "tokio::time::timeout(timeout_duration" in rmcp,
        "handlerRunsInDetachedServiceTask": (
            "spawn_service_task(async move" in rmcp
            and "service.handle_request(request, context)" in rmcp
        ),
        "waitingOnlyAwaitsServiceHandle": (
            "pub async fn waiting(mut self)" in rmcp
            and "Some(handle) => handle.await" in rmcp
        ),
    }
    p1["waitingCanReturnBeforeHandler"] = all(p1.values())

    p2 = {
        "rustixMapsFlock": "pub fn flock<Fd: AsFd>" in rustix_fd,
        "windowsSysMapsLockFileEx": 'fn LockFileEx(' in windows_fs,
        "linuxNonblockingContentionDocumented": (
            "EWOULDBLOCK" in linux_manual and "LOCK_NB" in linux_manual
        ),
        "linuxCloseReleaseDocumented": "lock is released" in linux_manual,
        "appleNonblockingContentionDocumented": (
            "EWOULDBLOCK" in apple_manual and "LOCK_NB" in apple_manual
        ),
        "windowsImmediateContentionDocumented": "LOCKFILE_FAIL_IMMEDIATELY" in windows_manual,
        "windowsTerminationReleaseDocumented": (
            "If a process terminates with a portion of a file locked" in windows_manual
            and "locks are unlocked by the operating system" in windows_manual
        ),
    }
    p2["leaseContractSupported"] = all(p2.values())

    print(
        json.dumps(
            {
                "P1": p1,
                "P2": p2,
                "sources": {
                    "rmcp": str(one(str(home / ".cargo/registry/src/*/rmcp-3.1.3/src/service.rs"))),
                    "linux": "https://man7.org/linux/man-pages/man2/flock.2.html",
                    "apple": "https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/flock.2.html",
                    "windows": "https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex",
                },
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
