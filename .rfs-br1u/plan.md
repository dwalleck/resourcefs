# Plan: rfs-br1u

Approved design: design.md, requester "Approve design", 2026-09-07. Empirical route, P1–P4 discharged in evidence.md. Cheapest C1 falsifier passed with same-target valid-marker positive control. No risk waivers. Glossary: no new terms.

## Module growth and PR partition

Module growth ledger: N/A — route/design record no module-shape change. Existing script remains owner of fixture lifecycle; no external interfaces, receipt schemas, modules or dependencies added.

One increment, CleanupResume, based on discovered upstream origin/main (7de8d14 at setup). One atomic slice: page purge semantics, fresh ownership checks, missing-parent discovery and their realistic provider fixture cannot safely land independently while preserving the existing moved-owned-page contract. Updated implementation/test/fixture allowance 1,200 changed lines, provenance/results 1,500, churn margin 400 = 3,100, below 4,000. Actual initial assembled code/test/fixture diff was 983 changed lines; captured native payloads and review proof increased the original artifact estimate. This is a plan-only partition correction, with no design/scope change. Increment is independently mergeable when its full gate and owned live interruption/resume proof pass; no later increment needed.

## Slice S1: make existing cleanup safely resumable

**Claim IDs:** C1, C2, C3, C4, C5, C6 (each appears only here).

**Expected behavior:** Fresh owned cleanup, resume after trash/purge/accepted-space deletion, and all-already-absent resume converge to exact resource absence with state removed; foreign/uncertain cases retain their target and receipt. Present-parent collection errors are not swallowed. Unknown native task payloads stop safely rather than advancing destruction.

**Oracle:** design.md C1–C6 independent provider state/manifest identity/paired ownership controls. Live verification uses independently provisioned IDs, direct endpoint absence and Jira trash queries rather than operator success text alone. Retained native P1–P4 evidence remains applicable because upstream APIs/credentials/fixture shapes are unchanged; production behavioral proofs rerun after implementation.

**Stress fixture:** Existing full fixture graph with page DELETE modeled as trash, v1 properties hidden on trash, v2 properties still available, and explicit purge; interrupted accepted space task; all parents/children absent; same-key foreign space replacement; late marker switch after preflight; empty/malformed/ambiguous/paginated property response; missing-parent and present-parent collection 404; task success with contradictory flags and unknown/malformed/error states; lingering exact target; unknown pending creation receipt. Expected outcomes are design C1–C6.

**Regression fence:** Existing crates/resourcefs-sources/tests/atlassian_fixture_operator_contract.rs and tests/fixtures/atlassian_fixture_operator/fake_curl.py. Create named tests from design.md in S1; consolidate a name only when it genuinely covers the same claim and record the exact mapping at checkpoint. Keep existing security/credential/bounds/pending/moved-resource tests meaningful. Remove incidental count/text assertions only when they pin plumbing rather than lifecycle behavior.

**Named mutation:** Apply C1–C6 exact semantic mutations from design.md to the S1 production script one at a time: accept foreign marker; omit verified purge; accept all collection 404s; accept unknown task state; omit immediate validation; omit final absence assertion. Each fence must fail under its mutation and pass after exact restoration. If a mutation is redundant with an independent upstream guard, select an equivalent defect injection exposing the same approved invariant and record the technical correction, not a weaker assertion.

**Complexity/production scale:** T3 is no; operator is a bounded discrete fixture command, not a production request path. The new page property traversal is O(P), P <= existing MAX_PAGES=10; all new reads share MAX_REQUESTS=512 and MAX_RESPONSE_BYTES=4 MiB. Per-target validation is O(N), not repeated all-target validation before every deletion; normal graph has 14 explicitly recorded targets, 3 pages (at most trash plus purge) and 2 space tasks. Existing polling remains <=30 reads per task. Maximum accepted total requests remains 512; budget exhaustion must retain state. New loops cannot exceed these existing ceilings. Verify normal full fake graph completes below 512 requests and property traversal errors at the existing page ceiling without destructive progress.

**Wall budget/phase:** N/A — one-off discrete cleanup command; no always-on phase. Existing per-request curl timeout and bounded polling remain unchanged.

**Module shape:** N/A — route/design record no module-shape change. Internal validator factoring stays in the script; no new production file or dependency.

**Files:**
- scripts/atlassian-fixture-bootstrap.sh
- crates/resourcefs-sources/tests/atlassian_fixture_operator_contract.rs
- crates/resourcefs-sources/tests/fixtures/atlassian_fixture_operator/fake_curl.py
- .rfs-br1u/{route.md,evidence.md,design.md,plan.md,review-decisions.md,probe-results.json,live-acceptance.json,verification-results.json,probe.sources.txt}. Probe sources are retained as non-executable audit text; active throwaway instruments are removed after successful live smoke.
- Existing owning operator usage documentation if its cleanup description needs updating; locate exact owning section before editing.
- .rivets/issues.jsonl, through Rivets only, closure atomically with implementation after all required verification.

**Estimate:** One tightly coupled implementation/checkpoint; timing is a scheduling signal only, never a gate or permission to narrow scope.

**Diff estimate:** 1,200 implementation/test/fixture changed lines; partition including provenance/churn recorded above.

**PR increment:** CleanupResume.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract cleanup_ -- --nocapture` against new regressions before source integration → affected resume/purge/fresh-validation cases fail for the approved behavior, not compilation or unrelated transport.
- Same command after integration → owned graphs converge; foreign/replacement objects survive; uncertain receipts remain; independently controlled collection/task failure states are distinguished.
- `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract -- --nocapture` → all existing and new fixture operator security/lifecycle contracts pass.
- Individual exact test filters named by C1–C6, under each source mutation then after restoration → expected localized red and restored green; write exact commands/results/source hashes in checkpoint evidence.
- `bash scripts/atlassian-fixture-bootstrap.sh cleanup --site https://kiro-tethys.atlassian.net --state <private copied all-absent receipt>` → all-already-absent recovery completes and removes only that state file, zero owned resources recreated.
- `bash scripts/atlassian-fixture-bootstrap.sh bootstrap --site https://kiro-tethys.atlassian.net`, then verify, then cleanup with response-capturing interrupt shim, then unmodified-path cleanup resume → setup/verify succeed, interruption retains state, standard resume exits 0 and removes state/pending. Independently check all recorded IDs and Jira trash absent. No ad hoc reconciliation can substitute for this postimplementation live PASS.
- `python scripts/ci-gates.py` → complete repository gate PASS, including its own authoritative budget and ignored-row inventory. Run once on assembled source after writing agents finish; no concurrent validation in writers.
- Independent reviewer audits all C1–C6 and security/receipt invariants before publication; evidence-backed fixes revalidate affected results.

## Execution ownership

Main integrates and verifies. Two isolated writers can proceed concurrently with disjoint ownership: production Bash file versus fake provider plus Rust regression file. Tests exercise the external CLI and shared REST shapes, not internal helper names. The provider must model captured native RUNNING/FINISH_SUCCESS envelopes and trash/purge lifecycle; abnormal task shapes are deliberate fault injection, never alleged live captures. Integrate tests first for pre-fix red, then production source. Writers skip all validation, formatting and commits; Main performs checkpoint validation and atomic commit.

## Self-review

All six design rows assigned once; all fourteen slice fields present; all fences and named mutations belong to S1; new loops retain existing explicit ceilings; no always-on phase or module-shape change; corrected partition 3,100 <=4,000; no intended-future-work deferrals; no slice completion declared by this planning stage.

## S1 checkpoint

Checkpoint judgment: PASS. Implementation and technical proof repairs R1/F1 and R2/F2 remain within approved C1–C6. Named commands, source hashes and observations belong to evidence.md, live-acceptance.json and verification-results.json; review decisions belong to review-decisions.md.

| Gate | State | Evidence and applicability |
|---|---|---|
| 1. Affected tests | PASS | All 33 operator tests passed; the final authoritative repository gate also passed on assembled source. |
| 2. Assigned falsifiers | PASS | C1–C6 negative/positive oracles exercised. Native P1–P4 retained because upstream API/tenant/fixture semantics are unchanged; final repaired-operator live proof is fresh. |
| 3. Stress fixture | PASS | Actual CLI against trash/purge, missing parents, reachable reply404, malformed/paginated ownership, uncertain/timeout tasks, late ownership/reused keys, lingering targets and unknown pending creation. |
| 4. Changed implementation versus oracle | PASS | Provider state and same-target controls agree with actual CLI outcomes. Live ordinary resume agrees with 14 direct404s, two empty terminal Jira trash searches and absent local receipts. |
| 5. Module shape | N/A — unchanged responsibility ownership | Approved route T2 is no; no new production module/interface/dependency. Repository placement gate separately passed. |
| 6. Budgets | PASS | Measured request/page counts in lifecycle fixtures remain within 512 requests and 10 property pages; task timeout fixture exhausts exactly 30 polls. Existing 4 MiB response ceiling unchanged. Production-scale/wall classes: N/A — bounded one-off operator per approved plan. Repository production-budget inventory also passed. |
| 7. Regression fences | PASS | All named C1–C6 fences passed, including same-target valid controls and final live standard resume. |
| 8. Named mutations | PASS | Six single-match semantic mutations each passed Bash syntax and caused its named fence to fail behaviorally with exit101. |
| 9. Exact restored fences | PASS | Each exact source restoration reproduced SHA-256 dacb1c082993402d3f951c66ec99d1d6d2541e8d0bc82d819b6ee74260866479 and its same complete fence passed with exit0. |

Final assembled verification: PASS — `python scripts/ci-gates.py`, including dependency vetting. Independent review and verified R1/R2 repairs complete. The post-live help-only edit was exercised through the actual CLI and did not invalidate lifecycle evidence; the full gate and mutation/restoration runs used that final source. Domain docs intentionally unchanged: no new domain terms or contracts. Active throwaway probes retired after live success, source provenance archived as text.

Final partition measurement before this two-line ledger entry: 2,554 changed lines across 14 staged files (987 implementation/test/fixture, 1,565 provenance/research, 2 tracker). Including this entry: 2,556, versus original 2,000 (+556), corrected allowance 3,100 (-544), and hard ceiling 4,000. Provenance used 65 lines of the planned churn allowance; no PR split or scope reduction is needed.
