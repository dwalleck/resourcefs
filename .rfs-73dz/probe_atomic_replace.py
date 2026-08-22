#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["pywinrm==0.5.0"]
# ///
"""Probe atomic replacement and no-clobber rename on supported platforms."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import uuid
from pathlib import Path

import winrm

CARGO_TOML = """\
[package]
name = "resourcefs-atomic-replace-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
serde_json = "1.0.143"

[target.'cfg(unix)'.dependencies]
rustix = { version = "=1.1.4", features = ["fs"] }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "=0.61.2", features = ["Win32_Foundation", "Win32_Storage_FileSystem", "Win32_System_WindowsProgramming"] }
"""

MAIN_RS = r'''
use std::{
    fs,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

use serde_json::json;

const CONTENT_BYTES: usize = 1024 * 1024;
const REPLACEMENTS: usize = 128;

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(unix)]
fn replace_existing(replacement: &Path, target: &Path) -> io::Result<()> {
    fs::rename(replacement, target)
}

#[cfg(unix)]
fn replacement_primitive() -> &'static str {
    "rename"
}

#[cfg(windows)]
fn windows_rename(source: &Path, destination: &Path, flags: u32) -> io::Result<()> {
    use std::{
        fs::OpenOptions,
        mem::offset_of,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        ptr,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FileRenameInfoEx, SetFileInformationByHandle,
    };

    const DELETE_ACCESS: u32 = 0x0001_0000;
    let source_file = OpenOptions::new()
        .access_mode(DELETE_ACCESS)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(source)?;
    let mut destination = wide(destination);
    destination.pop();
    let buffer_bytes = offset_of!(FILE_RENAME_INFO, FileName)
        .checked_add(destination.len() * size_of::<u16>())
        .ok_or_else(|| io::Error::other("rename buffer overflow"))?;
    let mut buffer = vec![0_u8; buffer_bytes];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.Flags = flags;
        (*info).RootDirectory = ptr::null_mut();
        (*info).FileNameLength = u32::try_from(destination.len() * size_of::<u16>())
            .map_err(|_| io::Error::other("destination name is too long"))?;
        ptr::copy_nonoverlapping(
            destination.as_ptr(),
            (*info).FileName.as_mut_ptr(),
            destination.len(),
        );
    }
    let result = unsafe {
        SetFileInformationByHandle(
            source_file.as_raw_handle(),
            FileRenameInfoEx,
            info.cast(),
            u32::try_from(buffer_bytes)
                .map_err(|_| io::Error::other("rename buffer is too large"))?,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn replace_existing(replacement: &Path, target: &Path) -> io::Result<()> {
    use windows_sys::Win32::System::WindowsProgramming::{
        FILE_RENAME_FLAG_POSIX_SEMANTICS, FILE_RENAME_FLAG_REPLACE_IF_EXISTS,
    };
    windows_rename(
        replacement,
        target,
        FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS,
    )
}

#[cfg(windows)]
fn replacement_primitive() -> &'static str {
    "SetFileInformationByHandle(FileRenameInfoEx:REPLACE|POSIX)"
}

#[cfg(unix)]
fn move_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

#[cfg(windows)]
fn move_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    use windows_sys::Win32::System::WindowsProgramming::FILE_RENAME_FLAG_POSIX_SEMANTICS;
    windows_rename(source, destination, FILE_RENAME_FLAG_POSIX_SEMANTICS)
}

#[cfg(unix)]
fn configure_replacement_permissions(target: &Path, replacement: &Path) -> io::Result<u32> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mode = fs::metadata(target)?.mode() & 0o7777;
    fs::set_permissions(replacement, fs::Permissions::from_mode(mode))?;
    Ok(mode)
}

#[cfg(unix)]
fn permission_value(path: &Path) -> io::Result<u32> {
    use std::os::unix::fs::MetadataExt;
    Ok(fs::metadata(path)?.mode() & 0o7777)
}

#[cfg(windows)]
fn configure_replacement_permissions(_target: &Path, _replacement: &Path) -> io::Result<u32> {
    Ok(0)
}

#[cfg(windows)]
fn permission_value(_path: &Path) -> io::Result<u32> {
    Ok(0)
}

fn write_replacement(root: &Path, iteration: usize, content: &[u8]) -> io::Result<PathBuf> {
    let replacement = root.join(format!("replacement-{iteration}.tmp"));
    fs::write(&replacement, content)?;
    Ok(replacement)
}

fn main() -> io::Result<()> {
    let root = std::env::temp_dir().join(format!(
        "resourcefs-atomic-replace-probe-{}-{}",
        std::process::id(),
        std::env::consts::OS,
    ));
    fs::create_dir_all(&root)?;

    let old = vec![b'A'; CONTENT_BYTES];
    let new = vec![b'B'; CONTENT_BYTES];
    let target = root.join("target.txt");
    fs::write(&target, &old)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640))?;
    }
    let expected_permission = permission_value(&target)?;

    let stop = Arc::new(AtomicBool::new(false));
    let invalid_observations = Arc::new(AtomicUsize::new(0));
    let sharing_read_errors = Arc::new(AtomicUsize::new(0));
    let not_found_read_errors = Arc::new(AtomicUsize::new(0));
    let access_denied_read_errors = Arc::new(AtomicUsize::new(0));
    let other_read_errors = Arc::new(AtomicUsize::new(0));
    let observed_old = Arc::new(AtomicUsize::new(0));
    let observed_new = Arc::new(AtomicUsize::new(0));
    let reader = {
        let stop = Arc::clone(&stop);
        let invalid_observations = Arc::clone(&invalid_observations);
        let sharing_read_errors = Arc::clone(&sharing_read_errors);
        let not_found_read_errors = Arc::clone(&not_found_read_errors);
        let access_denied_read_errors = Arc::clone(&access_denied_read_errors);
        let other_read_errors = Arc::clone(&other_read_errors);
        let observed_old = Arc::clone(&observed_old);
        let observed_new = Arc::clone(&observed_new);
        let target = target.clone();
        let old = old.clone();
        let new = new.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match fs::read(&target) {
                    Ok(content) if content == old => {
                        observed_old.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(content) if content == new => {
                        observed_new.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(_) => {
                        invalid_observations.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) if error.raw_os_error() == Some(32) => {
                        sharing_read_errors.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) if error.raw_os_error() == Some(2) => {
                        not_found_read_errors.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) if error.raw_os_error() == Some(5) => {
                        access_denied_read_errors.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        other_read_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        })
    };

    for iteration in 0..REPLACEMENTS {
        let content = if iteration % 2 == 0 { &new } else { &old };
        let replacement = write_replacement(&root, iteration, content)?;
        let source_permission = configure_replacement_permissions(&target, &replacement)?;
        if source_permission != expected_permission {
            return Err(io::Error::other("source permission changed before replace"));
        }
        replace_existing(&replacement, &target)?;
        if permission_value(&target)? != expected_permission {
            return Err(io::Error::other("replacement permission did not match target"));
        }
    }
    stop.store(true, Ordering::Release);
    reader.join().map_err(|_| io::Error::other("reader panicked"))?;

    let move_source = root.join("move-source.txt");
    let move_destination = root.join("move-destination.txt");
    fs::write(&move_source, b"source")?;
    fs::write(&move_destination, b"destination")?;
    let existing_destination_error = move_noreplace(&move_source, &move_destination).is_err();
    let existing_source_unchanged = fs::read(&move_source)? == b"source";
    let existing_destination_unchanged = fs::read(&move_destination)? == b"destination";

    fs::remove_file(&move_destination)?;
    move_noreplace(&move_source, &move_destination)?;
    let absent_destination_moved = !move_source.exists() && fs::read(&move_destination)? == b"source";

    let report = json!({
        "platform": std::env::consts::OS,
        "replacement": {
            "iterations": REPLACEMENTS,
            "primitive": replacement_primitive(),
            "bytesPerVersion": CONTENT_BYTES,
            "observedOld": observed_old.load(Ordering::Relaxed),
            "observedNew": observed_new.load(Ordering::Relaxed),
            "invalidObservations": invalid_observations.load(Ordering::Relaxed),
            "sharingViolationReadErrors": sharing_read_errors.load(Ordering::Relaxed),
            "notFoundReadErrors": not_found_read_errors.load(Ordering::Relaxed),
            "accessDeniedReadErrors": access_denied_read_errors.load(Ordering::Relaxed),
            "otherReadErrors": other_read_errors.load(Ordering::Relaxed),
            "permissionPreserved": permission_value(&target)? == expected_permission,
        },
        "noClobberMove": {
            "existingDestinationRejected": existing_destination_error,
            "sourceUnchangedOnConflict": existing_source_unchanged,
            "destinationUnchangedOnConflict": existing_destination_unchanged,
            "absentDestinationMoved": absent_destination_moved,
        },
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    fs::remove_dir_all(&root)?;
    Ok(())
}
'''

UPLOAD_CHUNK_BYTES = 60_000


def run(command: list[str], *, cwd: Path, env: dict[str, str] | None = None) -> str:
    completed = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {command!r}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed.stdout.strip()


def credentials() -> dict[str, str]:
    path = Path.home() / ".config" / "resourcefs" / "windows-vm" / "credentials.env"
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        name, value = line.split("=", 1)
        values[name] = value
    return values


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


def run_windows(executable: Path) -> dict[str, object]:
    values = credentials()
    session = winrm.Session(
        "http://127.0.0.1:55985/wsman",
        auth=(values["RFS_WINDOWS_USER"], values["RFS_WINDOWS_PASSWORD"]),
        transport="basic",
        server_cert_validation="ignore",
    )
    remote = rf"C:\Windows\Temp\resourcefs-atomic-probe-{uuid.uuid4().hex}.exe"
    upload(session, executable, remote)
    try:
        result = session.run_cmd(remote)
        if result.status_code != 0:
            raise RuntimeError(result.std_err.decode("utf-8", errors="replace"))
        return json.loads(result.std_out.decode("utf-8"))
    finally:
        session.run_ps(f"Remove-Item -LiteralPath '{remote}' -Force -ErrorAction SilentlyContinue")


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-atomic-probe-build-") as temporary:
        project = Path(temporary)
        (project / "src").mkdir()
        (project / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
        (project / "src" / "main.rs").write_text(MAIN_RS, encoding="utf-8")

        linux = json.loads(run(["cargo", "run", "--quiet"], cwd=project))
        run(["cargo", "check", "--quiet", "--target", "x86_64-apple-darwin"], cwd=project)

        build_env = os.environ.copy()
        build_env["RUSTFLAGS"] = "-C target-feature=+crt-static"
        run(
            ["cargo", "xwin", "build", "--quiet", "--target", "x86_64-pc-windows-msvc"],
            cwd=project,
            env=build_env,
        )
        windows = run_windows(
            project / "target" / "x86_64-pc-windows-msvc" / "debug" / "resourcefs-atomic-replace-probe.exe"
        )

        print(
            json.dumps(
                {
                    "linuxRuntime": linux,
                    "windowsRuntime": windows,
                    "macosCrossCheck": {
                        "target": "x86_64-apple-darwin",
                        "status": "cargo check passed",
                        "rustixRenameFlags": "NOREPLACE",
                    },
                },
                indent=2,
                sort_keys=True,
            )
        )


if __name__ == "__main__":
    main()
