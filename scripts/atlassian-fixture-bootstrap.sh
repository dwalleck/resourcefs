#!/usr/bin/env bash
# Manifest-driven, disposable Jira and Confluence Cloud fixture operator.
# Requires Bash 4.4+ for associative maps, mapfile, and empty arrays under nounset.
if (( BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 4) )); then
  printf 'error: Bash 4.4 or newer is required; install modern Bash (macOS: brew install bash) and put its bin directory first in PATH, then rerun this script.\n' >&2
  exit 1
fi

set -Eeuo pipefail
IFS=$'\n\t'
umask 077
# Credentials arrive exported from the invoking shell, but all child processes
# must receive a credential-free environment. Keep the shell values available.
export -n \
  ATLASSIAN_PROVISIONER_EMAIL \
  ATLASSIAN_PROVISIONER_API_TOKEN \
  ATLASSIAN_READER_EMAIL \
  ATLASSIAN_READER_API_TOKEN

readonly MAX_MANIFEST_BYTES=1048576
readonly MAX_RESPONSE_BYTES=4194304
readonly MAX_PAGES=10
readonly MAX_REQUESTS=512
readonly MAX_POLLS=30
readonly POLL_SECONDS=2
# Confluence strips leading HTML comments from stored bodies and injects
# ac:schema-version/ac:macro-id into macros, so ownership markers ride in a
# v1 content property and body comparisons normalize those two attributes.
readonly PROPERTY_KEY='rfs-owner'
readonly STORAGE_NORMALIZE='gsub(" ac:schema-version=\"[^\"]*\"";"") | gsub(" ac:macro-id=\"[^\"]*\"";"")'

REPOSITORY_ROOT=$(
  cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P
) || {
  printf 'error: repository_root\n' >&2
  exit 1
}
readonly REPOSITORY_ROOT
MODE=""
SITE=""
MANIFEST_PATH="$REPOSITORY_ROOT/fixtures/atlassian-live/manifest.json"
STATE_PATH="$REPOSITORY_ROOT/.resourcefs/atlassian-fixture-state.json"
MANIFEST_SET=0
STATE_SET=0
LOCK_PATH=""
RUN_DIR=""
RESPONSE_FILE=""
REQUESTS=0

# Associative maps are populated only from validated upstream responses.
declare -A JIRA_PROJECT_IDS=() JIRA_PROJECT_KEYS=() JIRA_ISSUE_IDS=() JIRA_ISSUE_KEYS=()
declare -A JIRA_TASK_TYPE_IDS=() JIRA_SUBTASK_TYPE_IDS=() JIRA_COMMENT_IDS=()
declare -A CONF_SPACE_IDS=() CONF_SPACE_HOMEPAGE=() CONF_PAGE_IDS=() CONF_COMMENT_IDS=()

RESPONSE_STATUS=""
die() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: scripts/atlassian-fixture-bootstrap.sh <bootstrap|verify|cleanup> --site <https-origin> [--manifest <path>] [--state <path>]

Requires Bash 4.4 or newer (macOS: brew install bash and put its bin directory first in PATH).

Environment:
  ATLASSIAN_PROVISIONER_EMAIL / ATLASSIAN_PROVISIONER_API_TOKEN
  ATLASSIAN_READER_EMAIL / ATLASSIAN_READER_API_TOKEN (required by verify)

Cleanup permanently deletes marker-owned Jira projects, including owned trash.
State defaults to .resourcefs/atlassian-fixture-state.json. Keep its .pending
creation receipt for recovery with the same manifest; an unknown creation
outcome requires upstream inspection before the receipt can be cleared.
EOF
}
on_exit() {
  local status=$?
  trap - EXIT
  if [[ -n "${RUN_DIR}" && -d "${RUN_DIR}" ]]; then
    if ! rm -rf -- "${RUN_DIR}"; then
      printf 'error: temp_cleanup\n' >&2
      status=1
    fi
  fi
  if [[ -n "${LOCK_PATH}" && -d "${LOCK_PATH}" ]]; then
    if ! rmdir -- "${LOCK_PATH}"; then
      printf 'error: lock_cleanup\n' >&2
      status=1
    fi
  fi
  exit "$status"
}
trap on_exit EXIT

parse_args() {
  if (( $# == 1 )) && [[ "$1" == "--help" ]]; then
    usage
    exit 0
  fi
  (( $# > 0 )) || { usage >&2; exit 2; }
  MODE=$1
  shift
  case "$MODE" in
    bootstrap|verify|cleanup) ;;
    *) usage >&2; exit 2 ;;
  esac
  while (( $# > 0 )); do
    case "$1" in
      --site)
        (( $# >= 2 )) || die "invalid_arguments"
        [[ -z "$SITE" ]] || die "invalid_arguments"
        SITE=$2
        shift 2
        ;;
      --manifest)
        (( $# >= 2 )) || die "invalid_arguments"
        (( MANIFEST_SET == 0 )) || die "invalid_arguments"
        MANIFEST_PATH=$2
        MANIFEST_SET=1
        shift 2
        ;;
      --state)
        (( $# >= 2 )) || die "invalid_arguments"
        (( STATE_SET == 0 )) || die "invalid_arguments"
        [[ -n "$2" ]] || die "invalid_state_path"
        STATE_PATH=$2
        STATE_SET=1
        shift 2
        ;;
      --help|*) usage >&2; exit 2 ;;
    esac
  done
  [[ -n "$SITE" ]] || die "missing_site"
}

validate_site() {
  local raw=$SITE oldifs
  local -a labels
  [[ "$raw" == https://* ]] || die "invalid_site"
  raw=${raw#https://}
  if [[ "$raw" == */ ]]; then
    raw=${raw%/}
  fi
  [[ -n "$raw" && "$raw" != *[/?#]* && "$raw" != *:* ]] || die "invalid_site"
  [[ "$raw" != .* && "$raw" != *..* && "$raw" != *. && "$raw" != *- ]] ||
    die "invalid_site"
  [[ "$raw" =~ ^[a-z0-9.-]+$ ]] || die "invalid_site"
  (( ${#raw} <= 253 )) || die "invalid_site"
  oldifs=$IFS
  IFS='.'
  read -r -a labels <<< "$raw"
  IFS=$oldifs
  (( ${#labels[@]} > 0 )) || die "invalid_site"
  for label in "${labels[@]}"; do
    (( ${#label} <= 63 )) || die "invalid_site"
    [[ "$label" =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ ]] || die "invalid_site"
  done
  SITE="https://$raw"
}

validate_credentials() {
  local name value
  for name in ATLASSIAN_PROVISIONER_EMAIL ATLASSIAN_PROVISIONER_API_TOKEN; do
    value=${!name-}
    [[ -n "$value" && "$value" != *$'\r'* && "$value" != *$'\n'* ]] || die "missing_or_invalid_provisioner_credentials"
  done
  if [[ "$MODE" == verify ]]; then
    for name in ATLASSIAN_READER_EMAIL ATLASSIAN_READER_API_TOKEN; do
      value=${!name-}
      [[ -n "$value" && "$value" != *$'\r'* && "$value" != *$'\n'* ]] || die "missing_or_invalid_reader_credentials"
    done
  fi
}

check_tools() {
  command -v jq >/dev/null 2>&1 || die "missing_jq"
  command -v curl >/dev/null 2>&1 || die "missing_curl"
  command -v stat >/dev/null 2>&1 || die "missing_stat"
  command -v env >/dev/null 2>&1 || die "missing_env"
}

validate_manifest() {
  [[ -L "$MANIFEST_PATH" ]] && die "invalid_manifest_path"
  [[ -f "$MANIFEST_PATH" ]] || die "manifest_missing"
  local size
  size=$(wc -c < "$MANIFEST_PATH") || die "manifest_unreadable"
  (( size <= MAX_MANIFEST_BYTES )) || die "manifest_too_large"
  if ! jq -e '
    def logical: type == "string" and test("^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$");
    def key: type == "string" and test("^[A-Z][A-Z0-9]{1,9}$");
    def text: type == "string" and length <= 4096 and all(explode[]; . == 9 or . == 10 or . == 13 or . >= 32);
    def adfnode: type == "object" and (.type|type) == "string" and ((.content // [])|type) == "array" and all((.content // [])[]; adfnode);
    def adf: type == "object" and .type == "doc" and .version == 1 and (.content|type) == "array" and (.content|length) > 0 and all(.content[]; adfnode);
    def storage: type == "object" and .representation == "storage" and (.value|type) == "string" and (.value|length) > 0 and (.value|length) <= 262144;
    def distinct: length == (unique|length);

    ([keys[]] | sort) == ["confluence","jira","owner_marker","version"] and
    .version == 1 and (.owner_marker|text) and
    (.jira|type) == "object" and (.jira|keys|sort) == ["issues","projects"] and
    (.confluence|type) == "object" and (.confluence|keys|sort) == ["comments","pages","spaces"] and
    (.jira.projects|type) == "array" and (.jira.projects|length) >= 2 and (.jira.projects|length) <= 64 and
    (.jira.issues|type) == "array" and (.jira.issues|length) >= 3 and (.jira.issues|length) <= 128 and
    (.confluence.spaces|type) == "array" and (.confluence.spaces|length) >= 2 and (.confluence.spaces|length) <= 64 and
    (.confluence.pages|type) == "array" and (.confluence.pages|length) >= 3 and (.confluence.pages|length) <= 128 and
    (.confluence.comments|type) == "array" and (.confluence.comments|length) >= 2 and (.confluence.comments|length) <= 128 and
    ([.jira.projects[].id] | distinct) and
    ([.jira.projects[].key] | distinct) and
    ([.jira.issues[].id] | distinct) and
    ([.confluence.spaces[].id] | distinct) and
    ([.confluence.spaces[].key] | distinct) and
    ([.confluence.pages[].id] | distinct) and
    ([.confluence.pages[].title] | distinct) and
    ([.jira.issues[].comments[]?.id] | distinct) and
    ([.confluence.comments[].id] | distinct) and
    ([.jira.projects[].marker] | distinct) and
    ([.jira.issues[].marker] | distinct) and
    ([.jira.issues[].comments[]?.marker] | distinct) and
    ([.confluence.spaces[].marker] | distinct) and
    ([.confluence.pages[].marker] | distinct) and
    ([.confluence.comments[].marker] | distinct) and
    all(.jira.projects[];
      ([keys[]] | sort) == ["id","key","marker","name"] and
      (.id|logical) and (.key|key) and (.name|text) and (.marker|text)) and
    all(.jira.issues[];
      ([keys[]] - ["comments","description","id","marker","parent","project","summary"] | length) == 0 and
      (["description","id","marker","project","summary"] - [keys[]] | length) == 0 and
      (.id|logical) and (.project|logical) and (.summary|text) and (.marker|text) and
      (.description|adf) and (.parent == null or (.parent|logical)) and
      ((.comments // [])|type) == "array" and
      all((.comments // [])[];
        ([keys[]] | sort) == ["body","id","marker"] and
        (.id|logical) and (.marker|text) and (.body|adf))) and
    all(.confluence.spaces[];
      ([keys[]] | sort) == ["id","key","marker","name","private"] and
      (.id|logical) and (.key|key) and (.name|text) and (.marker|text) and (.private|type) == "boolean") and
    all(.confluence.pages[];
      ([keys[]] - ["body","id","marker","parent","space","title"] | length) == 0 and
      (["body","id","marker","space","title"] - [keys[]] | length) == 0 and
      (.id|logical) and (.space|logical) and (.title|text) and (.marker|text) and
      (.body|storage) and (.parent == null or (.parent|logical))) and
    all(.confluence.comments[];
      ([keys[]] - ["body","id","marker","page","parent"] | length) == 0 and
      (["body","id","marker","page"] - [keys[]] | length) == 0 and
      (.id|logical) and (.page|logical) and (.marker|text) and
      (.body|storage) and (.parent == null or (.parent|logical))) and
    (. as $root |
      [$root.jira.projects[].id] as $projects |
      [$root.confluence.spaces[].id] as $spaces |
      [$root.confluence.pages[].id] as $pages |
      [$root.confluence.comments[].id] as $comments |
      all($root.jira.issues | to_entries[];
        . as $entry |
        ($projects | index($entry.value.project) != null) and
        ($entry.value.parent == null or
          ([$root.jira.issues[0:$entry.key][] |
            select(.project == $entry.value.project) | .id] |
            index($entry.value.parent) != null))) and
      all($root.confluence.pages | to_entries[];
        . as $entry |
        ($spaces | index($entry.value.space) != null) and
        ($entry.value.parent == null or
          ([$root.confluence.pages[0:$entry.key][] |
            select(.space == $entry.value.space) | .id] |
            index($entry.value.parent) != null))) and
      all($root.confluence.comments | to_entries[];
        . as $entry |
        ($pages | index($entry.value.page) != null) and
        ($entry.value.parent == null or
          ([$root.confluence.comments[0:$entry.key][] |
            select(.page == $entry.value.page) | .id] |
            index($entry.value.parent) != null)))) and
    (. as $root |
      any($root.jira.projects[].id;
        . as $project | [$root.jira.issues[] | select(.project == $project)] | length >= 2)) and
    any(.jira.issues[]; ((.comments // []) | length) >= 2) and
    (. as $root |
      any($root.confluence.spaces[].id;
        . as $space | [$root.confluence.pages[] | select(.space == $space)] | length >= 2)) and
    (. as $root |
      any($root.confluence.pages[].id;
        . as $page | [$root.confluence.comments[] | select(.page == $page)] | length >= 2)) and
    (([.jira.projects[], .jira.issues[], .jira.issues[].comments[]?, .confluence.spaces[], .confluence.pages[], .confluence.comments[]] | length) <= 256)
  ' "$MANIFEST_PATH" >/dev/null 2>&1; then
    die "invalid_manifest"
  fi
}

reject_symlink_ancestors() {
  local raw=$1 current component oldifs
  local -a components
  if [[ "$raw" == /* ]]; then
    current='/'
    raw=${raw#/}
  else
    current=$PWD
  fi
  oldifs=$IFS
  IFS='/' read -r -a components <<<"$raw"
  IFS=$oldifs
  for component in "${components[@]}"; do
    case "$component" in
      ''|.) continue ;;
      ..) current=$(dirname -- "$current") ;;
      *)
        current="${current%/}/$component"
        [[ ! -L "$current" ]] || die "state_parent_symlink"
        [[ -e "$current" ]] || break
        ;;
    esac
  done
}

prepare_state_and_lock() {
  local parent logical_parent physical_parent
  case "$STATE_PATH" in
    ''|.|..|*/|*/.|*/..) die "invalid_state_path" ;;
  esac
  parent=$(dirname -- "$STATE_PATH")
  [[ -n "$parent" && "$parent" != . ]] || parent='.'
  reject_symlink_ancestors "$parent"
  mkdir -p -- "$parent" 2>/dev/null || die "state_parent_unwritable"
  logical_parent=$(cd -L -- "$parent" 2>/dev/null && pwd -L) ||
    die "state_parent_unwritable"
  physical_parent=$(cd -P -- "$parent" 2>/dev/null && pwd -P) ||
    die "state_parent_unwritable"
  [[ "$logical_parent" == "$physical_parent" ]] ||
    die "state_parent_symlink"
  [[ ! -L "$parent" ]] || die "state_parent_symlink"
  [[ -L "$STATE_PATH" ]] && die "state_symlink"
  [[ -d "$STATE_PATH" ]] && die "state_is_directory"
  LOCK_PATH="${STATE_PATH}.lock"
  [[ ! -L "$LOCK_PATH" ]] || die "lock_symlink"
  if ! mkdir -- "$LOCK_PATH" 2>/dev/null; then
    die "lock_busy"
  fi
  RUN_DIR=$(mktemp -d "${STATE_PATH}.tmp.XXXXXX") || die "temp_unavailable"
  chmod 700 "$RUN_DIR" || die "temp_unavailable"
}

# Build a curl config on stdin. Credentials never occur in process arguments or files.
api_request() {
  local actor=$1 method=$2 path=$3 body=$4 expected=$5 operation=$6
  (( REQUESTS < MAX_REQUESTS )) ||
    die "request_limit actor=$actor operation=$operation"
  ((REQUESTS += 1))
  local email token response err status
  local user_json url_json method_json response_json body_json
  case "$actor" in
    provisioner)
      email=$ATLASSIAN_PROVISIONER_EMAIL
      token=$ATLASSIAN_PROVISIONER_API_TOKEN
      ;;
    reader)
      email=$ATLASSIAN_READER_EMAIL
      token=$ATLASSIAN_READER_API_TOKEN
      ;;
    *) die "invalid_actor operation=$operation" ;;
  esac
  response="$RUN_DIR/response-${REQUESTS}"
  err="$RUN_DIR/error-${REQUESTS}"
  if ! user_json=$(printf '%s' "$email:$token" | jq -Rsr '@json' 2>/dev/null); then
    die "transport_config actor=$actor operation=$operation"
  fi
  if ! url_json=$(printf '%s' "$SITE$path" | jq -Rsr '@json' 2>/dev/null); then
    die "transport_config actor=$actor operation=$operation"
  fi
  if ! method_json=$(printf '%s' "$method" | jq -Rsr '@json' 2>/dev/null); then
    die "transport_config actor=$actor operation=$operation"
  fi
  if ! response_json=$(printf '%s' "$response" | jq -Rsr '@json' 2>/dev/null); then
    die "transport_config actor=$actor operation=$operation"
  fi
  if [[ -n "$body" ]]; then
    if ! body_json=$(printf '%s' "$body" | jq -Rsr '@json' 2>/dev/null); then
      die "transport_config actor=$actor operation=$operation"
    fi
  fi
  if ! status=$(
    {
      printf 'silent\nshow-error\nconnect-timeout = 10\nmax-time = 30\nmax-filesize = 4194304\nrequest = %s\nurl = %s\nuser = %s\nheader = %s\nheader = %s\noutput = %s\nwrite-out = "%%{http_code}"\n' \
        "$method_json" "$url_json" "$user_json" \
        '"Accept: application/json"' '"Content-Type: application/json"' \
        "$response_json"
      if [[ -n "$body" ]]; then
        printf 'data-binary = %s\n' "$body_json"
      fi
    } | env \
      -u ATLASSIAN_PROVISIONER_EMAIL \
      -u ATLASSIAN_PROVISIONER_API_TOKEN \
      -u ATLASSIAN_READER_EMAIL \
      -u ATLASSIAN_READER_API_TOKEN \
      curl --config - 2>"$err"
  ); then
    die "transport_failure actor=$actor operation=$operation"
  fi
  [[ "$status" =~ ^[0-9]{3}$ ]] ||
    die "transport_failure actor=$actor operation=$operation"
  case ",$expected," in
    *",$status,"*) ;;
    *) die "upstream_failure actor=$actor operation=$operation status=$status" ;;
  esac
  RESPONSE_STATUS=$status
  if [[ ! -f "$response" ]]; then
    : > "$response" || die "response_failure actor=$actor operation=$operation"
  fi
  local response_size
  response_size=$(wc -c < "$response") ||
    die "response_failure actor=$actor operation=$operation"
  (( response_size <= MAX_RESPONSE_BYTES )) ||
    die "response_too_large actor=$actor operation=$operation"
  RESPONSE_FILE=$response
}

response_string() {
  local filter=$1 operation=$2 value
  if ! value=$(jq -er "$filter" "$RESPONSE_FILE" 2>/dev/null); then
    die "invalid_response operation=$operation"
  fi
  printf '%s' "$value"
}
response_id() {
  local filter=$1 operation=$2 value
  if ! value=$(jq -er "($filter) as \$value |
    if (\$value|type) == \"string\" and (\$value|test(\"^[0-9]+$\"))
    then \$value
    elif (\$value|type) == \"number\" and \$value >= 0 and \$value == (\$value|floor)
    then (\$value|tostring)
    else error
    end" "$RESPONSE_FILE" 2>/dev/null); then
    die "invalid_response operation=$operation"
  fi
  printf '%s' "$value"
}
decode_json_field() {
  local encoded=$1
  jq -Rrn --arg encoded "$encoded" '$encoded | @base64d' 2>/dev/null ||
    die "invalid_manifest"
}
pending_validate_file() {
  local pending="${STATE_PATH}.pending" size mode
  [[ -L "$pending" ]] && die "pending_symlink"
  [[ -f "$pending" ]] || die "pending_corrupt"
  size=$(wc -c < "$pending") || die "pending_unreadable"
  (( size <= 4096 )) || die "pending_too_large"
  mode=$(stat -c '%a' "$pending" 2>/dev/null) || die "pending_unreadable"
  [[ "$mode" == 600 ]] || die "pending_permissions"
  jq -e --arg site "$SITE" '
    ([keys[]]|sort) == ["container_id","id","kind","logical_id","parent_id","site","version"] and
    .version == 1 and
    (.site|keys|sort) == ["id","origin"] and .site.id == "atlassian-live" and .site.origin == $site and
    (.kind == "page" or .kind == "comment") and
    (.logical_id|type) == "string" and (.logical_id|test("^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$")) and
    (.container_id|type) == "string" and (.container_id|test("^[0-9]+$")) and
    ((.parent_id == null) or ((.parent_id|type) == "string" and (.parent_id|test("^[0-9]+$")))) and
    ((.id == null) or ((.id|type) == "string" and (.id|test("^[0-9]+$"))))
  ' "$pending" >/dev/null 2>&1 || die "pending_corrupt"
}

pending_write_json() {
  local pending="${STATE_PATH}.pending" tmp=$RUN_DIR/pending-write json=$1
  [[ ! -L "$pending" ]] || die "pending_symlink"
  [[ ! -e "$pending" ]] || die "pending_busy"
  printf '%s\n' "$json" > "$tmp" || die "pending_write"
  chmod 600 "$tmp" || die "pending_write"
  mv -f -- "$tmp" "$pending" || die "pending_commit"
}

write_pending_receipt() {
  local kind=$1 logical=$2 container_id=$3 parent_id=${4:-} parent_arg=''
  [[ "$container_id" =~ ^[0-9]+$ ]] || die "pending_invalid"
  if [[ -n "$parent_id" ]]; then
    [[ "$parent_id" =~ ^[0-9]+$ ]] || die "pending_invalid"
    parent_arg=$parent_id
  fi
  local json
  json=$(jq -cn --arg site "$SITE" --arg kind "$kind" --arg logical "$logical" \
    --arg container "$container_id" --arg parent "$parent_arg" \
    '{version:1,site:{id:"atlassian-live",origin:$site},kind:$kind,logical_id:$logical,container_id:$container,parent_id:(if $parent == "" then null else $parent end),id:null}') ||
    die "pending_write"
  pending_write_json "$json"
}

update_pending_receipt_id() {
  local pending="${STATE_PATH}.pending" tmp=$RUN_DIR/pending-write id=$1 json
  [[ "$id" =~ ^[0-9]+$ ]] || die "pending_invalid"
  pending_validate_file
  json=$(jq -c --arg id "$id" '.id=$id' "$pending") || die "pending_write"
  printf '%s\n' "$json" > "$tmp" || die "pending_write"
  chmod 600 "$tmp" || die "pending_write"
  mv -f -- "$tmp" "$pending" || die "pending_commit"
}

clear_pending_receipt() {
  local pending="${STATE_PATH}.pending"
  [[ ! -e "$pending" && ! -L "$pending" ]] && return 0
  pending_validate_file
  rm -f -- "$pending" || die "pending_remove"
}

ensure_no_pending_receipt() {
  local pending="${STATE_PATH}.pending"
  [[ ! -e "$pending" && ! -L "$pending" ]] && return 0
  pending_validate_file
  die "pending_receipt"
}

recover_pending_receipt() {
  local pending="${STATE_PATH}.pending" kind logical container parent id marker body object_response record homepage
  local title space page manifest_parent parent_marker parent_space space_key space_name space_marker
  local page_marker parent_response page_response space_response space_record parent_record
  [[ ! -e "$pending" && ! -L "$pending" ]] && return 0
  pending_validate_file
  kind=$(jq -r '.kind' "$pending")
  logical=$(jq -r '.logical_id' "$pending")
  container=$(jq -r '.container_id' "$pending")
  parent=$(jq -r 'if .parent_id == null then "" else .parent_id end' "$pending")
  id=$(jq -r 'if .id == null then "" else .id end' "$pending")
  [[ -n "$id" ]] || die "creation_outcome_unknown logical=$logical repair_required=inspect_creation_before_clearing_receipt"
  if [[ "$kind" == page ]]; then
    record=$(jq -er --arg id "$logical" '
      [.confluence.pages[] | select(.id == $id)] |
      if length == 1 then .[0] | [.space,.title,.marker,(.body|tojson|@base64),(.parent // "")] | @tsv else error end
    ' "$MANIFEST_PATH" 2>/dev/null) || die "pending_manifest_missing"
    IFS=$'\t' read -r space title marker body manifest_parent <<<"$record"
    body=$(decode_json_field "$body")
    if [[ -z "$manifest_parent" ]]; then
      [[ -z "$parent" ]] || die "identity_mismatch logical=page"
    else
      [[ -n "$parent" ]] || die "identity_mismatch logical=page"
    fi
    api_request provisioner GET "/wiki/api/v2/pages/${id}?body-format=storage" '' 200,404 pending_page_get
    [[ "$RESPONSE_STATUS" == 404 ]] && { clear_pending_receipt; return 0; }
    object_response=$RESPONSE_FILE
    space_record=$(jq -er --arg id "$space" '
      [.confluence.spaces[] | select(.id == $id)] |
      if length == 1 then .[0] | [.key,.name,.marker] | @tsv else error end
    ' "$MANIFEST_PATH" 2>/dev/null) || die "pending_manifest_missing"
    IFS=$'\t' read -r space_key space_name space_marker <<<"$space_record"
    api_request provisioner GET "/wiki/api/v2/spaces/${container}?description-format=plain" '' 200 pending_space_get
    space_response=$RESPONSE_FILE
    homepage=$(jq -er '
      if (.homepageId|type) == "number" and .homepageId > 0 then (.homepageId|tostring)
      elif (.homepageId|type) == "string" and (.homepageId|test("^[0-9]+$")) then .homepageId
      else error end
    ' "$space_response" 2>/dev/null) || die "invalid_response operation=pending_space_get"
    jq -e --arg id "$container" --arg key "$space_key" --arg name "$space_name" --arg marker "$space_marker" '
      (.id|tostring) == $id and .key == $key and .name == $name and
      ((if (.description|type) == "string" then .description
        elif (.description|type) == "object" and (.description.value?|type) == "string" then .description.value
        elif (.description|type) == "object" and (.description.plain?|type) == "object"
          and (.description.plain.value?|type) == "string" then .description.plain.value
        else empty end) == $marker)
    ' "$space_response" >/dev/null 2>&1 || die "identity_mismatch logical=page_container"
    jq -e --arg id "$id" --arg container "$container" --arg title "$title" --arg parent "$parent" \
      --arg homepage "$homepage" --argjson body "$body" '
        (.id|tostring) == $id and (.spaceId|tostring) == $container and .title == $title and
        ((.body.storage? // .body?).value | '"$STORAGE_NORMALIZE"') == ($body.value | '"$STORAGE_NORMALIZE"') and
        (($parent == "" and ((.parentId? == null) or (.parentId|tostring) == $homepage)) or
         ($parent != "" and (.parentId|tostring) == $parent))
      ' "$object_response" >/dev/null 2>&1 || die "identity_mismatch logical=page"
    if [[ -n "$parent" ]]; then
      parent_record=$(jq -er --arg id "$manifest_parent" '
        [.confluence.pages[] | select(.id == $id)] |
        if length == 1 then .[0] | [.space,.marker] | @tsv else error end
      ' "$MANIFEST_PATH" 2>/dev/null) || die "pending_manifest_missing"
      IFS=$'\t' read -r parent_space parent_marker <<<"$parent_record"
      [[ "$parent_space" == "$space" ]] || die "identity_mismatch logical=page_parent"
      api_request provisioner GET "/wiki/api/v2/pages/${parent}?body-format=storage" '' 200 pending_parent_page_get
      parent_response=$RESPONSE_FILE
      jq -e --arg id "$parent" --arg container "$container" \
        '(.id|tostring) == $id and (.spaceId|tostring) == $container' "$parent_response" >/dev/null 2>&1 ||
        die "identity_mismatch logical=page_parent"
      read_owner_property provisioner "$parent"
      [[ "$OWNER_PROPERTY_VALUE" == "$parent_marker" ]] ||
        die "foreign_collision logical=page_parent"
    fi
    read_owner_property provisioner "$id"
    if [[ -n "$OWNER_PROPERTY_VALUE" && "$OWNER_PROPERTY_VALUE" != "$marker" ]]; then
      die "foreign_collision logical=page"
    fi
    [[ -n "$OWNER_PROPERTY_VALUE" ]] || write_owner_property provisioner "$id" "$marker"
    CONF_PAGE_IDS[$logical]=$id
  else
    record=$(jq -er --arg id "$logical" '
      [.confluence.comments[] | select(.id == $id)] |
      if length == 1 then .[0] | [.page,.marker,(.body|tojson|@base64),(.parent // "")] | @tsv else error end
    ' "$MANIFEST_PATH" 2>/dev/null) || die "pending_manifest_missing"
    IFS=$'\t' read -r page marker body manifest_parent <<<"$record"
    body=$(decode_json_field "$body")
    if [[ -z "$manifest_parent" ]]; then
      [[ -z "$parent" ]] || die "identity_mismatch logical=comment"
    else
      [[ -n "$parent" ]] || die "identity_mismatch logical=comment"
    fi
    api_request provisioner GET "/wiki/api/v2/footer-comments/${id}?body-format=storage" '' 200,404 pending_comment_get
    [[ "$RESPONSE_STATUS" == 404 ]] && { clear_pending_receipt; return 0; }
    object_response=$RESPONSE_FILE
    jq -e --arg id "$id" --arg container "$container" --arg parent "$parent" --argjson body "$body" '
      (.id|tostring) == $id and (.pageId|tostring) == $container and
      ((.body.storage? // .body?).value | '"$STORAGE_NORMALIZE"') == ($body.value | '"$STORAGE_NORMALIZE"') and
      (($parent == "" and (.parentCommentId? == null)) or
       ($parent != "" and (.parentCommentId|tostring) == $parent))
    ' "$object_response" >/dev/null 2>&1 || die "identity_mismatch logical=comment"
    api_request provisioner GET "/wiki/api/v2/pages/${container}?body-format=storage" '' 200 pending_comment_page_get
    page_response=$RESPONSE_FILE
    page_marker=$(jq -er --arg id "$page" '.confluence.pages[] | select(.id == $id) | .marker' "$MANIFEST_PATH" 2>/dev/null) ||
      die "pending_manifest_missing"
    jq -e --arg id "$container" '(.id|tostring) == $id' "$page_response" >/dev/null 2>&1 ||
      die "identity_mismatch logical=comment_page"
    read_owner_property provisioner "$container"
    [[ "$OWNER_PROPERTY_VALUE" == "$page_marker" ]] ||
      die "foreign_collision logical=comment_page"
    if [[ -n "$parent" ]]; then
      parent_marker=$(jq -er --arg id "$manifest_parent" '.confluence.comments[] | select(.id == $id) | .marker' "$MANIFEST_PATH" 2>/dev/null) ||
        die "pending_manifest_missing"
      api_request provisioner GET "/wiki/api/v2/footer-comments/${parent}?body-format=storage" '' 200 pending_parent_comment_get
      parent_response=$RESPONSE_FILE
      jq -e --arg id "$parent" --arg container "$container" \
        '(.id|tostring) == $id and (.pageId|tostring) == $container' "$parent_response" >/dev/null 2>&1 ||
        die "identity_mismatch logical=comment_parent"
      read_owner_property provisioner "$parent"
      [[ "$OWNER_PROPERTY_VALUE" == "$parent_marker" ]] ||
        die "foreign_collision logical=comment_parent"
    fi
    read_owner_property provisioner "$id"
    if [[ -n "$OWNER_PROPERTY_VALUE" && "$OWNER_PROPERTY_VALUE" != "$marker" ]]; then
      die "foreign_collision logical=comment"
    fi
    [[ -n "$OWNER_PROPERTY_VALUE" ]] || write_owner_property provisioner "$id" "$marker"
    CONF_COMMENT_IDS[$logical]=$id
  fi
  clear_pending_receipt
}


# Read the rfs-owner content property; sets OWNER_PROPERTY_VALUE ('' when absent).
read_owner_property() {
  local actor=$1 content_id=$2
  OWNER_PROPERTY_VALUE=''
  api_request "$actor" GET "/wiki/rest/api/content/${content_id}/property/${PROPERTY_KEY}" '' '200,404' owner_property_get
  [[ "$RESPONSE_STATUS" == 200 ]] || return 0
  OWNER_PROPERTY_VALUE=$(jq -er --arg key "$PROPERTY_KEY" '
    if (.key|type)=="string" and .key == $key and (.value|type)=="string" then .value else error end
  ' "$RESPONSE_FILE" 2>/dev/null) || die "invalid_response operation=owner_property_get"
}
declare -A CLEANUP_TARGET_IDS=() CLEANUP_JIRA_COMMENT_ISSUE=()
declare -A CLEANUP_CONF_COMMENT_PAGE=() CLEANUP_CONF_COMMENT_PARENT=()
declare -A CLEANUP_CONF_SPACE_KEY=()
declare -A JIRA_ISSUE_SEARCH_PROJECT=() CONF_SPACE_KEYS=()
declare -A CONF_PAGE_SEARCH_SPACE=() CONF_COMMENT_SEARCH_PAGE=() CONF_COMMENT_SEARCH_PARENT=()

cleanup_add_target() {
  local kind=$1 logical=$2 id=$3 existing
  local key="${kind}:${logical}"
  [[ "$id" =~ ^[0-9]+$ ]] || die "invalid_response operation=cleanup_target"
  existing=${CLEANUP_TARGET_IDS[$key]-}
  case " $existing " in
    *" $id "*) ;;
    *) CLEANUP_TARGET_IDS[$key]="${existing:+$existing }$id" ;;
  esac
}

cleanup_seed_state_targets() {
  local logical issue_logical issue_id page parent key
  while IFS= read -r logical; do
    [[ -n "${JIRA_PROJECT_IDS[$logical]-}" ]] &&
      cleanup_add_target jira_project "$logical" "${JIRA_PROJECT_IDS[$logical]}"
  done < <(jq -r '.jira.projects[].id' "$MANIFEST_PATH")
  while IFS= read -r logical; do
    [[ -n "${JIRA_ISSUE_IDS[$logical]-}" ]] &&
      cleanup_add_target jira_issue "$logical" "${JIRA_ISSUE_IDS[$logical]}"
  done < <(jq -r '.jira.issues[].id' "$MANIFEST_PATH")
  while IFS=$'\t' read -r logical issue_logical; do
    [[ -n "${JIRA_COMMENT_IDS[$logical]-}" ]] || continue
    cleanup_add_target jira_comment "$logical" "${JIRA_COMMENT_IDS[$logical]}"
    issue_id=${JIRA_ISSUE_IDS[$issue_logical]-}
    [[ -n "$issue_id" ]] && CLEANUP_JIRA_COMMENT_ISSUE[$logical]=$issue_id
  done < <(jq -r '.jira.issues[] as $issue | ($issue.comments // [])[]? | [.id,$issue.id] | @tsv' "$MANIFEST_PATH")
  while IFS=$'\t' read -r logical key; do
    CONF_SPACE_KEYS[$logical]=${CONF_SPACE_KEYS[$logical]-$key}
    [[ -n "${CONF_SPACE_IDS[$logical]-}" ]] &&
      cleanup_add_target conf_space "$logical" "${CONF_SPACE_IDS[$logical]}"
  done < <(jq -r '.confluence.spaces[] | [.id,.key] | @tsv' "$MANIFEST_PATH")
  while IFS= read -r logical; do
    [[ -n "${CONF_PAGE_IDS[$logical]-}" ]] &&
      cleanup_add_target conf_page "$logical" "${CONF_PAGE_IDS[$logical]}"
  done < <(jq -r '.confluence.pages[].id' "$MANIFEST_PATH")
  while IFS=$'\t' read -r logical page parent; do
    [[ -n "${CONF_COMMENT_IDS[$logical]-}" ]] || continue
    cleanup_add_target conf_comment "$logical" "${CONF_COMMENT_IDS[$logical]}"
    CLEANUP_CONF_COMMENT_PAGE[$logical]=${CONF_PAGE_IDS[$page]-}
    if [[ "$parent" != null && -n "$parent" ]]; then
      CLEANUP_CONF_COMMENT_PARENT[$logical]=${CONF_COMMENT_IDS[$parent]-}
    fi
  done < <(jq -r '.confluence.comments[] | [.id,.page,(.parent // "null")] | @tsv' "$MANIFEST_PATH")
}


write_owner_property() {
  local actor=$1 content_id=$2 marker=$3 body
  body=$(jq -cn --arg key "$PROPERTY_KEY" --arg value "$marker" '{key:$key,value:$value}')
  api_request "$actor" POST "/wiki/rest/api/content/${content_id}/property" "$body" '200,201' owner_property_create
  jq -e --arg key "$PROPERTY_KEY" --arg value "$marker" '
    (.key|type)=="string" and .key == $key and (.value|type)=="string" and .value == $value
  ' "$RESPONSE_FILE" >/dev/null 2>&1 ||
    die "identity_mismatch operation=owner_property_create"
}



# Append JSON rows to a temporary newline-delimited state collection.
append_row() {
  local file=$1 row=$2
  printf '%s\n' "$row" >> "$file" || die "state_write"
}


find_jira_project() {
  local key=$1 marker=$2 start=0 pages=0 is_last max values row row_key row_marker row_id
  local status=${3:-live}
  FOUND_ID=''
  FOUND_KEY=''
  while :; do
    (( pages < MAX_PAGES )) || die "pagination_limit product=jira operation=project_search"
    # Live project search omits `description` unless expanded, and the
    # description carries the ownership marker used for adoption.
    api_request provisioner GET "/rest/api/3/project/search?startAt=${start}&maxResults=1&expand=description&status=${status}" '' 200 project_search
    values=$(jq -ec '.values | if type == "array" then . else error end' "$RESPONSE_FILE" 2>/dev/null) ||
      die "invalid_response operation=project_search"
    while IFS= read -r row; do
      row_key=$(jq -r '.key // empty | strings' <<<"$row")
      row_marker=$(jq -r '.description // empty | strings' <<<"$row")
      if [[ "$row_key" == "$key" || "$row_marker" == "$marker" ]]; then
        [[ "$row_key" == "$key" && "$row_marker" == "$marker" ]] ||
          die "foreign_collision logical=project"
        row_id=$(jq -r '.id // empty | strings' <<<"$row")
        [[ "$row_id" =~ ^[0-9]+$ ]] || die "invalid_response operation=project_search"
        [[ -z "$FOUND_ID" || "$FOUND_ID" == "$row_id" ]] ||
          die "ambiguous_object logical=project"
        FOUND_ID=$row_id
        FOUND_KEY=$row_key
      fi
    done < <(jq -c '.[]' <<<"$values")
    is_last=$(jq -r 'if (.isLast|type) == "boolean" then .isLast else error end' "$RESPONSE_FILE" 2>/dev/null) ||
      die "invalid_pagination product=jira operation=project_search"
    [[ "$is_last" == true ]] && return 0
    max=$(jq -r 'if (.maxResults|type) == "number" then .maxResults else error end' "$RESPONSE_FILE" 2>/dev/null) ||
      die "invalid_pagination product=jira operation=project_search"
    [[ "$max" =~ ^[1-9][0-9]*$ ]] ||
      die "invalid_pagination product=jira operation=project_search"
    (( start + max > start )) ||
      die "invalid_pagination product=jira operation=project_search"
    start=$((start + max))
    ((pages += 1))
  done
}
find_jira_issue() {
  local project_key=$1 marker=$2 token='' pages=0 row row_marker row_id row_key is_last new_token body
  FOUND_ID=''
  FOUND_KEY=''
  while :; do
    (( pages < MAX_PAGES )) || die "pagination_limit product=jira operation=issue_search"
    if [[ -n "$token" ]]; then
      body=$(jq -cn --arg jql "project = \"$project_key\" ORDER BY key" --arg token "$token" \
        '{jql:$jql,maxResults:1,nextPageToken:$token,fields:["summary","description","project","parent"]}')
    else
      body=$(jq -cn --arg jql "project = \"$project_key\" ORDER BY key" \
        '{jql:$jql,maxResults:1,fields:["summary","description","project","parent"]}')
    fi
    api_request provisioner POST '/rest/api/3/search/jql' "$body" 200 issue_search
    jq -e '.issues | type == "array"' "$RESPONSE_FILE" >/dev/null 2>&1 ||
      die "invalid_response operation=issue_search"
    while IFS= read -r row; do
      row_marker=$(jq -r --arg marker "$marker" '
        [.fields.description? | .. | objects | select(.type? == "text") | .text? | strings] | any(. == $marker)
      ' <<<"$row")
      if [[ "$row_marker" == true ]]; then
        row_id=$(jq -r '.id // empty | strings' <<<"$row")
        row_key=$(jq -r '.key // empty | strings' <<<"$row")
        [[ "$row_id" =~ ^[0-9]+$ && "$row_key" =~ ^[A-Z][A-Z0-9]{1,9}-[1-9][0-9]*$ ]] ||
          die "invalid_response operation=issue_search"
        [[ -z "$FOUND_ID" || "$FOUND_ID" == "$row_id" ]] ||
          die "ambiguous_object logical=issue"
        FOUND_ID=$row_id
        FOUND_KEY=$row_key
      fi
    done < <(jq -c '.issues[]' "$RESPONSE_FILE")
    is_last=$(jq -r 'if (.isLast|type) == "boolean" then .isLast else error end' "$RESPONSE_FILE" 2>/dev/null) ||
      die "invalid_pagination product=jira operation=issue_search"
    [[ "$is_last" == true ]] && return 0
    new_token=$(jq -r 'if (.nextPageToken|type) == "string" then .nextPageToken else error end' "$RESPONSE_FILE" 2>/dev/null) ||
      die "invalid_pagination product=jira operation=issue_search"
    [[ -n "$new_token" && "$new_token" != "$token" ]] ||
      die "invalid_pagination product=jira operation=issue_search"
    token=$new_token
    ((pages += 1))
  done
}

find_jira_comment() {
  local issue_id=$1 marker=$2 start=0 pages=0 row row_body cid max total response_start
  FOUND_ID=''
  FOUND_KEY=''
  while :; do
    (( pages < MAX_PAGES )) || die "pagination_limit product=jira operation=comment_list"
    api_request provisioner GET "/rest/api/3/issue/${issue_id}/comment?startAt=${start}&maxResults=1" '' 200 comment_list
    jq -e '.comments | type == "array"' "$RESPONSE_FILE" >/dev/null 2>&1 ||
      die "invalid_response operation=comment_list"
    while IFS= read -r row; do
      row_body=$(jq -r --arg marker "$marker" '
        [.body? | .. | objects | select(.type? == "text") | .text? | strings] | any(. == $marker)
      ' <<<"$row")
      if [[ "$row_body" == true ]]; then
        cid=$(jq -r '.id // empty | strings' <<<"$row")
        [[ "$cid" =~ ^[0-9]+$ ]] || die "invalid_response operation=comment_list"
        [[ -z "$FOUND_ID" || "$FOUND_ID" == "$cid" ]] ||
          die "ambiguous_object logical=comment"
        FOUND_ID=$cid
      fi
    done < <(jq -c '.comments[]' "$RESPONSE_FILE")
    read -r response_start max total < <(
      jq -r 'if ([.startAt,.maxResults,.total] | all(type == "number"))
        then [.startAt,.maxResults,.total] | @tsv else error end' "$RESPONSE_FILE" 2>/dev/null
    ) || die "invalid_pagination product=jira operation=comment_list"
    [[ "$response_start" =~ ^[0-9]+$ && "$response_start" == "$start" &&
       "$max" =~ ^[1-9][0-9]*$ && "$total" =~ ^[0-9]+$ ]] ||
      die "invalid_pagination product=jira operation=comment_list"
    (( start + max < total )) || return 0
    start=$((start + max))
    ((pages += 1))
  done
}

find_confluence_space() {
  local key=$1 marker=$2 pages=0 row row_key row_marker sid next
  local path='/wiki/api/v2/spaces?limit=1&description-format=plain'
  FOUND_ID=''
  while :; do
    (( pages < MAX_PAGES )) || die "pagination_limit product=confluence operation=space_list"
    api_request provisioner GET "$path" '' 200 space_list
    jq -e '.results | type == "array"' "$RESPONSE_FILE" >/dev/null 2>&1 ||
      die "invalid_response operation=space_list"
    while IFS= read -r row; do
      row_key=$(jq -r '.key // empty | strings' <<<"$row")
      row_marker=$(jq -r '
        if (.description|type) == "string" then .description
        elif (.description|type) == "object" and (.description.value?|type) == "string" then .description.value
        elif (.description|type) == "object" and (.description.plain?|type) == "object"
          and (.description.plain.value?|type) == "string" then .description.plain.value
        else empty
        end
      ' <<<"$row")
      if [[ "$row_key" == "$key" || "$row_marker" == "$marker" ]]; then
        [[ "$row_key" == "$key" && "$row_marker" == "$marker" ]] ||
          die "foreign_collision logical=space"
        sid=$(jq -r '.id // empty | strings' <<<"$row")
        [[ "$sid" =~ ^[0-9]+$ ]] || die "invalid_response operation=space_list"
        [[ -z "$FOUND_ID" || "$FOUND_ID" == "$sid" ]] ||
          die "ambiguous_object logical=space"
        FOUND_ID=$sid
        FOUND_KEY=$row_key
      fi
    done < <(jq -c '.results[]' "$RESPONSE_FILE")
    next=$(jq -r 'if ._links.next == null then "" elif (._links.next|type) == "string" then ._links.next else error end' "$RESPONSE_FILE" 2>/dev/null) ||
      die "invalid_pagination product=confluence operation=space_list"
    [[ -n "$next" ]] || return 0
    [[ "$next" == /wiki/api/v2/spaces\?* && "$next" != "$path" ]] ||
      die "invalid_pagination product=confluence operation=space_list"
    path=$next
    ((pages += 1))
  done
}

find_confluence_page() {
  local space_id=$1 title=$2 marker=$3 pages=0 row row_title pid next list_response
  local path="/wiki/api/v2/spaces/${space_id}/pages?limit=1&body-format=storage"
  FOUND_ID=''
  while :; do
    (( pages < MAX_PAGES )) || die "pagination_limit product=confluence operation=page_list"
    api_request provisioner GET "$path" '' 200 page_list
    jq -e '.results | type == "array"' "$RESPONSE_FILE" >/dev/null 2>&1 ||
      die "invalid_response operation=page_list"
    list_response=$RESPONSE_FILE
    while IFS= read -r row; do
      row_title=$(jq -r '.title // empty | strings' <<<"$row")
      if [[ "$row_title" == "$title" ]]; then
        pid=$(jq -r '.id // empty | strings' <<<"$row")
        [[ "$pid" =~ ^[0-9]+$ ]] || die "invalid_response operation=page_list"
        read_owner_property provisioner "$pid"
        [[ "$OWNER_PROPERTY_VALUE" == "$marker" ]] ||
          die "foreign_collision logical=page"
        [[ -z "$FOUND_ID" || "$FOUND_ID" == "$pid" ]] ||
          die "ambiguous_object logical=page"
        FOUND_ID=$pid
      fi
    done < <(jq -c '.results[]' "$list_response")
    next=$(jq -r 'if ._links.next == null then "" elif (._links.next|type) == "string" then ._links.next else error end' "$list_response" 2>/dev/null) ||
      die "invalid_pagination product=confluence operation=page_list"
    [[ -n "$next" ]] || return 0
    [[ "$next" == "/wiki/api/v2/spaces/${space_id}/pages?"* && "$next" != "$path" ]] ||
      die "invalid_pagination product=confluence operation=page_list"
    path=$next
    ((pages += 1))
  done
}

find_confluence_comment() {
  local page_id=$1 marker=$2 parent_comment_id=${3:-} pages=0 row cid next list_response
  local row_page row_parent path
  if [[ -n "$parent_comment_id" ]]; then
    path="/wiki/api/v2/footer-comments/${parent_comment_id}/children?limit=1&body-format=storage"
  else
    path="/wiki/api/v2/pages/${page_id}/footer-comments?limit=1&body-format=storage"
  fi
  FOUND_ID=''
  while :; do
    (( pages < MAX_PAGES )) || die "pagination_limit product=confluence operation=comment_list"
    api_request provisioner GET "$path" '' 200 comment_list
    jq -e '.results | type == "array"' "$RESPONSE_FILE" >/dev/null 2>&1 ||
      die "invalid_response operation=comment_list"
    list_response=$RESPONSE_FILE
    while IFS= read -r row; do
      cid=$(jq -r '.id // empty | strings' <<<"$row")
      [[ "$cid" =~ ^[0-9]+$ ]] || die "invalid_response operation=comment_list"
      row_page=$(jq -r 'if (.pageId|type) == "number" then (.pageId|tostring) elif (.pageId|type) == "string" then .pageId else empty end' <<<"$row")
      [[ "$row_page" == "$page_id" ]] || die "invalid_response operation=comment_list"
      row_parent=$(jq -r 'if (.parentCommentId|type) == "number" then (.parentCommentId|tostring) elif (.parentCommentId|type) == "string" then .parentCommentId else "" end' <<<"$row")
      if [[ -n "$parent_comment_id" ]]; then
        [[ "$row_parent" == "$parent_comment_id" ]] || die "invalid_response operation=comment_list"
      else
        [[ -z "$row_parent" ]] || die "invalid_response operation=comment_list"
      fi
      read_owner_property provisioner "$cid"
      if [[ "$OWNER_PROPERTY_VALUE" == "$marker" ]]; then
        [[ -z "$FOUND_ID" || "$FOUND_ID" == "$cid" ]] ||
          die "ambiguous_object logical=comment"
        FOUND_ID=$cid
      fi
    done < <(jq -c '.results[]' "$list_response")
    next=$(jq -r 'if ._links.next == null then "" elif (._links.next|type) == "string" then ._links.next else error end' "$list_response" 2>/dev/null) ||
      die "invalid_pagination product=confluence operation=comment_list"
    [[ -n "$next" ]] || return 0
    if [[ -n "$parent_comment_id" ]]; then
      [[ "$next" == "/wiki/api/v2/footer-comments/${parent_comment_id}/children?"* && "$next" != "$path" ]] ||
        die "invalid_pagination product=confluence operation=comment_list"
    else
      [[ "$next" == "/wiki/api/v2/pages/${page_id}/footer-comments?"* && "$next" != "$path" ]] ||
        die "invalid_pagination product=confluence operation=comment_list"
    fi
    path=$next
    ((pages += 1))
  done
}

load_jira_issue_types() {
  local logical_id=$1 project_key=$2 task_id subtask_id
  api_request provisioner GET "/rest/api/3/issue/createmeta/${project_key}/issuetypes?startAt=0&maxResults=100" '' 200 issue_types
  jq -e '
    (.issueTypes|type) == "array" and
    ([.startAt,.maxResults,.total] | all(type == "number")) and
    .startAt == 0 and .maxResults > 0 and .total <= .maxResults and
    .total == (.issueTypes|length) and
    all(.issueTypes[]; (.id|type) == "string" and (.subtask|type) == "boolean")
  ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "invalid_response operation=issue_types"
  task_id=$(jq -er '[.issueTypes[] | select(.subtask == false and .name == "Task") | .id] |
    if length == 1 then .[0] else error end' "$RESPONSE_FILE" 2>/dev/null) ||
    die "missing_issue_type logical=project"
  subtask_id=$(jq -er '[.issueTypes[] | select(.subtask == true) | .id] | sort |
    if length > 0 then .[0] else error end' "$RESPONSE_FILE" 2>/dev/null) ||
    die "missing_subtask_type logical=project"
  [[ "$task_id" =~ ^[0-9]+$ && "$subtask_id" =~ ^[0-9]+$ ]] ||
    die "invalid_response operation=issue_types"
  JIRA_TASK_TYPE_IDS[$logical_id]=$task_id
  JIRA_SUBTASK_TYPE_IDS[$logical_id]=$subtask_id
}

bootstrap_jira_projects() {
  local id key name marker body account_id project_id project_key
  recover_pending_receipt
  api_request provisioner GET '/rest/api/3/myself' '' 200 account_get
  account_id=$(response_string 'if (.accountId|type)=="string" then .accountId else empty end' account_get)
  [[ "$account_id" =~ ^[A-Za-z0-9:_-]{1,128}$ ]] ||
    die "invalid_response operation=account_get"
  while IFS=$'\t' read -r id key name marker; do
    find_jira_project "$key" "$marker"
    if [[ -n "$FOUND_ID" ]]; then
      project_id=$FOUND_ID
      api_request provisioner GET "/rest/api/3/project/${key}" '' 200 project_get
      jq -e --arg id "$project_id" --arg key "$key" --arg marker "$marker" '
        .id == $id and .key == $key and .description == $marker
      ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "foreign_collision logical=project"
      if ! jq -e --arg name "$name" '.name == $name' "$RESPONSE_FILE" >/dev/null 2>&1; then
        delete_if_owned provisioner DELETE "/rest/api/3/project/${project_id}?enableUndo=false" jira_project_delete
        project_id=''
      fi
    else
      project_id=''
    fi
    if [[ -z "$project_id" ]]; then
      body=$(jq -cn --arg key "$key" --arg name "$name" --arg marker "$marker" --arg lead "$account_id" \
        '{key:$key,name:$name,description:$marker,leadAccountId:$lead,projectTypeKey:"business",projectTemplateKey:"com.atlassian.jira-core-project-templates:jira-core-simplified-project-management"}')
      api_request provisioner POST '/rest/api/3/project' "$body" 201 project_create
      project_id=$(response_id '.id' project_create)
      project_key=$(response_string 'if (.key|type)=="string" then .key else empty end' project_create)
      [[ "$project_id" =~ ^[0-9]+$ && "$project_key" == "$key" ]] ||
        die "identity_mismatch logical=project"
      api_request provisioner GET "/rest/api/3/project/${key}" '' 200 project_get
      jq -e --arg id "$project_id" --arg key "$key" --arg name "$name" --arg marker "$marker" '
        .id == $id and .key == $key and .name == $name and .description == $marker
      ' "$RESPONSE_FILE" >/dev/null 2>&1 ||
        die "identity_mismatch logical=project"
    fi
    JIRA_PROJECT_IDS[$id]=$project_id
    JIRA_PROJECT_KEYS[$id]=$key
    load_jira_issue_types "$id" "$key"
  done < <(jq -r '.jira.projects[] | [.id,.key,.name,.marker] | @tsv' "$MANIFEST_PATH")
}

bootstrap_jira_issues() {
  local id project summary marker description parent body issue_id issue_key parent_id issue_type_id
  while IFS=$'\t' read -r id project summary marker description parent; do
    description=$(decode_json_field "$description")
    local project_key=${JIRA_PROJECT_KEYS[$project]-}
    [[ -n "$project_key" ]] || die "manifest_parent_missing logical=issue"
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${JIRA_ISSUE_IDS[$parent]-}
      [[ -n "$parent_id" ]] || die "manifest_parent_missing logical=issue"
    fi
    find_jira_issue "$project_key" "$marker"
    if [[ -n "$FOUND_ID" ]]; then
      issue_id=$FOUND_ID
      issue_key=$FOUND_KEY
      api_request provisioner GET "/rest/api/3/issue/${issue_id}?fields=summary,description,project,parent" '' 200 issue_get
      if ! jq -e --arg id "$issue_id" --arg key "$project_key" --arg summary "$summary" \
        --arg parent "$parent_id" --argjson description "$description" '
          .id == $id and .fields.project.key == $key and .fields.summary == $summary and
          .fields.description == $description and
          (($parent == "" and (.fields.parent? == null)) or
           ($parent != "" and (.fields.parent.id == $parent)))
        ' "$RESPONSE_FILE" >/dev/null 2>&1; then
        delete_if_owned provisioner DELETE "/rest/api/3/issue/${issue_id}" jira_issue_delete
        issue_id=''
        issue_key=''
      fi
    else
      issue_id=''
      issue_key=''
    fi
    if [[ -z "$issue_id" ]]; then
      if [[ -z "$parent_id" ]]; then
        issue_type_id=${JIRA_TASK_TYPE_IDS[$project]-}
        [[ -n "$issue_type_id" ]] || die "missing_issue_type logical=project"
        body=$(jq -cn --arg key "$project_key" --arg summary "$summary" --arg type "$issue_type_id" --argjson desc "$description" \
          '{fields:{project:{key:$key},summary:$summary,issuetype:{id:$type},description:$desc}}')
      else
        issue_type_id=${JIRA_SUBTASK_TYPE_IDS[$project]-}
        [[ -n "$issue_type_id" ]] || die "missing_subtask_type logical=project"
        body=$(jq -cn --arg key "$project_key" --arg summary "$summary" --arg type "$issue_type_id" --argjson desc "$description" --arg parent "$parent_id" \
          '{fields:{project:{key:$key},summary:$summary,issuetype:{id:$type},description:$desc,parent:{id:$parent}}}')
      fi
      api_request provisioner POST '/rest/api/3/issue' "$body" 201 issue_create
      issue_id=$(response_id '.id' issue_create)
      issue_key=$(response_string 'if (.key|type)=="string" then .key else empty end' issue_create)
      [[ "$issue_key" =~ ^${project_key}-[1-9][0-9]*$ ]] ||
        die "identity_mismatch logical=issue"
      api_request provisioner GET "/rest/api/3/issue/${issue_id}?fields=summary,description,project,parent" '' 200 issue_get
      jq -e --arg id "$issue_id" --arg key "$project_key" --arg summary "$summary" \
        --arg parent "$parent_id" --argjson description "$description" '
          .id == $id and .fields.project.key == $key and .fields.summary == $summary and
          .fields.description == $description and
          (($parent == "" and (.fields.parent? == null)) or
           ($parent != "" and .fields.parent.id == $parent))
        ' "$RESPONSE_FILE" >/dev/null 2>&1 ||
        die "identity_mismatch logical=issue"
    fi
    [[ "$issue_id" =~ ^[0-9]+$ && "$issue_key" =~ ^${project_key}-[1-9][0-9]*$ ]] ||
      die "invalid_response operation=issue_create"
    JIRA_ISSUE_IDS[$id]=$issue_id
    JIRA_ISSUE_KEYS[$id]=$issue_key

    local comment_id comment_marker comment_body comment_id_value
    while IFS=$'\t' read -r comment_id comment_marker comment_body; do
      comment_body=$(decode_json_field "$comment_body")
      find_jira_comment "$issue_id" "$comment_marker"
      if [[ -n "$FOUND_ID" ]]; then
        comment_id_value=$FOUND_ID
        api_request provisioner GET "/rest/api/3/issue/${issue_id}/comment/${comment_id_value}" '' 200 comment_get
        if ! jq -e --arg id "$comment_id_value" --argjson body "$comment_body" \
          '.id == $id and .body == $body' "$RESPONSE_FILE" >/dev/null 2>&1; then
          delete_if_owned provisioner DELETE "/rest/api/3/issue/${issue_id}/comment/${comment_id_value}" jira_comment_delete
          comment_id_value=''
        fi
      else
        comment_id_value=''
      fi
      if [[ -z "$comment_id_value" ]]; then
        body=$(jq -cn --argjson body "$comment_body" '{body:$body}')
        api_request provisioner POST "/rest/api/3/issue/${issue_id}/comment" "$body" 201 comment_create
        comment_id_value=$(response_id '.id' comment_create)
        api_request provisioner GET "/rest/api/3/issue/${issue_id}/comment/${comment_id_value}" '' 200 comment_get
        jq -e --arg id "$comment_id_value" --argjson body "$comment_body" '
          .id == $id and .body == $body
        ' "$RESPONSE_FILE" >/dev/null 2>&1 ||
          die "identity_mismatch logical=jira_comment"
      fi
      [[ "$comment_id_value" =~ ^[0-9]+$ ]] ||
        die "invalid_response operation=comment_create"
      JIRA_COMMENT_IDS[$comment_id]=$comment_id_value
    done < <(jq -r --arg id "$id" '.jira.issues[] | select(.id==$id) | (.comments // [])[]? | [.id,.marker,(.body|tojson|@base64)] | @tsv' "$MANIFEST_PATH")
  done < <(jq -r '.jira.issues[] | [.id,.project,.summary,.marker,(.description|tojson|@base64),(.parent // "null")] | @tsv' "$MANIFEST_PATH")
}

bootstrap_confluence_spaces() {
  local id key name marker private body path space_id homepage
  homepage_validate='
    if (.homepageId|type) == "number" and .homepageId > 0 then (.homepageId|tostring)
    elif (.homepageId|type) == "string" and (.homepageId|test("^[0-9]+$")) then .homepageId
    else error end'
  while IFS=$'\t' read -r id key name marker private; do
    find_confluence_space "$key" "$marker"
    space_id=$FOUND_ID
    if [[ -n "$space_id" ]]; then
      api_request provisioner GET "/wiki/api/v2/spaces/${space_id}?description-format=plain" '' 200 space_get
      if ! jq -e --arg id "$space_id" --arg key "$key" --arg name "$name" --arg marker "$marker" '
        .id == $id and .key == $key and .name == $name and
        ((if (.description|type) == "string" then .description
          elif (.description|type) == "object" and (.description.value?|type) == "string" then .description.value
          elif (.description|type) == "object" and (.description.plain?|type) == "object"
            and (.description.plain.value?|type) == "string" then .description.plain.value
          else empty end) == $marker)
      ' "$RESPONSE_FILE" >/dev/null 2>&1; then
        delete_if_owned provisioner DELETE "/wiki/rest/api/space/${key}" space_delete
        space_id=''
      else
        homepage=$(response_string "$homepage_validate" space_get)
        CONF_SPACE_HOMEPAGE[$id]=$homepage
      fi
    fi
    if [[ -z "$space_id" ]]; then
      if [[ "$private" == true ]]; then
        path='/wiki/rest/api/space/_private'
      else
        path='/wiki/rest/api/space'
      fi
      body=$(jq -cn --arg key "$key" --arg name "$name" --arg marker "$marker" \
        '{key:$key,name:$name,description:{plain:{representation:"plain",value:$marker}}}')
      api_request provisioner POST "$path" "$body" 200,201 space_create
      space_id=$(response_id '.id' space_create)
      api_request provisioner GET "/wiki/api/v2/spaces/${space_id}?description-format=plain" '' 200 space_get
      jq -e --arg id "$space_id" --arg key "$key" --arg name "$name" --arg marker "$marker" '
        .id == $id and .key == $key and .name == $name and
        ((if (.description|type) == "string" then .description
          elif (.description|type) == "object" and (.description.value?|type) == "string" then .description.value
          elif (.description|type) == "object" and (.description.plain?|type) == "object"
            and (.description.plain.value?|type) == "string" then .description.plain.value
          else empty end) == $marker)
      ' "$RESPONSE_FILE" >/dev/null 2>&1 ||
        die "identity_mismatch logical=space"
      homepage=$(response_string "$homepage_validate" space_get)
      CONF_SPACE_HOMEPAGE[$id]=$homepage
    fi
    [[ "$space_id" =~ ^[0-9]+$ ]] || die "invalid_response operation=space_create"
    CONF_SPACE_IDS[$id]=$space_id
  done < <(jq -r '.confluence.spaces[] | [.id,.key,.name,.marker,.private] | @tsv' "$MANIFEST_PATH")
}

bootstrap_confluence_pages() {
  local id space title marker parent body response_body page_id parent_id homepage
  while IFS=$'\t' read -r id space title marker parent response_body; do
    response_body=$(decode_json_field "$response_body")
    local space_id=${CONF_SPACE_IDS[$space]-}
    [[ -n "$space_id" ]] || die "manifest_parent_missing logical=page"
    homepage=${CONF_SPACE_HOMEPAGE[$space]-}
    [[ -n "$homepage" ]] || die "manifest_parent_missing logical=space"
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${CONF_PAGE_IDS[$parent]-}
      [[ -n "$parent_id" ]] || die "manifest_parent_missing logical=page"
    fi
    find_confluence_page "$space_id" "$title" "$marker"
    page_id=$FOUND_ID
    if [[ -n "$page_id" ]]; then
      api_request provisioner GET "/wiki/api/v2/pages/${page_id}?body-format=storage" '' 200 page_get
      if ! jq -e --arg id "$page_id" --arg sid "$space_id" --arg title "$title" \
        --arg parent "$parent_id" --arg homepage "$homepage" --argjson body "$response_body" '
          (.id|tostring) == $id and (.spaceId|tostring) == $sid and .title == $title and
          ((.body.storage? // .body?).value | '"$STORAGE_NORMALIZE"') == ($body.value | '"$STORAGE_NORMALIZE"') and
          (($parent == "" and ((.parentId? == null) or (.parentId|tostring) == $homepage)) or
           ($parent != "" and (.parentId|tostring) == $parent))
        ' "$RESPONSE_FILE" >/dev/null 2>&1; then
        delete_if_owned provisioner DELETE "/wiki/api/v2/pages/${page_id}" confluence_page_delete
        page_id=''
      fi
    fi
    if [[ -z "$page_id" ]]; then
      if [[ -z "$parent_id" ]]; then
        body=$(jq -cn --arg sid "$space_id" --arg title "$title" --argjson pagebody "$response_body" \
          '{spaceId:$sid,status:"current",title:$title,body:$pagebody}')
      else
        body=$(jq -cn --arg sid "$space_id" --arg title "$title" --argjson pagebody "$response_body" --arg pid "$parent_id" \
          '{spaceId:$sid,status:"current",title:$title,parentId:$pid,body:$pagebody}')
      fi
      write_pending_receipt page "$id" "$space_id" "$parent_id"
      api_request provisioner POST '/wiki/api/v2/pages' "$body" 200,201 page_create
      page_id=$(response_id '.id' page_create)
      update_pending_receipt_id "$page_id"
      recover_pending_receipt
      [[ "${CONF_PAGE_IDS[$id]-}" == "$page_id" ]] || die "identity_mismatch logical=page"
    fi
    [[ "$page_id" =~ ^[0-9]+$ ]] || die "invalid_response operation=page_create"
    CONF_PAGE_IDS[$id]=$page_id
  done < <(jq -r '.confluence.pages[] | [.id,.space,.title,.marker,(.parent // "null"),(.body|tojson|@base64)] | @tsv' "$MANIFEST_PATH")
}

bootstrap_confluence_comments() {
  local id page marker expected_body parent request_body page_id comment_id parent_id
  while IFS=$'\t' read -r id page marker expected_body parent; do
    expected_body=$(decode_json_field "$expected_body")
    page_id=${CONF_PAGE_IDS[$page]-}
    [[ -n "$page_id" ]] || die "manifest_parent_missing logical=comment"
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${CONF_COMMENT_IDS[$parent]-}
      [[ -n "$parent_id" ]] || die "manifest_parent_missing logical=comment"
    fi
    find_confluence_comment "$page_id" "$marker" "$parent_id"
    comment_id=$FOUND_ID
    if [[ -n "$comment_id" ]]; then
      api_request provisioner GET "/wiki/api/v2/footer-comments/${comment_id}?body-format=storage" '' 200 comment_get
      if ! jq -e --arg id "$comment_id" --arg page "$page_id" --arg parent "$parent_id" \
        --argjson body "$expected_body" '
          (.id|tostring) == $id and (.pageId|tostring) == $page and
          ((.body.storage? // .body?).value | '"$STORAGE_NORMALIZE"') == ($body.value | '"$STORAGE_NORMALIZE"') and
          (($parent == "" and (.parentCommentId? == null)) or
           ($parent != "" and (.parentCommentId|tostring) == $parent))
        ' "$RESPONSE_FILE" >/dev/null 2>&1; then
        delete_if_owned provisioner DELETE "/wiki/api/v2/footer-comments/${comment_id}" confluence_comment_delete
        comment_id=''
      fi
    fi
    if [[ -z "$comment_id" ]]; then
      if [[ -z "$parent_id" ]]; then
        request_body=$(jq -cn --arg pid "$page_id" --argjson body "$expected_body" '{pageId:$pid,body:$body}')
      else
        request_body=$(jq -cn --arg parent "$parent_id" --argjson body "$expected_body" \
          '{parentCommentId:$parent,body:$body}')
      fi
      write_pending_receipt comment "$id" "$page_id" "$parent_id"
      api_request provisioner POST '/wiki/api/v2/footer-comments' "$request_body" 200,201 comment_create
      comment_id=$(response_id '.id' comment_create)
      update_pending_receipt_id "$comment_id"
      recover_pending_receipt
      [[ "${CONF_COMMENT_IDS[$id]-}" == "$comment_id" ]] || die "identity_mismatch logical=confluence_comment"
    fi
    [[ "$comment_id" =~ ^[0-9]+$ ]] || die "invalid_response operation=comment_create"
    CONF_COMMENT_IDS[$id]=$comment_id
  done < <(jq -r '.confluence.comments[] | [.id,.page,.marker,(.body|tojson|@base64),(.parent // "null")] | @tsv' "$MANIFEST_PATH")
}

write_state() {
  local state_json tmp destination
  local projects="$RUN_DIR/state-projects" issues="$RUN_DIR/state-issues" jcomments="$RUN_DIR/state-jcomments"
  local spaces="$RUN_DIR/state-spaces" pages="$RUN_DIR/state-pages" comments="$RUN_DIR/state-comments"
  : > "$projects"; : > "$issues"; : > "$jcomments"; : > "$spaces"; : > "$pages"; : > "$comments"
  local id key
  while IFS=$'\t' read -r id key; do
    append_row "$projects" "$(jq -cn --arg logical "$id" --arg sid "${JIRA_PROJECT_IDS[$id]}" --arg key "$key" \
      '{logical_id:$logical,id:$sid,key:$key}')"
  done < <(jq -r '.jira.projects[] | [.id,.key] | @tsv' "$MANIFEST_PATH")
  while IFS= read -r id; do
    append_row "$issues" "$(jq -cn --arg logical "$id" --arg sid "${JIRA_ISSUE_IDS[$id]}" --arg key "${JIRA_ISSUE_KEYS[$id]}" \
      '{logical_id:$logical,id:$sid,key:$key,reference:("jira://atlassian-live/issues/"+$sid)}')"
  done < <(jq -r '.jira.issues[].id' "$MANIFEST_PATH")
  while IFS= read -r id; do
    local issue_logical
    issue_logical=$(jq -r --arg c "$id" '.jira.issues[] | select(any(.comments[]?; .id==$c)) | .id' "$MANIFEST_PATH")
    append_row "$jcomments" "$(jq -cn --arg logical "$id" --arg sid "${JIRA_COMMENT_IDS[$id]}" --arg issue "${JIRA_ISSUE_IDS[$issue_logical]}" \
      '{logical_id:$logical,id:$sid,issue_id:$issue}')"
  done < <(jq -r '.jira.issues[].comments[]?.id' "$MANIFEST_PATH")
  while IFS=$'\t' read -r id key; do
    append_row "$spaces" "$(jq -cn --arg logical "$id" --arg sid "${CONF_SPACE_IDS[$id]}" --arg key "$key" \
      '{logical_id:$logical,id:$sid,key:$key}')"
  done < <(jq -r '.confluence.spaces[] | [.id,.key] | @tsv' "$MANIFEST_PATH")
  while IFS= read -r id; do
    local sid=${CONF_PAGE_IDS[$id]}
    append_row "$pages" "$(jq -cn --arg logical "$id" --arg sid "$sid" \
      '{logical_id:$logical,id:$sid,reference:("confluence://atlassian-live/pages/"+$sid)}')"
  done < <(jq -r '.confluence.pages[].id' "$MANIFEST_PATH")
  while IFS=$'\t' read -r id page; do
    local sid=${CONF_COMMENT_IDS[$id]} page_sid=${CONF_PAGE_IDS[$page]}
    append_row "$comments" "$(jq -cn --arg logical "$id" --arg sid "$sid" --arg page "$page_sid" \
      '{logical_id:$logical,id:$sid,page_id:$page}')"
  done < <(jq -r '.confluence.comments[] | [.id,.page] | @tsv' "$MANIFEST_PATH")
  state_json=$(jq -n \
    --arg origin "$SITE" --arg site_id "atlassian-live" \
    --slurpfile projects <(jq -s . "$projects") --slurpfile issues <(jq -s . "$issues") --slurpfile jcomments <(jq -s . "$jcomments") \
    --slurpfile spaces <(jq -s . "$spaces") --slurpfile pages <(jq -s . "$pages") --slurpfile comments <(jq -s . "$comments") \
    '{version:1,site:{id:$site_id,origin:$origin},manifest:{logical_ids:{jira_projects:[$projects[0][].logical_id],jira_issues:[$issues[0][].logical_id],jira_comments:[$jcomments[0][].logical_id],confluence_spaces:[$spaces[0][].logical_id],confluence_pages:[$pages[0][].logical_id],confluence_comments:[$comments[0][].logical_id]}},jira:{projects:$projects[0],issues:$issues[0],comments:$jcomments[0]},confluence:{spaces:$spaces[0],pages:$pages[0],comments:$comments[0]}}' \
    2>/dev/null) || die "state_write"
  tmp="$RUN_DIR/state-commit"
  printf '%s\n' "$state_json" > "$tmp" || die "state_write"
  chmod 600 "$tmp" || die "state_write"
  destination=$STATE_PATH
  STATE_PATH=$tmp
  load_state
  STATE_PATH=$destination
  [[ ! -L "$STATE_PATH" ]] || die "state_symlink"
  [[ ! -d "$STATE_PATH" ]] || die "state_is_directory"
  mv -f -- "$tmp" "$STATE_PATH" || die "state_commit"
}

load_state() {
  [[ -L "$STATE_PATH" ]] && die "state_symlink"
  [[ -f "$STATE_PATH" ]] || die "state_missing"
  local size
  size=$(wc -c < "$STATE_PATH") || die "state_unreadable"
  (( size <= MAX_RESPONSE_BYTES )) || die "state_too_large"
  local mode
  mode=$(stat -c '%a' "$STATE_PATH" 2>/dev/null) || die "state_unreadable"
  [[ "$mode" == 600 ]] || die "state_permissions"
  jq -e --arg site "$SITE" --slurpfile source "$MANIFEST_PATH" '
    def logical: type == "string" and test("^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$");
    def digits: type == "string" and test("^[0-9]+$");
    def key: type == "string" and test("^[A-Z][A-Z0-9]{1,9}$");
    . as $state |
    ([keys[]] | sort) == ["confluence","jira","manifest","site","version"] and .version == 1 and
    (.site|keys|sort) == ["id","origin"] and .site.id == "atlassian-live" and .site.origin == $site and
    (.manifest|keys) == ["logical_ids"] and
    (.manifest.logical_ids|keys|sort) == [
      "confluence_comments","confluence_pages","confluence_spaces",
      "jira_comments","jira_issues","jira_projects"
    ] and all(.manifest.logical_ids[]; type == "array" and all(.[]; logical)) and
    (.jira|keys|sort) == ["comments","issues","projects"] and
    (.confluence|keys|sort) == ["comments","pages","spaces"] and
    all(.jira.projects[];
      ([keys[]]|sort) == ["id","key","logical_id"] and (.logical_id|logical) and (.id|digits) and (.key|key)) and
    all(.jira.issues[];
      ([keys[]]|sort) == ["id","key","logical_id","reference"] and (.logical_id|logical) and (.id|digits) and
      (.key|type) == "string" and (.key|test("^[A-Z][A-Z0-9]{1,9}-[1-9][0-9]*$")) and
      .reference == ("jira://atlassian-live/issues/" + .id)) and
    all(.jira.comments[];
      ([keys[]]|sort) == ["id","issue_id","logical_id"] and (.logical_id|logical) and
      (.id|digits) and (.issue_id|digits)) and
    all(.confluence.spaces[];
      ([keys[]]|sort) == ["id","key","logical_id"] and (.logical_id|logical) and (.id|digits) and (.key|key)) and
    all(.confluence.pages[];
      ([keys[]]|sort) == ["id","logical_id","reference"] and (.logical_id|logical) and (.id|digits) and
      .reference == ("confluence://atlassian-live/pages/" + .id)) and
    all(.confluence.comments[];
      ([keys[]]|sort) == ["id","logical_id","page_id"] and (.logical_id|logical) and
      (.id|digits) and (.page_id|digits)) and
    all(.jira.comments[]; .issue_id as $id | any($state.jira.issues[]; .id == $id)) and
    all(.confluence.comments[]; .page_id as $id | any($state.confluence.pages[]; .id == $id)) and
    ([.jira.projects[].id] | unique | length) == (.jira.projects|length) and
    ([.jira.issues[].id] | unique | length) == (.jira.issues|length) and
    ([.jira.comments[].id] | unique | length) == (.jira.comments|length) and
    ([.confluence.spaces[].id] | unique | length) == (.confluence.spaces|length) and
    ([.confluence.pages[].id] | unique | length) == (.confluence.pages|length) and
    ([.confluence.comments[].id] | unique | length) == (.confluence.comments|length) and
    ([.jira.projects[].logical_id] | unique | length) == (.jira.projects|length) and
    ([.jira.issues[].logical_id] | unique | length) == (.jira.issues|length) and
    ([.jira.comments[].logical_id] | unique | length) == (.jira.comments|length) and
    ([.confluence.spaces[].logical_id] | unique | length) == (.confluence.spaces|length) and
    ([.confluence.pages[].logical_id] | unique | length) == (.confluence.pages|length) and
    ([.confluence.comments[].logical_id] | unique | length) == (.confluence.comments|length) and
    (.manifest.logical_ids.jira_projects|sort) == ([$source[0].jira.projects[].id]|sort) and
    (.manifest.logical_ids.jira_issues|sort) == ([$source[0].jira.issues[].id]|sort) and
    (.manifest.logical_ids.jira_comments|sort) == ([$source[0].jira.issues[].comments[]?.id]|sort) and
    (.manifest.logical_ids.confluence_spaces|sort) == ([$source[0].confluence.spaces[].id]|sort) and
    (.manifest.logical_ids.confluence_pages|sort) == ([$source[0].confluence.pages[].id]|sort) and
    (.manifest.logical_ids.confluence_comments|sort) == ([$source[0].confluence.comments[].id]|sort) and
    ([.jira.projects[].logical_id]|sort) == (.manifest.logical_ids.jira_projects|sort) and
    ([.jira.issues[].logical_id]|sort) == (.manifest.logical_ids.jira_issues|sort) and
    ([.jira.comments[].logical_id]|sort) == (.manifest.logical_ids.jira_comments|sort) and
    ([.confluence.spaces[].logical_id]|sort) == (.manifest.logical_ids.confluence_spaces|sort) and
    ([.confluence.pages[].logical_id]|sort) == (.manifest.logical_ids.confluence_pages|sort) and
    ([.confluence.comments[].logical_id]|sort) == (.manifest.logical_ids.confluence_comments|sort)
  ' "$STATE_PATH" >/dev/null 2>&1 || die "state_corrupt"
  while IFS=$'\t' read -r id sid key; do JIRA_PROJECT_IDS[$id]=$sid; JIRA_PROJECT_KEYS[$id]=$key; done < <(jq -r '.jira.projects[]? | [.logical_id,.id,.key] | @tsv' "$STATE_PATH")
  while IFS=$'\t' read -r id sid key; do JIRA_ISSUE_IDS[$id]=$sid; JIRA_ISSUE_KEYS[$id]=$key; done < <(jq -r '.jira.issues[]? | [.logical_id,.id,.key] | @tsv' "$STATE_PATH")
  while IFS=$'\t' read -r id sid; do JIRA_COMMENT_IDS[$id]=$sid; done < <(jq -r '.jira.comments[]? | [.logical_id,.id] | @tsv' "$STATE_PATH")
  while IFS=$'\t' read -r id sid; do CONF_SPACE_IDS[$id]=$sid; done < <(jq -r '.confluence.spaces[]? | [.logical_id,.id] | @tsv' "$STATE_PATH")
  while IFS=$'\t' read -r id sid; do CONF_PAGE_IDS[$id]=$sid; done < <(jq -r '.confluence.pages[]? | [.logical_id,.id] | @tsv' "$STATE_PATH")
  while IFS=$'\t' read -r id sid; do CONF_COMMENT_IDS[$id]=$sid; done < <(jq -r '.confluence.comments[]? | [.logical_id,.id] | @tsv' "$STATE_PATH")
}
verify_jira() {
  local id key name marker issue_id summary description project parent parent_id comment_id comment_body issue_logical
  while IFS=$'\t' read -r id key name marker; do
    api_request provisioner GET "/rest/api/3/project/${key}" '' 200 project_get
    jq -e --arg expected_id "${JIRA_PROJECT_IDS[$id]-}" --arg expected_key "$key" \
      --arg expected_name "$name" --arg expected_marker "$marker" '
        .id == $expected_id and .key == $expected_key and
        .name == $expected_name and .description == $expected_marker
      ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "authority_mismatch logical=project"
  done < <(jq -r '.jira.projects[] | [.id,.key,.name,.marker] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r id summary description project parent; do
    description=$(decode_json_field "$description")
    issue_id=${JIRA_ISSUE_IDS[$id]-}
    [[ -n "$issue_id" ]] || die "state_missing_object logical=issue"
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${JIRA_ISSUE_IDS[$parent]-}
    fi
    api_request provisioner GET "/rest/api/3/issue/${issue_id}?fields=summary,description,project,parent" '' 200 issue_get
    jq -e --arg expected_id "$issue_id" --arg expected_key "${JIRA_PROJECT_KEYS[$project]-}" \
      --arg expected_summary "$summary" --arg expected_parent "$parent_id" --argjson expected_description "$description" '
        .id == $expected_id and .fields.project.key == $expected_key and
        .fields.summary == $expected_summary and .fields.description == $expected_description and
        (($expected_parent == "" and (.fields.parent? == null)) or
         ($expected_parent != "" and .fields.parent.id == $expected_parent))
      ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "authority_mismatch logical=issue"
  done < <(jq -r '.jira.issues[] | [.id,.summary,(.description|tojson|@base64),.project,(.parent // "null")] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r comment_id issue_logical comment_body; do
    comment_body=$(decode_json_field "$comment_body")
    issue_id=${JIRA_ISSUE_IDS[$issue_logical]-}
    api_request provisioner GET "/rest/api/3/issue/${issue_id}/comment/${JIRA_COMMENT_IDS[$comment_id]-}" '' 200 comment_get
    jq -e --arg expected_id "${JIRA_COMMENT_IDS[$comment_id]-}" --argjson expected_body "$comment_body" '
      .id == $expected_id and .body == $expected_body
    ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "authority_mismatch logical=comment"
  done < <(jq -r '.jira.issues[] as $issue | ($issue.comments // [])[]? | [.id,$issue.id,(.body|tojson|@base64)] | @tsv' "$MANIFEST_PATH")
}

verify_jira_pagination() {
  local id key marker project issue_id comment_id issue_logical
  while IFS=$'\t' read -r id key marker; do
    find_jira_project "$key" "$marker"
    [[ "$FOUND_ID" == "${JIRA_PROJECT_IDS[$id]-}" ]] ||
      die "pagination_authority logical=project"
  done < <(jq -r '.jira.projects[] | [.id,.key,.marker] | @tsv' "$MANIFEST_PATH")
  while IFS=$'\t' read -r id project marker; do
    find_jira_issue "${JIRA_PROJECT_KEYS[$project]-}" "$marker"
    [[ "$FOUND_ID" == "${JIRA_ISSUE_IDS[$id]-}" && "$FOUND_KEY" == "${JIRA_ISSUE_KEYS[$id]-}" ]] ||
      die "pagination_authority logical=issue"
  done < <(jq -r '.jira.issues[] | [.id,.project,.marker] | @tsv' "$MANIFEST_PATH")
  while IFS=$'\t' read -r comment_id issue_logical marker; do
    issue_id=${JIRA_ISSUE_IDS[$issue_logical]-}
    find_jira_comment "$issue_id" "$marker"
    [[ "$FOUND_ID" == "${JIRA_COMMENT_IDS[$comment_id]-}" ]] ||
      die "pagination_authority logical=comment"
  done < <(jq -r '.jira.issues[] as $issue | ($issue.comments // [])[]? | [.id,$issue.id,.marker] | @tsv' "$MANIFEST_PATH")
}

verify_confluence() {
  local id key name marker sid page_id title space parent expected parent_id comment_id page marker_body homepage
  homepage_validate='
    if (.homepageId|type) == "number" and .homepageId > 0 then (.homepageId|tostring)
    elif (.homepageId|type) == "string" and (.homepageId|test("^[0-9]+$")) then .homepageId
    else error end'
  while IFS=$'\t' read -r id key name marker; do
    sid=${CONF_SPACE_IDS[$id]-}
    api_request provisioner GET "/wiki/api/v2/spaces/${sid}?description-format=plain" '' 200 space_get
    jq -e --arg expected_id "$sid" --arg expected_key "$key" --arg expected_name "$name" --arg expected_marker "$marker" '
      (.id|tostring) == $expected_id and .key == $expected_key and .name == $expected_name and
      ((if (.description|type) == "string" then .description
        elif (.description|type) == "object" and (.description.value?|type) == "string" then .description.value
        elif (.description|type) == "object" and (.description.plain?|type) == "object"
          and (.description.plain.value?|type) == "string" then .description.plain.value
        else empty end) == $expected_marker)
    ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "authority_mismatch logical=space"
    homepage=$(response_string "$homepage_validate" space_get)
    CONF_SPACE_HOMEPAGE[$id]=$homepage
  done < <(jq -r '.confluence.spaces[] | [.id,.key,.name,.marker] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r id title space parent expected; do
    expected=$(decode_json_field "$expected")
    page_id=${CONF_PAGE_IDS[$id]-}
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${CONF_PAGE_IDS[$parent]-}
    fi
    homepage=${CONF_SPACE_HOMEPAGE[$space]-}
    [[ -n "$homepage" ]] || die "state_missing_object logical=space"
    api_request provisioner GET "/wiki/api/v2/pages/${page_id}?body-format=storage" '' 200 page_get
    jq -e --arg expected_id "$page_id" --arg expected_space "${CONF_SPACE_IDS[$space]-}" \
      --arg expected_title "$title" --arg expected_parent "$parent_id" --arg homepage "$homepage" \
      --argjson expected_body "$expected" '
        (.id|tostring) == $expected_id and (.spaceId|tostring) == $expected_space and .title == $expected_title and
        ((.body.storage? // .body?).value | '"$STORAGE_NORMALIZE"') == ($expected_body.value | '"$STORAGE_NORMALIZE"') and
        (($expected_parent == "" and ((.parentId? == null) or (.parentId|tostring) == $homepage)) or
         ($expected_parent != "" and (.parentId|tostring) == $expected_parent))
      ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "authority_mismatch logical=page"
    read_owner_property provisioner "$page_id"
    [[ "$OWNER_PROPERTY_VALUE" == "$(jq -r --arg id "$id" '.confluence.pages[] | select(.id==$id) | .marker' "$MANIFEST_PATH")" ]] ||
      die "authority_mismatch logical=page"
  done < <(jq -r '.confluence.pages[] | [.id,.title,.space,(.parent // "null"),(.body|tojson|@base64)] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r id page parent expected; do
    expected=$(decode_json_field "$expected")
    comment_id=${CONF_COMMENT_IDS[$id]-}
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${CONF_COMMENT_IDS[$parent]-}
    fi
    api_request provisioner GET "/wiki/api/v2/footer-comments/${comment_id}?body-format=storage" '' 200 comment_get
    jq -e --arg expected_id "$comment_id" --arg expected_page "${CONF_PAGE_IDS[$page]-}" \
      --arg expected_parent "$parent_id" --argjson expected_body "$expected" '
        (.id|tostring) == $expected_id and (.pageId|tostring) == $expected_page and
        ((.body.storage? // .body?).value | '"$STORAGE_NORMALIZE"') == ($expected_body.value | '"$STORAGE_NORMALIZE"') and
        (($expected_parent == "" and (.parentCommentId? == null)) or
         ($expected_parent != "" and (.parentCommentId|tostring) == $expected_parent))
      ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "authority_mismatch logical=comment"
    read_owner_property provisioner "$comment_id"
    [[ "$OWNER_PROPERTY_VALUE" == "$(jq -r --arg id "$id" '.confluence.comments[] | select(.id==$id) | .marker' "$MANIFEST_PATH")" ]] ||
      die "authority_mismatch logical=comment"
  done < <(jq -r '.confluence.comments[] | [.id,.page,(.parent // "null"),(.body|tojson|@base64)] | @tsv' "$MANIFEST_PATH")
}

verify_confluence_pagination() {
  local id key marker space title page parent parent_id
  while IFS=$'\t' read -r id key marker; do
    find_confluence_space "$key" "$marker"
    [[ "$FOUND_ID" == "${CONF_SPACE_IDS[$id]-}" ]] ||
      die "pagination_authority logical=space"
  done < <(jq -r '.confluence.spaces[] | [.id,.key,.marker] | @tsv' "$MANIFEST_PATH")
  while IFS=$'\t' read -r id space title marker; do
    find_confluence_page "${CONF_SPACE_IDS[$space]-}" "$title" "$marker"
    [[ "$FOUND_ID" == "${CONF_PAGE_IDS[$id]-}" ]] ||
      die "pagination_authority logical=page"
  done < <(jq -r '.confluence.pages[] | [.id,.space,.title,.marker] | @tsv' "$MANIFEST_PATH")
  while IFS=$'\t' read -r id page marker parent; do
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${CONF_COMMENT_IDS[$parent]-}
      [[ -n "$parent_id" ]] || die "state_missing_object logical=comment"
    fi
    find_confluence_comment "${CONF_PAGE_IDS[$page]-}" "$marker" "$parent_id"
    [[ "$FOUND_ID" == "${CONF_COMMENT_IDS[$id]-}" ]] ||
      die "pagination_authority logical=comment"
  done < <(jq -r '.confluence.comments[] | [.id,.page,.marker,(.parent // "null")] | @tsv' "$MANIFEST_PATH")
}

verify_reader_visibility() {
  local id key name marker private path next pages=0 row row_key page_id space page
  local comment_id issue_id issue body title parent expected parent_id summary description project homepage
  declare -A seen_public_spaces=()
  path='/wiki/api/v2/spaces?limit=1&description-format=plain'
  while :; do
    (( pages < MAX_PAGES )) || die "pagination_limit product=confluence operation=reader_space_list"
    api_request reader GET "$path" '' 200 reader_space_list
    jq -e '.results | type == "array"' "$RESPONSE_FILE" >/dev/null 2>&1 ||
      die "invalid_response operation=reader_space_list"
    while IFS= read -r row; do
      row_key=$(jq -r '.key // empty | strings' <<<"$row")
      [[ -n "$row_key" ]] || die "invalid_response operation=reader_space_list"
      if jq -e --arg key "$row_key" '.confluence.spaces[] | select(.key == $key and .private == true)' "$MANIFEST_PATH" >/dev/null 2>&1; then
        die "reader_visibility logical=space"
      fi
      if jq -e --arg key "$row_key" '.confluence.spaces[] | select(.key == $key and .private == false)' "$MANIFEST_PATH" >/dev/null 2>&1; then
        seen_public_spaces[$row_key]=1
      fi
    done < <(jq -c '.results[]' "$RESPONSE_FILE")
    next=$(jq -r 'if ._links.next == null then "" elif (._links.next|type) == "string" then ._links.next else error end' "$RESPONSE_FILE" 2>/dev/null) ||
      die "invalid_pagination product=confluence operation=reader_space_list"
    [[ -n "$next" ]] || break
    [[ "$next" == /wiki/api/v2/spaces\?* && "$next" != "$path" ]] ||
      die "invalid_pagination product=confluence operation=reader_space_list"
    path=$next
    ((pages += 1))
  done

  while IFS=$'\t' read -r id key name marker private; do
    if [[ "$private" == true ]]; then
      api_request reader GET "/wiki/api/v2/spaces/${CONF_SPACE_IDS[$id]}?description-format=plain" '' 403,404 reader_private_space_get
    else
      [[ "${seen_public_spaces[$key]-}" == 1 ]] || die "reader_missing logical=space"
      api_request reader GET "/wiki/api/v2/spaces/${CONF_SPACE_IDS[$id]}?description-format=plain" '' 200 reader_space_get
      jq -e --arg expected_id "${CONF_SPACE_IDS[$id]}" --arg expected_key "$key" \
        --arg expected_name "$name" --arg expected_marker "$marker" '
          (.id|tostring) == $expected_id and .key == $expected_key and .name == $expected_name and
          ((if (.description|type) == "string" then .description
            elif (.description|type) == "object" and (.description.value?|type) == "string" then .description.value
            elif (.description|type) == "object" and (.description.plain?|type) == "object"
              and (.description.plain.value?|type) == "string" then .description.plain.value
            else empty end) == $expected_marker)
        ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "reader_missing logical=space"
      homepage=$(jq -er '
        if (.homepageId|type) == "number" and .homepageId > 0 then (.homepageId|tostring)
        elif (.homepageId|type) == "string" and (.homepageId|test("^[0-9]+$")) then .homepageId
        else error end' "$RESPONSE_FILE" 2>/dev/null) || die "invalid_response operation=reader_space_get"
      CONF_SPACE_HOMEPAGE[$id]=$homepage
    fi
  done < <(jq -r '.confluence.spaces[] | [.id,.key,.name,.marker,.private] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r id space title parent expected; do
    expected=$(decode_json_field "$expected")
    page_id=${CONF_PAGE_IDS[$id]-}
    private=$(jq -r --arg id "$space" '.confluence.spaces[] | select(.id==$id) | .private' "$MANIFEST_PATH")
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${CONF_PAGE_IDS[$parent]-}
    fi
    if [[ "$private" == true ]]; then
      api_request reader GET "/wiki/api/v2/pages/${page_id}?body-format=storage" '' 403,404 reader_private_page_get
    else
      homepage=${CONF_SPACE_HOMEPAGE[$space]-}
      [[ -n "$homepage" ]] || die "reader_missing logical=space"
      api_request reader GET "/wiki/api/v2/pages/${page_id}?body-format=storage" '' 200 reader_page_get
      jq -e --arg expected_id "$page_id" --arg expected_space "${CONF_SPACE_IDS[$space]-}" \
        --arg expected_title "$title" --arg expected_parent "$parent_id" --arg homepage "$homepage" \
        --argjson expected_body "$expected" '
          (.id|tostring) == $expected_id and (.spaceId|tostring) == $expected_space and
          .title == $expected_title and
          ((.body.storage? // .body?).value | '"$STORAGE_NORMALIZE"') == ($expected_body.value | '"$STORAGE_NORMALIZE"') and
          (($expected_parent == "" and ((.parentId? == null) or (.parentId|tostring) == $homepage)) or
           ($expected_parent != "" and (.parentId|tostring) == $expected_parent))
        ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "reader_missing logical=page"
    fi
  done < <(jq -r '.confluence.pages[] | [.id,.space,.title,(.parent // "null"),(.body|tojson|@base64)] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r id page parent body; do
    body=$(decode_json_field "$body")
    comment_id=${CONF_COMMENT_IDS[$id]-}
    space=$(jq -r --arg page "$page" '.confluence.pages[] | select(.id==$page) | .space' "$MANIFEST_PATH")
    private=$(jq -r --arg id "$space" '.confluence.spaces[] | select(.id==$id) | .private' "$MANIFEST_PATH")
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${CONF_COMMENT_IDS[$parent]-}
    fi
    if [[ "$private" == true ]]; then
      api_request reader GET "/wiki/api/v2/footer-comments/${comment_id}?body-format=storage" '' 403,404 reader_private_comment_get
    else
      api_request reader GET "/wiki/api/v2/footer-comments/${comment_id}?body-format=storage" '' 200 reader_comment_get
      jq -e --arg expected_id "$comment_id" --arg expected_page "${CONF_PAGE_IDS[$page]-}" \
        --arg expected_parent "$parent_id" --argjson expected_body "$body" '
          (.id|tostring) == $expected_id and (.pageId|tostring) == $expected_page and
          ((.body.storage? // .body?).value | '"$STORAGE_NORMALIZE"') == ($expected_body.value | '"$STORAGE_NORMALIZE"') and
          (($expected_parent == "" and (.parentCommentId? == null)) or
           ($expected_parent != "" and (.parentCommentId|tostring) == $expected_parent))
        ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "reader_missing logical=comment"
    fi
  done < <(jq -r '.confluence.comments[] | [.id,.page,(.parent // "null"),(.body|tojson|@base64)] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r id key name marker; do
    api_request reader GET "/rest/api/3/project/${key}" '' 200 reader_project_get
    jq -e --arg expected_id "${JIRA_PROJECT_IDS[$id]-}" --arg expected_key "$key" \
      --arg expected_name "$name" --arg expected_marker "$marker" '
        (.id|tostring) == $expected_id and .key == $expected_key and
        .name == $expected_name and .description == $expected_marker
      ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "reader_missing logical=project"
  done < <(jq -r '.jira.projects[] | [.id,.key,.name,.marker] | @tsv' "$MANIFEST_PATH")
  while IFS=$'\t' read -r id summary description project parent; do
    description=$(decode_json_field "$description")
    issue_id=${JIRA_ISSUE_IDS[$id]-}
    parent_id=''
    if [[ "$parent" != null && -n "$parent" ]]; then
      parent_id=${JIRA_ISSUE_IDS[$parent]-}
    fi
    api_request reader GET "/rest/api/3/issue/${issue_id}?fields=summary,description,project,parent" '' 200 reader_issue_get
    jq -e --arg expected_id "$issue_id" --arg expected_project "${JIRA_PROJECT_KEYS[$project]-}" \
      --arg expected_summary "$summary" --arg expected_parent "$parent_id" --argjson expected_description "$description" '
        (.id|tostring) == $expected_id and .fields.project.key == $expected_project and
        .fields.summary == $expected_summary and .fields.description == $expected_description and
        (($expected_parent == "" and (.fields.parent? == null)) or
         ($expected_parent != "" and (.fields.parent.id|tostring) == $expected_parent))
      ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "reader_missing logical=issue"
  done < <(jq -r '.jira.issues[] | [.id,.summary,(.description|tojson|@base64),.project,(.parent // "null")] | @tsv' "$MANIFEST_PATH")
  while IFS=$'\t' read -r id issue body; do
    body=$(decode_json_field "$body")
    issue_id=${JIRA_ISSUE_IDS[$issue]-}
    api_request reader GET "/rest/api/3/issue/${issue_id}/comment/${JIRA_COMMENT_IDS[$id]-}" '' 200 reader_jira_comment_get
    jq -e --arg expected_id "${JIRA_COMMENT_IDS[$id]-}" --argjson expected_body "$body" '
      .id == $expected_id and .body == $expected_body
    ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "reader_missing logical=comment"
  done < <(jq -r '.jira.issues[] as $issue | ($issue.comments // [])[]? | [.id,$issue.id,(.body|tojson|@base64)] | @tsv' "$MANIFEST_PATH")
}

verify_all() {
  ensure_no_pending_receipt
  load_state
  verify_jira
  verify_jira_pagination
  verify_confluence
  verify_confluence_pagination
  verify_reader_visibility
  printf 'verified site=%s\n' "$SITE"
}
cleanup_target_ids_for() {
  local key=$1
  CLEANUP_IDS=()
  [[ -n "${CLEANUP_TARGET_IDS[$key]-}" ]] || return 0
  local IFS=' '
  read -r -a CLEANUP_IDS <<< "${CLEANUP_TARGET_IDS[$key]}"
}

read_cleanup_jira_project() {
  local id=$1 operation=$2 normalized
  api_request provisioner GET "/rest/api/3/project/${id}" '' 200,404 "$operation"
  [[ "$RESPONSE_STATUS" == 404 ]] || return 0
  # Direct GET hides trash, but a permanent DELETE can still remove it.
  # Revalidate that hidden object before deleting, and include trash in absence proof.
  api_request provisioner GET "/rest/api/3/project/search?id=${id}&status=deleted&maxResults=1&expand=description" '' 200 "${operation}_trash"
  jq -e --arg id "$id" '
    (.values|type) == "array" and (.values|length) <= 1 and .isLast == true and
    all(.values[]; (.id|tostring) == $id)
  ' "$RESPONSE_FILE" >/dev/null 2>&1 || die "invalid_response operation=$operation"
  if jq -e '.values|length == 0' "$RESPONSE_FILE" >/dev/null; then
    RESPONSE_STATUS=404
    return 0
  fi
  normalized="$RUN_DIR/project-${REQUESTS}"
  jq '.values[0]' "$RESPONSE_FILE" >"$normalized" || die "invalid_response operation=$operation"
  RESPONSE_FILE=$normalized
}

cleanup_revalidate_targets() {
  local logical sid marker issue_logical issue_id page_logical page_id parent_id current object_response
  local -a target_ids=()
  while IFS= read -r logical; do
    marker=$(jq -r --arg id "$logical" '.jira.projects[] | select(.id==$id) | .marker' "$MANIFEST_PATH")
    cleanup_target_ids_for "jira_project:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      read_cleanup_jira_project "$sid" cleanup_project_get
      [[ "$RESPONSE_STATUS" == 404 ]] && continue
      jq -e --arg id "$sid" --arg marker "$marker" '
        (.id|tostring) == $id and .description == $marker
      ' "$RESPONSE_FILE" >/dev/null 2>&1 ||
        die "foreign_collision logical=project"
      current=$(jq -er '.key | strings' "$RESPONSE_FILE" 2>/dev/null) ||
        die "invalid_response operation=cleanup_project_get"
      JIRA_PROJECT_KEYS[$logical]=$current
    done
  done < <(jq -r '.jira.projects[].id' "$MANIFEST_PATH")

  while IFS= read -r logical; do
    marker=$(jq -r --arg id "$logical" '.jira.issues[] | select(.id==$id) | .marker' "$MANIFEST_PATH")
    cleanup_target_ids_for "jira_issue:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/rest/api/3/issue/${sid}?fields=summary,description,project,parent" '' 200,404 cleanup_issue_get
      [[ "$RESPONSE_STATUS" == 404 ]] && continue
      jq -e --arg id "$sid" --arg marker "$marker" '
        (.id|tostring) == $id and
        ([.fields.description? | .. | objects | select(.type? == "text") | .text? | strings] | any(. == $marker))
      ' "$RESPONSE_FILE" >/dev/null 2>&1 ||
        die "foreign_collision logical=issue"
      current=$(jq -er '.fields.project.key | strings' "$RESPONSE_FILE" 2>/dev/null) ||
        die "invalid_response operation=cleanup_issue_get"
      JIRA_ISSUE_SEARCH_PROJECT[$logical]=$current
    done
  done < <(jq -r '.jira.issues[].id' "$MANIFEST_PATH")

  while IFS=$'\t' read -r logical issue_logical; do
    marker=$(jq -r --arg id "$logical" '.jira.issues[]?.comments[]? | select(.id==$id) | .marker' "$MANIFEST_PATH")
    issue_id=${CLEANUP_JIRA_COMMENT_ISSUE[$logical]-${JIRA_ISSUE_IDS[$issue_logical]-}}
    [[ -n "$issue_id" ]] || continue
    cleanup_target_ids_for "jira_comment:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/rest/api/3/issue/${issue_id}/comment/${sid}" '' 200,404 cleanup_comment_get
      [[ "$RESPONSE_STATUS" == 404 ]] && continue
      jq -e --arg id "$sid" --arg marker "$marker" '
        (.id|tostring) == $id and
        ([.body? | .. | objects | select(.type? == "text") | .text? | strings] | any(. == $marker))
      ' "$RESPONSE_FILE" >/dev/null 2>&1 ||
        die "foreign_collision logical=jira_comment"
    done
  done < <(jq -r '.jira.issues[] as $issue | ($issue.comments // [])[]? | [.id,$issue.id] | @tsv' "$MANIFEST_PATH")

  while IFS= read -r logical; do
    marker=$(jq -r --arg id "$logical" '.confluence.spaces[] | select(.id==$id) | .marker' "$MANIFEST_PATH")
    cleanup_target_ids_for "conf_space:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/wiki/api/v2/spaces/${sid}?description-format=plain" '' 200,404 cleanup_space_get
      [[ "$RESPONSE_STATUS" == 404 ]] && continue
      jq -e --arg id "$sid" --arg marker "$marker" '
        (.id|tostring) == $id and
        ((if (.description|type) == "string" then .description
          elif (.description|type) == "object" and (.description.value?|type) == "string" then .description.value
          elif (.description|type) == "object" and (.description.plain?|type) == "object"
            and (.description.plain.value?|type) == "string" then .description.plain.value
          else empty end) == $marker)
      ' "$RESPONSE_FILE" >/dev/null 2>&1 ||
        die "foreign_collision logical=space"
      current=$(jq -er '.key | strings' "$RESPONSE_FILE" 2>/dev/null) ||
        die "invalid_response operation=cleanup_space_get"
      CONF_SPACE_KEYS[$logical]=$current
      CLEANUP_CONF_SPACE_KEY["${logical}:${sid}"]=$current
    done
  done < <(jq -r '.confluence.spaces[].id' "$MANIFEST_PATH")

  while IFS= read -r logical; do
    marker=$(jq -r --arg id "$logical" '.confluence.pages[] | select(.id==$id) | .marker' "$MANIFEST_PATH")
    cleanup_target_ids_for "conf_page:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/wiki/api/v2/pages/${sid}?body-format=storage" '' 200,404 cleanup_page_get
      [[ "$RESPONSE_STATUS" == 404 ]] && continue
      object_response=$RESPONSE_FILE
      jq -e --arg id "$sid" '(.id|tostring) == $id' "$object_response" >/dev/null 2>&1 ||
        die "foreign_collision logical=page"
      read_owner_property provisioner "$sid"
      [[ "$OWNER_PROPERTY_VALUE" == "$marker" ]] ||
        die "foreign_collision logical=page"
      current=$(jq -er '.spaceId | tostring' "$object_response" 2>/dev/null) ||
        die "invalid_response operation=cleanup_page_get"
      CONF_PAGE_SEARCH_SPACE[$logical]=$current
    done
  done < <(jq -r '.confluence.pages[].id' "$MANIFEST_PATH")

  while IFS=$'\t' read -r logical page_logical parent; do
    marker=$(jq -r --arg id "$logical" '.confluence.comments[] | select(.id==$id) | .marker' "$MANIFEST_PATH")
    page_id=${CLEANUP_CONF_COMMENT_PAGE[$logical]-${CONF_PAGE_IDS[$page_logical]-}}
    [[ -n "$page_id" ]] || continue
    cleanup_target_ids_for "conf_comment:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/wiki/api/v2/footer-comments/${sid}?body-format=storage" '' 200,404 cleanup_comment_get
      [[ "$RESPONSE_STATUS" == 404 ]] && continue
      object_response=$RESPONSE_FILE
      jq -e --arg id "$sid" '(.id|tostring) == $id' "$object_response" >/dev/null 2>&1 ||
        die "foreign_collision logical=confluence_comment"
      read_owner_property provisioner "$sid"
      [[ "$OWNER_PROPERTY_VALUE" == "$marker" ]] ||
        die "foreign_collision logical=confluence_comment"
      current=$(jq -er '.pageId | tostring' "$object_response" 2>/dev/null) ||
        die "invalid_response operation=cleanup_comment_get"
      CONF_COMMENT_SEARCH_PAGE[$logical]=$current
      current=$(jq -r 'if (.parentCommentId|type) == "number" then (.parentCommentId|tostring) elif (.parentCommentId|type) == "string" then .parentCommentId else "" end' "$object_response")
      CONF_COMMENT_SEARCH_PARENT[$logical]=$current
    done
  done < <(jq -r '.confluence.comments[] | [.id,.page,(.parent // "null")] | @tsv' "$MANIFEST_PATH")
}


poll_space_delete() {
  local task_id=$1 status='' attempt=0
  while (( attempt < MAX_POLLS )); do
    api_request provisioner GET "/wiki/rest/api/longtask/${task_id}" '' 200 longtask_status
    status=$(jq -er '(.status // .state) | strings' "$RESPONSE_FILE" 2>/dev/null) ||
      die "invalid_response operation=space_delete_poll"
    case "$status" in
      # Live tenants report terminal success as FINISH_SUCCESS.
      COMPLETE|completed|SUCCESS|success|FINISH_SUCCESS|finish_success) return ;;
      FAILED|failed|ERROR|error|FINISH_ERROR|finish_error|FINISH_FAILED|finish_failed|FINISH_CANCELLED|finish_cancelled)
        die "delete_failure product=confluence operation=space_delete_poll"
        ;;
      RUNNING|running|PENDING|pending|IN_PROGRESS|in_progress) ;;
      *) die "invalid_response operation=space_delete_poll" ;;
    esac
    ((attempt += 1))
    (( attempt < MAX_POLLS )) && sleep "$POLL_SECONDS"
  done
  die "delete_timeout product=confluence operation=space_delete_poll"
}

delete_if_owned() {
  local actor=$1 method=$2 path=$3 operation=$4 expected=${5:-'204,404'}
  if [[ "$operation" == space_delete ]]; then
    api_request "$actor" "$method" "$path" '' '202,404' "$operation"
    if [[ "$RESPONSE_STATUS" == 202 ]]; then
      local task
      task=$(jq -er '.id | strings' "$RESPONSE_FILE" 2>/dev/null) ||
        die "invalid_response operation=space_delete"
      [[ "$task" =~ ^[A-Za-z0-9:_-]{1,128}$ ]] ||
        die "invalid_response operation=space_delete"
      poll_space_delete "$task"
    fi
  else
    api_request "$actor" "$method" "$path" '' "$expected" "$operation"
  fi
}

discover_cleanup_objects() {
  local logical key marker project issue space title page parent
  local search_project search_space page_id parent_id
  while IFS=$'\t' read -r logical key marker; do
    JIRA_PROJECT_KEYS[$logical]=${JIRA_PROJECT_KEYS[$logical]-$key}
    find_jira_project "$key" "$marker"
    if [[ -n "$FOUND_ID" ]]; then
      JIRA_PROJECT_IDS[$logical]=$FOUND_ID
      JIRA_PROJECT_KEYS[$logical]=$FOUND_KEY
      cleanup_add_target jira_project "$logical" "$FOUND_ID"
    else
      unset 'JIRA_PROJECT_IDS[$logical]'
      find_jira_project "$key" "$marker" deleted
      [[ -z "$FOUND_ID" ]] || cleanup_add_target jira_project "$logical" "$FOUND_ID"
    fi
  done < <(jq -r '.jira.projects[] | [.id,.key,.marker] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r logical project marker; do
    [[ -n "${JIRA_ISSUE_SEARCH_PROJECT[$logical]-}" || -n "${JIRA_PROJECT_IDS[$project]-}" ]] || continue
    search_project=${JIRA_ISSUE_SEARCH_PROJECT[$logical]-${JIRA_PROJECT_KEYS[$project]-}}
    [[ -n "$search_project" ]] || continue
    find_jira_issue "$search_project" "$marker"
    if [[ -n "$FOUND_ID" ]]; then
      JIRA_ISSUE_IDS[$logical]=$FOUND_ID
      JIRA_ISSUE_KEYS[$logical]=$FOUND_KEY
      cleanup_add_target jira_issue "$logical" "$FOUND_ID"
      JIRA_ISSUE_SEARCH_PROJECT[$logical]=$search_project
    fi
  done < <(jq -r '.jira.issues[] | [.id,.project,.marker] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r logical issue marker; do
    issue_id=${JIRA_ISSUE_IDS[$issue]-}
    [[ -n "$issue_id" ]] || continue
    find_jira_comment "$issue_id" "$marker"
    if [[ -n "$FOUND_ID" ]]; then
      JIRA_COMMENT_IDS[$logical]=$FOUND_ID
      CLEANUP_JIRA_COMMENT_ISSUE[$logical]=$issue_id
      cleanup_add_target jira_comment "$logical" "$FOUND_ID"
    fi
  done < <(jq -r '.jira.issues[] as $issue | ($issue.comments // [])[]? | [.id,$issue.id,.marker] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r logical key marker; do
    CONF_SPACE_KEYS[$logical]=${CONF_SPACE_KEYS[$logical]-$key}
    find_confluence_space "$key" "$marker"
    if [[ -n "$FOUND_ID" ]]; then
      CONF_SPACE_IDS[$logical]=$FOUND_ID
      CONF_SPACE_KEYS[$logical]=$FOUND_KEY
      cleanup_add_target conf_space "$logical" "$FOUND_ID"
    fi
  done < <(jq -r '.confluence.spaces[] | [.id,.key,.marker] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r logical space title marker; do
    search_space=${CONF_PAGE_SEARCH_SPACE[$logical]-${CONF_SPACE_IDS[$space]-}}
    [[ -n "$search_space" ]] || continue
    find_confluence_page "$search_space" "$title" "$marker"
    if [[ -n "$FOUND_ID" ]]; then
      CONF_PAGE_IDS[$logical]=$FOUND_ID
      CONF_PAGE_SEARCH_SPACE[$logical]=$search_space
      cleanup_add_target conf_page "$logical" "$FOUND_ID"
    fi
  done < <(jq -r '.confluence.pages[] | [.id,.space,.title,.marker] | @tsv' "$MANIFEST_PATH")

  while IFS=$'\t' read -r logical page marker parent; do
    page_id=${CONF_COMMENT_SEARCH_PAGE[$logical]-${CONF_PAGE_IDS[$page]-}}
    [[ -n "$page_id" ]] || continue
    parent_id=${CONF_COMMENT_SEARCH_PARENT[$logical]-}
    if [[ -z "$parent_id" && "$parent" != null && -n "$parent" ]]; then
      parent_id=${CONF_COMMENT_IDS[$parent]-}
    fi
    if [[ "$parent" != null && -n "$parent" ]]; then
      [[ -n "$parent_id" ]] || continue
    else
      parent_id=''
    fi
    find_confluence_comment "$page_id" "$marker" "$parent_id"
    if [[ -n "$FOUND_ID" ]]; then
      CONF_COMMENT_IDS[$logical]=$FOUND_ID
      CLEANUP_CONF_COMMENT_PAGE[$logical]=$page_id
      CLEANUP_CONF_COMMENT_PARENT[$logical]=$parent_id
      CONF_COMMENT_SEARCH_PAGE[$logical]=$page_id
      CONF_COMMENT_SEARCH_PARENT[$logical]=$parent_id
      cleanup_add_target conf_comment "$logical" "$FOUND_ID"
    fi
  done < <(jq -r '.confluence.comments[] | [.id,.page,.marker,(.parent // "null")] | @tsv' "$MANIFEST_PATH")
}

cleanup_assert_absent() {
  local logical sid issue_logical issue_id key
  local -a target_ids=()
  while IFS= read -r logical; do
    cleanup_target_ids_for "jira_project:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      read_cleanup_jira_project "$sid" cleanup_project_absence
      [[ "$RESPONSE_STATUS" == 404 ]] || die "cleanup_incomplete logical=project"
    done
  done < <(jq -r '.jira.projects[].id' "$MANIFEST_PATH")
  while IFS= read -r logical; do
    cleanup_target_ids_for "jira_issue:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/rest/api/3/issue/${sid}?fields=summary,description,project,parent" '' 200,404 cleanup_issue_absence
      [[ "$RESPONSE_STATUS" == 404 ]] || die "cleanup_incomplete logical=issue"
    done
  done < <(jq -r '.jira.issues[].id' "$MANIFEST_PATH")
  while IFS=$'\t' read -r logical issue_logical; do
    issue_id=${CLEANUP_JIRA_COMMENT_ISSUE[$logical]-${JIRA_ISSUE_IDS[$issue_logical]-}}
    [[ -n "$issue_id" ]] || continue
    cleanup_target_ids_for "jira_comment:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/rest/api/3/issue/${issue_id}/comment/${sid}" '' 200,404 cleanup_comment_absence
      [[ "$RESPONSE_STATUS" == 404 ]] || die "cleanup_incomplete logical=jira_comment"
    done
  done < <(jq -r '.jira.issues[] as $issue | ($issue.comments // [])[]? | [.id,$issue.id] | @tsv' "$MANIFEST_PATH")
  while IFS= read -r logical; do
    cleanup_target_ids_for "conf_comment:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/wiki/api/v2/footer-comments/${sid}?body-format=storage" '' 200,404 cleanup_comment_absence
      [[ "$RESPONSE_STATUS" == 404 ]] || die "cleanup_incomplete logical=confluence_comment"
    done
  done < <(jq -r '.confluence.comments[].id' "$MANIFEST_PATH")
  while IFS= read -r logical; do
    cleanup_target_ids_for "conf_page:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/wiki/api/v2/pages/${sid}" '' 200,404 cleanup_page_absence
      [[ "$RESPONSE_STATUS" == 404 ]] || die "cleanup_incomplete logical=page"
    done
  done < <(jq -r '.confluence.pages[].id' "$MANIFEST_PATH")
  while IFS= read -r logical; do
    cleanup_target_ids_for "conf_space:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      api_request provisioner GET "/wiki/api/v2/spaces/${sid}" '' 200,404 cleanup_space_absence
      [[ "$RESPONSE_STATUS" == 404 ]] || die "cleanup_incomplete logical=space"
    done
  done < <(jq -r '.confluence.spaces[].id' "$MANIFEST_PATH")
}
cleanup_all() {
  [[ ! -L "$STATE_PATH" ]] || die "state_symlink"
  CLEANUP_TARGET_IDS=()
  CLEANUP_JIRA_COMMENT_ISSUE=()
  CLEANUP_CONF_COMMENT_PAGE=()
  CLEANUP_CONF_COMMENT_PARENT=()
  CLEANUP_CONF_SPACE_KEY=()
  if [[ -e "$STATE_PATH" ]]; then
    load_state
  fi
  recover_pending_receipt
  cleanup_seed_state_targets
  cleanup_revalidate_targets
  discover_cleanup_objects
  cleanup_revalidate_targets

  local logical sid issue_logical issue_id key page_logical
  local -a target_ids=() comment_logicals=() page_logicals=() issue_logicals=()
  mapfile -t comment_logicals < <(jq -r '.confluence.comments[].id' "$MANIFEST_PATH")
  for ((i=${#comment_logicals[@]} - 1; i >= 0; i--)); do
    logical=${comment_logicals[$i]}
    cleanup_target_ids_for "conf_comment:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      delete_if_owned provisioner DELETE "/wiki/api/v2/footer-comments/${sid}" confluence_comment_delete
    done
  done
  mapfile -t page_logicals < <(jq -r '.confluence.pages[].id' "$MANIFEST_PATH")
  for ((i=${#page_logicals[@]} - 1; i >= 0; i--)); do
    logical=${page_logicals[$i]}
    cleanup_target_ids_for "conf_page:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      delete_if_owned provisioner DELETE "/wiki/api/v2/pages/${sid}" confluence_page_delete
    done
  done
  while IFS= read -r logical; do
    cleanup_target_ids_for "conf_space:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      key=${CLEANUP_CONF_SPACE_KEY["${logical}:${sid}"]-${CONF_SPACE_KEYS[$logical]-}}
      [[ -n "$key" ]] ||
        die "state_missing_object logical=space"
      delete_if_owned provisioner DELETE "/wiki/rest/api/space/${key}" space_delete
    done
  done < <(jq -r '.confluence.spaces[].id' "$MANIFEST_PATH")

  mapfile -t issue_logicals < <(jq -r '.jira.issues[].id' "$MANIFEST_PATH")
  for ((i=${#issue_logicals[@]} - 1; i >= 0; i--)); do
    logical=${issue_logicals[$i]}
    cleanup_target_ids_for "jira_issue:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      delete_if_owned provisioner DELETE "/rest/api/3/issue/${sid}" jira_issue_delete '204,403,404'
    done
  done
  while IFS= read -r logical; do
    cleanup_target_ids_for "jira_project:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      delete_if_owned provisioner DELETE "/rest/api/3/project/${sid}?enableUndo=false" jira_project_delete
    done
  done < <(jq -r '.jira.projects[].id' "$MANIFEST_PATH")
  while IFS=$'\t' read -r logical issue_logical; do
    issue_id=${CLEANUP_JIRA_COMMENT_ISSUE[$logical]-${JIRA_ISSUE_IDS[$issue_logical]-}}
    [[ -n "$issue_id" ]] || continue
    cleanup_target_ids_for "jira_comment:$logical"
    target_ids=("${CLEANUP_IDS[@]}")
    for sid in "${target_ids[@]}"; do
      delete_if_owned provisioner DELETE "/rest/api/3/issue/${issue_id}/comment/${sid}" jira_comment_delete '204,403,404'
    done
  done < <(jq -r '.jira.issues[] as $issue | ($issue.comments // [])[]? | [.id,$issue.id] | @tsv' "$MANIFEST_PATH")

  cleanup_assert_absent
  if [[ -e "$STATE_PATH" ]]; then
    rm -f -- "$STATE_PATH" || die "state_remove"
  fi
  printf 'cleaned site=%s\n' "$SITE"
}

parse_args "$@"
validate_site
validate_credentials
check_tools
validate_manifest
prepare_state_and_lock
case "$MODE" in
  bootstrap)
    : > "$RUN_DIR/state-projects"; : > "$RUN_DIR/state-issues"; : > "$RUN_DIR/state-jcomments"; : > "$RUN_DIR/state-spaces"; : > "$RUN_DIR/state-pages"; : > "$RUN_DIR/state-comments"
    bootstrap_jira_projects
    bootstrap_jira_issues
    bootstrap_confluence_spaces
    bootstrap_confluence_pages
    bootstrap_confluence_comments
    write_state
    printf 'bootstrapped site=%s\n' "$SITE"
    ;;
  verify) verify_all ;;
  cleanup) cleanup_all ;;
esac
