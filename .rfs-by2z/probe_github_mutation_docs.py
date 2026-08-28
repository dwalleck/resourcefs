#!/usr/bin/env python3
"""Extract the six rfs-by2z mutation contracts from GitHub's official REST docs."""

from __future__ import annotations

import json
import re
import urllib.request

DOCS = {
    "issues": "https://docs.github.com/en/rest/issues/issues.md?apiVersion=2022-11-28",
    "comments": "https://docs.github.com/en/rest/issues/comments.md?apiVersion=2022-11-28",
    "pulls": "https://docs.github.com/en/rest/pulls/pulls.md?apiVersion=2022-11-28",
}

OPERATIONS = {
    "create_issue": ("issues", "Create an issue"),
    "update_issue": ("issues", "Update an issue"),
    "create_comment": ("comments", "Create an issue comment"),
    "update_comment": ("comments", "Update an issue comment"),
    "create_pull": ("pulls", "Create a pull request"),
    "update_pull": ("pulls", "Update a pull request"),
}


def fetch(url: str) -> str:
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "resourcefs-rfs-by2z-evidence/1.0"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read().decode("utf-8")


def section(document: str, heading: str) -> str:
    marker = f"## {heading}\n"
    start = document.find(marker)
    if start < 0:
        raise RuntimeError(f"missing heading: {heading}")
    end = document.find("\n## ", start + len(marker))
    return document[start:] if end < 0 else document[start:end]


def body_parameters(operation: str) -> dict[str, dict[str, object]]:
    marker = "#### Body parameters\n"
    start = operation.find(marker)
    if start < 0:
        return {}
    end = operation.find("\n### ", start + len(marker))
    body = operation[start:] if end < 0 else operation[start:end]
    found: dict[str, dict[str, object]] = {}
    pattern = re.compile(
        r"^[-*] \*\*`([^`]+)`\*\* \(([^)]+)\)( \(required\))?$",
        re.MULTILINE,
    )
    for match in pattern.finditer(body):
        found[match.group(1)] = {
            "type": match.group(2),
            "required": match.group(3) is not None,
        }
    return found


def status_codes(operation: str) -> list[int]:
    marker = "### HTTP response status codes\n"
    start = operation.find(marker)
    if start < 0:
        raise RuntimeError("missing status-code section")
    end = operation.find("\n### ", start + len(marker))
    statuses = operation[start:] if end < 0 else operation[start:end]
    return [
        int(value)
        for value in re.findall(r"^[-*] \*\*(\d{3})\*\*", statuses, re.MULTILINE)
    ]


def response_contract(operation: str) -> dict[str, object]:
    success = re.search(r"Response schema \(Status: (\d{3})\):", operation)
    if success is None:
        raise RuntimeError("missing success response schema")
    response = operation[success.end() :]
    keys = {
        key
        for key in ("number", "id", "title", "body")
        if re.search(rf"`{key}`", response)
    }
    return {"status": int(success.group(1)), "documented_fields": sorted(keys)}


def main() -> None:
    documents = {name: fetch(url) for name, url in DOCS.items()}
    result: dict[str, object] = {}
    for name, (document_name, heading) in OPERATIONS.items():
        operation = section(documents[document_name], heading)
        endpoint = re.search(r"```\n(GET|POST|PATCH|PUT|DELETE) ([^\n]+)\n```", operation)
        if endpoint is None:
            raise RuntimeError(f"missing endpoint for {heading}")
        result[name] = {
            "method": endpoint.group(1),
            "path": endpoint.group(2),
            "body": body_parameters(operation),
            "statuses": status_codes(operation),
            "response": response_contract(operation),
            "documents_if_match": "if-match" in operation.lower(),
        }
    print(json.dumps({"claim": "C1", "operations": result}, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
