# Route: rfs-bcym

Change: Add a manifest-driven operator command that bootstraps, verifies, and cleans up disposable Jira and Confluence Cloud live fixtures.
Date: 2026-08-30

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The command depends on current Atlassian Cloud behavior not yet covered by a ticket-local evidence artifact: which Jira and Confluence REST operations can idempotently create, rediscover, verify, and delete projects/spaces, issues/pages/comments, hierarchy, ADF/storage edge cases, reader-visible objects, and a reader-concealed object; which returned IDs and parent fields are stable enough for live-test state; and how provisioner-versus-reader visibility is observed. The 2026-08-29 Jira and Confluence research records published contracts, but no `.rfs-bcym/evidence.md` independently compares the complete fixture lifecycle against an oracle or a real disposable tenant. | yes |
| 2 | Structural boundary | The change adds an operator-facing command contract plus checked-in manifest and ignored-state schemas. Those are durable interfaces consumed by maintainers and live tests, even though the command remains independent of ResourceFS production mutation modules. | yes |
| 3 | Production-scale risk | The fixture is a fixed, disposable test dataset. Pagination can be forced with small request limits rather than tenant-scale object counts, and bootstrap/verify/cleanup do not enter ResourceFS production request paths. The command still needs bounded failure and retry behavior, but no production latency, throughput, memory, concurrency, or data-volume machinery is required. | no |
| 4 | Explicit behavior | Given a checked-in non-secret manifest, it describes at least two Jira projects and two Confluence spaces plus issues, pages, comments, parent/child hierarchy, ADF and storage-format edge cases, a fixture capable of forced pagination, and at least one object concealed from the reader principal. Given an operator-selected Atlassian Cloud tenant and execution-time provisioner/reader credentials, when `bootstrap` runs repeatedly, then it converges idempotently on the manifest without echoing or persisting credentials and writes only stable fixture IDs and ResourceFS profile references to ignored local state. Given the provisioned tenant and that state, when `verify` runs, then it checks the manifest shape, stable identity/parentage, edge representations, pagination fixture, and the reader-visible/concealed boundary through Atlassian REST using `curl` and `jq`, returning success only when all invariants hold. Given the provisioned fixture, when `cleanup` runs repeatedly, then it removes only manifest-owned disposable objects and treats already-absent owned objects as success. Given any invalid manifest, missing prerequisite, HTTP failure, malformed response, drift, or permission mismatch, when any mode fails, then it exits nonzero with bounded non-secret tenant/object/operation context sufficient for safe repair or rerun and leaves credentials absent from output and state. | yes |

Unknown tests: none

## Selected route

Empirical — the complete real Atlassian fixture lifecycle and visibility boundary are unverified external premises; Empirical takes precedence over the operator schema boundary.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior is fully explicit in T4 from `rfs-bcym` and parent `rfs-0zbv` decisions |
| evidence.md, probe.* | prove-it-prototype | required — Empirical route (T1 yes) |
| design.md | falsifiable-design | required |
| plan.md | budgeted-plan | required |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records PASS for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no FAIL.
