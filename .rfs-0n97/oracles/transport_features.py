#!/usr/bin/env python3
"""C09: fence the actual all-feature Cargo graph, not manifest spellings."""
import argparse
import json
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--all-features", "--locked"],
        cwd=root, text=True, capture_output=True, check=False,
    )
    if result.returncode:
        print("C09 FAIL: unable to resolve all-feature Cargo graph", file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        return 1
    graph = json.loads(result.stdout)
    packages = {package["id"]: package for package in graph["packages"]}
    nodes = graph.get("resolve", {}).get("nodes", [])
    clients = [node for node in nodes if packages[node["id"]]["name"] == "reqwest"]
    if not clients:
        print("C09 FAIL: no resolved reqwest node; feature premise unproved", file=sys.stderr)
        return 1
    for node in clients:
        forbidden = {"http2", "http3"}.intersection(node["features"])
        if forbidden:
            print(f"C09 FAIL: {node['id']} enables {sorted(forbidden)}; physical-attempt premise requires reproof", file=sys.stderr)
            return 1
        print(f"C09 reqwest {packages[node['id']]['version']}: {','.join(sorted(node['features']))}")
    print("C09 PASS: resolved all-feature reqwest graph enables neither http2 nor http3")
    return 0


if __name__ == "__main__":
    sys.exit(main())
