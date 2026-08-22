# Route: rfs-ewh2

Change: Describe the namespace from its rfs:// root
Date: 2026-08-22

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | Every premise is covered by current repository evidence. Client root-set change tracking is implemented and evidenced: `crates/resourcefs-mcp/src/server.rs` (`RootSync`, `mark_client_roots_pending`, `on_roots_list_changed` at line 967, `RootRefresh` from `FilesystemSource::begin_client_root_refresh`). Degraded-source state exists in `crates/resourcefs-sources/src/probe.rs` (`ProbeRunner`, `ProbeRecord`) and `resourcefs-core/src/probe.rs` (`ProbeState`, `ProbeOutcome`). The compiled registry is `CompiledSources` in `crates/resourcefs-sources/src/compiled.rs` (filesystem + artifacts today). Limits, result, and error contracts exist in `resourcefs-core/src/limits.rs`, `read.rs`, and `error.rs`. No premise depends on unverified external or runtime behavior; rmcp session/notification behavior was evidenced in `.rfs-34pz/evidence.md` and the code paths it covered are unchanged. | no |
| 2 | Structural boundary | The change extends the public `PathReference`/`ResourceAddress` grammar in `crates/resourcefs-core/src/reference.rs` (bare `rfs://` and bare `rfs://workspace` are currently not catalog addresses: `WORKSPACE_PREFIX` requires a trailing root segment and `ResourceAddress` has only `Workspace`/`Artifact` variants), adds a new catalog surface to the five-tool Behavior Contract, changes public tool descriptions and server `with_instructions` (`server.rs:980`), and requires a placement decision for the catalog module (core vs sources vs mcp). | yes |
| 3 | Production-scale risk | Catalog output is bounded by the compiled registry (2 sources today, constant-bounded growth) and `MAX_WORKSPACE_ROOTS = 256`; rendering reuses the existing `TextLimits`/document-budget machinery. No new latency, throughput, memory, concurrency, or data-volume exposure. | no |
| 4 | Explicit behavior | The issue and ADR-0005 fix the observable goals (bounded catalog of every mounted scheme with grammar + example; degraded marking; `rfs://workspace` enumeration tracking root-set changes; explicit mutation rejection; common limits/result/error contracts; discovery named in descriptions and instructions) but leave unresolved: the catalog text layout and structured-content schema, per-scheme grammar lines and example spellings, the degraded-marking presentation, the workspace-root listing shape and primary marker, the error category for catalog mutation attempts, and the catalog's specific bound values. Those observable decisions require interrogation. | no |

Unknown tests: none

## Selected route

Structural — public reference-grammar and Behavior Contract surface changes with unresolved observable behavior; no unverified empirical premise.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — catalog format, schema, and marking behavior unresolved (T4 no) |
| evidence.md, probe.* | prove-it-prototype | N/A — no unverified premise (T1 no) |
| design.md | falsifiable-design | required — Structural route |
| plan.md | budgeted-plan | required — Structural route |

Oracle checkpoint in `checkpointed-build`: required — Structural route

## Downstream sequence

interrogated-spec → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Structural — every downstream artifact satisfies its owning stage's completion criterion, ending with no FAIL in checkpointed-build's recorded gate.
