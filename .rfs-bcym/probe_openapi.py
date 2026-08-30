#!/usr/bin/env python3
"""Extract Atlassian fixture-lifecycle contracts from published OpenAPI schemas."""

from __future__ import annotations

import hashlib
import json
import urllib.request

JIRA_OPENAPI = "https://dac-static.atlassian.com/cloud/jira/platform/swagger-v3.v3.json"
CONFLUENCE_V2_OPENAPI = "https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json"
CONFLUENCE_V1_OPENAPI = "https://dac-static.atlassian.com/cloud/confluence/swagger.v3.json"
USER_AGENT = "resourcefs-rfs-bcym-evidence/1"


def fetch_json(url: str) -> tuple[dict[str, object], str, str]:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read()
        final_url = response.geturl()
    return json.loads(body), final_url, hashlib.sha256(body).hexdigest()


def resolve(document: dict[str, object], reference: str) -> dict[str, object]:
    value: object = document
    for component in reference.removeprefix("#/").split("/"):
        value = value[component]  # type: ignore[index]
    return value  # type: ignore[return-value]


def operation(document: dict[str, object], path: str, method: str) -> dict[str, object]:
    return document["paths"][path][method]  # type: ignore[index,return-value]


def request_schema(document: dict[str, object], value: dict[str, object]) -> dict[str, object]:
    request_body = value["requestBody"]
    if "$ref" in request_body:  # type: ignore[operator]
        request_body = resolve(document, request_body["$ref"])  # type: ignore[index]
    content = request_body["content"]  # type: ignore[index]
    media = content.get("application/json") or next(iter(content.values()))  # type: ignore[union-attr]
    schema = media["schema"]
    if "$ref" in schema:
        return resolve(document, schema["$ref"])
    return schema


def response_schema_reference(value: dict[str, object], status: str) -> str | None:
    response = value["responses"][status]  # type: ignore[index]
    content = response.get("content", {})
    if not content:
        return None
    media = content.get("application/json") or next(iter(content.values()))
    schema = media["schema"]
    if "$ref" in schema:
        return schema["$ref"]
    all_of = schema.get("allOf", [])
    if all_of and "$ref" in all_of[0]:
        return all_of[0]["$ref"]
    return None


def statuses(value: dict[str, object]) -> list[str]:
    return sorted(value["responses"])  # type: ignore[arg-type]


def parameters(document: dict[str, object], value: dict[str, object]) -> list[str]:
    names = []
    for parameter in value.get("parameters", []):  # type: ignore[union-attr]
        if "$ref" in parameter:
            parameter = resolve(document, parameter["$ref"])
        names.append(parameter["name"])
    return sorted(names)


def schema_shape(schema: dict[str, object]) -> dict[str, list[str]]:
    return {
        "required": sorted(schema.get("required", [])),  # type: ignore[arg-type]
        "properties": sorted(schema.get("properties", {})),  # type: ignore[arg-type]
    }


def endpoint(document: dict[str, object], path: str, method: str) -> dict[str, object]:
    value = operation(document, path, method)
    return {
        "method": method.upper(),
        "path": path,
        "statuses": statuses(value),
        "parameters": parameters(document, value),
    }


def main() -> None:
    jira, jira_url, jira_sha256 = fetch_json(JIRA_OPENAPI)
    confluence_v2, confluence_v2_url, confluence_v2_sha256 = fetch_json(
        CONFLUENCE_V2_OPENAPI
    )
    confluence_v1, confluence_v1_url, confluence_v1_sha256 = fetch_json(
        CONFLUENCE_V1_OPENAPI
    )

    jira_project_create = operation(jira, "/rest/api/3/project", "post")
    jira_issue_create = operation(jira, "/rest/api/3/issue", "post")
    jira_comment_create = operation(
        jira, "/rest/api/3/issue/{issueIdOrKey}/comment", "post"
    )
    jira_project_result = resolve(
        jira, response_schema_reference(jira_project_create, "201")
    )
    jira_issue_result = resolve(jira, response_schema_reference(jira_issue_create, "201"))
    jira_comment_result = resolve(
        jira, response_schema_reference(jira_comment_create, "201")
    )

    confluence_space_create = operation(confluence_v2, "/spaces", "post")
    confluence_space_create_v1 = operation(
        confluence_v1, "/wiki/rest/api/space", "post"
    )
    confluence_private_space_create_v1 = operation(
        confluence_v1, "/wiki/rest/api/space/_private", "post"
    )
    confluence_space_delete = operation(
        confluence_v1, "/wiki/rest/api/space/{spaceKey}", "delete"
    )
    confluence_page_create = operation(confluence_v2, "/pages", "post")
    confluence_comment_create = operation(confluence_v2, "/footer-comments", "post")
    confluence_space_result = resolve(
        confluence_v2, response_schema_reference(confluence_space_create, "201")
    )
    confluence_space_result_v1 = resolve(
        confluence_v1, response_schema_reference(confluence_space_create_v1, "200")
    )
    confluence_space_delete_result = resolve(
        confluence_v1, response_schema_reference(confluence_space_delete, "202")
    )
    confluence_page_result = resolve(
        confluence_v2, response_schema_reference(confluence_page_create, "200")
    )
    confluence_comment_result = resolve(
        confluence_v2, response_schema_reference(confluence_comment_create, "201")
    )

    result = {
        "sources": {
            "jira": {
                "url": jira_url,
                "version": jira["info"]["version"],
                "sha256": jira_sha256,
            },
            "confluence_v2": {
                "url": confluence_v2_url,
                "version": confluence_v2["info"]["version"],
                "sha256": confluence_v2_sha256,
            },
            "confluence_v1": {
                "url": confluence_v1_url,
                "version": confluence_v1["info"]["version"],
                "sha256": confluence_v1_sha256,
            },
        },
        "jira": {
            "project": {
                "create": endpoint(jira, "/rest/api/3/project", "post"),
                "create_request": schema_shape(request_schema(jira, jira_project_create)),
                "create_result": schema_shape(jira_project_result),
                "lookup": endpoint(jira, "/rest/api/3/project/{projectIdOrKey}", "get"),
                "delete": endpoint(jira, "/rest/api/3/project/{projectIdOrKey}", "delete"),
            },
            "issue": {
                "create": endpoint(jira, "/rest/api/3/issue", "post"),
                "create_request": schema_shape(request_schema(jira, jira_issue_create)),
                "create_result": schema_shape(jira_issue_result),
                "lookup": endpoint(jira, "/rest/api/3/issue/{issueIdOrKey}", "get"),
                "delete": endpoint(jira, "/rest/api/3/issue/{issueIdOrKey}", "delete"),
                "search": endpoint(jira, "/rest/api/3/search/jql", "get"),
            },
            "comment": {
                "create": endpoint(
                    jira, "/rest/api/3/issue/{issueIdOrKey}/comment", "post"
                ),
                "create_request": schema_shape(request_schema(jira, jira_comment_create)),
                "create_result": schema_shape(jira_comment_result),
                "list": endpoint(
                    jira, "/rest/api/3/issue/{issueIdOrKey}/comment", "get"
                ),
                "delete": endpoint(
                    jira, "/rest/api/3/issue/{issueIdOrKey}/comment/{id}", "delete"
                ),
            },
        },
        "confluence": {
            "space": {
                "create": endpoint(confluence_v2, "/spaces", "post"),
                "create_request": schema_shape(
                    request_schema(confluence_v2, confluence_space_create)
                ),
                "create_result": schema_shape(confluence_space_result),
                "create_v1": endpoint(
                    confluence_v1, "/wiki/rest/api/space", "post"
                ),
                "create_private_v1": endpoint(
                    confluence_v1, "/wiki/rest/api/space/_private", "post"
                ),
                "create_v1_request": schema_shape(
                    request_schema(confluence_v1, confluence_space_create_v1)
                ),
                "create_v1_result": schema_shape(confluence_space_result_v1),
                "lookup": endpoint(confluence_v2, "/spaces/{id}", "get"),
                "list": endpoint(confluence_v2, "/spaces", "get"),
                "delete": endpoint(
                    confluence_v1, "/wiki/rest/api/space/{spaceKey}", "delete"
                ),
                "delete_result": schema_shape(confluence_space_delete_result),
            },
            "page": {
                "create": endpoint(confluence_v2, "/pages", "post"),
                "create_request": schema_shape(
                    request_schema(confluence_v2, confluence_page_create)
                ),
                "create_result": schema_shape(confluence_page_result),
                "lookup": endpoint(confluence_v2, "/pages/{id}", "get"),
                "list": endpoint(confluence_v2, "/pages", "get"),
                "delete": endpoint(confluence_v2, "/pages/{id}", "delete"),
            },
            "footer_comment": {
                "create": endpoint(confluence_v2, "/footer-comments", "post"),
                "create_request": schema_shape(
                    request_schema(confluence_v2, confluence_comment_create)
                ),
                "create_result": schema_shape(confluence_comment_result),
                "list": endpoint(
                    confluence_v2, "/pages/{id}/footer-comments", "get"
                ),
                "delete": endpoint(
                    confluence_v2, "/footer-comments/{comment-id}", "delete"
                ),
            },
            "storage_body": {
                "page_representations": confluence_v2["components"]["schemas"]
                ["PageBodyWrite"]["properties"]["representation"]["enum"],
                "comment_representations": confluence_v2["components"]["schemas"]
                ["CommentBodyWrite"]["properties"]["representation"]["enum"],
            },
            "cql_search": endpoint(
                confluence_v1, "/wiki/rest/api/content/search", "get"
            ),
        },
    }
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
