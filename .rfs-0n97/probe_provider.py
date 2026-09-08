#!/usr/bin/env python3
"""Read-only public P1 observation; no credentials or provider body are printed."""
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request

URL = "https://api.github.com/repos/rust-lang/rust/pulls/159232"
VERSION = "2022-11-28"
CAP = 1024 * 1024
TIMEOUT = 20
OUT = Path(__file__).resolve().parent


def kind(value):
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, (int, float)):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    return "object"


def state(obj, key, include_value=False):
    result = {"present": key in obj, "type": kind(obj[key]) if key in obj else "missing"}
    if include_value and key in obj:
        result["value"] = obj[key]
    return result


def project(obj):
    result = {key: state(obj, key, True) for key in ("number", "id", "node_id")}
    result["urls"] = {key: state(obj, key, True) for key in ("url", "html_url", "diff_url", "patch_url", "issue_url", "commits_url", "comments_url", "review_comments_url", "review_comment_url", "statuses_url")}
    result["body"] = state(obj, "body")
    result["author"] = state(obj, "user")
    if isinstance(obj.get("user"), dict):
        result["author"]["identity"] = {key: state(obj["user"], key, True) for key in ("id", "node_id", "login", "html_url")}
    result["times"] = {key: state(obj, key, True) for key in ("created_at", "updated_at", "closed_at", "merged_at")}
    for side in ("head", "base"):
        ref = obj[side]
        result[side] = {key: state(ref, key, True) for key in ("sha", "ref", "label")}
        result[side]["repository"] = state(ref, "repo")
        if isinstance(ref.get("repo"), dict):
            result[side]["repository"]["identity"] = {key: state(ref["repo"], key, True) for key in ("id", "node_id", "full_name", "url", "html_url")}
    return result


# Independently expressed jq projection, not a JSON projection shared with Python.
JQ = r'''
def field($key; $value):
  {present: has($key), type: (if has($key) then .[$key] | type else "missing" end)}
  + (if $value and has($key) then {value: .[$key]} else {} end);
def fields($keys): . as $obj | reduce $keys[] as $key ({}; .[$key] = ($obj | field($key; true)));
def side:
  fields(["sha", "ref", "label"]) + {repository:
    (field("repo"; false) + (if (.repo | type) == "object" then
      {identity: (.repo | fields(["id", "node_id", "full_name", "url", "html_url"]))} else {} end))};
fields(["number", "id", "node_id"]) + {
  urls: fields(["url", "html_url", "diff_url", "patch_url", "issue_url", "commits_url", "comments_url", "review_comments_url", "review_comment_url", "statuses_url"]),
  body: field("body"; false),
  author: (field("user"; false) + (if (.user | type) == "object" then
    {identity: (.user | fields(["id", "node_id", "login", "html_url"]))} else {} end)),
  times: fields(["created_at", "updated_at", "closed_at", "merged_at"]),
  head: (.head | side), base: (.base | side)
}
'''


def metadata(headers, status):
    return {"status": status, "etag": headers.get("etag"), "last_modified": headers.get("last-modified"), "api_version_selected": headers.get("x-github-api-version-selected"), "date": headers.get("date")}


def save(name, client, observation, meta):
    record = {"client": client, "request": {"method": "GET", "url": URL, "api_version": VERSION, "authentication": "no token supplied; isolated nonexistent GH_CONFIG_DIR for gh"}, "observed_at_unix": time.time(), "observation": observation, "transport": meta}
    (OUT / name).write_text(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n")


def main():
    request = urllib.request.Request(URL, headers={"Accept": "application/vnd.github+json", "X-GitHub-Api-Version": VERSION, "User-Agent": "resourcefs-public-premise-probe"}, method="GET")
    # Reject redirects rather than follow a provider-controlled destination.
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            return None
    with urllib.request.build_opener(NoRedirect).open(request, timeout=TIMEOUT) as response:
        payload = response.read(CAP + 1)
        if len(payload) > CAP:
            raise RuntimeError("Python public response exceeds 1 MiB cap")
        python_observation = project(json.loads(payload))
        save("probe-provider-result.json", "Python urllib/json", python_observation, metadata({key.lower(): value for key, value in response.headers.items()}, response.status))
    gh_env = dict(os.environ, GH_TOKEN="", GITHUB_TOKEN="", GH_CONFIG_DIR="/nonexistent/resourcefs-public-probe-no-auth", GH_HOST="github.com", GH_PROMPT_DISABLED="1")
    gh_command = ["gh", "api", "--hostname", "github.com", "--method", "GET", "--include", "-H", "X-GitHub-Api-Version: " + VERSION, "-H", "Accept: application/vnd.github+json", "repos/rust-lang/rust/pulls/159232"]
    # Capture the native response privately; only jq-selected output is persisted.
    gh = subprocess.run(gh_command, env=gh_env, capture_output=True, timeout=TIMEOUT)
    if gh.returncode:
        raise RuntimeError("Independent gh API failed (exit " + str(gh.returncode) + "); stderr withheld to avoid accidental credential output")
    raw = gh.stdout.replace(b"\r\n", b"\n")
    header_block, body = raw.split(b"\n\n", 1)
    if len(body) > CAP:
        raise RuntimeError("gh public response exceeds 1 MiB cap")
    lines = header_block.decode("utf-8").splitlines()
    headers = {key.lower(): value.strip() for key, value in (line.split(":", 1) for line in lines[1:] if ":" in line)}
    jq = subprocess.run(["jq", "-c", JQ], input=body, capture_output=True, timeout=TIMEOUT, check=True)
    oracle_observation = json.loads(jq.stdout)
    save("oracle-provider-result.json", "gh api / jq", oracle_observation, metadata(headers, int(lines[0].split()[1])))
    print(json.dumps({"observation_equal": python_observation == oracle_observation, "python": "probe-provider-result.json", "oracle": "oracle-provider-result.json", "number": python_observation["number"], "id": python_observation["id"], "node_id": python_observation["node_id"]}, separators=(",", ":")))
    if python_observation != oracle_observation:
        raise RuntimeError("Projected observations disagree; inspect selected artifacts")


if __name__ == "__main__":
    main()
