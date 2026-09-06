#!/usr/bin/env python3
"""Deterministic fake for the operator's curl --config - boundary.

The fixture deliberately knows REST wire shapes, not manifest reconciliation.  Every
curl process reloads the JSON store, records a credential-free request row, and
atomically persists its response-side effects.
"""
import hashlib
import json
import os
import re
import sys
import tempfile
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlsplit


STORE_PATH = Path(os.environ.get("FAKE_CURL_STORE", "fake-store.json"))
LOG_PATH = Path(os.environ.get("FAKE_CURL_LOG", "fake-curl.log"))
SCENARIO = os.environ.get("FAKE_CURL_SCENARIO", "normal")
PROVISIONER_USER = "provisioner-canary@example.test:provisioner-token-canary"
READER_USER = "reader-canary@example.test:reader-token-canary"



def initial_store():
    return {
        "version": 1,
        "projects": [],
        "issues": [],
        "jira_comments": [],
        "spaces": [],
        "pages": [],
        "confluence_comments": [],
        "properties": {},
        "tasks": {},
        "faults": {},
    }


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=f".{path.name}.", dir=str(path.parent))
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(value, stream, sort_keys=True, separators=(",", ":"))
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        try:
            os.unlink(name)
        except FileNotFoundError:
            pass


def load_store():
    try:
        with STORE_PATH.open(encoding="utf-8") as stream:
            value = json.load(stream)
    except FileNotFoundError:
        value = initial_store()
    if not isinstance(value, dict):
        value = initial_store()
    base = initial_store()
    base.update(value)
    for key in ("projects", "issues", "jira_comments", "spaces", "pages", "confluence_comments"):
        if not isinstance(base.get(key), list):
            base[key] = []
    if not isinstance(base.get("tasks"), dict):
        base["tasks"] = {}
    if not isinstance(base.get("properties"), dict):
        base["properties"] = {}
    if not isinstance(base.get("faults"), dict):
        base["faults"] = {}
    return base


def append_log(row):
    LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
    with LOG_PATH.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")


def cfg_value(raw):
    raw = raw.strip()
    if raw.startswith('"'):
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            return raw[1:-1]
    return raw


def parse_config():
    config = {}
    for line in sys.stdin:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if "=" in line:
            key, value = line.split("=", 1)
            config[key.strip()] = cfg_value(value)
        else:
            config[line] = True
    return config


def text_of(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return " ".join(text_of(item) for item in value)
    if isinstance(value, dict):
        return " ".join(text_of(item) for item in value.values())
    return ""

def slug(prefix, value):
    tail = value.rsplit("/", 1)[-1].strip().lower()
    tail = re.sub(r"[^a-z0-9]+", "-", tail).strip("-") or "object"
    return f"{prefix}-{tail[:48]}"


def numeric_id(prefix, value):
    digest = hashlib.sha256(f"{prefix}:{value}".encode("utf-8")).digest()
    return str(int.from_bytes(digest[:7], "big") % 9_000_000_000 + 1_000_000_000)


def body_json(config):
    body = config.get("data-binary", "")
    if not body:
        return {}
    try:
        value = json.loads(body)
    except json.JSONDecodeError:
        return {}
    return value if isinstance(value, dict) else {}


def actor_for(config):
    # Authenticate the complete pair without persisting its value.
    user = str(config.get("user", ""))
    if user == PROVISIONER_USER:
        return "provisioner"
    if user == READER_USER:
        return "reader"
    return "invalid"


def ensure_seed(store):
    """Install explicit collision/fault scenarios without reading reconciliation code."""
    if SCENARIO in ("foreign_collision", "foreign_space_collision"):
        if SCENARIO == "foreign_collision" and not any(p.get("key") == "RFSFIX" for p in store["projects"]):
            store["projects"].append({"id": "9000000001", "key": "RFSFIX", "name": "Foreign", "description": "tenant-owned foreign object"})
        if SCENARIO == "foreign_space_collision" and not any(s.get("key") == "RFSFIXTURE" for s in store["spaces"]):
            store["spaces"].append({"id": "9000000002", "key": "RFSFIXTURE", "name": "Foreign", "description": "tenant-owned foreign object", "private": False, "homepageId": int(numeric_id("confluence-home", "RFSFIXTURE"))})
    if SCENARIO == "duplicate_marker" and not any(p.get("id") == "9000000003" for p in store["projects"]):
        store["projects"].append({"id": "9000000003", "key": "OTHERKEY", "name": "Foreign", "description": "ResourceFS disposable fixture / rfs-bcym / jira primary"})
    if (
        SCENARIO in ("foreign_page_property", "page_missing_property")
        and not any(s.get("key") == "RFSFIXTURE" for s in store["spaces"])
    ):
        space_id = numeric_id("confluence-space", "RFSFIXTURE")
        homepage_id = int(numeric_id("confluence-home", "RFSFIXTURE"))
        store["spaces"].append({
            "id": space_id,
            "key": "RFSFIXTURE",
            "name": "ResourceFS Fixture Public",
            "description": "ResourceFS disposable fixture / rfs-bcym / confluence public",
            "private": False,
            "homepageId": homepage_id,
        })
        store["pages"].append({
            "id": "9000000005",
            "space": space_id,
            "spaceId": space_id,
            "title": "ResourceFS Fixture Root / Unicode \u03c0",
            "body": {"representation": "storage", "value": "<p>drifted content</p>"},
            "parentId": str(homepage_id),
        })
        if SCENARIO == "foreign_page_property":
            store["properties"].setdefault("9000000005", {})["rfs-owner"] = "someone-else"
    if (
        SCENARIO == "embedded_marker_foreign_issue"
        and not any(item.get("id") == "9000000004" for item in store["issues"])
    ):
        store["issues"].append({
            "id": "9000000004",
            "key": "RFSFIX-900",
            "project": "RFSFIX",
            "fields": {
                "project": {"key": "RFSFIX"},
                "summary": "Foreign issue",
                "description": {
                    "type": "doc",
                    "version": 1,
                    "content": [{
                        "type": "paragraph",
                        "attrs": {
                            "foreign": "rfs-owner:rfs-bcym:jira-issue:primary-root",
                        },
                        "content": [{
                            "type": "text",
                            "text": "ordinary prose",
                        }],
                    }],
                },
            },
        })
        store["pages"].append({
            "id": "9000000005",
            "space": numeric_id("confluence-space", "RFSFIXTURE"),
            "spaceId": numeric_id("confluence-space", "RFSFIXTURE"),
            "title": "Foreign page",
            "body": {
                "representation": "storage",
                "value": "<p>ordinary data <!--rfs-owner:rfs-bcym:confluence-page:public-root--></p>",
            },
        })
    return store


def list_page(rows, query, item_name, continuation_name, start_name=None, next_prefix="cursor-"):
    # All normal pages intentionally contain one item.  This forces every caller
    # to consume the continuation instead of accidentally passing with one row.
    if start_name:
        start = int(query.get(start_name, ["0"])[0])
        page = rows[start:start + 1]
        last = start + 1 >= len(rows)
        result = {item_name: page, "isLast": last, "maxResults": 1}
        if not last:
            result[continuation_name] = start + 1
        return result
    cursor = unquote(query.get("cursor", [""])[0])
    index = int(cursor.rsplit("-", 1)[-1]) if cursor else 0
    page = rows[index:index + 1]
    result = {item_name: page}
    if index + 1 < len(rows):
        result["_links"] = {"next": f"{next_prefix}{index + 1}"}
    else:
        result["_links"] = {}
    return result


def respond(output_path, status, payload):
    if output_path:
        output = Path(output_path)
        output.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(payload, (dict, list)):
            output.write_text(json.dumps(payload, separators=(",", ":")), encoding="utf-8")
        else:
            output.write_text(str(payload), encoding="utf-8")
    print(status, end="")


def find_one(rows, object_id):
    return next((row for row in rows if str(row.get("id")) == object_id), None)


FAKE_MACRO_ID = "6f9619ff-8b86-d011-b42d-00cf4fc964ff"


def confluence_ingest(value):
    """Apply the production storage mutations observed on a live tenant:
    leading HTML comments are stripped, raw em dashes and pi become named
    entities, and macros gain provider-owned schema-version/macro-id attrs."""
    value = re.sub(r"\A<!--.*?-->", "", value)
    value = value.replace("\u2014", "&mdash;").replace("\u03c0", "&pi;")
    value = re.sub(
        r'<ac:structured-macro ac:name="([^"]*)">',
        r'<ac:structured-macro ac:name="\1" ac:schema-version="1" ac:macro-id="%s">' % FAKE_MACRO_ID,
        value,
    )
    return value


def confluence_content_exists(store, content_id):
    return (
        find_one(store["pages"], content_id) is not None
        or find_one(store["confluence_comments"], content_id) is not None
    )


def purge_content_properties(store, content_ids):
    for content_id in content_ids:
        store["properties"].pop(str(content_id), None)


def property_row(store, content_id, key, value):
    return {
        "id": numeric_id("content-property", f"{content_id}:{key}"),
        "key": key,
        "value": value,
        "version": {"number": 1, "minorEdit": False},
    }


def handle(config, store, actor, method, path, query, body):
    if actor == "invalid":
        return 401, {}, "authentication"
    # Scenario failures are injected at the transport's response boundary.
    if SCENARIO in ("raw400", "raw_prose") and method == "POST" and path.endswith("/project") and not store["faults"].get("raw400"):
        store["faults"]["raw400"] = True
        return 400, "RAW_PROSE_CANARY_DO_NOT_PRINT_" * 100, "project_create"
    if SCENARIO == "partial_once" and method == "POST" and path == "/rest/api/3/issue" and not store["faults"].get("partial"):
        store["faults"]["partial"] = True
        return 503, {"errorMessages": ["partial failure prose"]}, "issue_create"
    if SCENARIO == "transport_failure":
        return None, "", "transport"

    # Jira project search/create/get/delete.
    if method == "GET" and path == "/rest/api/3/project/search":
        rows = sorted(store["projects"], key=lambda row: row.get("key", ""))
        # Live search omits `description` unless it is explicitly expanded.
        if "description" not in query.get("expand", []):
            rows = [
                {key: value for key, value in row.items() if key != "description"}
                for row in rows
            ]
        if SCENARIO == "malformed_paging" and not store["faults"].get("malformed_project"):
            store["faults"]["malformed_project"] = True
            return 200, {"values": rows[:1], "isLast": False, "nextPage": 0, "maxResults": 1}, "project_search"
        return 200, list_page(rows, query, "values", "nextPage", "startAt"), "project_search"
    if SCENARIO == "drift" and method == "GET" and path.startswith("/rest/api/3/project/") and not store["faults"].get("drift"):
        if store["projects"]:
            store["projects"][0]["description"] = "drifted-owner-field"
            store["faults"]["drift"] = True
    if method == "GET" and path == "/rest/api/3/myself":
        return 200, {"accountId": "fixture-provisioner-account"}, "account_get"
    if method == "GET" and path.startswith("/rest/api/3/issue/createmeta/"):
        return 200, {"issueTypes": [
            {"id": "10001", "name": "Task", "subtask": False},
            {"id": "10002", "name": "Sub-task", "subtask": True},
        ], "startAt": 0, "maxResults": 100, "total": 2}, "issue_types"
    if method == "POST" and path == "/rest/api/3/project":
        key = str(body.get("key", ""))
        if any(row.get("key") == key for row in store["projects"]):
            return 400, {"errorMessages": ["project key already exists"]}, "project_create"
        marker = str(body.get("description", ""))
        row = {"id": numeric_id("jira-project", key), "key": key, "name": body.get("name", ""), "description": marker}
        store["projects"].append(row)
        return 201, {"id": row["id"], "key": key, "description": marker}, "project_create"
    match = re.fullmatch(r"/rest/api/3/project/([^/]+)", path)
    if match and method == "GET":
        row = next((item for item in store["projects"] if item.get("key") == match.group(1)), None)
        return (200, row, "project_get") if row else (404, {}, "project_get")
    if match and method == "DELETE":
        row = find_one(store["projects"], match.group(1))
        if not row:
            return 404, {}, "jira_project_delete"
        store["projects"].remove(row)
        project_key = row.get("key")
        issue_ids = {item["id"] for item in store["issues"] if item.get("project") == project_key}
        store["issues"][:] = [item for item in store["issues"] if item.get("project") != project_key]
        store["jira_comments"][:] = [item for item in store["jira_comments"] if item.get("issue") not in issue_ids]
        return 204, "", "jira_project_delete"

    # Jira issue search/create/get/delete.
    if method == "POST" and path == "/rest/api/3/search/jql":
        jql = str(body.get("jql", ""))
        project_match = re.search(r'project\s*=\s*"([^"]+)"', jql)
        rows = [item for item in store["issues"] if not project_match or item.get("project") == project_match.group(1)]
        rows.sort(key=lambda item: item.get("key", ""))
        token = str(body.get("nextPageToken", ""))
        index = int(token.rsplit("-", 1)[-1]) if token.startswith("issue-token-") else 0
        page = rows[index:index + 1]
        last = index + 1 >= len(rows)
        result = {"issues": page, "isLast": last}
        if not last:
            result["nextPageToken"] = f"issue-token-{index + 1}"
        if SCENARIO == "malformed_paging":
            count = int(store["faults"].get("malformed_issue", 0))
            if count < 2:
                store["faults"]["malformed_issue"] = count + 1
                result["isLast"] = False
                result["nextPageToken"] = "issue-token-0"
        return 200, result, "issue_search"
    if method == "POST" and path == "/rest/api/3/issue":
        fields = body.get("fields", {})
        project = fields.get("project", {}).get("key", "")
        description = fields.get("description", {})
        marker = text_of(description)
        row = {
            "id": numeric_id("jira-issue", marker),
            "key": f"{project}-{len([item for item in store['issues'] if item.get('project') == project]) + 1}",
            "project": project,
            "fields": {
                "project": {"key": project},
                "summary": fields.get("summary", ""),
                "description": description,
            },
        }
        parent = fields.get("parent", {}).get("id")
        if parent:
            row["fields"]["parent"] = {"id": parent, "key": next((x.get("key") for x in store["issues"] if x.get("id") == parent), "")}
        store["issues"].append(row)
        if SCENARIO == "mismatched_create_identity":
            return 201, {"id": "9999999999", "key": row["key"]}, "issue_create"
        return 201, {"id": row["id"], "key": row["key"]}, "issue_create"
    match = re.fullmatch(r"/rest/api/3/issue/([^/?]+)", path)
    if match and method == "GET":
        row = find_one(store["issues"], match.group(1))
        return (200, row, "issue_get") if row else (404, {}, "issue_get")
    if match and method == "DELETE":
        row = find_one(store["issues"], match.group(1))
        if SCENARIO == "jira_delete_denied":
            if not row:
                return 404, {}, "jira_issue_delete"
            return 403, {"errorMessages": ["delete issues permission denied"]}, "jira_issue_delete"
        if not row:
            return 404, {}, "jira_issue_delete"
        store["issues"].remove(row)
        store["jira_comments"][:] = [item for item in store["jira_comments"] if item.get("issue") != row.get("id")]
        return 204, "", "jira_issue_delete"
    match = re.fullmatch(r"/rest/api/3/issue/([^/]+)/comment", path)
    if match and method == "GET":
        rows = [item for item in store["jira_comments"] if item.get("issue") == match.group(1)]
        result = list_page(sorted(rows, key=lambda item: item.get("id", "")), query, "comments", "nextPage", "startAt")
        result["startAt"] = int(query.get("startAt", ["0"])[0])
        result["total"] = len(rows)
        return 200, result, "comment_list"
    if match and method == "POST":
        row = {"id": numeric_id("jira-comment", text_of(body.get("body", {}))), "issue": match.group(1), "body": body.get("body", {})}
        store["jira_comments"].append(row)
        return 201, {"id": row["id"], "body": row["body"]}, "comment_create"
    match = re.fullmatch(r"/rest/api/3/issue/([^/]+)/comment/([^/]+)", path)
    if match and method == "GET":
        row = next((item for item in store["jira_comments"] if item.get("issue") == match.group(1) and item.get("id") == match.group(2)), None)
        return (200, {"id": row["id"], "body": row["body"]}, "comment_get") if row else (404, {}, "comment_get")
    if match and method == "DELETE":
        row = next((item for item in store["jira_comments"] if item.get("issue") == match.group(1) and item.get("id") == match.group(2)), None)
        if not row:
            return 404, {}, "jira_comment_delete"
        store["jira_comments"].remove(row)
        return 204, "", "jira_comment_delete"

    # Confluence spaces, including reader-private filtering.
    if method == "GET" and path == "/wiki/api/v2/spaces":
        rows = sorted(store["spaces"], key=lambda item: item.get("key", ""))
        if actor == "reader":
            rows = [item for item in rows if not item.get("private") or SCENARIO == "reader_private_visible"]
        prefix = "/wiki/api/v2/spaces?limit=1&cursor="
        if "description-format" in query:
            prefix = "/wiki/api/v2/spaces?limit=1&description-format=plain&cursor="
        result = list_page(rows, query, "results", "cursor", next_prefix=prefix)
        return 200, result, "reader_space_list" if actor == "reader" else "space_list"
    if method == "POST" and path in ("/wiki/rest/api/space", "/wiki/rest/api/space/_private"):
        key = str(body.get("key", ""))
        if any(row.get("key") == key for row in store["spaces"]):
            return 400, {"message": "space key already exists"}, "space_create"
        description = body.get("description", {})
        if isinstance(description, dict) and isinstance(description.get("plain"), dict):
            description = description["plain"]
        row = {
            "id": numeric_id("confluence-space", key),
            "key": key,
            "name": body.get("name", ""),
            "description": description.get("value", "") if isinstance(description, dict) else str(description),
            "private": path.endswith("/_private"),
            "homepageId": int(numeric_id("confluence-home", key)),
        }
        store["spaces"].append(row)
        return 201, {"id": row["id"], "key": key}, "space_create"
    match = re.fullmatch(r"/wiki/api/v2/spaces/([^/]+)", path)
    if match and method == "GET":
        row = find_one(store["spaces"], match.group(1))
        if SCENARIO == "reader_public_hidden" and actor == "reader" and row and not row.get("private"):
            return 404, {}, "reader_space_get"
        if not row or (actor == "reader" and row.get("private")):
            return 404, {}, "reader_space_get" if actor == "reader" else "space_get"
        return 200, row, "reader_space_get" if actor == "reader" else "space_get"
    match = re.fullmatch(r"/wiki/rest/api/space/([^/]+)", path)
    if match and method == "DELETE":
        row = next((item for item in store["spaces"] if item.get("key") == match.group(1)), None)
        if not row:
            return 404, {}, "space_delete"
        task_id = slug("space-delete", row["key"])
        store["tasks"][task_id] = {"space": row["id"], "polls": 0}
        return 202, {"id": task_id}, "space_delete"
    match = re.fullmatch(r"/wiki/rest/api/longtask/([^/]+)", path)
    if match and method == "GET":
        task = store["tasks"].get(match.group(1))
        if not task:
            return 404, {}, "space_delete_poll"
        task["polls"] = int(task.get("polls", 0)) + 1
        if task["polls"] == 1:
            return 200, {"id": match.group(1), "status": "RUNNING"}, "space_delete_poll"
        space = find_one(store["spaces"], task["space"])
        if space:
            store["spaces"].remove(space)
            page_ids = {page["id"] for page in store["pages"] if page.get("space") == space["id"]}
            store["pages"][:] = [page for page in store["pages"] if page.get("space") != space["id"]]
            removed_ids = set(page_ids)
            removed_ids.update(
                comment["id"] for comment in store["confluence_comments"] if comment.get("page") in page_ids
            )
            store["confluence_comments"][:] = [comment for comment in store["confluence_comments"] if comment.get("page") not in page_ids]
            purge_content_properties(store, removed_ids)
        return 200, {"id": match.group(1), "status": "FINISH_SUCCESS", "finished": True}, "space_delete_poll"

    # Confluence pages and footer comments.
    match = re.fullmatch(r"/wiki/api/v2/spaces/([^/]+)/pages", path)
    if match and method == "GET":
        space = find_one(store["spaces"], match.group(1))
        if not space or (actor == "reader" and space.get("private")):
            return 404, {}, "page_list"
        rows = [item for item in store["pages"] if item.get("space") == match.group(1)]
        prefix = f"/wiki/api/v2/spaces/{match.group(1)}/pages?limit=1&body-format=storage&cursor="
        return 200, list_page(sorted(rows, key=lambda item: item.get("title", "")), query, "results", "cursor", next_prefix=prefix), "page_list"
    if method == "POST" and path == "/wiki/api/v2/pages":
        sid = body.get("spaceId", "")
        space = next((item for item in store["spaces"] if str(item.get("id")) == str(sid)), None)
        if space is None:
            return 400, {"message": "space not found"}, "page_create"
        page_body = body.get("body", {})
        if isinstance(page_body, dict) and isinstance(page_body.get("value"), str):
            page_body = dict(page_body, value=confluence_ingest(page_body["value"]))
        row = {
            "id": numeric_id("confluence-page", str(body.get("title", ""))),
            "space": sid,
            "spaceId": sid,
            "title": body.get("title", ""),
            "body": page_body,
        }
        if body.get("parentId"):
            row["parentId"] = body["parentId"]
        else:
            row["parentId"] = space.get("homepageId")
        store["pages"].append(row)
        return 201, {"id": row["id"], "title": row["title"]}, "page_create"
    match = re.fullmatch(r"/wiki/api/v2/pages/([^/]+)", path)
    if match and method == "GET":
        row = find_one(store["pages"], match.group(1))
        space = find_one(store["spaces"], row.get("space", "")) if row else None
        if not row or (actor == "reader" and space and space.get("private")):
            return 404, {}, "reader_page_get" if actor == "reader" else "page_get"
        return 200, row, "reader_page_get" if actor == "reader" else "page_get"
    if match and method == "DELETE":
        row = find_one(store["pages"], match.group(1))
        if not row:
            return 404, {}, "confluence_page_delete"
        store["pages"].remove(row)
        removed_comments = [item for item in store["confluence_comments"] if item.get("page") == row.get("id")]
        store["confluence_comments"][:] = [
            item for item in store["confluence_comments"] if item.get("page") != row.get("id")
        ]
        purge_content_properties(store, [row["id"]] + [item["id"] for item in removed_comments])
        return 204, "", "confluence_page_delete"
    match = re.fullmatch(r"/wiki/api/v2/pages/([^/]+)/footer-comments", path)
    if match and method == "GET":
        rows = [item for item in store["confluence_comments"] if item.get("page") == match.group(1)]
        prefix = f"/wiki/api/v2/pages/{match.group(1)}/footer-comments?limit=1&body-format=storage&cursor="
        return 200, list_page(sorted(rows, key=lambda item: item.get("id", "")), query, "results", "cursor", next_prefix=prefix), "comment_list"
    if method == "POST" and path == "/wiki/api/v2/footer-comments":
        pid = body.get("pageId", "")
        if find_one(store["pages"], str(pid)) is None:
            return 400, {"message": "page not found"}, "comment_create"
        comment_body = body.get("body", {})
        if isinstance(comment_body, dict) and isinstance(comment_body.get("value"), str):
            comment_body = dict(comment_body, value=confluence_ingest(comment_body["value"]))
        row = {"id": numeric_id("confluence-comment", text_of(body.get("body", {}))), "page": pid, "pageId": pid, "body": comment_body}
        if body.get("parentCommentId"):
            row["parent"] = body["parentCommentId"]
            row["parentCommentId"] = body["parentCommentId"]
        store["confluence_comments"].append(row)
        return 201, {"id": row["id"], "body": row["body"]}, "comment_create"
    match = re.fullmatch(r"/wiki/api/v2/footer-comments/([^/]+)", path)
    if match and method == "GET":
        row = find_one(store["confluence_comments"], match.group(1))
        page = find_one(store["pages"], row.get("page", "")) if row else None
        space = find_one(store["spaces"], page.get("space", "")) if page else None
        if not row or (actor == "reader" and space and space.get("private")):
            return 404, {}, "reader_comment_get" if actor == "reader" else "comment_get"
        return 200, row, "reader_comment_get" if actor == "reader" else "comment_get"
    if match and method == "DELETE":
        row = find_one(store["confluence_comments"], match.group(1))
        if not row:
            return 404, {}, "confluence_comment_delete"
        store["confluence_comments"].remove(row)
        purge_content_properties(store, [row["id"]])
        return 204, "", "confluence_comment_delete"

    # Confluence v1 content properties (pages and footer comments).
    match = re.fullmatch(r"/wiki/rest/api/content/([^/]+)/property", path)
    if match and method == "POST":
        key = body.get("key")
        value = body.get("value")
        if not isinstance(key, str) or not key or not isinstance(value, str) or not value:
            return 400, {"message": "invalid content property"}, "owner_property_create"
        if not confluence_content_exists(store, match.group(1)):
            return 404, {"message": "content not found"}, "owner_property_create"
        store["properties"].setdefault(str(match.group(1)), {})[key] = value
        return 200, property_row(store, match.group(1), key, value), "owner_property_create"
    match = re.fullmatch(r"/wiki/rest/api/content/([^/]+)/property/([^/]+)", path)
    if match and method == "GET":
        value = store["properties"].get(str(match.group(1)), {}).get(match.group(2))
        if value is None:
            return 404, {"message": "content property not found"}, "owner_property_get"
        return 200, {"key": match.group(2), "value": value}, "owner_property_get"

    return 404, {"message": "unimplemented fake endpoint"}, "unknown"


def main():
    config = parse_config()
    url = str(config.get("url", ""))
    parsed = urlsplit(url)
    path = parsed.path
    query = parse_qs(parsed.query, keep_blank_values=True)
    body = body_json(config)
    method = str(config.get("request", "GET")).upper()
    actor = actor_for(config)
    store = ensure_seed(load_store())
    response_path = str(config.get("output", ""))
    user_value = str(config.get("user", ""))
    argv_safe = sys.argv[1:] == ["--config", "-"]
    credential_env_absent = all(name not in os.environ for name in (
        "ATLASSIAN_PROVISIONER_EMAIL",
        "ATLASSIAN_PROVISIONER_API_TOKEN",
        "ATLASSIAN_READER_EMAIL",
        "ATLASSIAN_READER_API_TOKEN",
    ))
    status, payload, operation = handle(config, store, actor, method, path, query, body)
    row = {
        "actor": actor,
        "config_user_present": bool(user_value),
        "argv_safe": argv_safe,
        "credential_env_absent": credential_env_absent,
        "credential_pair_valid": actor != "invalid",
        "method": method,
        "path": path,
        "query": query,
        "body": body,
        "operation": operation,
        "status": status if status is not None else "transport",
        "scenario": SCENARIO,
    }
    append_log(row)
    atomic_json(STORE_PATH, store)
    if status is None:
        return 7
    respond(response_path, status, payload)
    if (
        SCENARIO == "state_commit_denied"
        and operation == "comment_get"
        and path.startswith("/wiki/api/v2/footer-comments/")
        and len(store["confluence_comments"]) >= 2
    ):
        STORE_PATH.parent.chmod(0o500)
    return 0


if __name__ == "__main__":
    sys.exit(main())
