#!/usr/bin/env python3
"""Extract the Jira issue-read and ADF contracts from Atlassian's published schemas."""

from __future__ import annotations

import hashlib
import json
import urllib.request

JIRA_OPENAPI = "https://dac-static.atlassian.com/cloud/jira/platform/swagger-v3.v3.json"
ADF_SCHEMA = "https://go.atlassian.com/adf-json-schema"
BASIC_AUTH = "https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/"
USER_AGENT = "resourcefs-rfs-pm0y-evidence/1"


def fetch_json(url: str) -> tuple[dict[str, object], str, str]:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read()
        final_url = response.geturl()
    return json.loads(body), final_url, hashlib.sha256(body).hexdigest()


def fetch_text(url: str) -> tuple[str, str, str]:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read()
        final_url = response.geturl()
    return body.decode("utf-8"), final_url, hashlib.sha256(body).hexdigest()


def main() -> None:
    jira, jira_url, jira_sha256 = fetch_json(JIRA_OPENAPI)
    adf, adf_url, adf_sha256 = fetch_json(ADF_SCHEMA)
    basic_auth, basic_auth_url, _basic_auth_page_sha256 = fetch_text(BASIC_AUTH)

    issue_get = jira["paths"]["/rest/api/3/issue/{issueIdOrKey}"]["get"]
    issue = jira["components"]["schemas"]["IssueBean"]
    issue_properties = issue["properties"]
    field_schema = jira["components"]["schemas"]["JsonTypeBean"]
    adf_doc = adf["definitions"]["doc_node"]

    path_parameter = next(
        parameter
        for parameter in issue_get["parameters"]
        if parameter["in"] == "path"
    )
    description = issue_get["description"]
    basic_auth_facts = {
        "email_and_token": "email address" in basic_auth and "API token" in basic_auth,
        "colon_join": "useremail:api_token" in basic_auth,
        "base64_encoding": "BASE64 encode the string" in basic_auth,
        "authorization_scheme": "Authorization: Basic" in basic_auth,
    }
    basic_auth_facts_sha256 = hashlib.sha256(
        json.dumps(basic_auth_facts, sort_keys=True).encode("utf-8")
    ).hexdigest()
    result = {
        "sources": {
            "jira": {
                "url": jira_url,
                "sha256": jira_sha256,
                "version": jira["info"]["version"],
            },
            "adf": {
                "url": adf_url,
                "sha256": adf_sha256,
            },
            "basic_auth": {
                "url": basic_auth_url,
                "facts_sha256": basic_auth_facts_sha256,
            },
        },
        "issue_lookup": {
            "method": "GET",
            "path": "/rest/api/3/issue/{issueIdOrKey}",
            "path_parameter": path_parameter["name"],
            "path_parameter_required": path_parameter["required"],
            "accepts_id_or_key": "identified by its ID or key" in description,
            "case_insensitive_fallback": "case-insensitive search" in description,
            "moved_issue_fallback": "check for moved issues" in description,
            "returns_found_key": "key returned in the response is the key of the issue found" in description,
            "redirect_not_used": "redirect is **not** returned" in description,
            "statuses": sorted(issue_get["responses"]),
            "success_schema": issue_get["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            "etag_response_header_documented": "ETag" in issue_get["responses"]["200"].get("headers", {}),
        },
        "issue_wire": {
            "schema_required": sorted(issue.get("required", [])),
            "identity_properties": sorted(
                name for name in ("id", "key", "self") if name in issue_properties
            ),
            "fields_are_unconstrained_json_values": issue_properties["fields"]["additionalProperties"] == {},
            "names_value_type": issue_properties["names"]["additionalProperties"]["type"],
            "schema_value_ref": issue_properties["schema"]["additionalProperties"]["$ref"],
            "field_schema_required": sorted(field_schema["required"]),
            "field_schema_type_property": field_schema["properties"]["type"]["type"],
        },
        "adf_wire": {
            "root_ref": adf["$ref"],
            "root_required": sorted(adf_doc["required"]),
            "root_type": adf_doc["properties"]["type"]["enum"],
            "root_version": adf_doc["properties"]["version"]["enum"],
            "content_type": adf_doc["properties"]["content"]["type"],
            "root_allows_unknown_members": adf_doc["additionalProperties"],
        },
        "basic_auth": basic_auth_facts,
    }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
