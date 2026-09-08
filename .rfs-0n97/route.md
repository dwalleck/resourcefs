# Route: rfs-0n97

Change: Deliver deployment-qualified PR Fact Resources through the existing public source and MCP read seams.
Date: 2026-09-08

## Request and isolation

Requester: "claim and implement rfs-0n97 in an isolated worktree".

Claimed in the authoritative primary-checkout Rivets store as in_progress, assignee omp. Active worktree: `/home/dwalleck/repos/resourcefs-wt-rfs-0n97-impl`; branch `feat/rfs-0n97`; source base `7de8d1477af6be825bada9015a6996b3f2917c7d`. Git reports this checkout's own top level and the primary repository's shared Git common directory. Use this checkout's `target` directory exclusively; never share build outputs with another revision. The earlier interrupted directory lacks Git metadata and remains untouched.

The approved eight-slice breakdown assigns only PR facts and their necessary first-read contract to this ticket. The shared behavior is recorded in the sibling documentation worktree's `docs/github-resource-contract-spec.md`, its Q1–Q41 ledger, and ADR-0007. Exact proposed schema/input spellings are design choices to present for approval, not a reason to repeat settled behavior questions. The original review's two corrections are accepted: continuation is optional for retained pages, and native /diff requires independent media/byte/truncation proof.

## Route tests

| # | Test | Evidence | Verdict |
|---|---|---|---|
| 1 | Empirical premise | Existing provider evidence does not explicitly retain the selected-version PR head/base repository/SHA/nullability observations required for the new projection; P1 compares independent clients on a fixed public PR before design. Current source/transport metadata and recovery capabilities require targeted applicability review (P2/P3). New fact projection, control/error propagation and numerical enforcement are feature/design obligations, not empirical premises; evidence.md classifies them N/A rather than pretending pre-implementation probes can prove them. Corporate ghe.com/native acceptance remains a separate gate, not an implicit claim. | yes |
| 2 | Structural module shape | Public reference grammar, read acquisition inputs, operational-error details and MCP/profile schemas change. Core owns validating reference identities and source-neutral controls/errors; the existing GitHub Source Adapter owns private provider decoding/acquisition/fact projection and deployment configuration; MCP owns boundary mapping/catalog/profile/CLI exposure. Keep core reference/read and MCP server/profile entrypoints as protected dispatch/validation parents, and GitHub mod.rs as a protected orchestration parent; substantial provider facts belong in focused GitHub-owned modules. Existing HTTP substrate/cache remain the only transport/cache implementation. No second provider interface or Cyril ownership move. | yes |
| 3 | Production-scale risk | One logical read must bound physical attempts, deadline/waits, individual/cumulative accepted bodies and complete serialized JSON while retaining provenance. Large PR bodies and protocol recovery can expose allocation/output issues. Final numerical feasibility requires real implementation measurement; local model arithmetic does not establish production enforcement or memory ceilings. | yes |
| 4 | Explicit behavior | The following given/when/then contract adopts the named ticket and approved interview decisions. Exact field/input naming and module placement remain design-approval choices; no new behavioral ambiguity is identified for this first slice. | yes |

Unknown tests: none.

### T4 observable behavior contract

1. **Given** one permitted configured GitHub deployment and repository, **when** an external Rust consumer or real MCP client reads `pr://owner/repo/number/facts`, **then** it receives the same complete bounded owned machine JSON facts through the existing source interface, with catalog grammar and applicable profile/CLI schema/check integration. Human PR/issue projections keep their meaning; no new tool/provider interface is introduced.
2. **Given** a provider PR with native IDs/links, different base/head SHAs and owning repositories, **when** facts are decoded, **then** those actual supplied identities survive distinctly, with null/deleted fork availability explicit. A requested-parent/repository/identity contradiction rejects the read; moving refs and synthetic merge identity cannot substitute.
3. **Given** supplied null, absent optional metadata, empty text or unknown provider enum values, **when** the owned schema is returned, **then** those meanings remain distinct; unsupported schema majors are rejected by the consumer and additive optional facts remain compatible. Missing knowledge is explicit rather than fabricated.
4. **Given** acquisition and upstream metadata, **when** facts are returned, **then** configured source/deployment, requested and observed native identity, original links, acquisition interval, selected REST version and supplied upstream times/versions remain separate. VersionTag hashes exact representation bytes, not commit identity or a whole-PR snapshot.
5. **Given** lower-only caller controls and operator ceilings, **when** a logical facts read executes, **then** zero/above-hard-ceiling controls fail before acquisition and valid controls intersect operator policy. At most ten physical attempts including parents/revalidation/retries and thirty seconds across send/body/wait are allowed; per-response accepted body is at most 8 MiB, cumulative accepted bodies and serialized JSON at most 16 MiB each. Outcome/provenance overhead counts. These are not wire-byte or peak-memory promises; implementation measurement must establish enforcement without silently raising them.
6. **Given** denied/hidden/not-found/rate/malformed/oversized/transport failures, **when** no usable fact Resource can be returned, **then** the existing ResourceError supplies stable category and bounded sanitized structured details. A 404 is not asserted absence, a denial is not asserted expired authentication, and control flow does not parse message text. Applicable retry guidance consumes the same deadline/attempt budget; stale cache cannot rescue failed revalidation.
7. **Given** cancelled authority or an explicit operation cancellation, **when** the current read would otherwise complete, **then** it does not publish a late accepted result. Previously completed independent reads remain the consumer's separate results. Collection partial retention/resumption is implemented in rfs-bfwa, not smuggled into this singular PR facts read.
8. **Given** configured enterprise web `SUBDOMAIN.ghe.com` and API `api.SUBDOMAIN.ghe.com`, **when** configuration is validated before credentialed requests, **then** conflicting pairing is rejected while existing supported public-GitHub configuration remains usable. Explicit repository/fork/origin and credential ownership apply to every request/indirection; links never grant authority. Principal changes require a new Path Session. The embedder owns credentials/sign-in; this slice performs no remote writes.
9. **Given** a fact document larger than inline display limits but within acquisition/representation ceilings, **when** library and MCP consumers read it, **then** library content is complete and MCP output recovery reconstructs the exact acquired JSON before parsing. Output recovery is not new source acquisition, and no generic continuation list is added.
10. **Given** the legacy unpaged PR `/diff`, **when** it is read against an independent upstream oracle, **then** the request uses `Accept: application/vnd.github.diff`, a complete valid UTF-8 native diff body returns byte-exactly and a truncated upstream body is refused. File-patch/tree-comparison checks are not substitutes; this mutable PR-number diff does not establish selected-revision binding.
11. **Given** implemented facts and applicable authorized public/controlled upstreams, **when** reusable acceptance runs, **then** public-library, independent fixture, actual MCP recovery and credential-gated live smoke evidence name their exact source and proof limits. Skipped live rows do not pass; Cyril actual ghe.com/native Windows/credentials/storage/UI acceptance remains separate.

## Selected route

Empirical — new public/schema behavior has provider-shape and bounded-acquisition premises not discharged by the existing source audit or synthetic experiments.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — first-slice behavior fully explicit above, inherited from the named ticket and completed contract interview |
| evidence.md, probe.* | prove-it-prototype | required — empirical premises and independent oracles |
| design.md | falsifiable-design | required — concrete schema/input/error/authority design, module placement and explicit approval |
| plan.md | budgeted-plan | required after approved design; sole owner of checkpointed partition/budget inputs |

Oracle checkpoint in checkpointed-build: required — Empirical route.

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

No production edits before design approval. No commits, pushes, PR creation or tracker closure are implied by entering this route.

## Terminal criterion

Empirical — every empirical premise has PASS evidence, every subsequent artifact meets its owning stage's completion criterion, and checkpointed-build records no FAIL including assembled repository verification and applicable separate live evidence. Not satisfied at routing; the ticket remains in progress.
