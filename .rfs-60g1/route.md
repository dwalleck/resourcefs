# Route: rfs-60g1

Change: Use Session Scratch as independently writable state (the `local://` Local Source Adapter)
Date: 2026-08-23

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | `local://` reuses only already-verified in-repo infrastructure: the source-neutral `MutationEngine`/`MutationAdapter{resolve,load,commit}` seam (rfs-73dz, proved on Linux + native Windows), `DiskSessionStorage` with the 64 MiB object / 256 MiB session quotas, digest dedupe, disconnect invalidation and TTL cleanup (rfs-34pz), and the `DiscoveryAdapter` search/glob engine (rfs-vl0u). The session-storage write path is already exercised by `artifact://` retention. No external system, OS-contract unknown, or stale evidence — the only "unknown" is whether the mutation seam generalizes off the filesystem, which is a design claim to falsify, not an empirical premise about existing/external behavior. | no |
| 2 | Structural boundary | Yes. Adds the `local://` scheme to the public Path Reference grammar (`resourcefs-core/src/reference.rs`, `ResourceAddress`); adds a new Local Source Adapter module in `resourcefs-sources` registered in `CompiledSources`; implements the `MutationAdapter` seam a second time (first non-filesystem impl) with Path-Session authority and quota policy instead of Workspace grants; adds a `SourceCatalogMetadata` entry so `local://` self-lists in `rfs://` (a catalog test currently asserts its absence). Placement of the adapter, its authority/quota wiring, and its relationship to `DiskSessionStorage` are cross-module decisions. | yes |
| 3 | Production-scale risk | No new dimension. Object/session quota accounting, per-canonical-Resource mutation serialization, and per-Path-Session isolation are inherited from rfs-34pz and rfs-73dz and already carry budgets and fences; scratch adds no new latency/throughput/data-volume surface beyond the existing 64 MiB/256 MiB ceilings. The design still owns concurrent-session-isolation and quota-exhaustion stress fixtures required by the acceptance criteria, but that is inherited machinery, not a new scale premise. | no |
| 4 | Explicit behavior | No — unresolved observable decisions requiring interrogation: (a) `local://` namespace shape — flat names (`local://<name>`) vs hierarchical paths (`local://<a/b/c>`), which cascades into glob, MV, and containment; (b) directory/listing and glob semantics over scratch; (c) what "same-source MV" means for scratch and whether `local://`↔workspace moves are rejected as cross-source; (d) scratch name/path grammar and validation (allowed characters, length, percent-encoding); (e) how `rfs_search` targets scratch; (f) coexistence with immutable `artifact://` in one shared Path Session and shared quota; (g) whether the numbered read projection and Structural Summary apply to scratch text; (h) create-vs-replace via `ifVersion` and seen-region discipline for scratch edits. | no |

Unknown tests: none

## Selected route

Structural — public Path Reference grammar plus a new source-adapter module and the first non-filesystem `MutationAdapter` implementation, with unresolved observable behavior; no unverified empirical premise (all reused infrastructure is proven in-repo).

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — T4 no: namespace shape, MV/search/glob semantics, name grammar, and artifact coexistence are unresolved |
| evidence.md, probe.* | prove-it-prototype | N/A — T1 no: no unverified premise; the mutation, storage, and discovery infrastructure is already proved in-repo (rfs-73dz/34pz/vl0u) |
| design.md | falsifiable-design | required — Structural route |
| plan.md | budgeted-plan | required — Structural route |

Oracle checkpoint in `checkpointed-build`: required — Structural route

## Downstream sequence

interrogated-spec → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Structural — every downstream artifact satisfies its owning stage's completion criterion, ending with no FAIL in checkpointed-build's recorded gate.
