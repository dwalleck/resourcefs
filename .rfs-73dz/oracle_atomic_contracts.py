#!/usr/bin/env python3
"""Check published OS contracts independently of the Rust runtime probe."""

from __future__ import annotations

import html
import json
import re
import urllib.request

URLS = {
    "linux": "https://man7.org/linux/man-pages/man2/rename.2.html",
    "macos": "https://keith.github.io/xcode-man-pages/rename.2.html",
    "windows_rename": "https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information",
    "windows_replace": "https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew",
    "windows_move": "https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw",
}


def fetch(url: str) -> str:
    request = urllib.request.Request(
        url,
        headers={"Accept": "text/markdown,text/html", "User-Agent": "resourcefs-evidence-probe"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        content = response.read().decode("utf-8", errors="replace")
    content = re.sub(r"<[^>]+>", " ", content)
    return re.sub(r"[^a-z0-9]+", " ", html.unescape(content).lower()).strip()


def contains(document: str, *phrases: str) -> bool:
    return all(phrase in document for phrase in phrases)

def main() -> None:
    documents = {name: fetch(url) for name, url in URLS.items()}
    checks = {
        "linux": {
            "replaceKeepsNamePresent": contains(
                documents["linux"],
                "atomically replaced",
                "no point at which another process attempting to access newpath will find it missing",
            ),
            "noClobberFlag": contains(
                documents["linux"],
                "rename noreplace",
                "return an error if newpath already exists",
            ),
        },
        "macos": {
            "replaceKeepsNamePresent": contains(
                documents["macos"],
                "guarantees that an instance of new will always exist",
            ),
            "noClobberFlag": contains(
                documents["macos"],
                "rename excl",
                "eexist to be returned if the destination already exists",
            ),
        },
        "windows": {
            "replaceOpensNewIdentity": contains(
                documents["windows_rename"],
                "file rename posix semantics",
                "existing handles to the replaced file continue to be valid",
                "subsequent opens of the target name will open the renamed file not the replaced file",
            ),
            "replaceExistingFlag": contains(
                documents["windows_rename"],
                "file rename replace if exists",
                "if a file with the given name already exists it should be replaced",
            ),
            "replaceFileAclContract": contains(
                documents["windows_replace"],
                "preserve all attributes and acls",
            ),
            "crossVolumeCopyIsExplicit": contains(
                documents["windows_move"],
                "movefile copy allowed",
                "simulates the move by using the copyfile",
            ),
        },
    }
    if not all(value for platform in checks.values() for value in platform.values()):
        raise SystemExit(json.dumps(checks, indent=2, sort_keys=True))
    print(json.dumps(checks, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
