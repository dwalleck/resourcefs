#!/usr/bin/env python3
"""Compute direct-argv and explicit-environment behavior through Python subprocess."""

from __future__ import annotations

import json
import os
import subprocess


def main() -> None:
    argument = "literal;$HOME&|<>()"
    child_environment = {
        "PATH": "/usr/bin",
        "RFS_LITERAL": "value with spaces;$HOME&|",
    }
    environment_run = subprocess.run(
        ["env", "-0"],
        check=True,
        stdout=subprocess.PIPE,
        env=child_environment,
    )
    observed_environment = {}
    for entry in environment_run.stdout.rstrip(b"\0").split(b"\0"):
        name, value = entry.decode("utf-8").split("=", 1)
        observed_environment[name] = value
    argument_run = subprocess.run(
        ["printf", "<%s>", argument],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        env=child_environment,
    )
    observed_argument = argument_run.stdout.removeprefix("<").removesuffix(">")
    print(
        json.dumps(
            {
                "platform": os.name,
                "arguments": [observed_argument],
                "environment": observed_environment,
                "parentSecretPresent": "RFS_PARENT_SECRET_SENTINEL"
                in observed_environment,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
