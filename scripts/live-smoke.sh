#!/usr/bin/env sh
# Runs every live smoke row (ignored tests named `live_*`) against real
# upstreams. The deterministic contracts are the permanent suite; these rows
# exist for the shapes a fake cannot be trusted to imitate. See AGENTS.md,
# "Live smoke tests".
#
# Gates:
#   RFS_LIVE=1          enables network rows (set here)
#   GITHUB_TOKEN        enables GitHub rows; taken from `gh auth token` when unset
#   ATLASSIAN_*         enables the Jira row; see jira_live_smoke.rs for names
#
# Extra arguments are passed to the test harness, e.g. `scripts/live-smoke.sh
# live_jira` to run one row.
set -eu
cd "$(dirname "$0")/.."

if [ -z "${GITHUB_TOKEN:-}" ]; then
    GITHUB_TOKEN="$(gh auth token 2>/dev/null || true)"
fi
if [ -z "${GITHUB_TOKEN:-}" ]; then
    echo "live-smoke: no GITHUB_TOKEN and \`gh auth token\` produced none; GitHub rows will skip" >&2
fi
export GITHUB_TOKEN
export RFS_LIVE=1

exec cargo test --workspace --all-features -- --ignored --nocapture live_ "$@"
