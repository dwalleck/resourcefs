# Plan: dependency vetting with cargo-deny and a CI gate

## Inputs and partition arithmetic

- Route: Structural (`.rfs-q12y/route.md`); `spec.md` signed 2026-08-23; `design.md` approved 2026-08-23 with no risk acceptances. Claims C1–C5; C1 is **PASS** (cheapest falsifier already run), C2–C5 are `PENDING — checkpointed-build`.
- Slice diff estimates: S1 165 + S2 180 + S3 140 = **485 changed lines**.
- Churn margin: **25% = 121 lines**. Rationale: the project's recent slices have run up to 24% over first decomposition (rfs-g2z9 increment A: 989 actual against 800 projected), driven by fixture scaffolding — and two of these three slices are almost entirely fixture code.
- Projected total: **606 changed lines** — far below the 4,000-line review-size gate, so the plan has **one PR increment: "Dependency vetting"**.

### PR increment: Dependency vetting

Slices 1–3. Mergeable definition against the repository default branch (`main`): `deny.toml` exists at the workspace root and `cargo deny check` exits zero for the current tree; the gate is proven to reject a disallowed license and a git source; and a CI workflow exists whose gate commands cannot drift from the documented local gates. Verifies entirely through the existing `resourcefs-mcp` integration-test harness plus the already-installed `cargo-deny`; adds no dependency and modifies no crate source. Depends on nothing later; **rfs-g2z9 increment B depends on this increment** per that change's recorded sequencing constraint.

> **Unblocked 2026-08-23.** The originally signed advisory policy ("unmaintained warns") proved unsatisfiable in cargo-deny 0.19.0 — `unmaintained` is a scope selector, not a severity — and was surfaced to the requester rather than self-served. `spec.md` **Revision 1** settled it: `unmaintained = "all"`, cleared only by an `[advisories] ignore` entry carrying a RUSTSEC id and a written rationale, never by weakening the scope. The same revision tightened C1's baseline to zero `[[licenses.exceptions]]` **and** zero `[advisories] ignore` entries. Slice 1 below reflects both; C2, C3, C5 and the arithmetic are unaffected.

## Slice 1: Add deny.toml and fence it against the signed policy

**Claim IDs:** C1, C4

**Expected behavior:** `deny.toml` exists at the workspace root declaring the eleven approved license identifiers with **zero `[[licenses.exceptions]]` and zero `[advisories] ignore` entries**, crates.io-only sources, `yanked = "deny"` and `unmaintained = "all"`, and a bans section that permits the workspace's internal path dependencies. `cargo deny check` exits zero for the current 221-package tree across all four categories. Three fences assert the committed configuration matches the signed policy (including that the advisory scope is neither `"transitive"` nor `"none"`), that the baseline carries no exceptions or ignores, and that the real tree passes.

**Oracle:** The literal eleven-identifier allow-list and the advisory policy transcribed from `spec.md`'s Decisions table (independent of the config file being parsed), plus `cargo-deny`'s own process exit status against the real graph — a different mechanism from the test's parsing, since the tool independently resolves and evaluates the tree.

**Stress fixture:** The real 221-package workspace graph, which contains every shape that stresses the allow-list: the single `MPL-2.0` (`option-ext` via `directories`), 18 × `Unicode-3.0`, five deprecated slash-form declarations (`MIT/Apache-2.0`, `Unlicense/MIT`), and three workspace-internal path dependencies carrying no version requirement. Expected: exit zero, no exceptions entries, and no `licenses FAILED`/`bans FAILED` line.

**Regression fence:** `crates/resourcefs-mcp/tests/supply_chain_contract.rs::{deny_config_matches_signed_policy, workspace_passes_the_gate, advisory_policy_matches_signed_policy}` — created in this slice.

**Named mutation:** C1 — remove `"MPL-2.0"` from `deny.toml`'s allow list; `deny_config_matches_signed_policy` turns red on the list comparison **and** `workspace_passes_the_gate` turns red because `option-ext` is rejected. C4 — change `unmaintained` from `"all"` to `"transitive"`, the exact weakening the signed decision forbids; `advisory_policy_matches_signed_policy` turns red. Restore each → green.

**Complexity/production scale:** N/A — reason: no new production loop; the fences invoke an external tool over the existing graph and the config parse is a fixed-size file read.

**Wall budget/phase:** One-off (test-time only, not a served path): `cargo deny check` over the 221-package tree must complete within the test harness's ordinary tolerance; measured at ~30 s in the design's falsifier run. Rationale: this runs in CI and on demand, never in the server's request path, so the ceiling is developer patience rather than a production budget. N/A for always-on phases — reason: the change introduces none.

**Files:** `deny.toml` (new, workspace root); `crates/resourcefs-mcp/tests/supply_chain_contract.rs` (new).

**Estimate:** 0.5 day. **Diff estimate:** 165 (45 config, 120 tests). **PR increment:** Dependency vetting.

**Commands and expected results:**
- `cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`, exit 0 against the real tree.
- `cargo test -p resourcefs-mcp --all-features --test supply_chain_contract deny_config_matches_signed_policy advisory_policy_matches_signed_policy workspace_passes_the_gate` → the parsed allow-list equals the literal eleven-identifier oracle item-by-item, exceptions count is zero, advisory severities match, and the real-tree gate exits zero.
- **Confirm both named mutations are applicable immediately after implementing, before running the gate.** Apply each, observe the named fence red, restore, observe green. If a mutation cannot turn its fence red, correct the **fixture** — never the assertion — and record the correction. (Across this project five of nineteen named mutations have needed exactly this.)

## Slice 2: Prove the gate bites on a disallowed license and a git source

**Claim IDs:** C2, C3

**Expected behavior:** Two fences build synthetic dependency graphs in temporary directories and run `cargo-deny` against them using the committed `deny.toml`, asserting a **non-zero** exit in each case: a path dependency declaring `GPL-3.0` fails `check licenses`, and a dependency resolved from a local bare git repository via a `file://` URL fails `check sources`. Both run offline and touch nothing in the real workspace.

**Oracle:** `cargo-deny`'s process exit status against hand-built manifests authored from the SPDX identifier and the git-source shape directly — independent of the production configuration's own parsing, and independent of the Slice 1 fences' mechanism.

**Stress fixture:** For C2, a two-crate temp project whose path dependency declares `license = "GPL-3.0"` while the root declares `MIT` — so only the transitive package is disallowed and a naive root-only check would pass. For C3, a temp project depending on a locally initialised bare git repository by `file://` URL, requiring no network. Expected: `licenses FAILED` and `sources FAILED` respectively, each with a non-zero exit (the design measured exit 8 for the source case).

**Regression fence:** `crates/resourcefs-mcp/tests/supply_chain_contract.rs::{license_gate_rejects_copyleft, source_gate_rejects_git}` — created in this slice.

**Named mutation:** C2 — add `"GPL-3.0"` to the committed `deny.toml` allow list; `license_gate_rejects_copyleft` turns red because the synthetic graph now passes. C3 — set `unknown-git = "allow"` in `deny.toml`; `source_gate_rejects_git` turns red because the synthetic git graph now passes. Restore each → green.

**Complexity/production scale:** N/A — reason: no new production loop; both fences are fixed-size temp-directory fixtures.

**Wall budget/phase:** One-off (test-time only): each fixture resolves a two-crate graph and invokes `cargo-deny`, measured at roughly 20 s each during design. N/A for always-on phases — reason: the change introduces none.

**Files:** `crates/resourcefs-mcp/tests/supply_chain_contract.rs` (extended).

**Estimate:** 0.5 day. **Diff estimate:** 180 (0 config, 180 tests). **PR increment:** Dependency vetting.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --all-features --test supply_chain_contract license_gate_rejects_copyleft source_gate_rejects_git` → both synthetic graphs are rejected with non-zero exits; the tests assert the exit status, not merely the presence of output text.
- Both fixtures must run with **no network access**; if either requires fetching, the fixture is wrong and must be rebuilt offline-safe.
- **Confirm both named mutations are applicable immediately after implementing, before running the gate**, correcting the fixture rather than the assertion if not.

## Slice 3: Add the CI workflow and fence it against gate drift

**Superseded 2026-09-06 (PR #1 F9):** the implementation below is historical. The current local/CI entry point is `python scripts/ci-gates.py`; its behavioral checks exercise real gates, continued execution after a gate fails, final nonzero failure, and missing/unknown compiled ignored-row rejection. Hosted matrix execution replaces workflow-text inference. The deleted `ci_workflow_mirrors_local_gates` is not a current fence. Original authoring evidence and approvals are retained below without being relabeled as new verification.

**Claim IDs:** C5

**Expected behavior:** `.github/workflows/ci.yml` exists, declares both `push` and `pull_request` triggers, runs a Linux/macOS/Windows matrix, and invokes exactly the four documented local gate commands — `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`, and `cargo deny check`. A fence extracts the workflow's gate commands and trigger keys and compares them to a literal expectation, so CI cannot silently diverge from local practice. **The workflow's execution is not claimed** — no remote exists.

**Oracle:** The four gate commands transcribed literally from `spec.md`'s Behavior section, independent of the workflow file being read. One-time YAML validity is confirmed at authoring time with PyYAML (available in this environment) and recorded in the commit message; the permanent fence does not depend on it, since no YAML crate is a workspace dependency and this change adds none.

**Stress fixture:** The committed workflow file itself, checked for: each of the four gate commands present byte-for-byte; both trigger keys present; and **no tab characters** (the classic YAML breakage, and the most likely way a hand-written workflow fails the first time it runs). Expected: all four commands matched, both triggers found, zero tabs.

**Regression fence:** `crates/resourcefs-mcp/tests/supply_chain_contract.rs::ci_workflow_mirrors_local_gates` — created in this slice.

**Named mutation:** C5 — change the clippy step's `--all-features` to `--features test-support` in the workflow; `ci_workflow_mirrors_local_gates` turns red on the command comparison. Restore → green.

**Complexity/production scale:** N/A — reason: no new production loop; the fence is a fixed-size file read.

**Wall budget/phase:** N/A — reason: one-off test-time file read; the change introduces no always-on phase.

**Files:** `.github/workflows/ci.yml` (new); `crates/resourcefs-mcp/tests/supply_chain_contract.rs` (extended).

**Estimate:** 0.25 day. **Diff estimate:** 140 (60 workflow, 80 tests). **PR increment:** Dependency vetting.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --all-features --test supply_chain_contract ci_workflow_mirrors_local_gates` → all four gate commands found byte-for-byte, both triggers present, no tabs.
- `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"` → parses without error (authoring-time check; record the result in the commit message, and do **not** assert the workflow has ever executed).
- **Confirm the named mutation is applicable immediately after implementing, before running the gate**, correcting the fixture rather than the assertion if not.

## Tracker taxonomy

- **Permanent non-goals** (rationale in `design.md`; no tracker issue): fencing that a live advisory fails the gate (the advisory database is external and mutable, so such a fixture rots); adding a YAML parser dependency to validate the workflow; creating a git remote or publishing the repository.
- **Intended future work** (verified tracker IDs): **rfs-g2z9** (in progress) — increment B introduces reqwest and must pass this gate. **rfs-58r1** (open, verified to cover prebuilt binaries *and* `cargo install`) — owns both the deferred verification that this workflow actually executes on push/pull request, and the publishability defect the design surfaced (the three workspace crates are publishable but their internal path dependencies carry no version requirement, which crates.io rejects; fixing it requires editing the workspace `Cargo.toml`, which this change is scoped out of).
- No new deferral phrase was introduced by this plan.

## Self-review

- [x] Every design row is assigned to exactly one slice: S1 C1/C4; S2 C2/C3; S3 C5. Every slice's Claim IDs exist in the design table. Every `PENDING` falsifier (C2–C5) is discharged by the slice implementing its claim; C1 is already `PASS` and is re-confirmed by S1's `workspace_passes_the_gate` fence rather than re-run as a one-shot.
- [x] Every slice records all thirteen mandatory fields, with `N/A — reason` in conditional cells.
- [x] Every claim's fence is created in the slice implementing it; every new fence carries its design-approved named mutation; no fence-less claim exists (design approved zero risk acceptances).
- [x] No slice introduces a production loop, so every complexity cell is `N/A — reason`; the two slices invoking `cargo-deny` record one-off wall measurements from the design run, and every slice records `N/A — reason` for always-on phases because the change introduces none.
- [x] The partition rule was applied: 485 + 25% churn (121) = 606 ≤ 4,000 → one PR increment ("Dependency vetting"); every slice names it; the increment has a mergeable definition verifying without any later change.
- [x] The tracker taxonomy is applied; both intended-future-work items cite verified IDs (rfs-g2z9, rfs-58r1 — the latter confirmed to cover `cargo install` distribution).
- [x] The plan declares no slice complete; `checkpointed-build` exclusively judges completion.
