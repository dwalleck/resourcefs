# Route: rfs-i0c9

Change: Move bounded idempotent-read retry policy into the source-neutral HTTP module and add HTTP-date guidance, bounded jitter, cancellation, and logical-deadline enforcement.
Date: 2026-08-29

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The behavior is local and deterministically testable: `crates/resourcefs-sources/src/http/mod.rs` owns response headers, request method, cancellation, and HTTP attempts; Tokio time and fixture HTTP responses cover waits and deadlines. No external-system behavior or production data shape is a design premise. | no |
| 2 | Structural boundary | Retry ownership moves from GitHub's private `Operation`/`fetch` loop in `crates/resourcefs-sources/src/github/mod.rs` to the shared HTTP module used by GitHub, HTTPS, and future Atlassian Sources. This is a cross-module placement decision, though the existing public `HttpSubstrate::fetch(HttpRequest, &OperationGuard)` interface can remain compatible. | yes |
| 3 | Production-scale risk | Each eligible GET remains bounded to one retry, at most 250 ms jitter, the configured HTTP body/header ceilings, and the original logical deadline. No unbounded queue, concurrency, memory, pagination, or data-volume behavior is introduced. | no |
| 4 | Explicit behavior | Given an idempotent GET receives 429 or 503 with valid non-negative delta-seconds or future/equal HTTP-date guidance and the guided delay plus 0–250 ms jitter fits the original logical deadline, when it is fetched, then ResourceFS waits once and makes exactly one retry. Given guidance is missing, malformed, negative/past, or cannot fit before that deadline, when the response is handled, then ResourceFS performs no retry and returns the owning Source's stable `source_unavailable` failure without sleeping past the deadline. Given cancellation occurs during the retry wait, when the wait observes it, then it returns `cancelled` promptly and sends no second attempt. Given a response is non-retryable or a request is non-idempotent, when it is fetched, then it remains single-attempt. Given an existing GitHub transport failure is conclusively retryable, when the shared path handles it, then it retains the existing one-retry behavior. | yes |

Unknown tests: none

## Selected route

Structural — the behavior is explicit and needs no empirical premise, but retry ownership moves across the GitHub/HTTP module seam for reuse by HTTPS and future Atlassian Sources.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior fully explicit in the ticket and T4 verdict; requester explicitly directed that interrogation be skipped |
| evidence.md, probe.* | prove-it-prototype | N/A — no unverified premise (T1 verdict) |
| design.md | falsifiable-design | required |
| plan.md | budgeted-plan | required |

Oracle checkpoint in `checkpointed-build`: required — Structural route

## Downstream sequence

falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Structural — every downstream artifact satisfies its owning stage's completion criterion, ending with no FAIL in checkpointed-build's recorded gate.

Result: 2026-08-29 | Slice 1 checkpoint | PASS — C1–C7 falsifiers and fences green; every named mutation red and restored green; affected contracts, stress matrix, independent listener/time/category oracles, production budget, formatter, and clippy passed with no gate `FAIL`.
