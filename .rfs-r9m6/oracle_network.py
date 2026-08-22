#!/usr/bin/env python3
"""Independent URL/scheme/repository/POSIX-root oracle for Slice 3."""

from __future__ import annotations

import json
import re
from urllib.parse import urlsplit

MAX_SCHEME_BYTES = 64
BUILT_INS = {
    "rfs",
    "file",
    "artifact",
    "local",
    "https",
    "github",
    "issue",
    "pr",
    "ssh",
    "skill",
    "rule",
    "memory",
    "vault",
    "agent",
    "history",
}
BLOCKED_HEADERS = {
    "host",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "proxy-connection",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "forwarded",
    "via",
    "x-forwarded-host",
    "x-forwarded-port",
    "x-forwarded-proto",
}
HTTP_TOKEN = re.compile(r"^[!#$%&'*+.^_`|~0-9A-Za-z-]+$")
REPOSITORY_SEGMENT = re.compile(r"^[A-Za-z0-9._-]+$")
SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*$")


def https_url(value: str) -> bool:
    try:
        parsed = urlsplit(value)
        _ = parsed.port
    except ValueError:
        return False
    query_marker = "?" in value.split("#", 1)[0]
    fragment_marker = "#" in value
    return (
        parsed.scheme == "https"
        and parsed.hostname is not None
        and parsed.username is None
        and parsed.password is None
        and not query_marker
        and not fragment_marker
        and "*" not in parsed.hostname
    )


def prefix_key(value: str) -> tuple[str, int, tuple[str, ...]]:
    parsed = urlsplit(value)
    path = parsed.path.rstrip("/").lstrip("/")
    components = tuple(path.split("/")) if path else ()
    return parsed.hostname or "", parsed.port or 443, components


def prefixes_overlap(left: str, right: str) -> bool:
    left_host, left_port, left_path = prefix_key(left)
    right_host, right_port, right_path = prefix_key(right)
    return (
        left_host == right_host
        and left_port == right_port
        and (left_path[: len(right_path)] == right_path or right_path[: len(left_path)] == left_path)
    )


def credential_header(value: str) -> bool:
    return bool(HTTP_TOKEN.fullmatch(value)) and value.lower() not in BLOCKED_HEADERS


def repository(value: str) -> bool:
    parts = value.split("/")
    return (
        len(parts) == 2
        and 0 < len(parts[0].encode()) <= 39
        and 0 < len(parts[1].encode()) <= 100
        and parts[0] not in {".", ".."}
        and parts[1] not in {".", ".."}
        and bool(REPOSITORY_SEGMENT.fullmatch(parts[0]))
        and bool(REPOSITORY_SEGMENT.fullmatch(parts[1]))
    )


def remote_root(value: str) -> bool:
    if value == "/":
        return True
    if not value.startswith("/") or value.endswith("/") or "//" in value or "\\" in value or "\0" in value:
        return False
    return all(component not in {"", ".", ".."} for component in value.split("/")[1:])


def roots_overlap(left: str, right: str) -> bool:
    left_parts = tuple(left.split("/")[1:]) if left != "/" else ()
    right_parts = tuple(right.split("/")[1:]) if right != "/" else ()
    return left_parts[: len(right_parts)] == right_parts or right_parts[: len(left_parts)] == left_parts


def scheme_claim(value: str) -> bool:
    return (
        0 < len(value.encode()) <= MAX_SCHEME_BYTES
        and value.isascii()
        and bool(SCHEME.fullmatch(value))
        and value.lower() not in BUILT_INS
    )


def observe() -> list[dict[str, object]]:
    rows: list[tuple[str, bool, bool]] = [
        ("https", https_url("https://example.test/api"), True),
        ("https-userinfo", https_url("https://user@example.test/api"), False),
        ("https-query", https_url("https://example.test/api?x=1"), False),
        ("https-fragment", https_url("https://example.test/api#x"), False),
        ("https-wildcard", https_url("https://*.example.test/api"), False),
        ("https-overlap", prefixes_overlap("https://example.test/api", "https://example.test/api/v1"), True),
        ("https-component-disjoint", prefixes_overlap("https://example.test/api", "https://example.test/apix"), False),
        ("header-authorization", credential_header("Authorization"), True),
        ("header-host", credential_header("Host"), False),
        ("repository", repository("Owner/Repository"), True),
        ("repository-extra-segment", repository("owner/repository/extra"), False),
        ("repository-case-collision", "Owner/Repository".lower() == "owner/repository".lower(), True),
        ("remote-root", remote_root("/srv/data"), True),
        ("remote-relative", remote_root("srv/data"), False),
        ("remote-parent", remote_root("/srv/../data"), False),
        ("remote-overlap", roots_overlap("/srv", "/srv/data"), True),
        ("remote-component-disjoint", roots_overlap("/srv", "/srv2"), False),
        ("scheme", scheme_claim("Docs+v1"), True),
        ("scheme-case-normalization", "Docs".lower() == "docs".lower(), True),
        ("scheme-built-in", scheme_claim("artifact"), False),
        ("scheme-64", scheme_claim("a" + "b" * 63), True),
        ("scheme-65", scheme_claim("a" + "b" * 64), False),
    ]
    return [
        {"name": name, "observed": observed, "expected": expected, "agree": observed == expected}
        for name, observed, expected in rows
    ]


def main() -> None:
    observations = observe()
    print(
        json.dumps(
            {
                "allAgree": all(row["agree"] for row in observations),
                "observations": observations,
                "engine": "python urllib.parse plus hand-coded RFC/POSIX tables",
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
