#!/usr/bin/env python3
"""Compare read-only enhanced POST pages with owned identities and direct GETs."""

import base64
import json
import os
from pathlib import Path
import re
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def main():
    if os.environ.get("RFS_LIVE") != "1":
        print(json.dumps({"skipped": "RFS_LIVE is not enabled"}))
        return
    origin = os.environ["RFS_ATLASSIAN_SITE"]
    parsed = urllib.parse.urlsplit(origin)
    if parsed.scheme != "https" or not parsed.hostname or not parsed.hostname.endswith(".atlassian.net") or parsed.username or parsed.password or parsed.port or parsed.path or parsed.query or parsed.fragment:
        raise RuntimeError("invalid disposable tenant origin")
    state = json.loads(Path(".resourcefs/atlassian-fixture-state.json").read_text())
    if state["site"]["origin"] != origin:
        raise RuntimeError("fixture receipt origin mismatch")
    project = state["jira"]["projects"][0]
    rows = [row for row in state["jira"]["issues"] if row["key"].startswith(project["key"] + "-")]
    if not 2 <= len(rows) <= 10:
        raise RuntimeError("fixture must supply two through ten project issues")
    for row in rows:
        if not re.fullmatch(r"[1-9][0-9]*", row["id"]) or not re.fullmatch(r"[A-Z][A-Z0-9_]*-[1-9][0-9]*", row["key"]):
            raise RuntimeError("invalid fixture identity")
    rows.sort(key=lambda row: int(row["key"].rsplit("-", 1)[1]))
    pair = os.environ["ATLASSIAN_READER_EMAIL"] + ":" + os.environ["ATLASSIAN_READER_API_TOKEN"]
    authorization = "Basic " + base64.b64encode(pair.encode()).decode("ascii")
    opener = urllib.request.build_opener(NoRedirect())
    requests = 0

    def request(path, payload=None):
        nonlocal requests
        requests += 1
        if requests > 40:
            raise RuntimeError("probe request bound exceeded")
        headers = {"Authorization": authorization, "Accept": "application/json", "User-Agent": "resourcefs-rfs-nae2-evidence/1"}
        data = None
        if payload is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        outgoing = urllib.request.Request(origin + path, data=data, headers=headers, method="POST" if payload is not None else "GET")
        try:
            with opener.open(outgoing, timeout=30) as response:
                body = response.read(4 * 1024 * 1024 + 1)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"native request failed with HTTP {error.code}") from None
        if len(body) > 4 * 1024 * 1024:
            raise RuntimeError("probe response bound exceeded")
        return json.loads(body)

    direct = {}
    for row in rows:
        document = request("/rest/api/3/issue/" + row["id"] + "?fields=summary,status,project")
        if document["id"] != row["id"] or document["key"] != row["key"] or document["self"] != origin + "/rest/api/3/issue/" + row["id"]:
            raise RuntimeError("direct identity disagrees with independent receipt")
        if document["fields"]["project"]["id"] != project["id"]:
            raise RuntimeError("direct issue escaped fixture project")
        direct[row["id"]] = document

    predicate = "key IN (" + ", ".join(json.dumps(row["key"]) for row in rows) + ")"
    traversals = []
    for direction in ("ASC", "DESC"):
        query = predicate + " ORDER BY key " + direction
        token = None
        seen = set()
        received = []
        pages = []
        for _ in range(10):
            payload = {"jql": query, "fields": ["key", "summary", "status", "project"], "maxResults": 1}
            if token is not None:
                payload["nextPageToken"] = token
            document = request("/rest/api/3/search/jql", payload)
            values = document["issues"]
            if len(values) > 1:
                raise RuntimeError("POST exceeded requested maximum")
            for value in values:
                expected = direct[value["id"]]
                if any(value[key] != expected[key] for key in ("id", "key", "self", "fields")):
                    raise RuntimeError("selected POST row disagrees with direct issue GET")
                received.append(value["id"])
            pages.append({"rows": len(values), "isLast": document.get("isLast"), "has_token": "nextPageToken" in document, "has_maxResults": "maxResults" in document})
            if "nextPageToken" not in document:
                if document.get("isLast") not in (None, True):
                    raise RuntimeError("contradictory terminal POST page")
                break
            token = document["nextPageToken"]
            if not isinstance(token, str) or not token or token in seen or document.get("isLast") not in (None, False):
                raise RuntimeError("invalid or repeated native POST token")
            seen.add(token)
        else:
            raise RuntimeError("POST traversal exceeded ten pages")
        expected_ids = [row["id"] for row in (rows if direction == "ASC" else reversed(rows))]
        if received != expected_ids:
            raise RuntimeError("POST order disagrees with independent fixture-key order")
        traversals.append({"direction": direction, "ordered_ids": received, "order_matches": True, "direct_rows_match": True, "pages": pages})

    complete = request("/rest/api/3/search/jql", {"jql": predicate + " ORDER BY key DESC", "fields": ["key", "summary", "status", "project"], "maxResults": 100})
    if [row["id"] for row in complete["issues"]] != [row["id"] for row in reversed(rows)] or complete.get("isLast") is not True or "nextPageToken" in complete:
        raise RuntimeError("complete POST page disagrees with fixture oracle")
    empty = request("/rest/api/3/search/jql", {"jql": predicate + " AND NOT (" + predicate + ")", "maxResults": 1})
    if empty["issues"] or empty.get("isLast") is not True or "nextPageToken" in empty:
        raise RuntimeError("contradictory predicate did not yield an empty terminal page")
    print(json.dumps({"P2": "PASS", "traversals": traversals, "requested_100": {"rows": len(complete["issues"]), "isLast": complete["isLast"], "returned_maxResults": complete.get("maxResults"), "clamp_inferred": False}, "empty_terminal": {"rows": 0, "isLast": empty["isLast"], "has_token": False}, "requests": requests}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(json.dumps({"P2": "FAIL", "error_type": type(error).__name__}))
        raise SystemExit(1) from None
