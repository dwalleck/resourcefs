# Route: rfs-pm0y

Change: Add API-token-authenticated Jira issue Aggregate and native Field reads at the Source Adapter seam.
Date: 2026-08-29

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The change depends on Atlassian Cloud REST v3 behavior that the current tree and existing Gilfoyle evidence artifacts do not prove: stable-ID and issue-key lookup must resolve one authoritative issue; the returned issue/field metadata must distinguish complete ADF values from ordinary JSON; validators, status classes, nullable shapes, and identity-bearing links must match production wire behavior. The parent ticket and closed research ticket `rfs-4df1` record decisions, but no current `.rfs-pm0y/evidence.md` or probe independently compares those premises against an oracle. | yes |
| 2 | Structural boundary | The implementation adds exported validated Jira identifier/address types to `resourcefs-core`, a new compiled Source Adapter in `resourcefs-sources`, and exhaustive dispatch/canonical-identity obligations across core and compiled-source modules. These are public API and module-boundary changes. | yes |
| 3 | Production-scale risk | One Jira issue read is bounded by the existing `HttpSubstrate` body ceiling and does not introduce pagination, fan-out, background work, or an unbounded collection. All visible fields are decoded and rendered within that existing response bound, so no new latency, throughput, concurrency, memory, or data-volume machinery is required. | no |
| 4 | Explicit behavior | Given a configured API-token Atlassian Site Mount and a validated canonical stable issue ID or issue-key alias, when the issue is read, then both addresses resolve the same validated upstream issue and only `jira://<site>/issues/<stable-id>` is published as canonical identity. Given a validated issue Aggregate read, when visible fields are returned, then deterministic Markdown lists every visible field in stable field-ID order with its name, native type, mutability state, and canonical Field reference. Given a Field read, when its value is ordinary JSON or complete ADF, then the exact decided canonical bytes, decided content type, and content-derived Version Tag are returned without ResourceFS frontmatter. Given a well-formed unsupported ADF node, when the Aggregate is rendered, then a typed warning appears at source position and the authoritative Field remains lossless; given malformed native authority, the whole read fails without partial output. Given invalid, ambiguous, unsafe, cross-site, parent-confused, malformed, unauthorized, absent, rate-limited, over-limit, cancelled, or validator-inconsistent input/upstream behavior, when resolution occurs, then validation precedes egress where possible and the stable decided `ResourceError` category is returned without secrets, raw upstream prose, request bodies, account email, or credentials. Given an echoable session-local ETag, when the same GET is repeated, then it revalidates and only a matching cached generation may satisfy 304; without a usable validator it fetches unconditionally. Given a retryable read failure or valid 429/503 `Retry-After`, when one bounded retry fits the original logical deadline, then at most one retry occurs and cancellation remains effective. | yes |

Unknown tests: none

## Selected route

Empirical — required Jira wire, identity, representation, and validator premises are not covered by a current evidence artifact; Empirical takes precedence over the structural boundary change.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior is fully explicit in T4 from `rfs-pm0y` and parent `rfs-0zbv` decisions |
| evidence.md, probe.* | prove-it-prototype | required — Empirical route (T1 yes) |
| design.md | falsifiable-design | required |
| plan.md | budgeted-plan | required |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records PASS for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no FAIL.

Result: 2026-08-29 | `python .rfs-pm0y/probe_openapi.py`; per-slice falsifier/oracle/budget/fence/mutation gates; `cargo test --workspace --all-features` | PASS — empirical premises P1–P5 agree with independent oracles; C1–C16 fences passed; 519 workspace tests passed with 17 ignored; `live_jira_issue_read` registered and skipped cleanly because the real-tenant gate environment was absent; post-fix code/spec reviews report no residual findings; no gate recorded `FAIL`.
