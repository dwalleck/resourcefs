#!/usr/bin/env bash
# Independent oracle for probe_provider.py: same factual questions computed by
# the Go-based gh HTTP client and jq, not Python urllib/json.
set -euo pipefail

API="https://api.github.com"
REPO="rust-lang/rust"
BUSY_PR=112049
EMPTY_PR=162473
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fetch() { # $1 name, $2 path
  gh api --include -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" "$2" >"$WORK/$1.raw" 2>"$WORK/$1.err" || true
  # Split the raw include output at the first blank line: headers, then body.
  awk 'BEGIN{h=1} h && /^\r?$/ {h=0; next} h {print > "'"$WORK"'/'"$1"'.headers"} !h {print > "'"$WORK"'/'"$1"'.body"}' "$WORK/$1.raw"
}

project() { # $1 name, $2 path
  fetch "$1" "$2"
  local status link next count keys
  status="$(awk 'NR==1{print $2}' "$WORK/$1.headers")"
  link="$(awk 'BEGIN{IGNORECASE=1} /^link:/{sub(/^[^:]*:[ ]*/,""); print}' "$WORK/$1.headers")"
  # Independent rel="next" extraction: jq splits link-values, then filters.
  next="$(printf '%s' "$link" | jq -Rr '
    split(",")
    | map(select(test("rel=\"?next\"?"; "i")))
    | (.[0] // "")
    | (capture("<(?<t>[^>]*)>")?.t // "")' 2>/dev/null || true)"
  count="$(jq 'if type=="array" then length else 0 end' "$WORK/$1.body" 2>/dev/null || echo 0)"
  keys="$(jq -r 'if type=="array" and length>0 then (.[0]|keys|sort|join(",")) else "" end' "$WORK/$1.body" 2>/dev/null || echo "")"
  jq -n --arg path "$2" --arg status "$status" --arg next "$next" \
        --arg link "$link" --arg count "$count" --arg keys "$keys" \
        --slurpfile body "$WORK/$1.body" '
    {
      path: $path,
      status: ($status | tonumber),
      next_target: (if $next == "" then null else $next end),
      link_present: ($link != ""),
      count: ($count | tonumber),
      record_keys: (if $keys == "" then [] else ($keys | split(",")) end),
      field_presence: (
        ["id","node_id","url","html_url","issue_url","user","body","created_at","updated_at"]
        | map(. as $f | {
            key: $f,
            value: {
              present: ([$body[0][] | select(has($f))] | length),
              null: ([$body[0][] | select(has($f) and .[$f] == null)] | length),
              types: ([$body[0][] | select(has($f)) | .[$f] | type] | unique)
            }
          })
        | from_entries
      ),
      pull_request_key_present: ([$body[0][] | select(has("pull_request"))] | length > 0)
    }'
}

BUSY="/repos/$REPO/issues/$BUSY_PR/comments"
EMPTY="/repos/$REPO/issues/$EMPTY_PR/comments"

{
  printf '{\n  "observations": {\n'
  printf '    "busy_page1": %s,\n' "$(project busy_page1 "$BUSY?per_page=100&page=1")"
  printf '    "busy_page2": %s,\n' "$(project busy_page2 "$BUSY?per_page=100&page=2")"
  printf '    "busy_past_end": %s,\n' "$(project busy_past_end "$BUSY?per_page=100&page=99999")"
  printf '    "busy_oversized_per_page": %s,\n' "$(project busy_oversized "$BUSY?per_page=1000&page=1")"
  printf '    "empty_page1": %s,\n' "$(project empty_page1 "$EMPTY?per_page=100&page=1")"
  printf '    "empty_past_end": %s\n' "$(project empty_past_end "$EMPTY?per_page=100&page=2")"
  printf '  }\n}\n'
}
