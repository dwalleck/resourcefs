# Plan: rfs-bcym

## Integration budget and PR increments

- Slice diff estimates: Slice 1 = 2,650 changed lines; Slice 2 = 30 changed lines; sum = **2,680**.
- Churn margin: **25% = 670 lines**. Shell transport/state code and the stateful fake-curl harness are escaping-sensitive and likely to grow during named-mutation repair; 25% covers that risk without masking a second feature.
- Projected total: **3,350 changed lines**.
- Review-size gate: 3,350 ≤ 4,000, so one PR increment is sufficient.
- **Increment A — Complete fixture operator:** Slices 1–2. Mergeable definition: the checked-in manifest and independent operator satisfy deterministic lifecycle contracts, the real disposable-tenant gate and independent visibility/absence oracle pass, the evidence/route terminal result is recorded, and the ticket is closed. Slice 1 verifies without Slice 2 through the permanent fake-curl contract; Slice 2 adds the external acceptance proof.

## Slice 1: Implement the complete manifest-driven operator and deterministic lifecycle fence

**Claim IDs:** C1, C2, C3, C4, C5, C6, C7, C8, C9, C10, C11

**Expected behavior:** `scripts/atlassian-fixture-bootstrap.sh` provides strict `bootstrap`, `verify`, and `cleanup` modes over the checked-in manifest; bootstrap is collision-safe and idempotent, verify checks provisioner/reader authority and forced pagination, cleanup is owned/repeat-safe and polls asynchronous space deletion, state is atomic/minimal, and every failure is bounded and secret-free.

**Oracle:** The Rust integration harness computes the manifest logical graph independently, drives an actor-aware fake REST object store through a PATH-injected `curl`, and compares request/state logs, filesystem metadata, captured channels, and final object graphs rather than shell internals. C1 additionally retains the independently rendered Atlassian operation-page comparison from `.rfs-bcym/evidence.md`.

**Stress fixture:** A strict manifest containing two Jira projects, two issues plus two comments in the primary project, one issue in the secondary project, one ordinary and one private Confluence space, a root/child/sibling page hierarchy, two footer comments, complete ADF with Unicode and nested nodes, storage markup with entities/macros/tables/Unicode, and owner-marker strings with spaces. Negative rows cover empty/single/duplicate collections, duplicate secondary keys, bad parent references, Unicode/control/path inputs, foreign reserved-key collisions, malformed authority, repeated continuations, partial state, concurrent lock contention, canary credentials/raw prose, and 202 long-task progress/failure/timeout. Expected: the valid graph reaches exact stable identity and visibility; each negative row fails before forbidden egress/state/deletion/output.

**Regression fence:** `crates/resourcefs-sources/tests/atlassian_fixture_operator_contract.rs` — creates the fake `curl`, invokes the shipped script through its interface, and owns all C2–C11 named tests; `.rfs-bcym/probe_openapi.py` remains the permanent C1 vendor-contract fence.

**Named mutation:** Apply each approved design mutation at checkpoint: C1 change expected Confluence delete 202→204 in `.rfs-bcym/probe_openapi.py`; C2 accept mode `apply`; C3 remove duplicate Jira key rejection; C4 add `set -x`; C5 skip lookup before POST; C6 replace temp+rename with direct state write; C7 run reader verification with provisioner credentials; C8 treat Confluence 202 as complete; C9 ignore `_links.next`; C10 echo a captured 400 body; C11 delete the second-cleanup scenario. Each named fence must turn red on its own claim and green after restoration.

**Complexity/production scale:** Manifest validation/reconciliation is $O(n + r)$ for $n$ manifest objects and $r$ REST observations. The command rejects manifests above 1 MiB, more than 256 total objects, more than ten pages per collection walk, responses above 4 MiB, and more than 512 REST requests per invocation. The checked-in fixture is below 20 objects and its happy lifecycle is below 100 requests. Maximum accepted cost: 1 MiB manifest, 4 MiB per response, 256 objects, ten pages per walk, 512 requests, and 30 asynchronous polls per space; these ceilings keep accidental tenant-scale walks and unbounded cursor/task loops impossible while leaving ample room for the required fixed fixture.

**Wall budget/phase:** N/A — reason: one-off operator command; no always-on request or background phase is introduced. Each REST request has a 10-second connect and 30-second total timeout; asynchronous delete polling is capped at 30 two-second waits per space.

**Files:** Create `scripts/atlassian-fixture-bootstrap.sh`, `fixtures/atlassian-live/manifest.json`, `crates/resourcefs-sources/tests/atlassian_fixture_operator_contract.rs`, and its private fake-response/request fixtures if separation improves locality; modify `.gitignore` to ignore only `/.resourcefs/atlassian-fixture-state.json` plus its lock/temp names. No ResourceFS production `.rs` file changes.

**Estimate:** 1.5–2 engineering days; signal only.

**Diff estimate:** 2,650 changed lines: operator ~900, manifest ~180, Rust harness/fake upstream ~1,350, private fixtures ~200, ignore rule ~20.

**PR increment:** Increment A — Complete fixture operator.

**Commands and expected results:**

- `python .rfs-bcym/probe_openapi.py` → C1 emits the pinned Jira/Confluence lifecycle, identity, storage, pagination, private-space, and long-task facts and agrees item-by-item with `.rfs-bcym/evidence.md` P1–P5.
- `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract -- --nocapture` → the valid lifecycle yields the exact manifest-derived provisioner/reader graphs, second bootstrap performs no creates, verify follows every second page and enforces private-space concealment, cleanup reaches an empty owned graph twice, and every negative matrix row has its claim-specific failure/output/request/state result.
- Checkpointed-build applies the C1–C11 named mutations one at a time → each named regression fence goes red for its own claim; after restoration the same focused commands return green.
- `sh -n scripts/atlassian-fixture-bootstrap.sh` and `jq -e . fixtures/atlassian-live/manifest.json` → shell syntax and manifest JSON parse successfully; these are secondary static checks, not substitutes for the behavioral fence.

## Slice 2: Prove the complete lifecycle against the disposable Atlassian tenant and close the evidence gate

**Claim IDs:** C12

**Expected behavior:** With an approved disposable Jira+Confluence tenant and separate provisioner/reader principals, bootstrap twice, verify, cleanup twice, and final independent absence checks all pass; the second bootstrap performs no fixture writes, the reader sees every public fixture and no private-space object, cleanup leaves no reserved keys/markers, and no captured channel or state contains either credential.

**Oracle:** The operator result is compared with Atlassian's independent web surfaces under the provisioner and reader principals: provisioner UI/API explorer confirms the full stable-ID/parent graph before cleanup and reserved-key absence afterward; reader UI confirms public visibility and private-space absence. This differs from both the production `curl`/`jq` command and the Python OpenAPI probe.

**Stress fixture:** The exact checked-in manifest on the real tenant, including two-row forced pagination, nested pages, ADF/storage edge values, two comments, a private space, repeated bootstrap/cleanup, and final absence. Expected: exact shape/visibility agreement without assertions on mutable timestamps, tenant-wide counts, ETag presence, or ordering absent an explicit sort.

**Regression fence:** `cleanup_waits_for_owned_space_deletion` and `operator_contract_covers_lifecycle_matrix` in `crates/resourcefs-sources/tests/atlassian_fixture_operator_contract.rs`; the live run is one-shot evidence while these deterministic fences retain the measured lifecycle semantics.

**Named mutation:** From C12, change v1 space-delete polling to one attempt; checkpointed-build must observe `cleanup_waits_for_owned_space_deletion` red before restoring it and rerunning green prior to the live command.

**Complexity/production scale:** The real run uses the Slice 1 ceilings: fewer than 20 fixed objects, fewer than 100 happy-path requests, at most ten pages per walk, 4 MiB response bodies, and 30 two-second asynchronous polls per space. Maximum accepted cost: the command must remain inside those exact ceilings; exceeding any is a failure, not a reason to widen the tenant walk.

**Wall budget/phase:** N/A — reason: one-off approved lifecycle run; no always-on phase. Each request and long-task poll retains Slice 1's explicit timeout/bound.

**Files:** Modify `.rfs-bcym/evidence.md` with the dated lifecycle fixture/oracle comparison, `.rfs-bcym/route.md` with the terminal PASS result, and `.rivets/issues.jsonl` only through `rivets close`; ignored local state is removed by cleanup and never committed.

**Estimate:** 1–2 hours plus tenant response time; signal only.

**Diff estimate:** 30 changed lines in evidence, route result, and tracker store.

**PR increment:** Increment A — Complete fixture operator.

**Commands and expected results:**

- `scripts/atlassian-fixture-bootstrap.sh bootstrap --site "$ATLASSIAN_FIXTURE_SITE_URL"` twice → both commands succeed; the first produces the exact stable graph and atomic state, the second makes zero fixture-create/replace requests according to operator summary and independent tenant observation; captured output/state contains no credential bytes.
- `scripts/atlassian-fixture-bootstrap.sh verify --site "$ATLASSIAN_FIXTURE_SITE_URL"` → provisioner and reader graphs agree with the manifest; reader sees public objects, cannot see the private space/object, and every offset/token/cursor family advances through page size one.
- `scripts/atlassian-fixture-bootstrap.sh cleanup --site "$ATLASSIAN_FIXTURE_SITE_URL"` twice → both succeed; the first polls each space deletion to terminal completion and removes every reserved marker/key, the second observes absence without a destructive request.
- Independent provisioner/reader Atlassian web checks → before cleanup, stable IDs/parents/representations and visibility agree item-by-item with state/manifest; after cleanup, reserved Jira project keys, Confluence space keys, and child markers are absent for both principals.
- `rivets close --json -y rfs-bcym --reason "Implemented and proved the manifest-driven Atlassian fixture lifecycle; recorded bootstrap/verify/cleanup and independent visibility evidence."` → ticket is closed only after every comparison above passes.

## Self-review

- [x] C1–C12 are each assigned exactly once; every PENDING falsifier is owned by the slice implementing its claim.
- [x] Both slices contain all thirteen mandatory fields; every conditional field carries an explicit `N/A — reason` where applicable.
- [x] Every claim's permanent fence is created or retained in its implementing slice, and every approved named mutation is copied mechanically.
- [x] Every new loop records asymptotic cost, production-shaped bounds, and a checkable maximum; no always-on phase is introduced.
- [x] The 2,680-line sum, 670-line/25% churn margin, 3,350-line total, and one mergeable increment satisfy the review-size rule.
- [x] Deferral taxonomy applied: no intended future work is introduced; permanent non-goals remain in the approved design.
- [x] No slice is declared complete; checkpointed-build exclusively judges completion.
