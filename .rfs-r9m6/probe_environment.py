#!/usr/bin/env python3
"""Probe Rust direct-argv lookup and env_clear behavior on the current native platform."""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path

MAIN_RS = r'''
use std::{env, fs, process::Command};

fn child() {
    for argument in env::args().skip(2) {
        println!("ARG={argument}");
    }
    let mut variables: Vec<(String, String)> = env::vars().collect();
    variables.sort();
    for (name, value) in variables {
        println!("ENV={name}={value}");
    }
}

fn main() {
    if env::args().nth(1).as_deref() == Some("--child") {
        child();
        return;
    }

    let executable = env::current_exe().expect("current executable");
    let directory = env::temp_dir().join(format!("resourcefs-env-probe-{}", std::process::id()));
    fs::create_dir(&directory).expect("create helper directory");
    let helper_name = if cfg!(windows) { "rfs-env-helper.exe" } else { "rfs-env-helper" };
    let helper = directory.join(helper_name);
    fs::copy(&executable, &helper).expect("copy helper executable");

    let mut command = Command::new(if cfg!(windows) { "rfs-env-helper" } else { "rfs-env-helper" });
    command
        .arg("--child")
        .arg("literal;$HOME&|<>()")
        .env_clear()
        .env("PATH", &directory)
        .env("RFS_LITERAL", "value with spaces;$HOME&|");
    if cfg!(windows) {
        command.env("SystemRoot", env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned()));
    }
    let output = command.output().expect("run helper by child PATH");
    assert!(output.status.success(), "helper failed: {:?}", output.status);
    print!("{}", String::from_utf8(output.stdout).expect("UTF-8 helper output"));

    fs::remove_file(helper).expect("remove helper executable");
    fs::remove_dir(directory).expect("remove helper directory");
}
'''


def parse(output: str) -> dict[str, object]:
    arguments: list[str] = []
    environment: dict[str, str] = {}
    for line in output.splitlines():
        if line.startswith("ARG="):
            arguments.append(line.removeprefix("ARG="))
        elif line.startswith("ENV="):
            name, value = line.removeprefix("ENV=").split("=", 1)
            environment[name] = value
        else:
            raise RuntimeError(f"unexpected probe line: {line!r}")
    return {"arguments": arguments, "environment": environment}


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-environment-probe-build-") as temp_dir:
        root = Path(temp_dir)
        source = root / "probe.rs"
        binary = root / ("probe.exe" if os.name == "nt" else "probe")
        source.write_text(MAIN_RS, encoding="utf-8")
        subprocess.run(["rustc", "--edition=2024", str(source), "-o", str(binary)], check=True)
        environment = dict(os.environ)
        environment["RFS_PARENT_SECRET_SENTINEL"] = "must-not-cross-boundary"
        completed = subprocess.run(
            [str(binary)],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            env=environment,
        )
        observation = parse(completed.stdout)
        observation["platform"] = os.name
        observation["parentSecretPresent"] = (
            "RFS_PARENT_SECRET_SENTINEL" in observation["environment"]
        )
        print(json.dumps(observation, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
