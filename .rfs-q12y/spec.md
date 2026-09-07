# Spec: Vet dependencies with cargo-deny and a CI gate

## Current gate entry point — 2026-09-06 supersession

The C5 workflow-mirroring mechanism below is a historical specification, superseded during PR #1 review (F6/F8/F9/F10). CI and maintainers now invoke `python scripts/ci-gates.py`: one owner runs formatting, all-target/all-feature lint, complete functional and release suites, explicitly inventoried ignored production budgets, and dependency vetting. Independent gate failures do not suppress subsequent checks, and any failure makes the entry point fail. Live rows remain excluded. Actual hosted runs establish trigger/matrix behavior; source-text comparisons are not execution evidence. The original decisions and observations below remain historical, not claims that the deleted mirror test still exists.

## Request (verbatim)
> yes, start rfs-q12y

Issue rfs-q12y — "Add supply-chain vetting the workspace currently has none of: a deny.toml restricting licenses to an allow-list and pinning sources to crates.io, plus a CI job running cargo-deny (and cargo-audit or equivalent advisory checking) so a new or updated dependency cannot land unvetted."

Sequencing: scheduled between rfs-g2z9 increment A (complete, `26898e7`) and increment B, so that reqwest's transitive tree — the largest dependency addition this project has made, on the network egress path — enters the workspace under a vetting gate.

## What this is

The workspace has no supply-chain vetting: no `deny.toml`, no CI workflow, no advisory checking, while resolving 221 packages. This change adds a `deny.toml` that constrains licenses, sources, and advisories, and a CI workflow that runs the project's existing gates plus `cargo-deny`. The `deny.toml` half is verifiable now; the workflow half is written and statically checked but cannot execute until a git remote exists.

## Roles

- **ResourceFS maintainer**: adds or upgrades dependencies, and needs an unvetted or badly licensed dependency to fail before it lands rather than being discovered later.
- **Downstream consumer / auditor**: needs the project's license and source posture to be stated in-repo and mechanically enforced, not implied.

## Behavior

### Reject a disallowed license
- **Given**: `deny.toml` declaring the approved allow-list, and a resolved dependency graph.
- **When**: `cargo deny check licenses` runs.
- **Then**: it exits zero when every package's license expression is satisfied by the allow-list, and non-zero naming the offending package and license when one is not. The current 221-package tree exits zero with **no exceptions entries**.

### Reject a non-crates.io source
- **Given**: `deny.toml` pinning sources to crates.io.
- **When**: `cargo deny check sources` runs.
- **Then**: it exits zero for the current tree, and non-zero naming the package when any dependency resolves to a git or path source outside the workspace.

### Reject a vulnerable or yanked dependency
- **Given**: `deny.toml`'s advisory configuration and a fetched advisory database.
- **When**: `cargo deny check advisories` runs.
- **Then**: a package with a security vulnerability, a yanked version, or an unmaintained advisory exits non-zero naming it. An unmaintained advisory is cleared only by adding its RUSTSEC id to `[advisories] ignore` with a written rationale — never by weakening the scope.

### Run the whole gate in one command
- **Given**: the committed `deny.toml`.
- **When**: a maintainer runs `cargo deny check`.
- **Then**: licenses, sources, advisories, and bans are all evaluated and the process exits zero for the current tree.

### Provide a CI workflow that mirrors the local gates
- **Given**: the repository's documented local gates (`cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-features`; `cargo deny check`).
- **When**: the committed workflow file is parsed.
- **Then**: it is valid workflow YAML, triggers on push and pull request, and its job steps invoke exactly those four gates with the same flags — so the workflow cannot silently diverge from what a maintainer runs locally. **Its execution is deferred**: with no git remote configured it has never run, and this spec makes no claim that it does.

## Success criteria

- **Binary / structural / security**: `cargo deny check` exits zero against the current tree with **zero `[[licenses.exceptions]]` entries and zero `[advisories] ignore` entries**, checked by running it.
- **Binary / structural / security**: the allow-list admits exactly the approved set (MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, 0BSD, Unlicense, Unicode-3.0, MPL-2.0) and nothing else; injecting a synthetic GPL-3.0-licensed package into the check fails it, checked by a fixture that runs `cargo deny` against a manifest declaring such a dependency.
- **Binary / structural / security**: a git-sourced dependency fails `cargo deny check sources`, checked by the same fixture mechanism.
- **Binary / structural / security**: the workflow file parses as valid YAML and its step commands are byte-identical to the four documented local gate commands, checked by a test that extracts the `run:` lines and compares them against a literal list — so drift between CI and local gates fails in-repo, without needing CI to execute.
- **Deferred verification (explicitly not claimed)**: that the workflow actually executes, that it fires on push and pull request, and that an unvetted dependency fails it in a real run. No remote exists; these become checkable only after the repository is pushed, and are recorded as deferred rather than asserted.

## Out of scope

This change does NOT include: creating a git remote or publishing the repository; running or validating the workflow in a real CI environment; `cargo vet` or supply-chain attestation beyond license/source/advisory checks; pinning or auditing the reqwest tree that rfs-g2z9 increment B will introduce (this change builds the gate; that change passes through it); vendoring dependencies; a release or publishing workflow; and changes to the workspace `Cargo.toml` or any dependency version.

## Related issues

- **rfs-g2z9** (in progress): motivated this issue. Increment A is complete with zero new dependencies; increment B introduces reqwest 0.13.2 and must pass this gate. The sequencing constraint is recorded in `.rfs-g2z9/plan.md`.
- **rfs-jk0d** (parent epic): DESIGN.md's release-evidence section already assumes CI exists (protected runs, prebuilt binary smoke tests), so this change partially serves that assumption.
- **rfs-58r1** (open): platform distribution; will extend this workflow rather than add a second one.
- **rfs-5os7** (open): the Kiro release gate expects protected CI runs with a secret; this workflow is the place that lands.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| Which licenses are allowed? | MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, 0BSD, Unlicense, Unicode-3.0, **MPL-2.0** — a blanket allow-list with **no per-package exceptions**. | Requester selected "Permissive + MPL-2.0 + Unicode-3.0". MPL-2.0 is file-level copyleft: obligations attach to modified MPL files, not to code that links them, so a blanket allow is normal for a Rust project and keeps `directories` (the session cache-dir dependency, which reaches `option-ext`) working. | The current tree passes with zero exceptions; a future GPL/AGPL dependency fails the gate. A new MPL dependency passes silently — accepted as the cost of no exception churn. |
| How are the five deprecated slash-form SPDX declarations handled? | Accept them as-is; cargo-deny parses `MIT/Apache-2.0` and `Unlicense/MIT` as OR expressions and both operands are allowed. | The declarations belong to upstream crates (`ident_case`, `rustc-demangle`, `version_check`, `same-file`, `walkdir`); rewriting them is not ours to do. | No configuration needed. If cargo-deny later hard-errors on the deprecated form, that surfaces as a gate failure to address then. |
| What is the advisory policy? | **Revision 1 (2026-08-23).** Vulnerabilities and yanked versions **deny**. `unmaintained = "all"` — every crate is examined and any unmaintained advisory **denies** — and a specific advisory is cleared only by an `[advisories] ignore` entry carrying its RUSTSEC id and a written rationale. | The original decision ("unmaintained warns") is **unsatisfiable**: cargo-deny 0.19.0 treats `unmaintained` as a *scope selector* (`all`/`workspace`/`transitive`/`none`), not a severity — `unmaintained = "warn"` is rejected as an unexpected value, and any in-scope hit is a hard error. Verified against a fixture depending on `ansi_term` 0.12.1 (RUSTSEC-2021-0139). Offered the achievable options, the requester chose `"all"` plus documented ignores: it costs nothing today (the real 221-package tree reports `advisories ok` under the broadest scope) and matches how this project handles every other exception — deliberate and written down rather than silent. The original worry was a noisy gate training maintainers to bypass it; an ignore entry with a stated reason is the non-bypassing way to record "we looked at this one." | `cargo deny check advisories` fails on vulnerabilities, yanks, **and** unmaintained crates. A future unmaintained dependency stops the build until someone records why it is acceptable, rather than entering silently. |
| Are sources restricted? | crates.io only; no git or non-workspace path dependencies. | The project already has no git dependencies; pinning prevents one appearing unnoticed. | A deliberate future git dependency requires an explicit config change. |
| Is the CI workflow verified? | **No — explicitly deferred.** It is written, parsed, and its steps are compared against the documented local gates, but it has never executed because no remote exists. | Requester selected "Both now, CI unverified until pushed" after being shown the gap. | The spec restates the two unfalsifiable criteria as in-repo checkable ones and records execution as deferred. A workflow that has never run is usually broken the first time it does; this is stated, not implied. |
| Which platforms does the workflow target? | Linux, macOS, and Windows, mirroring DESIGN.md's cross-platform release evidence. | The project already runs native Windows and macOS cross-checks manually (rfs-r9m6, rfs-73dz); CI is where that becomes routine. | The matrix is written now and proven only when the workflow first runs. |
| Empty set | A tree with no dependencies would trivially pass; not reachable here (221 packages). | N/A. | No special handling. |
| Max scale | 221 packages today; reqwest's tree will add substantially more in rfs-g2z9 increment B. | Measured this session. | The gate must run in reasonable time on the larger tree; increment B re-runs it. |
| Null / missing field | A package declaring **no** license fails the licenses check rather than being skipped. | Default cargo-deny behavior and the safer posture. | None exist in the current tree. |
| Concurrent writes | N/A — configuration files, no runtime state. | No applicable domain object. | None. |
| Permission denied / unauthenticated | Advisory-database fetch may fail offline; the check reports the fetch failure rather than silently passing. | Silent pass would defeat the gate. | Offline runs surface an error, not a false green. |
| Partial failure | Each cargo-deny check (licenses/sources/advisories/bans) reports independently; one failing does not mask another. | cargo-deny default. | A maintainer sees every category's verdict. |
| Retries / idempotency | Checks are pure functions of the lockfile plus the advisory database; re-running yields the same verdict absent a database update. | Deterministic inputs. | Safe to re-run. |
| Soft-deleted records | N/A. | No applicable domain object. | None. |
| Multi-tenancy boundaries | N/A — a single repository's configuration. | No applicable domain object. | None. |
| Time-zone / DST | N/A — no wall-clock value in the contract. | Advisory dates are compared by the tool, not by this contract. | None. |
| Replication lag | N/A — the advisory database is fetched, not replicated by us. | External tool concern. | A stale local database is the tool's freshness concern. |
| Cache invalidation | The advisory database is refreshed by cargo-deny on fetch; no cache is owned by this change. | External tool concern. | None. |

## Approval

Requester approval (verbatim): "I agree"
Date: 2026-08-23

Confirmed at sign-off without objection: acceptance of the five upstream deprecated slash-form SPDX declarations as-is, the Linux/macOS/Windows matrix, and the restatement of the two unfalsifiable CI criteria as in-repo checks with execution recorded as deferred verification.

Revision 1 — queue reopened 2026-08-23 when implementation found the signed advisory decision unsatisfiable by cargo-deny 0.19.0 (`unmaintained` is a scope selector, not a severity; `"warn"` is rejected as an unexpected value). The requester was shown the achievable settings and chose `"all"` plus documented `ignore` entries. No other decision changed.

Requester approval (verbatim): "\"all\" + documented ignores"
Date: 2026-08-23

Revision 2 — queue reopened 2026-08-23 during rfs-g2z9 Slice 3, the first dependency addition to face this gate. Adding reqwest 0.13.2 produced exactly one rejection: `webpki-root-certs` v1.0.9 (the Mozilla trusted-root CA bundle, reached via `rustls-platform-verifier`) declares **CDLA-Permissive-2.0**, a permissive *data* license with no copyleft reach into code that ships the data. It is unavoidable on this path — both of reqwest 0.13.2's rustls feature routes include `rustls-platform-verifier`, and the only rustls-free alternative (native-tls) would link system OpenSSL on Linux and break the static distribution this project relies on. Advisories, bans, and sources all passed; this was the sole failure. Offered a blanket allow-list entry, a targeted per-package exception, or switching to native-tls, the requester chose the allow-list entry — preserving the zero-exceptions posture chosen in the original sign-off. The approved set is therefore **twelve** identifiers; `APPROVED_LICENSES` in `crates/resourcefs-mcp/tests/supply_chain_contract.rs` was updated in the same commit so C1's fence still pins the exact signed list. The success criterion is unchanged and still holds: zero `[[licenses.exceptions]]` entries and zero `[advisories] ignore` entries.

Requester approval (verbatim): "Add CDLA-Permissive-2.0 to the allow-list"
Date: 2026-08-23
