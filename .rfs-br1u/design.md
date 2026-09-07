# Design: rfs-br1u narrow Bash cleanup repair

## Route and inputs

- Route: Empirical, from route.md. Full given/when/then behavior set is route.md T4; spec.md is N/A because the ticket makes behavior explicit.
- Inputs: evidence.md P1–P3 and probe-results.json establish native task progress/completion, trashed-page ownership visibility and deleted-space collection 404. Supplemental P4 in probe.purge.py establishes explicit trash → purge while the independently verified owner space remains current.
- User scope decision on 2026-09-07, verbatim: "Narrow Bash repair". This rejects a language/SDK migration for this ticket; it is not yet approval of this design.
- Keep the operator's bootstrap/verify/cleanup interface, manifest, receipt formats, existing request/page/poll ceilings, credential separation and strict unknown-creation receipt policy.
- Original unretained task failure remains unexplained: FINISH_SUCCESS support predates it. Do not pretend adding existing status words fixes the old event. Unknown responses remain errors; this repair makes restarting cleanup after such errors safe.

## Input shapes

| Shape | Coverage |
|---|---|
| Page current, trashed, absent, unknown/malformed status; page in its original or another current space | C1, C2, C5 |
| Property collection empty, one exact owned marker, wrong key/value, malformed scalar/container, duplicate or additional rows, continuation present/repeated/untrusted, response 404/error | C1, C2 |
| Space/current parent present, gone, renamed, receipt-key replacement with different ID/marker; page/comment child collection 200 empty/nonempty or 404 with present/absent parent | C3, C5 |
| Longtask known progress/success/failure aliases, missing/null/nonstring/unknown status, optional booleans absent/present/malformed/contradictory, errors empty/nonempty/malformed, transport failure and poll exhaustion | C4 |
| Receipt absent, complete, partially cleaned, all objects absent; pending creation receipt known or unknown outcome | C6 |
| Failure between page trash/purge, between containers, after accepted async delete, or after final resource deletion but before receipt removal | C2, C4, C6 |
| Jira issue/comment/project, Confluence comment/page/space ownership changed after initial preflight but before destruction | C5 |
| Arbitrary unvalidated external identifiers, alternative manifests/receipt schemas, new page statuses to provision, new concurrency modes | N/A — existing validation and fixture shapes unchanged; no new surface introduced |

## Removed-invariant sweep

Two constraints are relaxed, not removed without substitutes:

1. Page ownership no longer depends exclusively on the v1 singleton being visible. Substitute: v2 filtered page-property result must identify one exact rfs-owner marker with validated collection shape and completed bounded pagination; a receipt never grants ownership. C1/C2 defend refusal.
2. Cleanup child collection discovery no longer requires HTTP 200 after its parent was removed. Substitute: cleanup-only missing-parent handling, independent direct parent read on collection 404, and final direct-ID absence proof for saved targets. A collection 404 with a still-present parent remains an error. Bootstrap/verify do not gain 404-as-empty behavior. C3/C6 defend the distinction.

Initial all-target preflight remains; destructive calls additionally perform their own fresh target validation. This narrows, rather than relaxes, the prior stale-validation window. A validation read and external DELETE are not an atomic server transaction; no new atomicity guarantee is claimed.

## Placement

| Capability | Owner | New seam | Forbidden |
|---|---|---|---|
| Cleanup ownership and per-target destructive validation | Existing scripts/atlassian-fixture-bootstrap.sh cleanup responsibility | None externally; factor existing validation into per-target helper reused by preflight and deletion | No receipt-as-authority, copied competing validation rules, generic SDK, production Rust dependency |
| Trashed-page property read/purge and parent-aware discovery | Same script, existing page ownership/discovery/cleanup functions | Existing curl transport and CLI remain the test seam | No global 404 suppression; no deleting a foreign container to reach an owned moved page |
| Native deletion state interpretation | Same script, poll_space_delete | Existing bounded polling interface | No guessed status variants, success on unknown/malformed/contradictory payload, raw native-prose logging |
| Provider lifecycle fixtures and regression assertions | Existing fake_curl.py and atlassian_fixture_operator_contract.rs | Existing real-script/fake-curl seam | Do not model page DELETE as immediate purge; do not claim deterministic fixtures prove native behavior |

## Module shape

N/A — route T2 records unchanged responsibility ownership. No production module split, new dependency direction or external interface/receipt schema change. Internal factoring stays within the existing cleanup responsibility. No migration, SDK or new production file.

## Proposed behavior

- Preserve the two initial validation/discovery passes; reuse per-target validation directly before each destructive action. Absent targets do not produce DELETE requests, particularly when a saved Confluence space key might now belong to another ID.
- Cleanup reads page ownership through the evidenced v2 page-properties collection, filtered by rfs-owner. Require exact key/string marker, unambiguous result and bounded, validated continuation handling. Do not fall back to trusting saved identity if a property is missing or corrupt.
- For a current owned page, issue the normal DELETE to trash it, then read status and ownership again before `DELETE ...?purge=true`. For an already-trashed owned page, revalidate and purge it directly. This permits completion for moved owned pages without deleting their foreign destination space. Unknown page status fails conservatively. A concurrent transition that is not evidenced safe fails with the receipt retained.
- In cleanup discovery only, skip a genuinely absent parent; if collection discovery races with deletion and returns 404, verify exact parent absence before accepting it. Apply to page and Confluence comment discovery; keep all saved target IDs for final independent absence checks. A current parent plus collection 404 is an error, not empty success.
- Retain existing known task status aliases; validate status as a string, and validate finished/successful/error fields when present. Known success must not contradict unfinished/unsuccessful/error evidence. Known failure, unknown/missing/malformed status, contradictory fields, transport failure and timeout fail with receipts retained. Do not infer status vocabulary completeness from two successful live captures.
- Final cleanup still requires all exact target endpoints absent and Jira project trash empty. Only then remove the state file. Preserve unknown creation pending receipts exactly as today.

## Claims

1. C1: Missing, foreign or ambiguous page ownership never authorizes page deletion and leaves the receipt intact.
2. C2: Owned current and trashed pages can be permanently removed with fresh ownership validation at each destructive transition, without deleting a foreign destination space.
3. C3: Cleanup can discover around absent parents without interpreting a collection failure under a present parent as absence.
4. C4: Native task progress is bounded and completion is accepted only from a known noncontradictory success envelope; uncertainty preserves resumability.
5. C5: Each destructive operation validates the exact live target immediately beforehand, and absent IDs cannot authorize deletion of replacement objects at stale keys.
6. C6: Interrupted and all-already-absent cleanup converges, while uncertain outcomes retain receipts until complete direct absence is proved.

## Falsification

All named mutations apply only to the postimplementation existing code at the owning checkpoint. Provider store state and exact resource identity are independent oracles; request traces establish transition order, not success by themselves. Positive controls use the same owned target and valid marker to demonstrate that the intended destructive path is reachable.

The Status column records the approved pre-build handoff. Completed implementation, falsifier and regression outcomes are owned by plan.md's S1 checkpoint and verification-results.json, rather than retroactively changing this design snapshot.

| ID | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Non-owned page cannot be deleted | Missing/wrong/duplicate/malformed marker, current/trashed page | Real script must fail, preserve target and receipt; same target with correct single marker must clean successfully. Earlier unrelated transport failures do not count. | Independent fake store plus paired valid-marker control; live manifest exact marker comparison | In script's cleanup page validator replace marker equality guard with unconditional acceptance; foreign target disappearance/false success must turn fence red | cleanup_refuses_unproven_trashed_page_ownership | Seconds plus fixture setup | PASS — existing real-script probe confirms missing-marker refusal and retained target/receipt; checkpoint renews for v2 validation and named mutation |
| C2 | Owned trash/purge converges safely | Current/trashed, interrupted transition, moved page | Owned page must become absent while foreign destination space stays unchanged; interruption before purge must retain recoverable state | Fake provider trash versus physical-removal state; native P4 page 404 while exact parent remains current | In page cleanup omit the purge=true operation after verified trash; expected absent page remains and fence fails | cleanup_resumes_trashed_pages_without_deleting_foreign_space | Seconds plus fixture setup | PENDING — checkpointed-build implementation slice |
| C3 | Absent parents are distinct from failed collections | Missing space/page/comment parent and present-parent collection 404 | Absent-parent case completes with exact children absent; same collection 404 under current parent fails and retains receipt | Fake store separately controls parent existence and response status; live P3 direct GETs | Replace cleanup parent-absence check with unconditional 404-as-empty success; present-parent case must fail the fence | cleanup_distinguishes_absent_parent_from_collection_failure | Seconds plus fixture setup | PENDING — checkpointed-build implementation slice |
| C4 | Unknown task state never means success | Captured progress/success, failed/unknown/missing/malformed/contradictory fields, timeout | Captured progress must advance to independently absent space; invalid/error cases must stop before next container deletion with state retained, then a safe retry converges | Provider task schedule and container existence controlled independently of script parser | Change unknown/malformed task branch from die to return success; next-container destructive progress/receipt loss must fail fence | cleanup_preserves_receipt_for_uncertain_space_task | Seconds with deterministic fake sleep; live polling separately | PENDING — checkpointed-build implementation slice |
| C5 | Fresh exact-target validation guards destruction | Ownership switch after preflight, deleted original plus foreign same-key replacement | Foreign replacement remains untouched; initially valid control deletes successfully; switch must occur after preflight to exclude the initial guard as explanation | Independent fake-store ownership switch and actual DELETE target identity | Remove the immediate per-target validation call in deletion path while retaining preflight; late-switch fence must turn red | cleanup_revalidates_each_destructive_target | Seconds plus fixture setup | PENDING — checkpointed-build implementation slice |
| C6 | Receipts follow complete absence | Interrupted/partial/all-absent state; unknown pending creation, successful task with lingering object | Resume completes and removes receipt only with all exact resources absent including project trash; one surviving target must force receipt retention | Independent provider collections and trash state, not script output; final live direct-ID checks | Bypass cleanup_assert_absent before receipt removal; lingering-object case must fail fence | cleanup_resumes_after_space_delete_interruption; cleanup_retains_receipt_until_all_targets_absent | Seconds plus fixture setup; live owned lifecycle minutes | PENDING — checkpointed-build implementation slice |

## Non-goals and future work

- No Python/Rust migration or SDK adoption in this change: user explicitly selected narrow Bash repair. No future migration is promised.
- No new public CLI flags, receipt schema, automatic retries, parallel destruction or production adapter changes.
- No recovery of the original unretained task payload, guessed status vocabulary, or claim that the original invalid_response event was reproduced.
- No deletion of foreign resources or automatic clearing of uncertain creation receipts.
- No new domain terminology: CONTEXT.md vocabulary remains unchanged.
- Intended future work: none introduced by this design; no tracker deferrals required.

## Falsifier run log

Cheapest C1 falsifier: `python .rfs-br1u/probe.interrupted.py`, run on unchanged 7de8d14 before approval. Trashed-page missing-owner case failed at the exact page ownership guard, retained the target and receipt, and issued zero DELETE requests. Positive control restored only that target's marker: cleanup then succeeded, target became absent and receipt was removed. Output names claim=C1 and same_target_valid_marker_control=PASS, excluding an unreachable destructive path as an alternative explanation. The same command independently reproduced missing-owner-space collection 404. Both reported_failure_reproduced=true; complete paired run took 29.80 seconds. Evidence.md owns the full result and native P2/P4 comparisons. No claim of postimplementation PASS.

## Approval

Requester approval, verbatim: "Approve design" (2026-09-07). Prior scope selection: "Narrow Bash repair" (2026-09-07).
Approved risk acceptances: None. All six claims have deterministic regression fences and applicable live verification; no gate is waived.
