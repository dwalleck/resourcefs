#!/usr/bin/env python3
"""Observe native offsets, opaque tokens, aliases, and selected issue fields."""

from __future__ import annotations

import base64
import json
import os
from pathlib import Path
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
    authority = urllib.parse.urlsplit(origin)
    if authority.scheme != "https" or not authority.hostname or not authority.hostname.endswith(".atlassian.net") or authority.username or authority.password or authority.port or authority.path or authority.query or authority.fragment:
        raise RuntimeError("invalid disposable tenant origin")
    pair = os.environ["ATLASSIAN_READER_EMAIL"] + ":" + os.environ["ATLASSIAN_READER_API_TOKEN"]
    authorization = "Basic " + base64.b64encode(pair.encode()).decode("ascii")
    opener = urllib.request.build_opener(NoRedirect())
    state = json.loads(Path(".resourcefs/atlassian-fixture-state.json").read_text())

    def get(path, query):
        request = urllib.request.Request(
            origin + path + "?" + urllib.parse.urlencode(query, doseq=True),
            headers={"Authorization": authorization, "Accept": "application/json", "User-Agent": "resourcefs-rfs-h212-pages/1"},
            method="GET",
        )
        with opener.open(request, timeout=30) as response:
            body = response.read(4 * 1024 * 1024 + 1)
        if len(body) > 4 * 1024 * 1024:
            raise RuntimeError("probe response exceeds bound")
        return json.loads(body)

    project_pages = []
    project_ids = []
    start = "0"
    for _ in range(10):
        document = get("/rest/api/3/project/search", {"maxResults": "1", "startAt": start})
        project_pages.append({"startAt": document["startAt"], "maxResults": document["maxResults"], "rows": len(document["values"]), "isLast": document["isLast"], "has_next": "nextPage" in document})
        project_ids.extend(row["id"] for row in document["values"])
        if document["isLast"]:
            break
        link = urllib.parse.urlsplit(document["nextPage"])
        if link.scheme != authority.scheme or link.netloc != authority.netloc or link.path != "/rest/api/3/project/search":
            raise RuntimeError("project continuation left authority")
        next_start = urllib.parse.parse_qs(link.query)["startAt"][0]
        if int(next_start) <= int(start):
            raise RuntimeError("project offset did not advance")
        start = next_start
    else:
        raise RuntimeError("project probe exceeded ten requests")

    project = state["jira"]["projects"][0]
    by_id = get("/rest/api/3/project/" + project["id"], {})
    by_key = get("/rest/api/3/project/" + urllib.parse.quote(project["key"], safe=""), {})
    alias_agrees = by_id["id"] == by_key["id"] == project["id"]
    expected_issue_ids = {row["id"] for row in state["jira"]["issues"]}
    issue_pages = []
    issue_rows = []
    token = None
    for _ in range(10):
        query = {"jql": "project IS NOT EMPTY ORDER BY key ASC", "maxResults": "1", "fields": "key,summary,status,project"}
        if token is not None:
            query["nextPageToken"] = token
        document = get("/rest/api/3/search/jql", query)
        issue_pages.append({"rows": len(document["issues"]), "isLast": document.get("isLast"), "has_token": "nextPageToken" in document})
        issue_rows.extend(document["issues"])
        if document["isLast"]:
            break
        new_token = document["nextPageToken"]
        if not isinstance(new_token, str) or not new_token or new_token == token:
            raise RuntimeError("issue token did not advance")
        token = new_token
    else:
        raise RuntimeError("issue probe exceeded ten requests")

    direct_agrees = True
    for row in issue_rows:
        if row["id"] not in expected_issue_ids:
            continue
        direct = get("/rest/api/3/issue/" + row["id"], {"fields": "summary,status,project"})
        direct_agrees = direct_agrees and all(row[key] == direct[key] for key in ("id", "key", "self", "fields"))
    default_fields = get("/rest/api/3/search/jql", {"jql": "project IS NOT EMPTY ORDER BY key ASC", "maxResults": "1"})
    scoped = get("/rest/api/3/search/jql", {"jql": "project = " + project["id"] + " ORDER BY key ASC", "maxResults": "100", "fields": "key,summary,status,project"})
    result = {
        "project_pages": project_pages,
        "fixture_projects_present": {row["id"] for row in state["jira"]["projects"]}.issubset(set(project_ids)),
        "project_alias_identity_agrees": alias_agrees,
        "issue_pages": issue_pages,
        "fixture_issues_present": expected_issue_ids.issubset({row["id"] for row in issue_rows}),
        "selected_issue_top_level_keys": sorted(issue_rows[0]) if issue_rows else [],
        "selected_issue_field_keys": sorted(issue_rows[0]["fields"]) if issue_rows else [],
        "selected_rows_agree_with_direct_reads": direct_agrees,
        "default_issue_top_level_keys": sorted(default_fields["issues"][0]) if default_fields["issues"] else [],
        "project_scoped_membership_valid": bool(scoped["issues"]) and all(row["fields"]["project"]["id"] == project["id"] for row in scoped["issues"]),
    }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
