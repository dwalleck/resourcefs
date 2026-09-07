# Review decisions: rfs-br1u

## Findings

| finding-id | finding | reviewer | evidence-state | evidence | decision | fix | note |
|---|---|---|---|---|---|---|---|
| F1 | Raw v2 marker extraction strips trailing LF (and cannot preserve NUL), allowing non-exact ownership; compare the JSON value directly. | CleanupReview-2 | Verified | `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract cleanup_refuses_unproven_trashed_page_ownership -- --exact --nocapture` exited 101: the first newline-marker case falsely succeeded and deleted the graph (0 passed, 1 failed). | Accept | R1: validate exact JSON ownership inside the bounded property helper; use the manifest logical ID to retain the exact expected JSON string too. | Root cause confirmed against actual Bash, not just reviewer analysis. NUL is a separate regression boundary; the demonstrated failure used LF. |
| F2 | Raw task-state extraction can normalize an unknown LF/NUL-suffixed value into a supported terminal status; require lossless equality to the native JSON string. | Main follow-through from F1 | Verified | Exact `cleanup_preserves_receipt_for_uncertain_space_task` rerun exited 101: `FINISH_SUCCESS` plus LF submitted deletion of the second space before the first task was known complete. | Accept | R2: require native status/state to equal the extracted dispatch string before matching existing aliases. | C4 already requires unknown/malformed states to stop progression; no vocabulary or protocol expansion. |

## Repair R1: exact v2 ownership boundary

- Ownership: F1, approved design C1/C5, plan.md S1. Inherit unchanged slice fields, receipt format, APIs, ownership rules and bounded costs.
- Expected root-cause repair: keep the v2 ownership value in JSON for equality; do not authorize through a Bash string that can normalize bytes. Existing helper should own exact marker, cardinality and continuation validation without exporting an untrusted raw marker. No new dependency or production file.
- Affected paths: scripts/atlassian-fixture-bootstrap.sh; crates/resourcefs-sources/tests/atlassian_fixture_operator_contract.rs. Add same-target newline/NUL cases to the existing C1 fence before changing production code.
- Planned commands: exact C1 regression on current implementation must fail because foreign ownership was accepted; after repair it must pass, including same-target exact-marker control. Rerun invalidated focused cleanup and full operator checks; exercise C1 named mutation and exact restoration; then full repository gate and live acceptance on final assembled source.
- Evidence disposition and result: original-source RED and native P1–P4 remain applicable. Repaired full operator suite passed all 33 tests; the exact C1 named mutation failed and its exact restoration passed the complete LF/NUL/ownership matrix. Final repository gate passed. R1 satisfies plan.md's S1 checkpoint; no unresolved review finding remains.

## Repair R2: lossless native task dispatch

- Ownership: F2, approved C4, plan.md S1; unchanged slice fields inherited.
- Root cause and paths: Bash normalization could promote an unknown task string into a known alias. Add one native-JSON equality condition to scripts/atlassian-fixture-bootstrap.sh, plus LF/NUL cases in the existing Rust task fence. Existing aliases and optional-envelope policy remain unchanged.
- Reproduction: `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract cleanup_preserves_receipt_for_uncertain_space_task -- --exact --nocapture` exited 101 (0 passed, 1 failed), with an independently recorded second-space task proving unsafe progression. LF was the demonstrated case; NUL receives separate regression coverage.
- Expected verification: the full task fault matrix must retain unchanged receipts and never submit the second space under uncertain input; removal of each fault must converge. Run assembled operator suite, C4 mutation/restoration, final live acceptance and full gate.
- Evidence disposition and result: native P1–P4 and original-source regressions remain valid; earlier task green without the byte-normalization boundary was superseded. Repaired full operator suite passed; exact C4 unknown-state mutation failed the next-container oracle and exact restoration passed the complete matrix. Final live resume and repository gate passed. R2 satisfies plan.md's S1 checkpoint.

Read-only follow-up by CleanupReview-2 confirmed both repaired JSON boundaries, all three migrated page-discovery callers and their same-target/next-container controls. It identified no new defect. This review conclusion is separate from the executed proof above. F1 and F2 are included in the atomic S1 commit message.
