# Route: rfs-34pz

Change: Recover selector-bounded reads through immutable Path Session artifacts
Date: 2026-08-19

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | Path Session invalidation depends on the current `rmcp` 3.1.3 connection lifecycle: the SDK exposes no `ServerHandler` disconnect hook, so teardown must use `RunningService::waiting()` and/or its cancellation token. No current repository evidence proves whether that completion point occurs only after in-flight tool handlers can no longer resolve session state. `.rfs-87cv/evidence.md` covers protocol negotiation and `.rfs-cgbq/evidence.md` covers Roots requests; neither covers disconnect ordering. This premise requires a compiled runtime probe and an independent SDK-source oracle. | yes |
| 2 | Structural boundary | The change adds common selector execution, bounded-result/recovery, Path Session identity, quotas, and errors to `resourcefs-core`; adds an Artifact Source Adapter and retained backing storage to `resourcefs-sources`; and expands `rfs_read`'s public input/output schema plus per-connection lifecycle in `resourcefs-mcp`. It therefore changes public schemas and interfaces across all three principal modules and requires cross-module placement decisions. | yes |
| 3 | Production-scale risk | Successful reads may retain up to 64 MiB per artifact and 256 MiB per Path Session while concurrent reads, disconnect invalidation, and TTL cleanup contend for session state. The design must bound memory and disk use, make quota admission atomic, prevent live-reference eviction, and prove cleanup cannot cross sessions. | yes |
| 4 | Explicit behavior | The issue fixes the required outcomes but not a complete observable contract. Given range, multi-range, raw, or paging selectors, exact content and valid continuation selectors must be returned; given per-call limits, they may lower but never raise the 48 KiB/3,000-line/512-column hard ceilings; given omitted successful output, a same-session immutable `artifact://` reference must recover it losslessly; given object/session quota exhaustion, the call must fail without eviction or a false Recovery Reference; and given disconnect/TTL expiry, session references must invalidate and retained state must be removed without crossing sessions. Unresolved decisions remain: exact range/paging semantics at EOF and around trailing newlines, limit field and invalid-value behavior, artifact reference/canonicalization rules, spill granularity, quota accounting/admission, cache/TTL defaults and cleanup trigger, and session-mismatch error behavior. | no |

Unknown tests: none

## Selected route

Empirical — safe disconnect invalidation depends on unverified `rmcp` lifecycle ordering; the public schema and all three module boundaries also change, storage/concurrency carries production-scale risk, and observable edge semantics require interrogation.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — T4 identifies unresolved selector, limit, artifact, quota, and lifecycle behavior |
| evidence.md, probe.* | prove-it-prototype | required — Empirical route (T1 verdict) |
| design.md | falsifiable-design | required |
| plan.md | budgeted-plan | required |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

interrogated-spec → prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records `PASS` for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no `FAIL`.
