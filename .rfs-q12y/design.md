# Design: dependency vetting with cargo-deny and a CI gate

## Route and inputs

- **Route:** Structural, from `.rfs-q12y/route.md` — no unverified empirical premise (the tree was measured and `cargo-deny` 0.19.0 is installed), no public API or scale risk, but the license policy was a requester decision and the CI half's criteria needed restating.
- **Behavior source:** `.rfs-q12y/spec.md`, signed 2026-08-23. Its Decisions table is authoritative: the eleven-license blanket allow-list with **zero exceptions**; crates.io-only sources; vulnerabilities and yanked deny, unmaintained non-blocking; a Linux/macOS/Windows workflow triggering on push and pull request whose steps are the four documented local gates; and the workflow's **execution explicitly deferred, never claimed**.
- **Empirical premises:** N/A — `route.md` T1 = no. The 221-package tree composition was measured directly and `cargo-deny`'s verdict is locally observable.
- **Evidence/probe input:** N/A — no probe on a Structural route.
- **Facts established while designing** (recorded so later stages need not rediscover them): with no config, `cargo deny check` reports `advisories ok, bans ok, licenses FAILED, sources ok` — so advisories and sources are already clean and only the allow-list is missing. Both "gate bites" fixtures were proven runnable **offline**: a path dependency declaring `GPL-3.0` fails `check licenses`, and a dependency sourced from a local bare repository via a `file://` URL fails `check sources` with exit 8.

## Input shapes

| Input | Production-reachable shapes | Status |
|---|---|---|
| `deny.toml` licenses section | the eleven approved identifiers; a twelfth (disallowed) identifier; zero vs non-zero `[[licenses.exceptions]]` | Covered by C1, C2 |
| Dependency graph — licenses | the real 221-package tree; a graph containing a `GPL-3.0` package; the five deprecated slash-form declarations (`MIT/Apache-2.0`, `Unlicense/MIT`); the single `MPL-2.0` (`option-ext` via `directories`); the 18 `Unicode-3.0` | Covered by C1, C2 |
| Dependency graph — sources | the real tree (crates.io only); a graph containing a git-sourced dependency | Covered by C1, C3 |
| Dependency graph — bans | the real tree, whose three workspace-internal path dependencies carry no version requirement | Covered by C1 |
| `deny.toml` advisories section | `yanked` severity; `unmaintained` severity; the vulnerability posture | Covered by C4 |
| Dependency graph — advisories | the real tree (currently clean) | Covered by C1; a graph *bearing* a live advisory is `N/A — reason`: the advisory database is external and mutable, so no fixture can pin a vulnerable package without the test rotting when the database changes. C4 fences the configuration that governs the response instead. |
| Workflow file | the committed file's trigger keys; its four gate `run:` commands; a fifth/absent/reworded command; tab characters | Covered by C5 |
| Workflow execution | a real push or pull-request run | `N/A — reason`: no git remote exists; the requester chose to accept deferred verification (spec Decisions). Recorded as deferred in Non-goals, not claimed. |

## Removed invariants

Purely additive. The change introduces two configuration files and one test file; it removes no guard, serialization point, uniqueness rule, ordering guarantee, precondition, or existing behavior. It does not modify the workspace `Cargo.toml`, any dependency version, or any crate source.

## Placement

### `deny.toml`
- **Owner:** the workspace root. `cargo-deny` discovers `deny.toml` there by default, so any other location would require every invocation (local and CI) to pass `--config`, creating two ways to run the gate.
- **New seam:** none — a tool configuration file.
- **Forbidden:** `[[licenses.exceptions]]` entries (the signed decision is a blanket allow-list with none); a second copy of the configuration anywhere; `--config` overrides in the committed workflow.

### CI workflow
- **Owner:** `.github/workflows/ci.yml`. GitHub Actions' fixed discovery path; the directory does not exist yet and is created by this change.
- **New seam:** none.
- **Forbidden:** gate commands that differ in any byte from the documented local gates; a claim anywhere in the repository that the workflow has run or is verified.

### Fences
- **Owner:** new `crates/resourcefs-mcp/tests/supply_chain_contract.rs`, beside the existing contract tests and following `architecture_contract.rs`'s established pattern of resolving the workspace root through `cargo_metadata` and reading repository files directly.
- **New seam:** none — it fills the existing integration-test harness. `tempfile` is already a dev-dependency of `resourcefs-mcp`, so the synthetic-graph fixtures need no new dependency.
- **Forbidden:** adding any dependency to run the fences (no YAML crate, no HTTP, no process helpers beyond `std::process::Command` invoking the already-installed `cargo-deny`); duplicating the four gate command strings anywhere except the single literal list in this file; asserting workflow *execution*.

## Claims

- **C1:** The committed `deny.toml` carries the eleven approved license identifiers, **zero `[[licenses.exceptions]]` entries, and zero `[advisories] ignore` entries**, and `cargo deny check` — licenses, sources, advisories, and bans together — exits zero against the real workspace tree, so the committed baseline is provably clean rather than clean by assertion.
- **C2:** The license gate bites: a dependency graph containing a `GPL-3.0` package fails `cargo deny check licenses` with a non-zero exit under the committed configuration.
- **C3:** The source gate bites: a dependency graph containing a git-sourced package fails `cargo deny check sources` with a non-zero exit under the committed configuration.
- **C4:** The committed advisory configuration is exactly the signed policy — yanked versions deny and `unmaintained = "all"`, so every unmaintained advisory denies — and the scope is never weakened to `"transitive"` or `"none"`, so a future unmaintained dependency stops the build until an `[advisories] ignore` entry records its RUSTSEC id and a written rationale.
- **C5:** The committed workflow declares both `push` and `pull_request` triggers and invokes the four documented local gate commands byte-for-byte, so CI cannot silently diverge from what a maintainer runs locally.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Allow-list content + clean baseline + whole gate green on the real tree | the committed `deny.toml`; the real 221-package graph | Parse `deny.toml` and compare its `allow` list, `[[licenses.exceptions]]` count, and `[advisories] ignore` count to a literal expectation (eleven identifiers, zero, zero), then run `cargo deny check` and assert exit zero; any missing/extra identifier, any exceptions or ignore entry, or a non-zero exit falsifies | The **literal eleven-identifier list transcribed from `spec.md`'s Decisions table**, plus `cargo-deny`'s own process exit status — a different mechanism from the test's own parsing, since the tool independently resolves and evaluates the graph | Remove `"MPL-2.0"` from `deny.toml`'s allow list; `supply_chain_contract::deny_config_matches_signed_policy` turns red on the list comparison **and** `cargo deny check` turns red on `option-ext` | `crates/resourcefs-mcp/tests/supply_chain_contract.rs::{deny_config_matches_signed_policy, workspace_passes_the_gate}` (created this change) | ~30 s | **PASS** |
| C2 | License gate bites | a synthetic graph containing a `GPL-3.0` path dependency | Build a temp project whose path dependency declares `license = "GPL-3.0"`, run `cargo deny check licenses` against it with the committed config, and assert a non-zero exit; a zero exit falsifies | **`cargo-deny`'s process exit status against a hand-built manifest** — the fixture is authored from the SPDX identifier, not derived from the production config's parsing | Add `"GPL-3.0"` to the committed allow list; `supply_chain_contract::license_gate_rejects_copyleft` turns red because the synthetic graph now passes | `crates/resourcefs-mcp/tests/supply_chain_contract.rs::license_gate_rejects_copyleft` (created this change) | ~20 s | PENDING — checkpointed-build, slice assigned in `plan.md` |
| C3 | Source gate bites | a synthetic graph containing a git-sourced dependency | Initialise a local git repository, depend on it by `file://` URL from a temp project, run `cargo deny check sources` with the committed config, and assert a non-zero exit; a zero exit falsifies | **`cargo-deny`'s process exit status against a real cargo-resolved git source** (proven offline during design: exit 8) — independent of the test's own config parsing | Set `unknown-git = "allow"` in the committed `deny.toml`; `supply_chain_contract::source_gate_rejects_git` turns red because the synthetic git graph now passes | `crates/resourcefs-mcp/tests/supply_chain_contract.rs::source_gate_rejects_git` (created this change) | ~20 s | PENDING — checkpointed-build, slice assigned in `plan.md` |
| C4 | Advisory policy matches the signed decision, scope never weakened | the committed `deny.toml` advisories section; the three rejected scope values | Parse the advisories section and compare `yanked` and `unmaintained` to a literal expectation from `spec.md` Revision 1, asserting `unmaintained == "all"` and explicitly that it is neither `"transitive"` nor `"none"`; any deviation falsifies | The **literal policy transcribed from `spec.md` Revision 1's Decisions row** ("vulnerabilities and yanked deny; `unmaintained = "all"`; cleared only by a documented `ignore`"), independent of the config file being read | Change `unmaintained` from `"all"` to `"transitive"` in `deny.toml` — the exact weakening the signed decision forbids; `supply_chain_contract::advisory_policy_matches_signed_policy` turns red | `crates/resourcefs-mcp/tests/supply_chain_contract.rs::advisory_policy_matches_signed_policy` (created this change) | <1 s | PENDING — checkpointed-build, slice assigned in `plan.md` |
| C5 | CI mirrors the local gates and cannot drift | the committed workflow file; its triggers; its `run:` commands; tab characters | Read the workflow file, extract every gate command it runs and its trigger keys, and compare to a literal expectation; a missing/extra/reworded command, a missing trigger, or a tab character falsifies | The **four gate commands transcribed literally from `spec.md`'s Behavior section** (and cross-checked against `AGENTS.md`/session practice), independent of the workflow file being parsed | Change the clippy step's `--all-features` to `--features test-support` in the workflow; `supply_chain_contract::ci_workflow_mirrors_local_gates` turns red on the command comparison | `crates/resourcefs-mcp/tests/supply_chain_contract.rs::ci_workflow_mirrors_local_gates` (created this change) | <1 s | PENDING — checkpointed-build, slice assigned in `plan.md` |

## Non-goals and future work

### Permanent non-goals (rationale recorded; no tracker issue)
- **Fencing that a live advisory fails the gate.** The advisory database is external and mutable; any fixture pinning a vulnerable package rots when the database changes or the package is patched. C4 fences the governing configuration instead, which is the stable, in-repo part.
- **Adding a YAML parser to validate the workflow.** No YAML crate is a workspace dependency and this change adds none; C5's fence checks the substantive properties (gate-command drift, triggers, tabs) without one. One-time YAML validity is confirmed at authoring time and recorded in the falsifier run log.
- **Creating a git remote or publishing the repository.** Outward-facing and the requester's decision alone; recorded in `spec.md`'s Out of scope.

### Intended future work (verified tracker IDs)
- **rfs-g2z9** (in progress): increment B introduces reqwest 0.13.2 and must pass this gate; the sequencing constraint is recorded in `.rfs-g2z9/plan.md`.
- **rfs-58r1** (open, "Distribute ResourceFS across supported platforms"): two items belong there rather than here. (a) **Workflow execution** — the deferred verification that this workflow actually runs on push and pull request, checkable only once a remote exists. (b) **A publishability defect surfaced while designing**: the three workspace crates are publishable (no `publish = false`) but their internal path dependencies carry no version requirement, which `cargo-deny` reports as wildcard dependencies and which **crates.io rejects outright**. DESIGN.md promises `cargo install` distribution, so this must be fixed before publishing; it requires editing the workspace `Cargo.toml`, which this change is scoped out of.

## Advisory-scope investigation and its resolution (2026-08-23)

The originally signed spec fixed "unmaintained **warns** … produces a warning without failing". **cargo-deny 0.19.0 cannot express that**, which was surfaced to the requester rather than self-served. `spec.md` **Revision 1** resolved it: `unmaintained = "all"` with any advisory cleared **only** by an `[advisories] ignore` entry carrying its RUSTSEC id and a written rationale — never by weakening the scope. C4 above fences exactly that, including explicit assertions against the two rejected scope values. The investigation is retained below because it is the evidence the decision rests on.

Verified empirically against a fixture depending on `ansi_term` 0.12.1 (RUSTSEC-2021-0139, unmaintained):

| `[advisories] unmaintained` | Result | Exit |
|---|---|---|
| `"all"` | `advisories FAILED` | 1 |
| `"workspace"` | `advisories FAILED` | 1 |
| `"transitive"` | `advisories ok` | 0 |
| `"none"` | `advisories ok` | 0 |

`unmaintained` is a **scope selector**, not a severity: it chooses *which* crates are examined, and any in-scope unmaintained advisory is a hard error. There is no warn-level setting — neither in the config schema (the 0.19 `cargo deny init` template exposes only `db-path`, `db-urls`, `ignore`, and `git-fetch-with-cli` under `[advisories]`) nor as a CLI override (`cargo deny check --help` offers no per-diagnostic severity flag; `--log-level` only filters message emission). Vulnerabilities are always-deny with no field, which does match the signed decision.

Note the scope semantics are not intuitive: `"workspace"` **failed** while `"transitive"` **passed** for the same directly-depended-upon crate, so the values select by graph position rather than by depth in the way the names suggest.

**This was a specification decision, not an implementation choice, so it was not self-served.** The achievable options were `"none"` (never checked), `"transitive"` (blocks only on deeper crates), and `"workspace"`/`"all"` (blocks).

**Requester resolution (Revision 1):** `"all"` — every crate examined, every unmaintained advisory denies — with clearance only via a documented `ignore` entry. The rationale recorded in the spec: it costs nothing today (the real 221-package tree reports `advisories ok` under the broadest scope, so **every option passed at the time of choosing**), and it matches how this project handles every other exception — deliberate and written down rather than silent. A future unmaintained dependency stops the build until someone records why it is acceptable.

The same revision tightened C1's baseline criterion to **zero `[[licenses.exceptions]]` entries AND zero `[advisories] ignore` entries**, so the committed baseline is provably clean rather than clean by assertion — and so the first use of the `ignore` escape hatch is a visible, reviewable change rather than an edit to an already-populated list.

## Falsifier run log

- 2026-08-23 — **C1 cheapest falsifier: PASS.** `cargo deny check --config <candidate>` against the real workspace tree → `advisories ok, bans ok, licenses ok, sources ok`, **exit=0**, with zero `[[licenses.exceptions]]` entries. This confirms `spec.md`'s central assertion that the approved allow-list accepts the current 221-package tree unmodified. Reaching exit zero required two configuration findings recorded here so the build stage does not rediscover them: `unused-allowed-license = "allow"` silences three unavoidable "license was not encountered" warnings for allow-list headroom (BSD-2-Clause, BSD-3-Clause, ISC are approved but not currently present), and `wildcards` must be `"allow"` because the workspace's three internal path dependencies carry no version requirement — `allow-wildcard-paths = true` does **not** help, as cargo-deny states it "does not apply to public crates as crates.io disallows path dependencies". External dependencies are version-pinned in `[workspace.dependencies]` (several with `=` exact pins), so allowing wildcards forfeits nothing here; the underlying publishability defect is routed to rfs-58r1 above.
- 2026-08-23 — Fixture mechanisms proven runnable **offline** before being promised as falsifiers: the `GPL-3.0` path-dependency graph fails `check licenses`, and a `file://` local-bare-repository dependency fails `check sources` with exit 8.

## Approval

Requester approval (verbatim): `"all" + documented ignores` (spec Revision 1 sign-off 2026-08-23; design adds no decision beyond the signed spec)
Date: 2026-08-23
Approved risk acceptances: None (no `N/A — approved risk` rows in the Falsification table).

Revision history: the originally signed advisory policy ("unmaintained warns") proved unsatisfiable in cargo-deny 0.19.0; the finding was surfaced rather than self-served, `spec.md` Revision 1 settled it as `"all"` plus documented ignores, and C1 and C4 were amended accordingly. No other claim changed.
