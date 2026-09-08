# Route: rfs-br1u

Change: Safely resume Atlassian fixture cleanup after asynchronous space deletion.
Date: 2026-09-07

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | rfs-br1u records an unretained native longtask payload, owned trashed pages losing accessible ownership properties, and page-list 404 after space deletion. Native task envelope and intermediate-state semantics require new live evidence; the later successful run does not establish them. | yes |
| 2 | Structural module shape | Current owner is scripts/atlassian-fixture-bootstrap.sh: cleanup_revalidate_targets, poll_space_delete, discover_cleanup_objects, cleanup_all (1694–2092 at 7de8d14). Repair stays inside existing cleanup responsibility; no public API, schema or responsibility move is yet proposed. Reassess if evidence requires a receipt schema or module-owner change. | no |
| 3 | Production-scale risk | Bounded operator fixture set, existing MAX_POLLS and request limits. No production-scale latency, concurrency or volume change requested. Destructive ownership safety is mandatory, not a throughput target. | no |
| 4 | Explicit behavior | Given marker-owned live fixtures, when a space deletion is accepted, interpret the evidenced native progress/success/failure envelope; unknown or failed responses retain uncertain receipts. Given interrupted cleanup with trashed pages or deleted owner spaces, when receipt-based cleanup resumes, safely finish owned cleanup despite absent child collections. Given foreign/replaced objects, when cleanup approaches a destructive action, revalidate ownership immediately and refuse foreign objects. Given uncertain absence, retain receipts; only authoritative final absence permits removal. Given the repair, deterministic interrupted-operation regressions and live bootstrap → interrupted cleanup → resume must prove it. | yes |

Unknown tests: none. T2 is revisited if empirical evidence changes the required interface.

## Selected route

Empirical — native async and interrupted lifecycle semantics are not covered by retained evidence.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior fully explicit in ticket (T4) |
| evidence.md, probe.* | prove-it-prototype | required — native semantics unverified (T1) |
| design.md | falsifiable-design | required |
| plan.md | budgeted-plan | required |

Oracle checkpoint in checkpointed-build: required — Empirical route.

## Downstream sequence

prove-it-prototype → falsifiable-design → user approval → budgeted-plan → checkpointed-build.

## Terminal criterion

Every empirical premise PASS; every downstream artifact satisfies its owning stage's completion criterion, ending with no FAIL in checkpointed-build. Ticket closure additionally requires deterministic regression coverage and owned live bootstrap → interrupted cleanup → resume evidence. No completion is claimed before those conditions hold.

## Scope confirmation

On 2026-09-07 the requester selected "Narrow Bash repair" after considering Python/Rust package alternatives. The route remains Empirical with unchanged external interfaces and responsibility ownership; no migration or SDK adoption is part of this ticket. Supplemental P4 evidence verifies native page trash/purge semantics needed to preserve the existing moved-owned-page cleanup contract. See evidence.md and probe.purge.py.
