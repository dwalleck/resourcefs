#!/usr/bin/env python3
"""Independently extract mutation routes/types from the published go-github client."""

from __future__ import annotations

import io
import json
import re
import urllib.request
import zipfile

MODULE_URL = "https://proxy.golang.org/github.com/google/go-github/v74/@v/v74.0.0.zip"

OPERATIONS = {
    "create_issue": ("IssuesService", "Create"),
    "update_issue": ("IssuesService", "Edit"),
    "create_comment": ("IssuesService", "CreateComment"),
    "update_comment": ("IssuesService", "EditComment"),
    "create_pull": ("PullRequestsService", "Create"),
    "update_pull": ("PullRequestsService", "Edit"),
}


def fetch_zip() -> dict[str, str]:
    request = urllib.request.Request(
        MODULE_URL,
        headers={"User-Agent": "resourcefs-rfs-by2z-evidence/1.0"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        archive = zipfile.ZipFile(io.BytesIO(response.read()))
    return {
        name.rsplit("/", 1)[-1]: archive.read(name).decode("utf-8")
        for name in archive.namelist()
        if name.endswith(".go") and not name.endswith("_test.go")
    }


def method_block(source: str, service: str, method: str) -> str:
    marker = re.search(rf"^func \(s \*{service}\) {method}\(", source, re.MULTILINE)
    if marker is None:
        raise RuntimeError(f"missing {service}.{method}")
    next_function = re.search(r"^func \(", source[marker.end() :], re.MULTILINE)
    end = len(source) if next_function is None else marker.end() + next_function.start()
    return source[marker.start() : end]


def struct_fields(sources: dict[str, str], type_name: str) -> list[str]:
    for source in sources.values():
        marker = re.search(rf"^type {re.escape(type_name)} struct \{{", source, re.MULTILINE)
        if marker is None:
            continue
        end = source.find("\n}", marker.end())
        if end < 0:
            raise RuntimeError(f"unterminated struct {type_name}")
        block = source[marker.end() : end]
        fields = re.findall(r'^\s*\w+\s+[^`\n]+`json:"([^",]+)', block, re.MULTILINE)
        return sorted(set(fields))
    raise RuntimeError(f"missing request struct {type_name}")


def operation_contract(sources: dict[str, str], service: str, method: str) -> dict[str, object]:
    candidates = [source for source in sources.values() if f"func (s *{service}) {method}(" in source]
    if len(candidates) != 1:
        raise RuntimeError(f"ambiguous {service}.{method}: {len(candidates)} files")
    block = method_block(candidates[0], service, method)
    signature_end = block.find(" {")
    signature = block[:signature_end]
    route = re.search(r'fmt\.Sprintf\("([^"]+)"', block)
    request = re.search(r'NewRequest\("(POST|PATCH|PUT|DELETE)",\s*u,\s*(\w+)\)', block)
    if route is None or request is None:
        raise RuntimeError(f"missing route/request in {service}.{method}")
    request_variable = request.group(2)
    request_type = re.search(rf"\b{request_variable} \*(\w+)", signature)
    if request_type is None:
        pointer_arguments = re.findall(r"\b\w+ \*(\w+)", signature.split(") (", 1)[0])
        if pointer_arguments:
            request_type_name = pointer_arguments[-1]
        else:
            request_type_name = None
    else:
        request_type_name = request_type.group(1)
    response_type = re.search(r"\) \(\*(\w+), \*Response, error\)", signature)
    if request_type_name is None or response_type is None:
        raise RuntimeError(f"missing types in {service}.{method}: {signature!r}")
    return {
        "method": request.group(1),
        "path_format": route.group(1),
        "request_type": request_type_name,
        "request_json_fields": struct_fields(sources, request_type_name),
        "response_type": response_type.group(1),
        "sets_if_match": "If-Match" in block or "if-match" in block.lower(),
    }


def main() -> None:
    sources = fetch_zip()
    result = {
        name: operation_contract(sources, service, method)
        for name, (service, method) in OPERATIONS.items()
    }
    print(json.dumps({"claim": "C1", "operations": result}, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
