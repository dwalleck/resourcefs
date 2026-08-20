#!/usr/bin/env python3
"""Independent byte-preserving oracle for the rfs-34pz selector corpus."""

from __future__ import annotations

import json
import sys
from pathlib import Path


def parse_range(atom: str) -> tuple[int, int | None]:
    if "+" in atom:
        start_text, count_text = atom.split("+", 1)
        start = positive(start_text)
        count = positive(count_text)
        return start, start + count - 1
    if "-" in atom:
        start_text, end_text = atom.split("-", 1)
        start = positive(start_text)
        if not end_text:
            return start, None
        end = positive(end_text)
        if end < start:
            raise ValueError("descending range")
        return start, end
    return positive(atom), None


def positive(value: str) -> int:
    if not value or not value.isascii() or not value.isdigit():
        raise ValueError("not an ASCII integer")
    number = int(value)
    if number == 0 or value != str(number):
        raise ValueError("not canonical positive decimal")
    return number


def select(content: str, spelling: str) -> str:
    if spelling == "raw":
        return content
    ranges = spelling.removeprefix("raw:")
    if not ranges or ranges == spelling and spelling.startswith("raw"):
        raise ValueError("malformed raw selector")
    lines = content.splitlines(keepends=True)
    selected: list[str] = []
    for atom in ranges.split(","):
        start, end = parse_range(atom)
        if start > len(lines):
            raise IndexError("start past EOF")
        stop = len(lines) if end is None else min(end, len(lines))
        selected.extend(lines[start - 1 : stop])
    return "".join(selected)


def main() -> None:
    fixture_path = Path(sys.argv[1])
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    checked = 0
    for case in fixture["cases"]:
        actual = select(case["content"], case["selector"])
        if actual.encode() != case["expected"].encode():
            raise AssertionError(f"{case['name']}: {actual!r} != {case['expected']!r}")
        checked += 1
    for case in fixture["errors"]:
        try:
            select(case["content"], case["selector"])
        except IndexError:
            if case["category"] != "invalid_reference":
                raise AssertionError(f"{case['name']}: wrong expected category")
        else:
            raise AssertionError(f"{case['name']}: expected start-past-EOF failure")
        checked += 1
    for spelling in fixture["invalidSelectors"]:
        try:
            if spelling.startswith("page:"):
                offset = spelling.removeprefix("page:")
                if positive(offset) == 0:
                    raise ValueError("zero page")
            else:
                probe = "x" if spelling == "raw" else spelling
                if probe != "raw":
                    for atom in probe.removeprefix("raw:").split(","):
                        parse_range(atom)
        except ValueError:
            checked += 1
        else:
            raise AssertionError(f"{spelling!r}: expected syntax failure")
    print(json.dumps({"checked": checked, "status": "ok"}, sort_keys=True))


if __name__ == "__main__":
    main()
