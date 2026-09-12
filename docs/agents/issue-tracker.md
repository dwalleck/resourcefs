# Issue tracker: Rivets

Issues and specs for this repository live in the local Rivets store at `.rivets/issues.jsonl`. Run the `rivets` CLI from the repository root for every operation; treat the JSONL file as storage and never edit it directly.

## Repository settings

- Issue IDs use the `rfs-` prefix configured in `.rivets/config.yaml`.
- The storage backend is repository-local JSONL.
- Use `-y` for non-interactive writes.
- Pull requests are not a triage request surface.
- `--json` gives machine-readable output, but it **suppresses the CLI's load warnings**. Those warnings are how the store reports schema migrations it applied on read — for example `AssignmentStateMigrated`, which clears an assignee from a closed issue because a closed issue cannot stay assigned. That rewrites lines for issues you did not touch. Run once without `--json` when a write produces a larger `.rivets/issues.jsonl` diff than you expect, rather than assuming the store was corrupted.
- Commands below are verified against `rivets 0.1.0`. If one is rejected, trust `rivets <command> --help` over this file and fix this file.

## Committing ticket changes

`.rivets/issues.jsonl` is versioned, and its changes belong with the work they record. Commit a ticket closure, a durable note, or a newly filed follow-up in the same commit as the code and tests that produced it, and name the ticket IDs in the commit message. When a change spans several commits, the ticket store rides with the final one. Do not make standalone `chore(rivets): …` commits — a closure separated from its work leaves the history unable to say which commit finished the ticket.

## Conventions

- **Create an issue**: `rivets create -y --title "..." --description "..." --kind task --priority 2`
  Optional at creation: `--assignee`, `--labels`, `--design`, `--acceptance`, `--notes`, `--prerequisite <issue-id>`.
- **Read an issue**: `rivets show <issue-id>...`
- **List issues**: `rivets list --limit <n>` — **`--limit` is required.** Add `--kind`, `--label`, `--assignee`, or `--priority` to filter.
- **List unblocked work**: `rivets ready` (`--limit`, `--label`, `--sort`, `--all-assignees`)
- **Append a durable note**: `rivets note append <issue-id>... --content "..." -y`
  `rivets update` has **no** `--notes`; `--notes` exists only on `create`. `update` carries `--title`, `--description`, `--priority`, `--kind`, `--design`, `--acceptance`.
- **Apply a label**: `rivets label add -y <label> <issue-id>`
- **Remove a label**: `rivets label remove -y <label> <issue-id>`
- **Close an issue**: `rivets close -y <issue-id>... --reason "..."`
- **Reopen**: `rivets reopen -y <issue-id>...`
- **Add a blocker**: `rivets blocking-dependency add -y --dependent <blocked> --prerequisite <blocker>`
  Also `remove`, `list`, and `tree`. There is no `dep` command.

**Workflow status is moved by verbs, not by a flag.** `rivets update` has no `--status`. Use `claim -y --assignee <name> <issue-id>` to take an open, unblocked issue, `start -y <issue-id>...` to begin assigned work, `return-to-open -y` to put it back, `release -y` to drop the claim, and `close` / `reopen` for the ends.

Other associations, all directed and non-blocking except where noted:

- **Related** (symmetric): `rivets related add -y --issue <a> --related <b>`
- **Discovery** (this issue was found while working on that one): `rivets discovery add -y --discovered <new> --source <origin>`
- **Parentage** (one epic owns a child): `rivets parent set -y --child <child> --parent <epic>`, plus `clear`, `move`, `show`

Use labels for triage roles. Workflow status records whether work is open, in progress, blocked, or closed; it does not replace the triage label state.

## When a skill says "publish to the issue tracker"

Create a Rivets issue. Preserve the generated `rfs-...` ID in any response or cross-reference.

## When a skill says "fetch the relevant ticket"

Run `rivets show <issue-id>` and use its complete description, notes, labels, status, dependencies, and timestamps.

## Wayfinding operations

Used by `/wayfinder`. A map is an epic with child issues.

- **Map**: create an epic labelled `wayfinder-map`; keep Notes, Decisions-so-far, and Fog in its description and appended notes.
- **Child ticket**: create an issue labelled `wayfinder-<type>` and `wayfinder-<map-id>`, where type is `research`, `prototype`, `grilling`, or `task`. Link it to the map with `rivets parent set -y --child <child-id> --parent <map-id>`.
- **Blocking**: `rivets blocking-dependency add -y --dependent <child-id> --prerequisite <blocker-id>`. A child is unblocked when all prerequisites are closed.
- **Frontier**: run `rivets ready --label wayfinder-<map-id> --sort oldest --limit 1`, discard assigned issues, and take the first result.
- **Claim**: `rivets claim -y --assignee <name> <child-id>`, then `rivets start -y <child-id>` before doing work.
- **Resolve**: append the answer as a note, close the child, then append a context pointer containing the gist and child ID to the map's Decisions-so-far.
