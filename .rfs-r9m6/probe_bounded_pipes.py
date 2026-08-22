#!/usr/bin/env python3
"""Probe tokio 1.47.1 concurrent bounded stdout/stderr draining without deadlock."""

from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

CARGO_TOML = """\
[package]
name = "resourcefs-bounded-pipe-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
tokio = { version = "=1.47.1", features = ["io-util", "macros", "process", "rt-multi-thread", "time"] }
"""

MAIN_RS = r'''
use std::{env, io::Write, process::Stdio, thread, time::Instant};
use tokio::{io::AsyncReadExt, process::Command, time::{Duration, timeout}};

const LIMIT: usize = 65_536;
const CHUNK: [u8; 4_096] = [b'x'; 4_096];

fn child() {
    let stdout = thread::spawn(|| {
        let mut stream = std::io::stdout().lock();
        loop {
            if stream.write_all(&CHUNK).is_err() || stream.flush().is_err() {
                break;
            }
        }
    });
    let stderr = thread::spawn(|| {
        let mut stream = std::io::stderr().lock();
        loop {
            if stream.write_all(&CHUNK).is_err() || stream.flush().is_err() {
                break;
            }
        }
    });
    stdout.join().expect("stdout writer");
    stderr.join().expect("stderr writer");
}

async fn bounded(mut stream: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<usize> {
    let mut total = 0usize;
    let mut buffer = [0u8; 8_192];
    while total <= LIMIT {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read);
    }
    Ok(total.min(LIMIT + 1))
}

#[tokio::main]
async fn main() {
    if env::args().nth(1).as_deref() == Some("--child") {
        child();
        return;
    }
    let started = Instant::now();
    let mut child = Command::new(env::current_exe().expect("current executable"))
        .arg("--child")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn writer child");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (stdout_count, stderr_count) = timeout(Duration::from_secs(5), async {
        tokio::join!(bounded(stdout), bounded(stderr))
    })
    .await
    .expect("bounded readers did not deadlock");
    let stdout_count = stdout_count.expect("read stdout");
    let stderr_count = stderr_count.expect("read stderr");
    let _ = child.start_kill();
    let status = timeout(Duration::from_secs(1), child.wait())
        .await
        .expect("child was not reaped")
        .expect("wait for child");
    println!(
        "{{\"tokio\":\"1.47.1\",\"stdoutObserved\":{stdout_count},\"stderrObserved\":{stderr_count},\"stdoutOverLimit\":{},\"stderrOverLimit\":{},\"reaped\":true,\"statusSuccess\":{},\"elapsedMs\":{}}}",
        stdout_count > LIMIT,
        stderr_count > LIMIT,
        status.success(),
        started.elapsed().as_millis()
    );
}
'''


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-bounded-pipe-probe-") as temp_dir:
        root = Path(temp_dir)
        (root / "src").mkdir()
        (root / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
        (root / "src" / "main.rs").write_text(MAIN_RS, encoding="utf-8")
        completed = subprocess.run(
            ["cargo", "run", "--quiet", "--manifest-path", str(root / "Cargo.toml")],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
        )
        print(completed.stdout, end="")


if __name__ == "__main__":
    main()
