# Evidence: rfs-jrz7

Date: 2026-09-09. Checked source and upstream revision: `635ab170ab57542c18272921298d575da2f8b08a`. Canonical directory: `.rfs-jrz7/` in `../resourcefs-wt-rfs-jrz7`. Route: `route.md`. spec.md: N/A — approved ticket behavior is fully recorded in route T4.

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|---|---|---|---|
| P1 | Selected-version GitHub immutable acquisition seam | Do REST 2022-11-28 commit metadata, nonrecursive tree entries and base64 blob bytes for this pinned nested regular file match the local Git object database? | PASS |
| P2 | Proposed github:// segment grammar and selector handling | N/A — feature not yet built: current ResourceAddress has no github:// variant. The proposed once-only decoding/refusal behavior must be demonstrated at design/build, not inherited from PR parsing. | N/A — feature claim |
| P3 | Hard numerical limits, cancellation, forbidden egress, retries, cache generations and exact binary/special-object acceptance | N/A — requirements on the implementation to be built. Existing probes cannot establish its future enforcement. | N/A — design/checkpoint obligations |

## Data

- Source: safe production-shaped existing repository snapshot, `dwalleck/resourcefs` pinned to `635ab170ab57542c18272921298d575da2f8b08a`, already available in the local Git object database.
- Shape: merge commit with two parents, three non-recursive directory levels and regular blob `crates/resourcefs-core/Cargo.toml`; complete sibling tree entries compared at every level.
- Safety: only explicit GET calls through existing gh credential handling to the repository's configured GitHub API; local Git plumbing is read-only. No upstream changes, fork expansion, URL following or tenant access. The probe never prints credentials. This is not a production forbidden-egress observer or adapter acceptance run.
- Approval: request to implement the issue includes applicable read-only public-upstream proof; no new corporate deployment/principal operations.

## Probe

- File: `probe.py`.
- Mechanism: gh REST GET responses decoded in Python; select the pinned tree chain by exact path component and decode the terminal blob's base64.
- Run: `python .rfs-jrz7/probe.py` from `../resourcefs-wt-rfs-jrz7`.
- Result: exit 0; exact retained output in `probe-result.json`.
- Five explicit GETs: commit, root tree, crates tree, resourcefs-core tree, blob. REST pin: `2022-11-28`. This is probe request count, not implementation budget evidence.

## Oracle

- Mechanism: local `git cat-file commit`, `git ls-tree -z` and `git cat-file blob` read the Git object database, independent of provider REST payload construction, provider paths, JSON/base64 decoding and future production HTTP acquisition.
- Run: invoked by `probe.py`, pinned object IDs only. Comparison output records the requested SHA, selected trees/blob and decoded SHA-256. Message comparison ignores final newlines solely to compare native provider text with Git storage; both observed trailing-newline counts are separately recorded (zero in this sample). Commit message is provider metadata, not a claim of raw commit-object bytes.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|---|---|---|---|
| P1 commit | SHA `635ab170...`, tree `c440246484e340dfe1d43fda5dcc31ea6b03a89a`, two parents recorded in JSON, native message | Local commit object tree/parents/message agree; both message trailing-newline counts 0 | PASS |
| P1 root tree | `c440246484e340dfe1d43fda5dcc31ea6b03a89a`, 43 entries, not truncated | Every path/mode/type/SHA agrees with ls-tree, independent of ordering | PASS |
| P1 crates tree | `3970e21fe17460acbbb7e51fbe7ebbce621cd2a2`, 3 entries, not truncated | Every path/mode/type/SHA agrees | PASS |
| P1 core tree | `560584718bd3ad724703d9612a49692dab774646`, 3 entries, not truncated | Every path/mode/type/SHA agrees | PASS |
| P1 blob | `8b57d630ab71fbc6daa17d949b311029362ff5aa`, 1,063 decoded bytes, SHA-256 `dbef110105ca17fba63d1785fce00321b083c30310062abd2218c0154619698b` | Exact byte equality with cat-file; provider size equals decoded length | PASS |

## Validated / learned

- P1: validated prior understanding — the selected-version commit/non-recursive-tree/blob seam provides the pinned nested regular-file identities and exact bytes, independently corroborated by Git plumbing. No Contents/raw-download endpoint is required for this observed chain.
- P2 classification correction: code inspection shows no existing github:// parser to reuse. Core must gain an explicit family arm and segment-aware grammar. PR's decode-whole-body-before-splitting is not the requested source grammar.
- Limits: this probe does not establish binary/symlink/submodule/LFS provider shapes, pathological Unicode trees, decoded boundary enforcement, actual ResourceFS request/deadline/allocation bounds, live adapter or MCP acceptance, cache/integrity overrides or corporate/native acceptance. Those remain design/build falsifiers and live smoke obligations, not inherited PASS results.

## Related issues

- Consulted: rfs-jrz7; rfs-0n97 prerequisite is present on main via merged PR #9 despite stale tracker status. One bounded tracker search (`rivets list --json --limit 200`, filtering immutable commit/exact source/Git tree/commit-path terms) additionally found rfs-e5cv (immutable tree comparisons) and rfs-iktl (PR commit/native-file facts); both describe separate future families, not prerequisites to this singular source path.
- Filed: none — no underlying-system defect found.

## Hand-off

P1 PASS with retained runnable probe and independent comparison; P2/P3 are classified non-premises. No production code or feature tests changed. Hand off to falsifiable-design; exact grammar/schema/placement still require requester approval.

## S2 implementation evidence — 2026-09-10

The preceding hand-off records the historical pre-design probe, not current implementation status. S1 is committed as `5d941fb`, draft PR #12; both Actions runs passed. S2 implements only immutable commit Facts; exact source content remains S3.

- Independent native/Git comparison rerun: `commit-oracle-result.json`. At `635ab170ab57542c18272921298d575da2f8b08a`, exact tree, ordered parents and all 69 provider message bytes agree with `git cat-file commit`; this is the external oracle, not production proof by itself.
- Immutable SourceAdapter contracts pass — sixteen at this head (ten at the S2 tip; the review-repair round added GitHub App account acceptance and its refusals, unusable logins, the absent/null required link, and the escaped-deployment-prefix case) — including correct/contradictory identities, malformed native metadata, zero native ID, absent/null accounts, hostile account text, separate response ETags, and an Enterprise `/api/v3/` deployment.
- Deterministic stdio contract over the production serve path (`github_commit_facts_stdio_reconstructs_overflow_without_reacquisition`): the row starts the compiled `stdio_mcp_contract` test binary (`std::env::current_exe()` with `--ignored --exact profile_https_test_server`), not the shipped `resourcefs` binary; that helper calls `resourcefs_mcp::test_support::serve_profile_with_https_root` (`LaunchPlan::from_profile_with_https_root` then `server::serve`), so repository then commit under two attempts, owned schema and native values, selector/mutable-operand refusal, catalog exposure, and byte-exact artifact recovery without further acquisition all pass over the real MCP stdio transport and the shared production launch/serve/acquisition path. Only CLI argv/profile plumbing and shipped-binary process identity are bypassed (the fixture CA is not selectable through the CLI); the shipped binary is started by the credential-gated live row below, which asserts none of these selector/catalog/refusal specifics.
- C3/C9: compiled red and exact-restored green; receipts in `mutation-c3-commit.txt` and `mutation-c9-commit.txt`. S2-F2's initially masked mutant is explicitly recorded, not counted as successful proof.

### Live commit rows

Both rows were invoked with `RFS_LIVE=1` and a nonempty credential obtained from `gh auth token`; credentials were not written to evidence. Both used `dwalleck/resourcefs` at the exact pinned commit above. Skip status is not receipt-observable: both rows begin `let Some(token) = live_token() else { return };`, which prints a skip notice and returns green when `RFS_LIVE`/`GITHUB_TOKEN` are unset, and libtest captures output from passing tests, so the retained receipt is identical either way. The recorded test times (1.97 s and 2.09 s; a skip returns in microseconds) are the only receipt-side signal that both rows ran, and a genuine live run silently omits its `gh`-CLI native comparison when `gh` is unavailable, so the receipt cannot prove that half either.

| Row | Exercised surface | Result |
|---|---|---|
| `live_github_immutable_commit_facts_hold_up` | Real configured SourceAdapter against GitHub, comparing native repository/commit observations | PASS; 1.97s test time |
| `live_stdio_github_commit_facts_match_native_observation` | Actual `resourcefs check --probe`, actual server binary and MCP read/recovery against GitHub, native metadata comparison | PASS; 2.09s test time |

Raw nonsecret row output: `live-commit-results.txt`. These results prove commit acquisition only; no source-file byte or object-validation claim is made here.

## S3 implementation evidence — 2026-09-10

The source implementation uses the requester-approved gix-object/gix-hash crates for native Git format semantics, with ResourceFS acceptance policy around their typed objects and streaming hashing writer. `git-object-adoption.json` records the adopted dependency graph; cargo-deny passed. The independent corpus generator `build_git_corpus.py` uses local Git plumbing, not production hashing, and emits `tests/fixtures/github_immutable.json`. The separate crate-adoption experiment reproduced 13 trees, two width templates and seven blobs with streamed native headers and Git's directory-aware ordering.

- Assembled commit/source contracts: 21 PASS across `github_immutable_contract` and `github_source_contract`. The source matrix exercises exact bytes/once-decoded paths, complete-tree integrity, 1,000/1,001 entries, native modes, decoded exact/over limits with equal encoded lengths and absent sizes, metadata-only retention, one shared deep traversal budget, 304 provenance, failed revalidation, cancellation and generation invalidation.
- Actual configured binary stdio source contract: PASS, including the operator/caller decoded bound, binary reconstruction, artifact recovery and zero reacquisition during recovery. This found and fixed a duplicate `github://` catalog entry that had prevented configured server initialization; the single entry now advertises both families.
- C4/C5/C6/C7/C8/C10/C11: named mutants RED, exact-restored GREEN. Receipts are the corresponding `mutation-c*-*.txt` files. C4's initial masked fence is retained transparently; its corrected fixture changes only an unselected valid tree-entry name while retaining the SHA/link metadata. C8 also turns RED when the final generation check is moved before serialization, isolating the required boundary rather than merely detecting any generation check.
- C8's calibrated allocation callback and external generation controller pass in both debug and optimized release. The callback is test-only and disarms before controller/channel work; calibration failure fails the test rather than silently passing. Its allocation-shape sensitivity remains an explicit maintenance consideration.
- Maximum source release measurement: 5,594,189 native response bytes; 5,595,590 owned output bytes; 25,591,923ns local wall; 22,407,541 incremental tracked heap bytes. These are below the approved 3s/160MiB limits, not claims about network latency or process RSS. The fixture is registered in the repository runner as `immutable_source_production_budget`.
- The exact representation-bound fixture now freezes Tokio elapsed time while retaining real loopback I/O, so its fixed point does not depend on volatile `elapsedMs` width. Exact equality and one-byte-under refusal remain asserted; the corrected test passes.
- Isolated final conformance: `blind-module-map.md` preserves the production-only map before ledger disclosure; `design-conformance-review.json` preserves the initial verdict. The reviewer found no actionable defect or ownership mismatch. Its subsequent review of the final public docs and clock-only fixture delta found no new issue and preserved applicability. This review ran no validation; full quality/platform results belong to the checkpoint record.

### Live source and assembled roster

Both targeted source rows were explicitly enabled with `RFS_LIVE=1` and a nonempty credential obtained through existing `gh auth token` handling. No credential was written to evidence and neither row skipped. Both use `dwalleck/resourcefs` at `635ab170ab57542c18272921298d575da2f8b08a` and compare the exact native blob.

| Row | Exercised surface | Result |
|---|---|---|
| `live_github_immutable_source_facts_hold_up` | Configured SourceAdapter, validated tree chain and exact native blob comparison | PASS; 3.97s targeted test time |
| `live_stdio_github_source_facts_match_native_blob` | Actual profile/probe/server binary, MCP source read and recovery, exact native blob comparison | PASS; 4.38s targeted test time |

`live-source-results.txt` retains targeted output. The complete `scripts/live-smoke.sh` roster then completed successfully in37.94s: 13 actual network rows passed (six adapter GitHub, six stdio and one HTTPS); five Jira rows skipped for absent environment and are not counted as live proof. `live-roster-results.txt` retains the row outcomes and explicit skips. Corporate/Jira live acceptance is not a claim of this GitHub ticket.
## PR #13 review repair round — 2026-09-11

Review: `docs/research/pr13-code-review-2026-09-11.md`, 13 findings and 14 below-the-line items against head `1cfa020`; the re-review that followed is `docs/research/pr13-rereview-2026-09-11.md`. Per-finding decisions: `review-decisions.md` § "PR #13 review repair round".

Repairs by area, all on this branch:

- **F1 (the increment's only behavioural defect on ordinary upstream data).** `validate_account` no longer rebuilds an account's links from its login. Each supplied link's deployment authority is checked, and a link whose path is one of the account's own routes is compared on **decoded** path segments, so a GitHub App account is accepted on `/users/dependabot%5Bbot%5D` (API) or `/apps/<slug>` (web). A second declaration of the deployment-authority predicate was avoided by sharing `identity::clean_authority`; the placement gate's identity entry-point census therefore permits it explicitly.
- **F5, F12.** Presence is required before the shared required-link verdict, so an absent commit `url` is `upstream_malformed` like every other absent required field rather than `not_found`. An empty or `.`/`..` login is refused as `upstream_malformed`; with decoded-segment comparison no expectation can be built through a segment-skipping API.
- **F6, F10, T1.** `ParentLinks` deleted in favour of the shared `facts::Links`; the second address match in `facts_resource` no longer re-derives the first match's guarantee; one `missing!` macro is shared by the commit and pull families.
- **F7, F9.** The family refusal sentence is reworded to a claim the family can still make, and the read arm refuses the still-unserved `…/source/…` spelling before consulting the mount, so that spelling answers identically mounted or unmounted and with or without caller controls.
- **F2, T5, T6 (placement gate).** Owner uniqueness again compares every same-named declaration a counterpart declares — main's coverage, which the branch had narrowed — while keeping the `native!` subject; the docstring states what is actually hashed; the absence guard is kept because `production_nodes` tolerates a path the raw-text read would raise on.
- **F3, F4, F8, F11, F13, T4, T11, T13.** Fence and record repairs: key-presence assertion for the null account half, a `cap + 1` refusal member on the commit budget row, a commit-route effective-limits fence, the `/source/` spelling fenced in both mount states, the WrongSha row's deliberate omissions recorded, the shared live-profile source row extracted, and the stdio/live/control-route/family-index/count records corrected.

### Named mutations, re-proved on the repaired head

Both receipts in this directory were taken at the pre-repair revision and were re-run here because the repair edited the same module. Each mutation was applied to `commit.rs`, the named row observed RED, the file restored byte-identically (`sha256` compared before and after), and the row observed GREEN.

| Mutation | Target row | RED | Restored |
|---|---|---|---|
| `if observed_sha != requested.as_str() {` → `if false && …` | `immutable_commit_facts_refuse_wrong_observed_sha_even_with_correct_links` | FAILED (exit 101, 1 failed) | ok (exit 0) |
| `author_account`'s `skip_serializing_if = "Presence::omitted"` removed | `immutable_commit_facts_preserve_absent_and_null_accounts` | FAILED (exit 101, 1 failed) | ok (exit 0) |

### Fence discrimination proofs

Most fences added this round were shown to fail against a deliberately broken variant, with the file restored byte-identically afterwards; the exceptions are named so the record does not claim more than was run:

- **F1**: with `commit.rs` reverted to the pre-repair revision, `immutable_commit_facts_accept_github_app_accounts` FAILED with `upstream_identity_mismatch` — the defect the review reported, reproduced by the new fixture. It passes on the repaired source. *(Recorded mutant.)*
- **F1 (route comparison)**: decoding the provider's segment while leaving the deployment's own prefix as spelled fails `immutable_commit_facts_accept_percent_encoded_deployment_prefix`; comparing the prefix as spelled passes it and leaves the file identical. This is the regression the re-review found in the first repair, and the fence exists so it cannot return silently. *(Recorded mutant.)*
- **F3**: collapsing `Presence::Null` onto `Presence::Omitted` FAILS the repaired row ('a null account keeps its key') and **passes** the pre-repair row, which the review showed an omitted key satisfies. *(Recorded mutant.)*
- **F5**: removing `identity::required(&commit.url)?` FAILS `immutable_commit_facts_refuse_absent_or_null_required_commit_link`; restoring it passes. *(Recorded mutant.)*
- **F13**: raising the effective response ceiling makes the repaired budget row FAIL at the new over-ceiling member, while the pre-repair row (which only ever serves the ceiling) still passes — i.e. the old row could not notice a loosened ceiling and the new one can. *(Recorded mutant.)*
- **F8**: removing the profile∩caller intersection, or forcing the attempt bound up, fails the new effective-limits row at the corresponding dimension. *(Recorded mutants.)*
- **F9**: removing the read arm's `GithubAddress::Source` refusal fails both the unmounted and mounted spelling rows. *(Recorded mutant.)*
- **F2 (gate)**: the restored rule fails a verbatim copy placed in a counterpart that already declares the name twice. The regenerated `mutation-review3.txt` carries case M10 against both sides: no ownership failure line against the pre-repair `1cfa020` (the copy was accepted), and `symbol parse_pull_request_reference belongs in …` against this head. M1–M7 and M9 reproduce the previous receipt byte for byte; M8's repaired column is identical with only its reviewed label moved, because the `6e1c55c` reviewed side does not know this increment. *(Recorded harness run.)*
- **F12 and the F1 negative rows**: not separately mutated. Their discrimination was established by derivation from the pre-repair behaviour instead — the malformed-login rows fail pre-repair (which answered a link contradiction, not `upstream_malformed`), and the four negative app-account rows pass pre-repair by construction, since they exist to bound a relaxation the pre-repair code did not have.

### Full repository gate

`python scripts/ci-gates.py` on the repaired tree, 2026-09-11: **all repository gates passed** (module placement, clippy `-D warnings`, all-feature debug tests, release workspace tests, the ignored production budgets including the amended `immutable_commit_production_budget` and `immutable_reference_parse_budget` rows, and `cargo deny`). Wall clock 1840 s; the run followed a `cargo clean` of `resourcefs-sources`/`resourcefs-core` after a scratch build leaked into the shared target directory, so no result in this record predates that clean.
