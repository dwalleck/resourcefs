#!/usr/bin/env python3
"""Read-only empirical probe for rfs-45ww GitHub REST premises."""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any
import urllib.parse

API = "https://api.github.com"
RUST_REPO = "/repos/rust-lang/rust"
PULL_NUMBER = 159232
BINARY_REPO = "/repos/github/docs"
BINARY_PULL_NUMBER = 41852


@dataclass(frozen=True)
class Response:
    status: int
    headers: dict[str, str]
    body: bytes


def get(path: str, *, accept: str = "application/vnd.github+json", etag: str | None = None) -> Response:
    headers = {
        "Accept": accept,
        "User-Agent": "ResourceFS-rfs-45ww-empirical-probe",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if etag:
        headers["If-None-Match"] = etag

    request = urllib.request.Request(f"{API}{path}", headers=headers, method="GET")
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return Response(
                response.status,
                {name.lower(): value for name, value in response.headers.items()},
                response.read(),
            )
    except urllib.error.HTTPError as error:
        return Response(
            error.code,
            {name.lower(): value for name, value in error.headers.items()},
            error.read(),
        )


def decode(response: Response) -> Any:
    return json.loads(response.body)


def required_shape(rows: list[dict[str, Any]], fields: set[str]) -> dict[str, Any]:
    return {
        "count": len(rows),
        "ids_are_unique_integers": all(isinstance(row.get("id"), int) for row in rows)
        and len({row["id"] for row in rows}) == len(rows),
        "missing_fields": sorted(
            {
                field
                for row in rows
                for field in fields
                if field not in row
            }
        ),
    }


def selected_headers(response: Response) -> dict[str, str]:
    names = (
        "etag",
        "link",
        "retry-after",
        "x-ratelimit-limit",
        "x-ratelimit-remaining",
        "x-ratelimit-reset",
        "x-ratelimit-resource",
        "x-ratelimit-used",
    )
    return {name: response.headers[name] for name in names if name in response.headers}


def main() -> None:
    issue = get(f"{RUST_REPO}/issues/{PULL_NUMBER}")
    pull = get(f"{RUST_REPO}/pulls/{PULL_NUMBER}")
    issue_object = decode(issue)
    pull_object = decode(pull)

    conversation_response = get(f"{RUST_REPO}/issues/{PULL_NUMBER}/comments?per_page=100")
    reviews_response = get(f"{RUST_REPO}/pulls/{PULL_NUMBER}/reviews?per_page=100")
    inline_response = get(f"{RUST_REPO}/pulls/{PULL_NUMBER}/comments?per_page=100")
    files_response = get(f"{RUST_REPO}/pulls/{PULL_NUMBER}/files?per_page=100")
    binary_files_response = get(
        f"{BINARY_REPO}/pulls/{BINARY_PULL_NUMBER}/files?per_page=100"
    )

    conversation = decode(conversation_response)
    reviews = decode(reviews_response)
    inline = decode(inline_response)
    files = decode(files_response)
    binary_files = decode(binary_files_response)

    diff = get(
        f"{RUST_REPO}/pulls/{PULL_NUMBER}",
        accept="application/vnd.github.diff",
    )

    listing = get(f"{RUST_REPO}/issues?state=all&sort=updated&direction=desc&per_page=1&page=1")
    link_target = listing.headers["link"].split(";", 1)[0].strip("<>")
    parsed_link = urllib.parse.urlsplit(link_target)
    linked_page = get(f"{parsed_link.path}?{parsed_link.query}")
    numeric_page = get(
        f"{RUST_REPO}/issues?state=all&sort=updated&direction=desc&per_page=1&page=2"
    )

    etag = listing.headers.get("etag")
    conditional = (
        get(
            f"{RUST_REPO}/issues?state=all&sort=updated&direction=desc&per_page=1&page=1",
            etag=etag,
        )
        if etag
        else None
    )

    missing = get(f"{RUST_REPO}/issues/999999999")
    missing_object = decode(missing)

    facts = {
        "P1": {
            "issue_status": issue.status,
            "pull_status": pull.status,
            "issue_number": issue_object.get("number"),
            "pull_number": pull_object.get("number"),
            "same_number": issue_object.get("number") == pull_object.get("number"),
            "issue_has_pull_request_discriminator": isinstance(
                issue_object.get("pull_request"), dict
            ),
        },
        "P2": {
            "conversation_comments": required_shape(
                conversation,
                {"id", "body", "user", "created_at", "updated_at", "issue_url"},
            ),
            "reviews": required_shape(
                reviews,
                {"id", "body", "user", "state", "submitted_at", "commit_id"},
            ),
            "inline_review_comments": required_shape(
                inline,
                {
                    "id",
                    "body",
                    "user",
                    "path",
                    "diff_hunk",
                    "created_at",
                    "updated_at",
                    "pull_request_review_id",
                },
            ),
        },
        "P3": {
            "diff_status": diff.status,
            "diff_content_type": diff.headers.get("content-type"),
            "diff_starts_with_file_header": diff.body.startswith(b"diff --git "),
            "diff_bytes": len(diff.body),
            "files_status": files_response.status,
            "filenames_in_upstream_order": [row.get("filename") for row in files],
            "fixed_pull_patch_presence": ["patch" in row and row.get("patch") is not None for row in files],
            "binary_fixture": [
                {
                    "filename": row.get("filename"),
                    "has_patch_key": "patch" in row,
                    "patch_is_null": row.get("patch") is None,
                }
                for row in binary_files
            ],
        },
        "P4": {
            "list_status": listing.status,
            "list_count": len(decode(listing)),
            "list_link": listing.headers.get("link"),
            "etag_present": etag is not None,
            "conditional_status": conditional.status if conditional else None,
            "conditional_body_bytes": len(conditional.body) if conditional else None,
        },
        "P5": {
            "success_status": issue.status,
            "success_headers": selected_headers(issue),
            "missing_status": missing.status,
            "missing_headers": selected_headers(missing),
            "missing_error_keys": sorted(missing_object.keys()),
            "missing_message_is_string": isinstance(missing_object.get("message"), str),
        },
        "P6": {
            "linked_page_status": linked_page.status,
            "numeric_page_status": numeric_page.status,
            "linked_first_id": decode(linked_page)[0]["id"],
            "numeric_first_id": decode(numeric_page)[0]["id"],
            "same_first_id": decode(linked_page)[0]["id"] == decode(numeric_page)[0]["id"],
        },
    }
    print(json.dumps(facts, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
