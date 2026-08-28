#!/usr/bin/env python3
"""Extract mutation contracts from Octokit's published GitHub OpenAPI types."""

from __future__ import annotations

import io
import json
import re
import tarfile
import urllib.request

METADATA_URL = "https://registry.npmjs.org/%40octokit%2Fopenapi-types/latest"

OPERATIONS = {
    "create_issue": "issues/create",
    "update_issue": "issues/update",
    "create_comment": "issues/create-comment",
    "update_comment": "issues/update-comment",
    "create_pull": "pulls/create",
    "update_pull": "pulls/update",
}


def fetch_types() -> tuple[str, str]:
    request = urllib.request.Request(
        METADATA_URL,
        headers={"User-Agent": "resourcefs-rfs-by2z-evidence/1.0"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        metadata = json.load(response)
    tarball_url = metadata["dist"]["tarball"]
    request = urllib.request.Request(
        tarball_url,
        headers={"User-Agent": "resourcefs-rfs-by2z-evidence/1.0"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        archive = tarfile.open(fileobj=io.BytesIO(response.read()), mode="r:gz")
    member = archive.extractfile("package/types.d.ts")
    if member is None:
        raise RuntimeError("package/types.d.ts is absent")
    types = member.read().decode("utf-8")
    types = re.sub(r"/\*.*?\*/", "", types, flags=re.DOTALL)
    types = re.sub(r"//[^\n]*", "", types)
    return metadata["version"], types


def braced_block(text: str, marker: str) -> str:
    start = text.find(marker)
    if start < 0:
        raise RuntimeError(f"missing marker: {marker}")
    opening = text.find("{", start + len(marker))
    if opening < 0:
        raise RuntimeError(f"missing opening brace: {marker}")
    depth = 0
    quote: str | None = None
    escaped = False
    for index in range(opening, len(text)):
        character = text[index]
        if quote is not None:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == quote:
                quote = None
            continue
        if character in {'"', "'", "`"}:
            quote = character
        elif character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return text[start : index + 1]
    raise RuntimeError(f"unterminated block: {marker}")


def operation_route(types: str, operation_id: str) -> tuple[str, str]:
    pattern = re.compile(r'^\s+"(/repos/[^"]+)": \{', re.MULTILINE)
    for match in pattern.finditer(types):
        block = braced_block(types, match.group(0)[:-1])
        method = re.search(
            rf"^\s+(post|patch): operations\[\"{re.escape(operation_id)}\"\];",
            block,
            re.MULTILINE,
        )
        if method is not None:
            return method.group(1).upper(), match.group(1)
    raise RuntimeError(f"missing route for {operation_id}")


def operation_contract(types: str, operation_id: str) -> dict[str, object]:
    marker = f'"{operation_id}": '
    if marker not in types:
        candidates = sorted(
            set(re.findall(r'\"((?:issues|pulls)/[^\\\"]*(?:create|update)[^\\\"]*)\"', types))
        )
        raise RuntimeError(f"missing {operation_id}; candidates={candidates}")
    block = braced_block(types, marker)
    method, path = operation_route(types, operation_id)
    request_fields: list[str] = []
    if "requestBody:" in block:
        request_block = braced_block(block, "requestBody: ")
        request_fields = sorted(
            set(
                re.findall(
                    r"^\s{8,}([A-Za-z_][A-Za-z0-9_]*)(?:\?)?:",
                    request_block,
                    re.MULTILINE,
                )
            )
        )
    response_block = braced_block(block, "responses: ")
    statuses = sorted(
        {
            int(value)
            for value in re.findall(r"^\s+(\d{3}):", response_block, re.MULTILINE)
        }
    )
    success = 201 if method == "POST" else 200
    success_block = braced_block(response_block, f"{success}: ")
    schema = re.search(r'components\["schemas"\]\["([^"]+)"\]', success_block)
    return {
        "method": method,
        "path": path,
        "request_json_fields": request_fields,
        "statuses": statuses,
        "success_status": success,
        "response_schema": None if schema is None else schema.group(1),
        "documents_if_match": "if-match" in block.lower(),
    }


def main() -> None:
    version, types = fetch_types()
    result = {
        name: operation_contract(types, operation_id)
        for name, operation_id in OPERATIONS.items()
    }
    print(
        json.dumps(
            {"claim": "C1", "package_version": version, "operations": result},
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
