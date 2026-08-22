# Route: rfs-73dz

Change: Mutate workspace text with versioned writes and hashline edits
Date: 2026-08-22

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The acceptance criterion requires atomic filesystem commits on Linux, macOS, and Windows. The repository has atomic session-artifact storage (`SessionStorage::write_atomic` in `crates/resourcefs-core/src/session.rs`) but no workspace mutation implementation or platform evidence for safely replacing an existing file while preserving stale-state checks and same-source move semantics. The design therefore depends on unverified filesystem and library behavior on all three target platforms. | yes |
| 2 | Structural boundary | The fixed public MCP surface currently exposes only `rfs_read`, `rfs_search`, and `rfs_glob` through `ResourceFsServer` in `crates/resourcefs-mcp/src/server.rs`; adding `rfs_write` and `rfs_edit` changes public tool schemas and dispatch. `SourceAdapter` in `crates/resourcefs-core/src/source.rs` is read-only, while mutation authority is held by `MutationGrants` in `resourcefs-sources`, so the change also requires a source-neutral mutation seam and coordinated placement across core, sources, and MCP. | yes |
| 3 | Production-scale risk | Whole-resource Version Tag verification and atomic replacement touch file-size-dependent I/O and memory; the requested per-canonical-Resource serialization introduces concurrency, lock-lifetime, and contention behavior. Existing workspace reads are bounded, but no mutation budget, stress fixture, or lock registry exists. | yes |
| 4 | Explicit behavior | Given a missing workspace text Resource, when `rfs_write` supplies UTF-8 content and create authority, then it creates the Resource; given an existing text Resource, when `rfs_write` supplies its current Version Tag and update authority, then it atomically replaces it; given a prior read snapshot and exact Version Tag, when a valid hashline `PUT` or `CUT` targets only displayed regions, then the requested edit commits; given `REM`, then deletion is the only accepted deletion form; given same-source `MV` with delete/create authority, then it moves, while cross-source `MV` is rejected; given stale, unseen, malformed, conflicting, unauthorized, catalog, or concurrent mutation input, then authoritative content is unchanged and a stable error is returned. Unresolved decisions remain: exact patch grammar/header binding, which prior read views constitute editable seen regions, error-category mapping and precedence, move destination/version conflict rules, symlink and non-regular-file behavior, cancellation boundary, and exact atomic/durability guarantees. | no |

Unknown tests: none

## Selected route

Empirical — cross-platform atomic filesystem replacement is an unverified system premise; the change also alters public schemas and module boundaries and has concurrency/scale risk.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — T4 has unresolved observable behavior |
| evidence.md, probe.* | prove-it-prototype | required — Empirical route and T1 verdict |
| design.md | falsifiable-design | required |
| plan.md | budgeted-plan | required |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

interrogated-spec → prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records PASS for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no FAIL.
