# Issue tracker: Rivets

Issues and specs for this repository live in the local Rivets store at `.rivets/issues.jsonl`. Run the `rivets` CLI from the repository root for every operation; treat the JSONL file as storage and never edit it directly.

## Repository settings

- Issue IDs use the `rfs-` prefix configured in `.rivets/config.yaml`.
- The storage backend is repository-local JSONL.
- Use `--json` for programmatic reads and writes.
- Use `-y` for non-interactive writes.
- Pull requests are not a triage request surface.

## Conventions

- **Create an issue**: `rivets create --json -y --title "..." --description "..." --kind task --priority 2`
- **Read an issue**: `rivets show --json <issue-id>`
- **List issues**: `rivets list --json`, adding `--status`, `--kind`, `--label`, `--assignee`, or `--priority` filters as needed
- **List unblocked work**: `rivets ready --json`
- **Append a comment or durable note**: `rivets update --json -y <issue-id> --notes "..."`
- **Apply a label**: `rivets label add --json -y <label> <issue-id>`
- **Remove a label**: `rivets label remove --json -y <label> <issue-id>`
- **Change workflow status**: `rivets update --json -y <issue-id> --status open|in_progress|blocked|closed`
- **Close an issue**: `rivets close --json -y <issue-id> --reason "..."`
- **Add a blocker**: `rivets dep add --json -y <blocked-issue> <blocker> --type blocks`

Use labels for triage roles. Rivets workflow status records whether work is open, in progress, blocked, or closed; it does not replace the triage label state.

## When a skill says "publish to the issue tracker"

Create a Rivets issue. Preserve the generated `rfs-...` ID in any response or cross-reference.

## When a skill says "fetch the relevant ticket"

Run `rivets show --json <issue-id>` and use its complete description, notes, labels, status, dependencies, and timestamps.

## Wayfinding operations

Used by `/wayfinder`. A map is an epic with child issues.

- **Map**: create an epic labelled `wayfinder-map`; keep Notes, Decisions-so-far, and Fog in its description and appended notes.
- **Child ticket**: create an issue labelled `wayfinder-<type>` and `wayfinder-<map-id>`, where type is `research`, `prototype`, `grilling`, or `task`. Link it to the map with `--deps parent-child:<map-id>` during creation or `rivets dep add --json -y <child-id> <map-id> --type parent-child`.
- **Blocking**: `rivets dep add --json -y <child-id> <blocker-id> --type blocks`. A child is unblocked when all blocking dependencies are closed.
- **Frontier**: run `rivets ready --json --label wayfinder-<map-id> --sort oldest`, discard assigned issues, and take the first result.
- **Claim**: set the child to `in_progress` and assign it before doing work.
- **Resolve**: append the answer as a note, close the child, then append a context pointer containing the gist and child ID to the map's Decisions-so-far.
