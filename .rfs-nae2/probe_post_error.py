#!/usr/bin/env python3
"""Observe native POST validation shape without emitting response diagnostics."""

import base64
import json
import os
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
    pair = os.environ["ATLASSIAN_READER_EMAIL"] + ":" + os.environ["ATLASSIAN_READER_API_TOKEN"]
    authorization = "Basic " + base64.b64encode(pair.encode()).decode("ascii")
    request = urllib.request.Request(origin + "/rest/api/3/search/jql", data=json.dumps({"jql": "project =", "maxResults": 1}).encode(), headers={"Authorization": authorization, "Accept": "application/json", "Content-Type": "application/json", "User-Agent": "resourcefs-rfs-nae2-evidence/1"}, method="POST")
    opener = urllib.request.build_opener(NoRedirect())
    try:
        response = opener.open(request, timeout=30)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        status = response.status
        content_type = response.headers.get_content_type()
        body = response.read(65537)
    if len(body) > 65536 or status != 400 or content_type != "application/json":
        raise RuntimeError("native rejection differs from documented status/media/bound")
    document = json.loads(body)
    if not isinstance(document, dict):
        raise RuntimeError("native error is not a structured collection")
    messages = document.get("errorMessages", [])
    fields = document.get("errors", {})
    if not isinstance(messages, list) or not all(isinstance(value, str) for value in messages):
        raise RuntimeError("global diagnostic shape differs from Error Collection")
    if not isinstance(fields, dict) or not all(isinstance(key, str) and isinstance(value, str) for key, value in fields.items()):
        raise RuntimeError("field diagnostic shape differs from Error Collection")
    if "status" in document and type(document["status"]) is not int:
        raise RuntimeError("structured status differs from Error Collection")
    if not messages and not fields:
        raise RuntimeError("native rejection has no validation diagnostics")
    print(json.dumps({"P3": "PASS", "http_status": status, "content_type": content_type, "body_bytes": len(body), "global_diagnostics": len(messages), "field_diagnostics": len(fields), "has_structured_status": "status" in document, "matches_error_collection": True, "raw_prose_emitted": False}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(json.dumps({"P3": "FAIL", "error_type": type(error).__name__}))
        raise SystemExit(1) from None
