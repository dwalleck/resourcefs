#!/usr/bin/env python3
"""Throwaway probe: provider shape/pagination for PR conversation comments.

Read-only GETs against public github.com at REST 2022-11-28. Records only
presence/type/count projections, never comment bodies. No credential is used.
"""

import json
import urllib.error
import urllib.request

API = "https://api.github.com"
ACCEPT = "application/vnd.github+json"
VERSION = "2022-11-28"
REPO = "rust-lang/rust"
BUSY_PR = 112049
EMPTY_PR = 162473
FIELDS = ["id", "node_id", "url", "html_url", "issue_url", "user", "body",
          "created_at", "updated_at"]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def get(path):
    url = f"{API}{path}"
    request = urllib.request.Request(url)
    request.add_header("Accept", ACCEPT)
    request.add_header("X-GitHub-Api-Version", VERSION)
    request.add_header("User-Agent", "resourcefs-rfs-bfwa-probe")
    opener = urllib.request.build_opener(NoRedirect)
    try:
        with opener.open(request, timeout=20) as response:
            body = response.read()
            return {
                "status": response.status,
                "link": response.headers.get("Link"),
                "api_version": response.headers.get("X-GitHub-Api-Version"),
                "etag": response.headers.get("ETag"),
                "body_bytes": len(body),
                "json": json.loads(body.decode("utf-8")),
            }
    except urllib.error.HTTPError as error:
        body = error.read()
        return {
            "status": error.code,
            "link": error.headers.get("Link"),
            "body_bytes": len(body),
            "json": None,
        }


def next_target(link):
    if not link:
        return None
    for value in link.split(","):
        value = value.strip()
        if not value.startswith("<"):
            continue
        target, _, parameters = value[1:].partition(">")
        if any(
            part.strip().lower().startswith("rel=")
            and "next" in part.split("=", 1)[1].strip().strip('"').split()
            for part in parameters.split(";")
        ):
            return target
    return None


def page_projection(path):
    response = get(path)
    records = response["json"] if isinstance(response["json"], list) else []
    keys = sorted(records[0].keys()) if records else []
    presence = {}
    for field in FIELDS:
        observed = [record.get(field, "<absent>") for record in records]
        presence[field] = {
            "present": sum(1 for value in observed if value != "<absent>"),
            "null": sum(1 for value in observed if value is None),
            "types": sorted({type(value).__name__ for value in observed
                             if value != "<absent>"}),
        }
    return {
        "path": path,
        "status": response["status"],
        "next_target": next_target(response["link"]),
        "link_present": response["link"] is not None,
        "count": len(records),
        "record_keys": keys,
        "field_presence": presence,
        "pull_request_key_present": any("pull_request" in record for record in records),
        "body_bytes": response["body_bytes"],
    }


def main():
    busy = f"/repos/{REPO}/issues/{BUSY_PR}/comments"
    empty = f"/repos/{REPO}/issues/{EMPTY_PR}/comments"
    result = {
        "api": API,
        "rest_version": VERSION,
        "repository": REPO,
        "observations": {
            "busy_page1": page_projection(f"{busy}?per_page=100&page=1"),
            "busy_page2": page_projection(f"{busy}?per_page=100&page=2"),
            "busy_past_end": page_projection(f"{busy}?per_page=100&page=99999"),
            "busy_oversized_per_page": page_projection(f"{busy}?per_page=1000&page=1"),
            "empty_page1": page_projection(f"{empty}?per_page=100&page=1"),
            "empty_past_end": page_projection(f"{empty}?per_page=100&page=2"),
        },
    }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
