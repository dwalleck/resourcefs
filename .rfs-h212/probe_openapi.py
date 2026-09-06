#!/usr/bin/env python3
"""Extract only the public Jira browsing facts named by P1–P3."""

from __future__ import annotations

import hashlib
import json
import urllib.request

JIRA_OPENAPI = "https://dac-static.atlassian.com/cloud/jira/platform/swagger-v3.v3.json"


def main() -> None:
    request = urllib.request.Request(
        JIRA_OPENAPI, headers={"User-Agent": "resourcefs-rfs-h212-evidence/1"}
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read(16 * 1024 * 1024 + 1)
        final_url = response.geturl()
    if len(body) > 16 * 1024 * 1024:
        raise RuntimeError("public schema exceeds probe bound")
    document = json.loads(body)
    schemas = document["components"]["schemas"]
    output = {
        "source": {
            "url": final_url,
            "sha256": hashlib.sha256(body).hexdigest(),
            "version": document["info"]["version"],
        },
        "operations": [],
    }
    for path, method in (
        ("/rest/api/3/project/{projectIdOrKey}", "get"),
        ("/rest/api/3/project/search", "get"),
        ("/rest/api/3/search/jql", "get"),
        ("/rest/api/3/search/jql", "post"),
    ):
        operation = document["paths"][path][method]
        response_ref = operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"]
        response_schema = schemas[response_ref.rsplit("/", 1)[1]]
        facts = {
            "method": method.upper(),
            "path": path,
            "description": operation["description"],
            "parameters": [
                {key: parameter[key] for key in ("name", "in", "required", "description", "schema") if key in parameter}
                for parameter in operation.get("parameters", [])
            ],
            "statuses": sorted(operation["responses"]),
            "response_schema": response_ref,
            "response_required": response_schema.get("required", []),
            "response_properties": response_schema["properties"],
        }
        if "requestBody" in operation:
            request_ref = operation["requestBody"]["content"]["application/json"]["schema"]["$ref"]
            facts["request_schema"] = request_ref
            facts["request_properties"] = schemas[request_ref.rsplit("/", 1)[1]]["properties"]
        output["operations"].append(facts)
    output["project_issue_list_paths"] = [
        path for path in document["paths"]
        if path.startswith("/rest/api/3/project/") and path.endswith("/issues")
    ]
    print(json.dumps(output, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
