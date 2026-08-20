#!/usr/bin/env python3
"""Compile and run cross-process session-lease probes on Linux and Windows/Wine."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

CARGO_TOML = """\
[package]
name = "resourcefs-session-lease-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
serde_json = "1"

[target.'cfg(unix)'.dependencies]
rustix = { version = "=1.1.4", features = ["fs"] }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "=0.61.2", features = ["Win32_Foundation", "Win32_Storage_FileSystem", "Win32_System_IO"] }
"""

MAIN_RS = r'''
use std::{
    env,
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;

fn open_lock(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)
}

#[cfg(unix)]
fn lock_exclusive(file: &File) -> io::Result<()> {
    rustix::fs::flock(file, rustix::fs::FlockOperation::LockExclusive).map_err(io::Error::from)
}

#[cfg(unix)]
fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    match rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(true),
        Err(error) if error == rustix::io::Errno::WOULDBLOCK => Ok(false),
        Err(error) => Err(io::Error::from(error)),
    }
}

#[cfg(windows)]
fn windows_lock(file: &File, immediate: bool) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::ERROR_LOCK_VIOLATION,
        Storage::FileSystem::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
        },
        System::IO::OVERLAPPED,
    };

    let mut overlapped = OVERLAPPED::default();
    let flags = LOCKFILE_EXCLUSIVE_LOCK
        | if immediate { LOCKFILE_FAIL_IMMEDIATELY } else { 0 };
    let locked = unsafe {
        LockFileEx(
            file.as_raw_handle(),
            flags,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if locked != 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if immediate && error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(windows)]
fn lock_exclusive(file: &File) -> io::Result<()> {
    windows_lock(file, false).and_then(|locked| {
        if locked { Ok(()) } else { Err(io::Error::other("blocking lock did not acquire")) }
    })
}

#[cfg(windows)]
fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    windows_lock(file, true)
}

fn child(path: &Path) -> io::Result<()> {
    let file = open_lock(path)?;
    lock_exclusive(&file)?;
    println!("READY");
    io::stdout().flush()?;
    thread::sleep(Duration::from_secs(60));
    Ok(())
}

fn parent(path: &Path) -> io::Result<()> {
    let mut owner = Command::new(env::current_exe()?)
        .arg("--child")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = owner.stdout.take().ok_or_else(|| io::Error::other("owner stdout"))?;
    let mut reader = BufReader::new(stdout);
    let mut ready = String::new();
    reader.read_line(&mut ready)?;
    if ready.trim() != "READY" {
        return Err(io::Error::other(format!("owner failed to acquire: {ready:?}")));
    }

    let contender = open_lock(path)?;
    let contended_while_owner_live = !try_lock_exclusive(&contender)?;
    owner.kill()?;
    let status = owner.wait()?;

    let deadline = Instant::now() + Duration::from_secs(3);
    let acquired_after_owner_death = loop {
        let candidate = open_lock(path)?;
        if try_lock_exclusive(&candidate)? {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(20));
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "platform": env::consts::OS,
            "contendedWhileOwnerLive": contended_while_owner_live,
            "ownerKilled": !status.success(),
            "acquiredAfterOwnerDeath": acquired_after_owner_death,
        }))?
    );
    Ok(())
}

fn main() -> io::Result<()> {
    let arguments: Vec<_> = env::args_os().collect();
    if arguments.get(1).is_some_and(|argument| argument == "--child") {
        let path = arguments.get(2).ok_or_else(|| io::Error::other("missing child path"))?;
        return child(Path::new(path));
    }
    let root = env::temp_dir().join(format!("resourcefs-lease-probe-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let path = root.join("session.lock");
    let result = parent(&path);
    std::fs::remove_dir_all(&root)?;
    result
}
'''

MAIN_C = r'''
#include <windows.h>
#include <stdio.h>
#include <string.h>

static void fail(const char *operation) {
    fprintf(stderr, "%s failed with Win32 error %lu\n", operation, GetLastError());
    ExitProcess(2);
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "expected hold|try and lock path\n");
        return 2;
    }
    HANDLE file = CreateFileA(
        argv[2],
        GENERIC_READ | GENERIC_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        NULL,
        OPEN_ALWAYS,
        FILE_ATTRIBUTE_NORMAL,
        NULL
    );
    if (file == INVALID_HANDLE_VALUE) {
        fail("CreateFileA");
    }
    OVERLAPPED overlapped = {0};
    BOOL locked = LockFileEx(
        file,
        LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
        0,
        1,
        0,
        &overlapped
    );
    if (strcmp(argv[1], "try") == 0) {
        if (locked) {
            puts("acquired=true");
            CloseHandle(file);
            return 0;
        }
        if (GetLastError() == ERROR_LOCK_VIOLATION) {
            puts("acquired=false");
            CloseHandle(file);
            return 0;
        }
        fail("LockFileEx");
    }
    if (strcmp(argv[1], "hold") != 0) {
        fprintf(stderr, "unknown mode\n");
        CloseHandle(file);
        return 2;
    }
    if (!locked) {
        fail("LockFileEx");
    }
    puts("READY");
    fflush(stdout);
    Sleep(INFINITE);
    return 0;
}
'''


def run(command: list[str], *, env: dict[str, str] | None = None) -> str:
    completed = subprocess.run(
        command,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(command)}\n{completed.stderr}"
        )
    return completed.stdout


def wine_path(path: Path) -> str:
    wine_env = {**os.environ, "WINEDEBUG": "-all"}
    return run(["winepath", "-w", str(path)], env=wine_env).strip()


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-session-lease-probe-") as temp_dir:
        root = Path(temp_dir)
        (root / "src").mkdir()
        manifest = root / "Cargo.toml"
        (root / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
        (root / "src" / "main.rs").write_text(MAIN_RS, encoding="utf-8")
        linux = json.loads(run(["cargo", "run", "--quiet", "--manifest-path", str(manifest)]))
        run(
            [
                "cargo",
                "check",
                "--quiet",
                "--manifest-path",
                str(manifest),
                "--target",
                "x86_64-apple-darwin",
            ]
        )
        macos_cross_check = {
            "platform": "macos",
            "target": "x86_64-apple-darwin",
            "flockApiCompiled": True,
        }

        source = root / "lease_probe.c"
        executable = root / "lease_probe.exe"
        lock_path = root / "windows-session.lock"
        source.write_text(MAIN_C, encoding="utf-8")
        run(["winegcc", str(source), "-o", str(executable)])
        wine_env = {**os.environ, "WINEDEBUG": "-all"}
        owner = subprocess.Popen(
            [str(executable), "hold", wine_path(lock_path)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=wine_env,
        )
        if owner.stdout is None or owner.stdout.readline().strip() != "READY":
            stderr = owner.stderr.read() if owner.stderr is not None else ""
            raise RuntimeError(f"Windows owner failed to acquire lease: {stderr}")
        contended = run(
            [str(executable), "try", wine_path(lock_path)],
            env=wine_env,
        ).strip() == "acquired=false"
        owner.kill()
        owner.wait(timeout=5)
        acquired_after_death = False
        for _ in range(50):
            acquired_after_death = run(
                [str(executable), "try", wine_path(lock_path)],
                env=wine_env,
            ).strip() == "acquired=true"
            if acquired_after_death:
                break
            time.sleep(0.02)
        windows = {
            "platform": "windows",
            "runtime": "wine",
            "contendedWhileOwnerLive": contended,
            "ownerKilled": owner.returncode != 0,
            "acquiredAfterOwnerDeath": acquired_after_death,
        }
        print(
            json.dumps(
                {
                    "linux": linux,
                    "macosCrossCheck": macos_cross_check,
                    "windowsWine": windows,
                },
                indent=2,
                sort_keys=True,
            )
        )


if __name__ == "__main__":
    main()
