# Route: rfs-r31i

Change: Read review submissions and native inline anchors through four GitHub Fact Resource routes.
Date: 2026-09-09
Source: fix/pr10-review at 9f147b5086335d76318d543cb13c35a87a80ce70; implementation worktree /home/dwalleck/repos/resourcefs-wt-pr10-review. Ticket claimed by omp; prerequisite rfs-bfwa is closed.

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | P1: selected REST 2022-11-28 review/inline list and item endpoints supply native parent, review commit and current/original anchor fields without reconstruction. P2: these endpoint families' pagination links, parent identity and list/item correspondence support bounded traversal. .rfs-bfwa/evidence.md P1/P2 establish conversation-comment behavior only, not these families; its L7/S-comment proof likewise does not cover reviews/inline anchors. Fresh endpoint observations and independent comparison required. | yes |
| 2 | Structural module shape | Public core GitHub address variants and owned JSON family schemas expand. Existing core GitHub addressing owns validated references; sources GitHub owns acquisition, private decoding, parent validation and projection; MCP retains its read/recovery responsibility. Reuse the existing collection, transport, error and envelope seams, not a second HTTP client or provider trait. Protected parent is the existing GitHub adapter facade; new family behavior must remain with bounded family owners rather than accumulate there. Exact placement is design-stage work. | yes |
| 3 | Production-scale risk | New collections encounter response, cumulative body, JSON and record bounds, shared attempts/deadline, cancellation and partial retention. Native bodies/hunks can expand serialization and allocation. Actual implementation measurements are required; inherited synthetic results do not prove new paths. | yes |
| 4 | Explicit behavior | Complete given/when/then contract below, from rivets show rfs-r31i and approved specification stories 23–25 and 28–43. Proposed spellings/mechanism require design approval, not new behavioral interrogation. | yes |

Unknown tests: none.

### T4 complete observable behavior contract

1. Given a review collection or individual review, when read, then preserve distinct native kind and ID, PR parent, nullable or absent body/author/submission time, supplied state, original review commit and links. A later head push never retargets the old review.
2. Given an inline collection or individual inline comment, when read, then preserve supplied review/reply/thread relationships, current/original commits, path/hunk, side/line/start-side/start-line, original ranges/positions and subject type. Preserve absent versus null and unknown native values.
3. Given LEFT/RIGHT, multiline, outdated/original, file-level and null-line anchors, when represented, then retain native targeting without diff-offset reconstruction, output-coordinate substitution, auto-retargeting or general-comment fallback.
4. Given colliding conversation/review/inline IDs and text, when read, then kinds and identities stay distinct. Wrong parent/requested identity rejects the entire current component including prior pages. Ordinary later failure retains verified earlier whole pages with honest coverage and only valid continuation; first failure without usable data is typed failure. Cancellation and authority violations reject the current component.
5. Given each of the four new routes, when used through validated addressing, configured adapter, public library and real MCP, then complete owned versioned JSON is available using existing SourceAdapter/SourceResource and five tools, documented catalog grammar/examples and applicable profile/CLI schema/check paths. Human projections remain unchanged; no private inspector, provider trait or duplicate HTTP client.
6. Given configured deployment and repository authority and origin-bound credentials, when any acquisition, parent lookup, continuation, retry, revalidation or cancellation path runs, then independently observed requests remain permitted and read-only with zero forbidden egress and remote writes. Principal changes require a new Path Session; REST remains 2022-11-28 and selected ghe.com pairing remains validated.
7. Given facts, when serialized, then preserve deployment/source identity, supplied native repository/object IDs and links, requested versus observed immutable operands, acquisition interval, REST version and supplied upstream versions/times separately. Missing/null/unknown values and explicit coverage survive. VersionTag hashes representation bytes, not Git identity. Reuse reviewed envelope/error vocabulary; additive optional fields allowed and unsupported majors rejected by consumers.
8. Given lower-only acquisition controls, when invalid (zero/above ceiling), then reject; valid controls intersect operator policy separately from display limits. Measure applicable exact/over-limit enforcement: 10 physical attempts including parents/revalidation/retries, 30 seconds shared send/body/wait, 8 MiB response, 16 MiB accepted bodies, 16 MiB serialized JSON including all overhead, 1,000 records. The inherited 4 MiB decoded regular-file bound is not a new regular-file path here. Do not claim these are peak-memory/wire-traffic guarantees. Measure actual allocation/transport behavior.
9. Given a candidate page, when any whole-page record/body/final-JSON budget fails, then admit none of it, preserve usable prior pages only under ordinary failure, and never produce truncated JSON or invented recovery/continuation. Expose provider caps separately without exhaustive fallback. Honor applicable rate guidance within the same deadline and never return stale cache success after failed revalidation.
10. Given operational failure, when surfaced, then use existing ResourceError stable categories and bounded sanitized status/access ambiguity/retry/limit details; no provider prose/secrets, message parsing, hidden-404-to-empty or denial-to-expired-login inference.
11. Given bounded display of acquired JSON, when a library reads it, then it parses complete bounded SourceResource content; when MCP output spills, actual Recovery References reconstruct exactly acquired bytes before parsing. Source continuation instead acquires upstream data, is opaque/credential-free/session-scoped/resource-operand-authority-bound, and claims best-effort traversal rather than a snapshot.
12. Given new adapter and primary real rfs stdio paths, when RFS_LIVE=1 and GITHUB_TOKEN authorize public read-only smokes, then applicable ignored live_* rows prove all four routes and native anchor invariants against independent upstream observations. Absent gates skip cleanly and are not passes. Record executed evidence and exact revision. Controlled fixtures prove hazardous failures and unavailable native shapes without mutating upstream.
13. Given final implementation, when accepted, then the repository-owned complete gate passes with live evidence recorded separately. ResourceFS acceptance does not close Cyril corporate ghe.com/native Windows/credential/storage/UI acceptance. ResourceFS acquires facts; Cyril owns authorization/totals, retention and review meaning.

## Selected route

Empirical — unverified native review/inline endpoint premises take precedence over public grammar/schema expansion and scale risk.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior fully explicit; proposed spellings belong to design approval |
| evidence.md, probe.* | prove-it-prototype | required — fresh P1/P2 observations and independent oracle |
| design.md | falsifiable-design | required — schemas, placement, falsifiers and explicit approval |
| plan.md | budgeted-plan | required — independently green slices after design approval |

Oracle checkpoint in checkpointed-build: required — Empirical route.

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build.

## Terminal criterion

Empirical — every empirical premise records PASS, every downstream artifact satisfies its owning stage's completion criterion, and checkpointed-build records no FAIL.

Result: 2026-09-09 | `python scripts/ci-gates.py` on `f4128c1d` | PASS — `All repository gates passed.` P1/P2 PASS with an independent oracle; `spec.md` `N/A — behavior fully explicit`; `design.md` approved and its isolated conformance comparison PASS; `plan.md` slices 1-3 discharged with no `FAIL`; live adapter and stdio rows PASS on `7dfc8f1d` (revision before the gate-classification commit, which changes no production path).
