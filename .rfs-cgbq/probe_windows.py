#!/usr/bin/env python3
"""Exercise Win32 path classification and handle-final paths on a safe Wine fixture."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path

MAIN_C = r'''
#include <windows.h>
#include <shlwapi.h>
#include <stdio.h>
#include <string.h>
#include <strings.h>

static void fail_last_error(const char *operation) {
    fprintf(stderr, "%s failed with Win32 error %lu\n", operation, GetLastError());
    ExitProcess(2);
}

static void final_path(const char *path, char *output, DWORD capacity) {
    HANDLE handle = CreateFileA(
        path,
        FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        NULL,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS,
        NULL
    );
    if (handle == INVALID_HANDLE_VALUE) {
        fail_last_error("CreateFileA");
    }
    DWORD written = GetFinalPathNameByHandleA(
        handle,
        output,
        capacity,
        FILE_NAME_NORMALIZED | VOLUME_NAME_DOS
    );
    if (written == 0 || written >= capacity) {
        CloseHandle(handle);
        fail_last_error("GetFinalPathNameByHandleA");
    }
    CloseHandle(handle);
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "expected canonical root and escaping target\n");
        return 2;
    }

    printf("driveAbsolute=%s\n", PathIsRelativeA("C:\\workspace\\file.txt") ? "false" : "true");
    printf("uncAbsolute=%s\n", PathIsRelativeA("\\\\server\\share\\file.txt") ? "false" : "true");
    printf("driveRelative=%s\n", PathIsRelativeA("C:workspace\\file.txt") ? "false" : "true");
    printf("rootRelative=%s\n", PathIsRelativeA("\\workspace\\file.txt") ? "false" : "true");
    printf("slashRooted=%s\n", PathIsRelativeA("/workspace/file.txt") ? "false" : "true");

    char canonical_root[32768];
    char canonical_escape[32768];
    final_path(argv[1], canonical_root, sizeof(canonical_root));
    final_path(argv[2], canonical_escape, sizeof(canonical_escape));
    size_t root_length = strlen(canonical_root);
    int starts_with_root = strncasecmp(canonical_root, canonical_escape, root_length) == 0
        && (canonical_escape[root_length] == '\\' || canonical_escape[root_length] == '\0');
    printf("escapeStartsWithRoot=%s\n", starts_with_root ? "true" : "false");
    printf(
        "canonicalUsesVerbatimPrefix=%s\n",
        strncmp(canonical_root, "\\\\?\\", 4) == 0 ? "true" : "false"
    );
    return 0;
}
'''


def wine_path(path: Path) -> str:
    completed = subprocess.run(
        ["winepath", "-w", str(path)],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        env={**os.environ, "WINEDEBUG": "-all"},
    )
    return completed.stdout.strip()


def parse_observations(output: str) -> dict[str, bool]:
    observations: dict[str, bool] = {}
    for line in output.splitlines():
        key, value = line.split("=", 1)
        observations[key] = value == "true"
    return observations


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-windows-probe-") as temp_dir:
        fixture = Path(temp_dir)
        root = fixture / "root"
        outside = fixture / "outside"
        root.mkdir()
        outside.mkdir()
        (outside / "secret.txt").write_text("outside\n", encoding="utf-8")
        subprocess.run(
            [
                "wine",
                "cmd",
                "/c",
                "mklink",
                "/D",
                wine_path(root / "escape"),
                wine_path(outside),
            ],
            check=True,
            stdout=subprocess.PIPE,
            env={**os.environ, "WINEDEBUG": "-all"},
        )

        source = fixture / "probe.c"
        executable = fixture / "probe.exe"
        source.write_text(MAIN_C, encoding="utf-8")
        subprocess.run(
            ["winegcc", str(source), "-o", str(executable), "-lshlwapi"],
            check=True,
        )
        completed = subprocess.run(
            [
                str(executable),
                wine_path(root),
                wine_path(root / "escape" / "secret.txt"),
            ],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            env={**os.environ, "WINEDEBUG": "-all"},
        )
        print(
            json.dumps(
                {
                    "api": "Win32",
                    "runtime": "wine",
                    "observations": parse_observations(completed.stdout),
                },
                indent=2,
                sort_keys=True,
            )
        )


if __name__ == "__main__":
    main()
