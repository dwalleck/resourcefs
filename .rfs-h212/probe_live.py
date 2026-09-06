#!/usr/bin/env python3
"""Read-only endpoint-shape probe; emits no credentials or response prose."""

from __future__ import annotations

import base64
import json
import os
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def main() -> None:
    if os.environ.get("RFS_LIVE") != "1":
        print(json.dumps({"skipped": "RFS_LIVE is not enabled"}))
        return
    origin = os.environ["RFS_ATLASSIAN_SITE"].rstrip("/")
    parsed = urllib.parse.urlsplit(origin)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or not parsed.hostname.endswith(".atlassian.net")
        or parsed.username is not None
        or parsed.password is not None
        or parsed.port is not None
        or parsed.path
        or parsed.query
        or parsed.fragment
    ):
        raise RuntimeError("invalid disposable tenant origin")
    pair = os.environ["ATLASSIAN_READER_EMAIL"] + ":" + os.environ["ATLASSIAN_READER_API_TOKEN"]
    authorization = "Basic " + base64.b64encode(pair.encode()).decode("ascii")
    opener = urllib.request.build_opener(NoRedirect())
    cases = (
        ("projects", "/rest/api/3/project/search", {"maxResults": "100", "startAt": "0"}),
        ("project_id_order", "/rest/api/3/project/search", {"maxResults": "1", "orderBy": "id"}),
        ("issues_bounded", "/rest/api/3/search/jql", {"jql": "project IS NOT EMPTY ORDER BY key ASC", "maxResults": "100", "fields": "key,summary,status,project"}),
        ("issues_id_range_counterexample", "/rest/api/3/search/jql", {"jql": "id > 0 ORDER BY id ASC", "maxResults": "100", "fields": "key,summary,status,project"}),
        ("issues_unbounded", "/rest/api/3/search/jql", {"jql": "ORDER BY id ASC", "maxResults": "1", "fields": "summary,status,project"}),
    )
    observed = []
    for name, path, query in cases:
        request = urllib.request.Request(
            origin + path + "?" + urllib.parse.urlencode(query),
            headers={"Authorization": authorization, "Accept": "application/json", "User-Agent": "resourcefs-rfs-h212-evidence/1"},
            method="GET",
        )
        try:
            response = opener.open(request, timeout=30)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            status = response.code
            payload = response.read(4 * 1024 * 1024 + 1)
        if len(payload) > 4 * 1024 * 1024:
            raise RuntimeError("probe response exceeds bound")
        document = json.loads(payload)
        rows = document.get("values", document.get("issues", []))
        facts = {
            "case": name,
            "method": "GET",
            "status": status,
            "response_keys": sorted(document),
            "row_count": len(rows),
            "isLast": document.get("isLast"),
            "maxResults": document.get("maxResults"),
            "startAt": document.get("startAt"),
            "nextPageToken_present": "nextPageToken" in document,
            "nextPage_present": "nextPage" in document,
        }
        if rows:
            facts["first_row_keys"] = sorted(rows[0])
            facts["first_row_field_keys"] = sorted(rows[0].get("fields", {}))
        observed.append(facts)
    print(json.dumps(observed, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
