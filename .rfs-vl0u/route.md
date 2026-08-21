# Route: rfs-vl0u

Change: Add bounded search and glob discovery across workspace and artifact Resources
Date: 2026-08-20

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The design requires Rust-regex-first matching with PCRE2 fallback, gitignore/hidden traversal, prompt cancellation, and bounded continuation over large result streams. `Cargo.toml` and all crate manifests currently contain no regex, PCRE2, glob, ignore, or walker dependency, and the repository has no search/glob implementation or current evidence covering the fallback, traversal, cancellation, or streaming behavior of candidate libraries. Those existing-system and external-library premises must be measured against the selected versions before design. | yes |
| 2 | Structural boundary | The current public `SourceAdapter` trait exposes only `read`; `resourcefs-core` exports only read/session/resource contracts; `FilesystemSource` and `ArtifactSource` implement only reads; and the MCP server exposes and dispatches only `rfs_read`. Search/glob therefore add public source-neutral request/result contracts, adapter capabilities, implementations in both source adapters, rendered MCP schemas, and tool routing across all three crates. | yes |
| 3 | Production-scale risk | Search and glob operate over potentially large directory trees and up-to-64-MiB Path Session artifacts. Deterministic bounds, no eager whole-result materialization, cancellation latency, spill quota behavior, and output size make latency, memory, concurrency, and data-volume budgets observable requirements. | yes |
| 4 | Explicit behavior | The issue fixes the feature envelope but not a complete observable contract. Unresolved decisions include: how literal, Rust-regex, and PCRE2 patterns are selected and reported; match grouping and line/column fields; path-target grammar; defaults and precedence for case/hidden/gitignore/skip/pagination/limits; filesystem versus artifact glob semantics; continuation and lossless Recovery Reference contents; behavior for binary or invalid UTF-8 Resources; stable cancellation and invalid-pattern categories/messages; and ordering across roots and adapters. | no |

Unknown tests: none

## Selected route

Empirical — required behavior is structurally cross-cutting and depends on unverified library/runtime behavior under bounded traversal and cancellation.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | PASS — terminal criterion recorded below |
| spec.md | interrogated-spec | PASS — approved observable contract |
| evidence.md, probe.* | prove-it-prototype | PASS — P1–P4; P5–P6 are owned N/A |
| design.md | falsifiable-design | PASS — approved C1–C18 claim matrix |
| plan.md | budgeted-plan / checkpointed-build | PASS — six checkpointed slices and final integration |

Oracle checkpoint in `checkpointed-build`: PASS — final probes/oracles agree and every C1–C18 fence is green.

## Downstream sequence

interrogated-spec → prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records PASS for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no FAIL.

## Completion

PASS (2026-08-20) — the empirical premises remain PASS, every required artifact satisfies its owning stage, all six checkpointed slices and the final assembled integration record no FAIL, the full Rust workspace and strict Clippy are green, and a real stdio MCP session exercised read/search/glob successfully.
