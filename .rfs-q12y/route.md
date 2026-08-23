# Route: rfs-q12y

Change: Vet the dependency tree with cargo-deny and add a CI gate
Date: 2026-08-23

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | No. The design rests on the current tree's license/source composition, and that was **measured directly this session** (current repository evidence, not an assumption): `cargo metadata` resolves **221 packages**; licenses are 136 `MIT OR Apache-2.0`, 22 `MIT`, 18 `Unicode-3.0`, 11 `Apache-2.0 OR MIT`, 11 `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`, 7 `Unlicense OR MIT`, 5 `Apache-2.0`, 3 `MIT/Apache-2.0`, 2 `Unlicense/MIT`, 1 `0BSD OR MIT OR Apache-2.0`, 1 `MIT OR Zlib OR Apache-2.0`, and **1 `MPL-2.0` (`option-ext` 0.2.0, reached via `directories`)**. `cargo-deny` is installed at `~/.cargo/bin/cargo-deny`, so its verdict against this tree is directly observable at implementation time rather than being a premise the design rests on. No external service or unverified library behavior is involved. | no |
| 2 | Structural boundary | No. `deny.toml` and a CI workflow are project configuration: no crate public API, no schema, no module boundary, and no cross-module placement decision. The workspace's `Cargo.toml` is not modified. | no |
| 3 | Production-scale risk | No. The gate runs in CI and on demand, not in the served request path; it introduces no latency, throughput, memory, concurrency, or data-volume exposure to the running server. | no |
| 4 | Explicit behavior | **No — unresolved decisions requiring interrogation.** (a) **License policy is the requester's call**, not a default: whether to allow the single `MPL-2.0` dependency (`option-ext`, file-level copyleft reached through `directories`, which the session cache directory depends on), whether `Unicode-3.0` is acceptable, and how to treat the five packages declaring the deprecated slash-form SPDX (`MIT/Apache-2.0`, `Unlicense/MIT`) that strict parsers reject. (b) **The CI half is not verifiable in this repository**: there is **no git remote configured**, so a workflow can be written and syntax-checked but never observed to execute — two of the issue's acceptance criteria ("a CI workflow runs on push and pull request", "adding a dependency without vetting fails CI") are unfalsifiable as written and must be restated as checkable claims plus recorded deferred verification. (c) advisory policy (deny vs warn for unmaintained/yanked) and the CI job matrix are undecided. | no |

Unknown tests: none

## Selected route

Structural — no unverified empirical premise (the tree was measured and the tool is installed) and no public API or scale risk, but the license policy is a requester decision and the CI half's acceptance criteria are unfalsifiable in a repository with no remote, so both need interrogation before anything is built. Precedence: Empirical > Structural > Local; T4 fires.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — T4 no: license policy, advisory policy, and the restatement of the unverifiable CI criteria are unresolved |
| evidence.md, probe.* | prove-it-prototype | N/A — T1 no: the tree composition is current repository evidence measured this session, and `cargo-deny`'s verdict is locally observable rather than an unverified premise |
| design.md | falsifiable-design | required — Structural route |
| plan.md | budgeted-plan | required — Structural route |

Oracle checkpoint in `checkpointed-build`: required — Structural route

## Downstream sequence

interrogated-spec → falsifiable-design → budgeted-plan → checkpointed-build

## Scope note (requester decision, 2026-08-23)

The requester chose to build **both halves now**, accepting that the CI workflow is written and statically checked but **never executed** until a remote exists. The spec must therefore restate the two unfalsifiable criteria as claims that are checkable today — the workflow file parses and its job steps match the documented local gates — and record "has actually run against a real push/PR" as explicitly deferred verification rather than asserting it. A workflow that has never executed is usually broken the first time it does; the artifacts must say so rather than imply a working gate.

## Terminal criterion

Structural — every downstream artifact satisfies its owning stage's completion criterion, ending with no FAIL in checkpointed-build's recorded gate.
