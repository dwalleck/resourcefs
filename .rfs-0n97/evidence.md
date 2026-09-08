# Evidence: rfs-0n97

Source state: `7de8d1477af6be825bada9015a6996b3f2917c7d` in `/home/dwalleck/repos/resourcefs-wt-rfs-0n97-impl`; no production edits. Date: 2026-09-08.

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|---|---|---|---|
| P1 | Selected-version GitHub REST exposes directly usable native PR and base/head repository/commit facts. | Do Python urllib/json and gh HTTP plus jq agree on selected native identities, field presence/types, links and safe HTTP metadata for public rust-lang/rust PR 159232 with REST 2022-11-28? | PASS |
| P2 | Existing native diff and conditional/error signals are usable provider capabilities. | Does retained applicable provider evidence establish native diff media/bytes and ETag/status/header semantics? | PASS — retained external-capability evidence from rfs-45ww P3/P4/P5, not new adapter compatibility enforcement |
| P3 | Existing source/HTTP/engine seams already expose all required controls and response facts. | What do the current exported interfaces actually retain? | N/A — direct current-source inspection resolves capabilities and gaps; additions are design obligations, not a premise that existing code already implements them |
| N1 | New facts projection, safe detail mapping, per-call propagation and complete new MCP contract. | The feature does not exist yet. | N/A — design and checkpoint proof obligations |
| N2 | Ten-attempt/thirty-second/body/serialized targets and production allocation cost. | Targets are approved behavior, not empirical facts about a new implementation. | N/A — design/plan/checkpoint obligations; model arithmetic is not enforcement proof |
| N3 | Corporate ghe.com identity/scopes/native Windows operation. | Actual tenant and native acceptance belong to the separate Cyril gate. | N/A — not claimed by public-provider proof |

## Data

- Source: production-shaped public read-only GitHub REST response, fixed `rust-lang/rust` PR 159232.
- Shape: native PR with distinct actual head/base commits and repositories, native numeric/node identities, provider-supplied public URLs, field-presence/type observations and HTTP validators/times.
- Safety: GET only, explicit `api.github.com`/`github.com`, no remote mutations, private tenant, PR code or credential-file access. Both clients supplied no tokens; gh ran with empty token variables, a nonexistent isolated config directory, fixed public hostname and prompts disabled. Full provider bodies were not retained or printed.
- The sample has present, non-null selected metadata. Deleted forks, missing/null/empty alternatives and malformed identities require controlled implementation fixtures and schema validation; this sample is not evidence those alternatives were exercised.

## Probe

- File: `probe_provider.py`.
- Mechanism: Python urllib GET with selected REST/Accept headers, no redirects, 20-second socket timeout and 1 MiB plus one-byte body admission check; Python JSON projection records only selected public facts and field presence/types.
- Run: `python3 .rfs-0n97/probe_provider.py` from the isolated worktree; exited successfully.
- Output: `probe-provider-result.json`.

## Oracle

- Mechanism: independent gh HTTP client and external jq parser/projection, distinct from both Python urllib/json and production Rust reqwest/serde. Exact jq program is retained in the probe; it is separately expressed rather than a shared projection function.
- Run: `gh api --hostname github.com --method GET --include -H 'X-GitHub-Api-Version: 2022-11-28' -H 'Accept: application/vnd.github+json' repos/rust-lang/rust/pulls/159232`, with the isolated no-token environment described above; pipe captured body privately to the retained external jq program.
- Output: `oracle-provider-result.json`.
- Both gh and jq have 20-second subprocess limits. gh output size is checked after capture, not streaming; this oracle is not a transport-memory-bound demonstration.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|---|---|---|---|
| P1 | PR number 159232; native id 4045557328; node ID PR_kwDOAAsO6M7xIk5Q. Head 2686c8c0128c0cd672f5b6df5603f7743f64b846 in Rani367/rust (repository id 1299384074); base cc05892c8346313865afd91ca12ee1fde6d3603c in rust-lang/rust (repository id 724712). Selected field presence/types, public links and native times retained. | Independently projected semantic observations match exactly. Both responses report 200 and selected version 2022-11-28; observed ETag/Last-Modified/Date also happen to match and are compared separately. Main loaded and compared the two retained JSON observations, independently of the agent's summary. | PASS |
| P2 | Retained `.rfs-45ww/evidence.md` P3 records native diff media, 7,546 bytes and direct Python provider observation; P4 records conditional ETag/304; P5 records safe status/header signals and documented ambiguity. | That record contains independent gh results and official provider contracts. The same source revision still uses native Accept and ETag/status machinery; no change to those external-capability conclusions is introduced by this ticket. | PASS — retained capability only |

## Validated / learned

- P1: validated prior understanding — selected-version PR payloads directly supply actual head/base repository and commit identities, without acquiring fork content or substituting synthetic merge identity. This removes any need to infer them from Markdown or perform speculative identity fetches on this observed shape.
- P1: learning — HTTP Last-Modified is `Mon, 07 Sep 2026 10:31:55 GMT`, while native PR updated_at is `2026-08-26T01:33:54Z`. They are distinct metadata and must not overwrite one another. HTTP Date and acquisition interval are separate again.
- P2: retained understanding — provider native diff and conditional/status signals remain reusable, but existing adapter tests/live rows do not independently fence unpaged native `/diff` Accept, byte exactness and truncation refusal. The accepted first-slice compatibility scenario must add that real source-level proof; retained provider evidence does not discharge it.

### Existing-system inspection affecting design

- SourceAdapter::read currently takes reference and OperationGuard only; ReadRequest contains display controls only. OperationGuard is cancellation/commit state shared with mutations/discovery, not an acquisition-control container. The clean-cutover caller map includes seven production adapters, three core-test adapters, direct consumers, CompiledSources and GitHub discovery's self.read path.
- BoundedHttpResponse retains ETag/Link/retry-after/rate remaining, but GitHub fetch/cache discard much of that context and preserve only body/Link at the decoder. New fact provenance must distinguish original cached body metadata from the revalidation response, not invent a latest-body acquisition.
- Reqwest is pinned to 0.13.2. Its default retry constructor permits protocol-NACK retries, but the actual classifier (`src/retry.rs` 270–314) can return true only under http2/http3 features. `cargo tree --workspace --all-features -e features -i reqwest` completed successfully in 0.16 s and resolved only __rustls, __rustls-aws-lc-rs, __tls, rustls and stream; neither protocol feature is enabled, including transitive unification. This corrects the preliminary inference that the current binary could exercise those NACK retries. Do not change mutation/client retry behavior or enable protocols just to manufacture a test. Physical-request proof must observe the enabled transport and substrate retries, and a resolved-feature fence must preserve the premise. No NACK failure or mutation duplication was reproduced.
- Current budgeted fetches refuse redirects through the existing nonredirecting client. Per-call deadline/body lowering and cumulative/serialized admission are missing implementation behavior, not proven by existing attempt counters.
- The active Linux `.rfs-nae2/oracles/module_shape.py --stage query` freezes protected parent tokens/bodies and globally constrains new production files. The proposed owner additions require an approved successor fence transition with retained prior obligations; a new unenforced local ledger is insufficient.
- Existing test-only stdio fixture trust is routed only to HTTPS; GitHub mounting uses system roots. Real GitHub loopback MCP proof needs explicit test-only trust propagation while shipped profile/CLI/environment remain unable to select fixture trust.
- LSP references returned no matches for known SourceAdapter/read callers in the linked worktree. This was reported as a tooling issue; scoped caller searches provide the current map, not a false claim of semantic completeness. Repeat/fallback semantic verification at the actual clean cutover.

Inspection reports: `agent://PrFactSeamScout` and `agent://ReadContractSeamScout`; concrete source facts above are retained here rather than relying on those transient handles alone.

## Verification observations and limits

An attempted `python scripts/ci-gates.py --help` unexpectedly executed the runner because it does not implement help handling. Module shape, formatting check and clippy completed; the command timed out at 30 seconds during functional test compilation. **The full gate did not pass.** This is not implementation acceptance. It confirms the existing active shape fence; all later assembled checks remain required after implementation. No production file was changed by this invocation.

Public github.com observations do not prove selected corporate ghe.com compatibility, authentication/scopes, native Windows behavior, private access or end-to-end Cyril capture. Existing-body cache semantics, all failure variants, deadlines/bytes and new JSON reconstruction remain implementation proof obligations. No design or production code was written by the probe stage.

## Related issues

- rfs-0n97: owning PR-facts slice.
- rfs-45ww: retained native provider observations and original adapter/live behavior.
- rfs-g2z9: existing HTTP substrate; no duplicate implementation.
- rfs-jsx9: output recovery/source continuation, ADR-0006 unchanged.
- rfs-bfwa: subsequent conversation collection retention/resumption.
- rfs-jrz7: subsequent exact commit/source acquisition.
- No new issue filed by this stage; the current slice owns its required controls, metadata and proof-path changes.

## Probe-stage hand-off

P1 is fresh PASS; P2 is retained PASS with narrowed applicability; P3/N1–N3 are classified N/A with reasons. Both observed P1 artifacts and runnable probe are retained. Hand off to falsifiable-design; production implementation and design approval remain pending.

## S0 checkpoint — structural transition

Local commit authorization: requester selected **"Local checkpoint commits only"** on 2026-09-08. No push, PR, merge or ticket closure is authorized. The plan now records local review partitions.

Initial baseline qualification found two false positives: existing ErrorCategory serialization and the existing core ResourceAddress::Jira arm. Corrected the policy to preserve the existing category serialization while rejecting additional serialization ownership, and to reject concrete provider implementations rather than already-approved core address vocabulary. No production exemption or new source responsibility was introduced.

The changed oracle was run against the unchanged pinned production tree and independently constructed disposable source mutations. `oracles/s0-results.json` retains all nine observed outcomes. Parent-body and unrelated-server mutations preserve physical line counts, proving semantic placement/freeze detection independently of the size cap. The forbidden decoder mutation has a matching disposable Cargo dependency, rather than relying on unresolved Rust names. Disposable trees were removed; production source was never mutated.

| Gate | Result |
|---|---|
| 1. Affected unit tests | N/A — S0 changes no production behavior and names no Rust unit test; its executable CLI is exercised by gates 2–4. |
| 2. Falsifiers | PASS — baseline accepts existing owners; all four planned violations fail with their offending path and responsibility. |
| 3. Stress fixture | PASS — unlisted module, new parent serializer, core provider decoder, and unrelated frozen server body each rejected. |
| 4. Implementation / independent oracle | PASS — actual CLI verdicts agree with independently authored baseline/delta expectations; exact offending declarations and paths match the injected source, not a shared production parser. |
| 5. Approved module shape | PASS — successor baseline check retains inherited h212/nae2 assertions; no production ownership has moved. |
| 6. Budget | PASS — recorded oracle runs 3.767–4.363 s, below 10 s; runtime-loop and runtime wall budgets N/A per S0. Complexity description corrected conservatively in plan.md. |
| 7. Regression fence | PASS — baseline CLI accepts the unchanged production tree. |
| 8. Named mutations | PASS — all four RED; declaration/frozen-body cases remain RED at unchanged physical line counts. |
| 9. Restoration | PASS — each mutated tree restored and the same CLI returned GREEN. |

Caller/helper review: the runner's Linux shape command delegates to the successor; other runner commands remain unchanged. Existing inherited loaders, production masking, baseline discovery and responsibility checks are reused. Legacy direct nae2 invocation has unchanged default policy. S0 creates no production parallel path; runtime error/logging/fallback symmetry is N/A. Stale-reference review retains prior probe conclusions as historical evidence rather than claiming implementation completion.

S0 drift reconciliation: origin/main advanced by three Atlassian fixture-operator/test-bootstrap commits; its changed-path inventory contains no production Rust path and no S0-owned path. Rebased the unpublished local checkpoint onto that upstream without conflict. Fresh baseline C02 PASS after rebase (4.14 s command wall time). Gates 2–4 and 7–9 retain their exact unchanged-source mutation evidence; gate 5 is fresh. No publication occurred.

## S1 checkpoint — source-neutral read controls and details

Integrated the complete seven-adapter signature cutover and every compiled caller. Existing SourceAdapter/ReadEngine/CompiledSources seams remain; no extra provider trait or Facts placeholder. The writer's empty linked-worktree LSP result required scoped and macro-contained caller inspection. `oracles/read-callers.json` retains the post-cutover structural candidate locations; `cargo check --workspace --all-targets --all-features` is the semantic omission fence, including test-support consumers.

`oracles/s1-results.json` records commands, mutation outcomes and the actual standalone release probe source/output. Focused checks passed 17 core tests, 18 source tests and the dependency test. All-target/all-feature checking and clippy passed. The only qualification repair was replacing a manual Result-to-Option match with an audited `.ok()` for an unrepresentable optional diagnostic number: invalid Duration still returns the same typed error, never success. Existing guard-observation coverage is retained because it detects a source continuing expensive work after caller cancellation, not merely an echoed argument.

| Gate | Result |
|---|---|
| 1. Affected unit tests | PASS — acquisition/read-engine and compiled/direct-source contracts, 35 tests total. |
| 2. Falsifiers | PASS — dependency fence plus unequal five-dimensional limits, invalid boundaries and observable unsupported-control refusals. |
| 3. Stress fixture | PASS — independent unequal minima, integer/Duration invalid boundaries, absent/present details, and successful no-override controls paired with direct/compiled refusals. |
| 4. Implementation / independent oracle | PASS — explicit mathematical boundary/minimum corpus and actual public source outcomes; the standalone program performs real validating constructors/intersections, not a mock. |
| 5. Approved module shape | PASS — C02 core stage after formatting/restoration; the new core owner is pure, old source owners only reject/forward controls, protected unrelated bodies remain frozen. |
| 6. Budget | PASS — 100,000 constructor/intersection pairs: 1,363,632 ns, checksum 490,000, versus 100,000,000 ns ceiling. Limits/detail payloads remain finite scalar values without heap fields. Background phase N/A per S1. |
| 7. Regression fence | PASS — focused contracts and architecture fence. |
| 8. Named mutations | PASS — all-five min→max, zero→one clamping, disabled direct-local refusal, and actual compiling core JSON decoder plus dependency each produced its intended failure. |
| 9. Restoration | PASS — each exact source restoration followed by its focused GREEN; final architecture and core placement checks also passed. |

Symmetry/reuse: absent-control behavior is unchanged; explicit controls fail consistently with `unsupported_projection`/`acquisition_controls_unsupported`, rather than silently falling through. Existing error category/message constructors, cancellation guard, source dispatch, and read recovery remain authoritative. No new path normalization, logging or fallback subsystem. Core public doc comments describe the new validating values/details; external MCP/profile schema docs remain intentionally unchanged here because this slice does not yet expose new protocol fields.
