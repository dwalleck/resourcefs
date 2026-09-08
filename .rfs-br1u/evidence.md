# Evidence: rfs-br1u

Date: 2026-09-07. Initial premise probes checked operator/fake source at 7de8d14, before production changes. Machine: Linux x64; Bash, Python 3, curl and jq. Canonical nonsecret premise results: probe-results.json. Implementation verification follows in S1 below.

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|---|---|---|---|
| P1 | Native Confluence deletion task shape carries progress/terminal state. | Which fields and values occur in first and terminal real longtask reads? | PASS — first response and separate rapid-poll deletion comparison |
| P2 | Interrupted cleanup exposes owned trashed pages without normal ownership-property visibility. | Which supported read preserves the independently provisioned marker after trashing? | PASS — v1 singleton returns 404; v2 page properties returns exact marker |
| P3 | Deleted-space child collection discovery returns 404. | Do exact owner-space, saved-page and child collection reads agree on owner absence? | PASS — independent stable-ID reads and deterministic real-operator reproduction |
| P4 | Owned trashed pages can be purged independently of their owner space. | Does a freshly revalidated owned page become absent after purge=true while its exact space remains current? | PASS — probe.purge.py and independent parent identity/marker check |
| N1 | Foreign objects must be refused and uncertain receipts retained; validation must be immediate before destruction. | N/A — explicit ticket behavior, not a vendor premise. | N/A — specified behavior |

## Data

- Source: production-shaped, disposable marker-owned fixtures from fixtures/atlassian-live/manifest.json, separately set up through the unchanged ownership-validating operator on the previously established https://kiro-tethys.atlassian.net test tenant.
- Shape: two Jira projects, two Confluence spaces, owned issue/page/comment graph and a local identity receipt.
- Safety: only the manifest-owned fixture lifecycle mutated upstream. A throwaway curl observer recorded native task response data and interrupted the operator after the first task read; it never recorded credential-bearing stdin. Probe reads and independent endpoint reads did not mutate upstream. Raw first task response is private under ignored .resourcefs/br1u-capture; only a redacted envelope appears in probe-results.json. No production code changes.
- Setup/teardown are separate from read-only probing. Final audited reconciliation revalidated exact ID/key/manifest markers immediately before deleting remaining owned containers. Fourteen receipt resource endpoints then returned 404; both Jira trash searches were empty and terminal. No live fixtures remain. The now-absent-resource receipt is intentionally retained for repair verification; no pending receipt exists.

## Probe

- probe.curl.py: forwards the existing operator's exact curl invocation, retains its native task response separately, then returns transport exit 97 after the first task read. No invented native response is used in the live run.
- Commands (execution-only credentials loaded from the established ignored local environment file):
  - `bash scripts/atlassian-fixture-bootstrap.sh bootstrap --site https://kiro-tethys.atlassian.net`: exit 0, `bootstrapped`.
  - Same invocation with `verify`: exit 0, `verified`.
  - Same invocation with `cleanup`, PATH exposing probe.curl.py as curl, RFS_PROBE_CAPTURE pointing at the ignored capture directory and RFS_PROBE_INTERRUPT=1: exit 1, `transport_failure actor=provisioner operation=longtask_status`; receipt retained.
  - Normal receipt-based `cleanup`: exit 1, `foreign_collision logical=page`; receipt retained.
- probe.interrupted.py: invokes the actual operator against its existing fake transport, bootstraps independent temporary stores, then installs either observed trashed-page/property invisibility or an absent owner-space/page graph. `python .rfs-br1u/probe.interrupted.py`: reproduced both exact reported errors, receipt retained, zero DELETE requests in each retry; 16.19 seconds for both including setup. This is a preimplementation diagnostic instrument, not a feature test. First attempt used the wrong fake tenant origin and failed at account_get; corrected to the fake's existing https://example.test before recording reproduction results.

## Oracle

- P1: separately rendered first-party longtask schema plus independently read exact container/page existence, rather than the operator's status parser. Documented finished/successful booleans, percentageComplete, status and errors match observed field types. A second owned-space deletion was polled directly through Python urllib without the operator's curl wrapper/parser.
- P2: receipt stable ID and checked-in manifest marker determine expected ownership independently of native property endpoint selection. Separate page and owner-space reads establish status/parent identity; v2 property result matches the exact manifest marker. Normal v1 singleton, status=trashed singleton and status-filtered v1 collection are independent visibility comparisons, not alternative authority.
- P3: separate direct stable-ID/owner-space/child-collection reads establish resource absence independently of cleanup discovery; the fake-provider store is the independent state oracle for deterministic reproduction, not evidence of vendor behavior.
- Primary reference: https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-long-running-task/ . Documentation alone is not promoted into tenant-specific evidence.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|---|---|---|---|
| P1 | First native task HTTP 200: status RUNNING, finished false, successful false, percentageComplete 0; terminal HTTP 200: FINISH_SUCCESS, finished true, successful true, percentageComplete 100. Full redacted envelopes retained. | First space and both pages independently 404. Separate remaining-space direct deletion/poll produced RUNNING, RUNNING, FINISH_SUCCESS with matching boolean transitions; final space/page 404. Published longtask field types agree. | PASS |
| P2 | Remaining private page 26673153 was HTTP 200, status trashed, spaceId 26607618; v1 owner singleton returned 404 even with status=trashed. | Owner space 26607618 remained current. v2 `/pages/26673153/properties?key=rfs-owner` returned one exact key/value matching checked-in private-root-page marker. V1 status-filtered property collection returned no rows. Normal real cleanup retry and deterministic fake-state probe both rejected the page before any DELETE. | PASS |
| P3 | Deleted public space 26574850 and its page collection both returned 404; receipt child pages 26575042 and 26640385 also returned 404. | Exact stable-ID reads agree with completed task and independent provisioned identities. Deterministic actual-operator cleanup with absent owner/page graph fails `upstream_failure actor=provisioner operation=page_list status=404`, retains receipt and issues no DELETE. | PASS |

## Validated / learned

- P1: validated observed RUNNING → FINISH_SUCCESS envelope and finished/successful booleans against independent final absence. The original ticket's unretained invalid_response payload is still unknown. Two new deletions did not reproduce an unrecognized status; no claim that its original spelling/schema has been recovered. Defensive malformed/unknown/failure handling must not be presented as a captured live failure variant.
- P2: learned that trashed-page ownership remains available through the supported v2 page-property collection while normal v1 singleton visibility hides it. Receipt authority substitution or skipping ownership is unnecessary.
- P3: validated that absent owner-space collection 404 is expected, not a reason to discard uncertain authority. Saved identities survive space deletion, exposing the operator's stale-parent discovery assumption.

## Related issues

- Consulted upstream evidence: rfs-nae2 (.rfs-nae2/evidence.md L3–L5), rfs-bcym (existing fixture lifecycle evidence). These cover prior art; no repeated broad search.
- Filed: none. The observed operator failures are the scope of rfs-br1u.

## Hand-off

All P1–P3 PASS. No production implementation or feature tests changed. Hand off to falsifiable-design using this evidence; the original missing task envelope remains an explicit evidence limitation, not invented data.

## Status-vocabulary provenance

`git log --format='%h %ad %s' --date=iso-strict -S FINISH_SUCCESS -- scripts/atlassian-fixture-bootstrap.sh` identifies 518b99e, dated 2026-09-05T20:58:54-05:00, as introducing FINISH_SUCCESS support. This predates the rfs-nae2 failure on 2026-09-07. Therefore the new RUNNING/FINISH_SUCCESS captures do not reproduce or explain the original invalid_response, and adding those already-supported values would not fix it. Unknown, missing, malformed or contradictory task envelopes must conservatively fail while leaving cleanup resumable; no complete native vocabulary is inferred.

The interrupted receipt was parsed before the normal resume attempt and supplied the exact resource IDs for independent checks. It remains deliberately retained after complete upstream absence as an all-resources-already-gone recovery input for the eventual implementation. This is a local verification artifact, not an uncertain upstream cleanup outcome; pending receipt is absent.

## Supplemental P4: narrow Bash design prerequisite

The existing moved-owned-page regression requires deleting an owned page without deleting its foreign destination container. The native v2 deletion contract specifies normal DELETE as trash and DELETE with purge=true on an already-trashed page as permanent deletion: https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/#api-pages-id-delete .

Separate fresh fixture setup used the unchanged bootstrap operator (exit 0). `python .rfs-br1u/probe.purge.py` (execution-only credentials) exited 0: exact owned current page 26738905 → normal DELETE 204 → status trashed with exact v2 property marker → purge=true DELETE 204 → direct page GET 404. Independent direct space GET still returned the expected ID/key/description marker and status current, excluding container cascade as the cause. The trashed private page's empty footer-comment collection remained HTTP 200. This distinguishes page-level purge from the space-cascade mechanism used in the earlier probe. Source and receipt/manifest shape are unchanged.

P4 learning: explicit trash → fresh ownership check → purge permits removal without container deletion; a normal DELETE alone does not. The separate unchanged cleanup operator then removed the remaining fixture graph successfully (exit 0, cleaned); default state and pending receipt are absent. The earlier all-absent receipt has been copied to ignored `.resourcefs/br1u-capture/all-absent-receipt.json` for the eventual repair test, not left at the operator's default state path. Results are in probe-results.json under purge_probe.

Design C1 decisive positive control: the updated `python .rfs-br1u/probe.interrupted.py` rerun on unchanged 7de8d14 took 29.80 seconds. With only the missing marker restored on the same synthetic trashed target, normal cleanup succeeded, the page became absent and the receipt was removed. The missing-marker case still failed at the ownership guard with the target/receipt preserved and zero DELETEs; the deleted-owner case still reproduced page_list 404. Output names `claim=C1`, `same_target_valid_marker_control=PASS`. This proves the refusal observation is not caused by unreachable cleanup or broken transport. P1–P4 are discharged; production source remains unchanged.

## S1: pre-fix regression evidence

Integrated the isolated fake-provider/Rust regression changes before the production repair. Production SHA-256 remained `05160d56fa9c2b0c6c76316b9e88a9e5b3c838d45bea536387509eac6442b77f`. `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract cleanup_ -- --nocapture` compiled successfully, then exited 101: 3 passed, 12 failed, 18 filtered out. The test runner reported 33.00 seconds of test execution; compilation is separate.


The three pre-fix passes were pre-existing tests: cleanup_recovers_known_owner_property_receipt_after_failed_bootstrap, cleanup_purges_owned_tombstones_and_preserves_foreign_projects, and cleanup_waits_for_owned_space_deletion. No newly introduced fence passed on the unchanged implementation.
Observed failures include owned trash still rejected after restoring the exact marker; moved pages surviving normal DELETE (`cleanup_incomplete logical=page`); absent-parent discovery returning `page_list status=404`; inability to reach the second interruption after page trash; late foreign-marker and unknown-status cases falsely succeeding. These are behavioral failures against the actual unchanged Bash operator and independently inspected provider state, not compile failures.

One initial red was invalid evidence: the all-already-absent test recreated its removed receipt with default filesystem permissions, causing `state_permissions`. The test now preserves the original private permissions. Its exact rerun, `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract cleanup_resumes_all_already_absent_receipt -- --exact --nocapture`, still exited 101, now for the intended `upstream_failure actor=provisioner operation=page_list status=404` (0 passed, 1 failed). Only this corrected result is evidence for that fence.

The refusal helper now requires unchanged receipt bytes, not merely file existence. Added a contradictory status/state task-envelope row; it is synthetic fault injection, not a native capture. Main formatted the assembled Rust test with `rustfmt --edition 2024` before fixed-source verification. The production repair was integrated only after the above behavioral red results.

First integrated-source run exited 101: 14 of 15 cleanup tests passed. The remaining C3 failure was a proof-fixture error, not evidence of a production defect: the default manifest has two root comments and no reply, so its configured children-collection fault was unreachable. C3 now makes the second manifest comment a child of the first and uses that exact custom manifest throughout bootstrap, failure and recovery. Its present-parent branch requires the upstream-failure category, unchanged receipt bytes, surviving parent and zero DELETEs; absent-parent and repaired-fault controls require complete provider absence. The corrected proof is included in assembled verification.

Independent review produced F1, reproduced and dispositioned with R1 in review-decisions.md. Main followed the same normalization mechanism to C4 and reproduced F2; its R2 record owns that repair. A read-only follow-up review confirmed both repaired boundaries and all page-discovery callers; execution proof remains separate from that code-review conclusion.

## S1: assembled operator and live acceptance

`cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract -- --nocapture` exited 0: all 33 tests passed (303.17 seconds test execution). This includes corrected C3 reply discovery, R1/R2 LF/NUL cases, same-target positive controls, moved/foreign/reused-key cases, bounded task exhaustion, exact receipt retention and all existing credential/pending/bootstrap contracts.

Canonical live results and exact endpoint rows: live-acceptance.json. Source SHA-256 is recorded there. All native requests used the established disposable tenant and execution-only credentials:

1. Independently checked the preserved old receipt's 14 exact resources: all HTTP 404; both Jira trash searches HTTP 200, empty and terminal. Copied private receipt cleanup exited 0 and removed that copy.
2. Fresh normal bootstrap exited 0 (`bootstrapped`); normal verify exited 0 (`verified`).
3. Interrupted real curl only after the first ordinary page DELETE returned 204. Operator exited 1 with `transport_failure ... operation=confluence_page_delete`. Independent page GET returned HTTP 200/status trashed for 26869761; v2 properties contained exactly the manifest marker. Receipt bytes were unchanged.
4. Resumed cleanup with interruption after the first real longtask read. Operator exited 1 with `transport_failure ... operation=longtask_status`; native response was RUNNING, finished false, successful false, errors empty. Receipt bytes were still unchanged.
5. Ordinary cleanup with no shim exited 0 (`cleaned`). State and pending receipt were absent. Separate Python urllib reads confirmed all 14 recorded resource endpoints HTTP 404 and both exact Jira trash searches empty/terminal. The independently reread first task was FINISH_SUCCESS. No ad hoc reconciliation substituted for this repaired-operator PASS; no live fixtures remain.

After successful smoke, removed all three active throwaway Python instruments and the temporary PATH curl shim. Their exact source snapshots/hashes remain in non-executable probe.sources.txt; earlier probe filenames are historical references to that archive. The owning `--help` text now explains Confluence purge and retained-state resume; actual `bash scripts/atlassian-fixture-bootstrap.sh --help` exited 0 with that guidance. This subsequent help-only edit does not invalidate lifecycle/oracle evidence; final gate and mutations run on its assembled source.

## S1: named mutation and restoration proof

Exact commands, substitutions, mutant hashes, timings and outcomes are in verification-results.json. Every mutation changed exactly one production source match, passed `bash -n`, then ran its named exact Rust fence through the actual Bash CLI. Each produced exit 101 with a behavioral assertion failure; each exact source restoration produced exit 0 for the same complete fence. Restored SHA-256 throughout: `dacb1c082993402d3f951c66ec99d1d6d2541e8d0bc82d819b6ee74260866479`.

| Claim | Named fence | Mutant observation | Exact restoration |
|---|---|---|---|
| C1 | cleanup_refuses_unproven_trashed_page_ownership | Replacing the native JSON marker equality with true authorized newline-suffixed foreign ownership and falsely cleaned the graph. | PASS, including all same-target ownership controls |
| C2 | cleanup_resumes_trashed_pages_without_deleting_foreign_space | Removing purge=true left the moved owned page present; actual cleanup reported cleanup_incomplete. | PASS for current and already-trashed pages; foreign space unchanged |
| C3 | cleanup_distinguishes_absent_parent_from_collection_failure | Treating present-parent collection404 as absence falsely succeeded instead of reporting upstream failure. | PASS for pages, footer comments and reachable reply children; absence/repaired-error controls converge |
| C4 | cleanup_preserves_receipt_for_uncertain_space_task | Returning success for unknown task state submitted deletion of the second space while the first outcome remained uncertain. | PASS for full malformed/failure/transport/timeout matrix and normal retry controls |
| C5 | cleanup_revalidates_each_destructive_target | The original skip-call mutation could fail the late-fault setup assertion, so it did not establish the claimed behavioral failure. R3 supersedes it with preserved native reads/state and ignored foreign-ownership rejection; all six rows then falsely report successful cleanup. | R3 exact restoration passes all six target families and same-target restored-ownership controls |
| C6 | cleanup_retains_receipt_until_all_targets_absent | Skipping final absence proof reported success and discarded the receipt despite a surviving exact resource. | PASS for page, space and Jira-trash residues, then independently completed recovery |

C1's technical mutation targets the final JSON equality boundary introduced by R1 rather than an obsolete shell-string comparison. The approved foreign-authority invariant and negative/positive oracle are unchanged.

## Final repository gate

`python scripts/ci-gates.py` exited 0 with `All repository gates passed.` It ran its authoritative module-placement, formatting, lint, functional-test, release-workspace, ignored-production-budget and dependency-vetting sections. Final dependency output: `advisories ok, bans ok, licenses ok, sources ok`. Credential-gated live rows remained excluded by the runner; the actual owned fixture lifecycle was separately exercised above.

Rust pre-commit checklist reviewed: changes are isolated test code using the established TempDir/Result assertions and canonical harness paths; no production Rust API, domain type, platform FFI, error type or dependency changed. Literal checks of all 14 publication files found none of the four live credential values. Raw run logs remain local verification capture, not publication artifacts; canonical commands/results/hashes are preserved here and in verification-results.json.

## R3: rstest fixture optimization

User approved optimization before merge and proposed rstest. Seven loop matrices now expose the same 52 scenarios individually; the other 26 tests are unchanged. Two once-only immutable in-memory snapshots replace 52 matrix bootstraps. Every case materializes independent store/receipt files into a new Harness; original receipt bytes and private permissions are retained. Static fixtures retain no TempDir, paths or native resources. Fault, refusal, ownership repair and retry stay on the same case-local harness.

Normal deterministic harness sleep is a no-op; the actual bounded polling loop remains, and credential-audit cases still install their real-helper wrapper. Production Bash, Python fake, manifests, adapters and live clocks are unchanged. rstest is Unix-target dev-only with default features disabled; four packages are added without changing existing locked versions.

- Enumeration: 78 tests, with matrix counts 14/2/6/17/6/3/4.
- Focused debug execution: all 78 pass in 129.17s, versus the prior 33 grouped tests in 303.17s. This is a 57.4% reduction, with all original scenarios retained.
- Independent source review by RstestReview found no lost cases/assertions or isolation/dependency defects. Review is not substituted for execution.
- All C1–C6 mutations produce exit101 and exact restorations produce exit0. Generated groups use prefix filters without `--exact`; enumeration guards require the expected nonzero count before mutation. Current commands, selected counts, failure names, hashes and timings replace the superseded monolithic commands in verification-results.json.
- C5's former skip-call mutation could miss the fixture's late-switch trigger. Its replacement retains native reads and state propagation but ignores foreign-ownership rejection. Every family now fails because uncertain cleanup incorrectly succeeds, rather than because a fixture precondition was missed. The permanent tests and production source remain unchanged.
- The first full-gate attempt passed placement/format/lints, then failed compilation with ENOSPC on the 63GiB `/tmp` tmpfs. Only this task's 16GiB target cache was moved to NVMe; a symlink preserves its original Cargo path and test scratch remains on `/tmp`. No source files or other worktrees were removed.
- Full-gate retry: PASS, `All repository gates passed.`, 1122.94s wall time; advisories/bans/licenses/sources all pass. Debug/release workspace tests and the runner's complete ignored-budget inventory were exercised.
- Comparable serial release: `cargo test --release --config profile.release.debug-assertions=false -p resourcefs-sources --all-features --test atlassian_fixture_operator_contract -- --test-threads=1` passes all78 tests in 818.25s, versus1179.10s baseline: 30.6% lower test execution time, excluding compilation. This satisfies R3's30% acceptance target. The full-gate capture elided its operator rows, so this dedicated run provides the exact measurement rather than inferring it from total gate time.
- Rust pre-commit checklist reviewed: test-only immutable shared state, owned private scratch paths, explicit I/O failures, no new public/production API; formatter, clippy and complete tests pass. The integrated writing worktree was retired after proof. Domain/agent guidance and live records intentionally unchanged because no domain or production contract changed.
