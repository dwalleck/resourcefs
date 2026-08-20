# Route: rfs-cgbq

Change: Resolve safe multi-root Workspace References across launch and MCP client roots
Date: 2026-08-18

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The implementation depends on current `rmcp` 3.1.3 support for advertising Roots capability, obtaining the initialized client's Roots set, and receiving root-set change notifications. `.rfs-87cv/evidence.md` verifies protocol negotiation and tool schemas only; it does not cover Roots. The Windows drive/UNC and canonical containment contract also depends on behavior not exercised by this Linux checkout, including Windows path parsing and reparse-point resolution. No current repository evidence covers either premise. | yes |
| 2 | Structural boundary | The change replaces the single-root `PathReference::parse(input, primary_root)` contract in `resourcefs-core`, the single `FilesystemSource` wiring in `resourcefs-sources`, and the fixed root fields/lifecycle in `resourcefs-mcp`. It adds session root authority and root-change invalidation across all three principal modules, changes public core/source interfaces, and extends the externally observable Path Reference grammar. | yes |
| 3 | Production-scale risk | Workspace-root sets and Path Reference strings are bounded configuration/request inputs. Resolution remains one parse plus lookup and one contained filesystem operation per call; the issue introduces no throughput, concurrency, memory-volume, or production-data-size premise requiring a stress budget. Security correctness is covered by structural and empirical gates rather than a scale gate. | no |
| 4 | Explicit behavior | Given non-empty client MCP Roots, they replace launch authority; otherwise exactly one of profile or CLI roots supplies authority. Given one uniquely selected Primary Workspace Root, relative inputs resolve beneath it; otherwise relative inputs fail, while `rfs://workspace/<root>/<path>` addresses each declared root. Given a workspace Resource, its canonical identity remains `rfs://`; backing `file://` metadata is absent unless explicitly enabled. Given traversal, canonical escape, ambiguity, or a removed root, resolution fails. Given Windows drive/UNC, absolute, `file://`, canonical workspace, and relative inputs, all contained forms resolve deterministically; an existing literal path wins over selector parsing. Given the corpus and fuzz target, accepted and rejected forms remain bounded and non-panicking. Unresolved decisions: deterministic canonical `RootName` assignment for MCP Roots whose optional names are absent, invalid, or duplicated; duplicate/overlapping root handling; the exact Primary Workspace Root rule for single versus multiple client roots; accepted `file://` authorities and percent-decoding rules; cross-platform Windows drive/UNC canonical rendering; selector split/escaping rules needed to test literal precedence; the concrete opt-in surface and output field for backing-path metadata before Server Profiles exist; and what snapshot/reference invalidation state is observable before mutation snapshots exist. | no |

Unknown tests: none

## Selected route

Empirical — current SDK Roots lifecycle and Windows containment premises are unverified, and the public cross-module contract also has unresolved behavior requiring interrogation.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — T4 identifies unresolved root naming, primary selection, URI, selector, metadata, and invalidation behavior |
| evidence.md, probe.* | prove-it-prototype | required — Empirical route (T1 verdict) |
| design.md | falsifiable-design | required |
| plan.md | budgeted-plan | required |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

interrogated-spec → prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records `PASS` for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no `FAIL`.
