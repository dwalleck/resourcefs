# Evidence: rfs-bfwa

Source state: `feat/rfs-0n97` = `084d136e681179eb33a31f6b84a0fb17abf47daa` in `/home/dwalleck/repos/resourcefs-wt-rfs-bfwa-impl`; no production edits. Date: 2026-09-08.

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|---|---|---|---|
| P1 | Selected-version GitHub REST conversation-comment records supply the native facts the owned schema requires. | Does `GET /repos/rust-lang/rust/issues/112049/comments` at REST 2022-11-28 return records carrying native `id`, `node_id`, `url`, `html_url`, `issue_url`, `user`, `body`, `created_at` and `updated_at`, and do two independent clients agree on that shape? | PASS |
| P2 | The endpoint's pagination and coverage behavior is what the bounded-collection design assumes. | With `per_page=100`, is `per_page` capped at 100 regardless of request, is `rel="next"` the decisive "more pages" signal, and does a page past the end return `200` with an empty array? | PASS |
| P3 | Real MCP output recovery reconstructs an acquired facts JSON document before parsing. | Does the existing read/recovery path already reconstruct acquired Facts bytes on this revision? | PASS — retained from rfs-0n97 S6; recovery path unchanged on this branch |
| N1 | Atomic page admission, 1,000-record/16 MiB ceilings, attempt/deadline accounting and allocation cost. | Nothing exists yet; these are approved behavior and implementation-measurement obligations. | N/A — design/checkpoint obligations, not existing-system facts |
| N2 | Continuation session binding, expiry and credential-freedom. | What does the current Path Session API already expose? | N/A — direct current-source inspection resolves it (`PathSession::token`, `cache_generation`, `is_active`, `invalidate`); the value is a design obligation, not an unverified premise |
| N3 | Corporate `ghe.com` identity/scopes and native Windows operation. | Actual tenant and native acceptance belong to the separate Cyril gate. | N/A — not claimed by public-provider proof |
| N4 | Exact route/field spellings the specification marks PROPOSED. | Spelling is not existing-system behavior. | N/A — design-approval work |

## Data

- Source: production-shaped public read-only GitHub REST responses.
- Shape: conversation-comment collections for `rust-lang/rust` PR 112049 (2,385 comments, 24 provider pages at `per_page=100`) and PR 162473 (zero comments); field presence/type projections and HTTP `Link` headers only.
- Safety: GET only, explicit `api.github.com`, no redirects, no request bodies. Comment bodies were parsed but never retained or printed; only counts, key sets and type names are recorded. No upstream mutation, no private tenant, no credential file read. The probe used no credential; the oracle used the operator's existing `gh` credential for public read-only GETs, matching the repository's `scripts/live-smoke.sh` practice.

## Probe

- File: `.rfs-bfwa/probe_provider.py`
- Mechanism: Python 3 `urllib` with a no-redirect opener, explicit `Accept: application/vnd.github+json` and `X-GitHub-Api-Version: 2022-11-28`, 20-second timeout; projects each response to status, parsed `rel="next"` target, record count, first-record key set, per-field presence/null/type counts, and `pull_request`-key presence.
- Run: `python3 .rfs-bfwa/probe_provider.py > .rfs-bfwa/probe-provider-result.json`
- Output: `.rfs-bfwa/probe-provider-result.json`

## Oracle

- Mechanism: the Go-based `gh api --include` client (different HTTP stack, header handling and JSON parser from the probe's Python stack) with `jq` computing the same projections independently, including `rel="next"` extraction by splitting link-values and filtering on `rel`. The production adapter (`reqwest` + `serde`) is a third mechanism and was not used.
- Run: `bash .rfs-bfwa/oracle_provider.sh > .rfs-bfwa/oracle-provider-result.json`
- Output: `.rfs-bfwa/oracle-provider-result.json`

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|---|---|---|---|
| P1 | `busy_page1`: 100 records; every record carries `id` (number), `node_id`, `url`, `html_url`, `issue_url`, `created_at`, `updated_at`, `body` (string) and `user` (object), all non-null in this sample; key set `author_association, body, created_at, html_url, id, issue_url, minimized, node_id, performed_via_github_app, reactions, updated_at, url, user`; no `pull_request` key. | Identical status, count, key set, per-field presence/null/type counts after normalizing `int`/`str`/`dict` to `number`/`string`/`object`. | PASS |
| P2 | `busy_page1`/`busy_page2`: `200`, 100 records each, `rel="next"` → `…/repositories/724712/issues/112049/comments?per_page=100&page=2` then `…page=3`. `busy_oversized_per_page` (`per_page=1000`): `200`, 100 records, `rel="next"` echoes the requested `per_page=1000&page=2`. `busy_past_end` (`page=99999`): `200`, 0 records, `Link` present but no `rel="next"` (`rel="prev"/"last"/"first"`, `last` = page 24). `empty_page1`/`empty_past_end` (PR 162473): `200`, 0 records, no `Link` at all. | Identical for all six observations. | PASS |
| P3 | Retained: rfs-0n97 S6 recorded actual MCP recovery reconstructing acquired Facts bytes (`oracles/s6-results.json`, `oracles/live-smoke.log`). | Retained oracle: the same S6 record's independent output bytes/hash and real stdio observations. | PASS — retained |

Both clients' full projections are compared field-by-field in `.rfs-bfwa/probe-oracle-comparison.json`; `all_match: true`, zero mismatches. One comparison defect was found and fixed in the oracle's own jq projection (an extra `select(type=="array")` filtered every record); the fix changed only the oracle's expression, never the recorded provider output.

## Validated / learned

- P1: validated prior understanding — the selected REST version directly supplies `node_id`, `url`, `html_url` and `issue_url` on conversation comments, which the current wire type does not retain (`crates/resourcefs-sources/src/github/wire.rs:71-78`). The owned schema can therefore carry native identity and links without fabricating them.
- P1: learning — the record carries more keys than the schema needs (`author_association`, `minimized`, `performed_via_github_app`, `reactions`) and no `pull_request` key, so conversation comments are issue comments: parent linkage is the native `issue_url`/parent identity, and the existing kind/parent gate (a real `pulls/{number}` read) stays necessary to keep an issue out of `pr://`.
- P1: limitation — every sampled record had a present non-null body and user. Deleted users, null/minimized bodies and malformed identities are not exercised by this sample and remain controlled-fixture obligations.
- P2: validated prior understanding — `per_page` is capped at 100 by the provider regardless of the requested value, and the provider's own `rel="next"` link uses the `/repositories/{id}/…` path form already handled by `fetch.rs::is_repository_id_path`.
- P2: learning — an empty page past the end is `200` with `[]` **and a `Link` header that has no `rel="next"`**; "Link present" is therefore not a completeness signal, only a parsed `rel="next"` is. The provider also echoes the caller's requested `per_page` in its own next link while serving 100 records, so preserving the native link (or its exact query) is what keeps continuation meaning intact.
- P3: retained understanding — recovery of an oversized acquired document is existing behavior; this slice adds a collection representation, not a new recovery mechanism.

## Related issues

- Consulted (bounded tracker search over "conversation comment", "collection", "continuation", "cursor", "partial", "pagination", "facts", "comments"):
  - `rfs-0n97` — base slice; supplies the owned envelope, bounded HTTP acquisition, structured errors, MCP recovery and gated live smokes this slice reuses.
  - `rfs-45ww` — closed; established GitHub issue/PR browsing with distinct conversation/review/inline projections and the parent-verification behavior the facts collection must respect.
  - `rfs-jsx9` — ADR-0006; source continuation is a typed `PathReference` selector surfaced alongside artifact recovery, never a generic continuation list.
  - `rfs-r31i`, `rfs-nchb`, `rfs-iktl`, `rfs-e5cv`, `rfs-3r0s`, `rfs-jrz7` — successor slices that own reviews/inline anchors, checks/statuses, PR commits/files, comparisons and exact source; out of scope here.
  - `rfs-nae2` — established the opaque `:cursor:` continuation pattern (`SourceCursor` + owner-bound base64url envelope) this slice follows.
- Filed: none — no underlying-system defect was observed.

## Probe-stage hand-off

P1 and P2 are fresh PASS; P3 is a retained PASS with its applicability reason; N1–N4 are `N/A — <reason>`. The probe and oracle artifacts and both result files are retained. Hand off to [`falsifiable-design`](../.agents/skills/gilfoyle/references/falsifiable-design.md) with this `evidence.md` path. Production implementation and design approval remain pending; no production file was changed and no feature test was added.

## S6 live read-only proof (2026-09-08)

Revision: the tree committed as S6 on `feat/rfs-bfwa` (base `feat/rfs-0n97` = `084d136`). Gate: `RFS_LIVE=1`, `GITHUB_TOKEN=$(gh auth token)`; public `rust-lang/rust` PR 159232, read-only.

| Row | Command | Observed |
|---|---|---|
| L7 adapter | `RFS_LIVE=1 GITHUB_TOKEN=… cargo test -p resourcefs-sources --test github_live_smoke -- --ignored --nocapture` | PASS — `live_github_conversation_comment_facts_hold_up` read 7 conversation comments with `state: complete`; the first record's `id`/`nodeId`/`body` and the parent `id`/`number` matched an independent `gh api …/issues/159232/comments` observation, every record named the verified parent, no `providerCap` was invented, and the single-comment read matched the same native record. |
| S-comment stdio | `RFS_LIVE=1 GITHUB_TOKEN=… cargo test -p resourcefs-mcp --test stdio_live_smoke -- --ignored --nocapture` | PASS — `live_stdio_github_comment_facts_match_native_observation` drove the real `rfs serve` over stdio: the collection matched the independent observation (7 records) and the single-comment document overflowed the inline page, so its artifact continuation chain was followed and the reconstructed bytes parsed to the same native record. Credential sentinel absent from output. |

Without the gate both rows print an explicit skip and are never counted as passes. These are Linux runtime plus public GitHub live proofs; corporate `ghe.com`, native Windows and Cyril acceptance remain the separate gate.
