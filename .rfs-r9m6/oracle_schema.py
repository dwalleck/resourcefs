#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["jsonschema==4.25.1"]
# ///
"""Validate the product schema with an independent JSON Schema engine and hand table."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

from jsonschema import Draft202012Validator



def main() -> None:
    probe = Path(__file__).with_name("probe_schema.py")
    completed = subprocess.run(
        [str(probe)],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    )
    result = json.loads(completed.stdout)
    validator = Draft202012Validator(result["schema"])
    comparisons = []
    for observation in result["observations"]:
        name = observation["name"]
        schema_accepted = validator.is_valid(observation["value"])
        comparisons.append(
            {
                "name": name,
                "expected": observation["expected"],
                "schemaAccepted": schema_accepted,
                "agree": observation["expected"] == schema_accepted,
            }
        )
    print(
        json.dumps(
            {
                "engine": "python-jsonschema 4.25.1 Draft202012Validator",
                "allAgree": all(row["agree"] for row in comparisons),
                "comparisons": comparisons,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
