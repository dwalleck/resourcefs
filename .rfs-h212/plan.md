# Plan: rfs-h212

## Approved inputs and partition

Design revision 1 was approved on 2026-09-06 UTC with “Approve revision 1”; risk acceptances: None. Route is Empirical; evidence P1–P3 PASS. C1's dependency and egress baseline checks passed. All other claims have independent oracles, explicit mutations and permanent fences; no FAIL or waiver is carried.

The approved project/issue delivery groups are partitioned further at the preserved-behavior extraction seam to honor the review-size gate:

| PR increment | Slice | Implementation/tests/fixtures/artifacts estimate | 15% churn margin | Review bound | Independently mergeable result |
|---|---|---:|---:|---:|---|
| Project preparation | S1 | 2,300 lines | 345 | 2,645 | Existing direct Jira behavior unchanged behind extracted owners; no new unsupported grammar. |
| Bounded project browsing | S2 | 2,800 lines | 420 | 3,220 | Project collections, stable project Resources and aliases work through read/search/recovery with real live evidence. |
| Bounded issue browsing | S3 | 2,300 lines | 345 | 2,645 | Site/project issue collections and typed cursors complete h212; nae2 may then proceed. |

Total 7,400 + 1,110 margin = 8,510 projected changed lines. Each increment is below 4,000. The extraction allowance includes all existing evidence probes and workflow artifacts, not only moved Rust lines. The margin covers strict-wire cases, caller migration and oracle/gate records; actual size is checked after each slice. The upstream was discovered as origin/main at 0b81d31 before branching. Each boundary opens a draft PR; remaining work continues stacked rather than accumulating one oversized review.

Claims have one final discharge owner: S1 C1; S2 C3/C6/C11/C16; S3 C2/C4/C5/C7/C8/C9/C10/C12/C13/C14/C15. Cross-cutting claims stay PENDING until all their input families exist; their project cases are nevertheless implemented, fenced and verified in S2. S3 extends/reuses those fences for issue/cursor cases and discharges the complete claims. No partially implemented source grammar is published. Existing fences are reused, never duplicated merely to claim new test creation.

Intentional structured changes are the native Rivets claims/notes for h212 and nae2, workflow artifacts, and any lockfile change required by a reused workspace dependency. These are not unexpected drift. No source/profile model configuration is changed for agent execution.

## S1: Extract existing Jira and bounded-read responsibilities without behavior changes

**Claim IDs:** C1. The approved placement ledger also governs these moves; its full new-capability gate C16 is discharged in S2.

**Expected behavior:** Existing direct issue, alias, Field, cache, error and retry/deadline behavior remains byte/category equivalent. Public crate exports and caller-visible signatures remain intact; no new Jira grammar is accepted.

**Oracle:** Existing hand-authored Jira wire/TLS contracts and core grammar examples; Cargo metadata and independent dependency/egress scan. Existing rfs-pm0y direct-read evidence remains the applicable empirical oracle for moved code.

**Stress fixture:** Existing arbitrary-precision/deep/wrong-authority wire cases, alias identity change, cache invalidation, response ceiling and retry/deadline cases retain their expected categories and canonical identities. This is a move, not a new fixture implementation.

**Regression fence:** Reuse `jira_reference_contract`, `jira_wire_contract`, `jira_render_contract`, `atlassian_jira_adapter_contract`, and architecture dependency/egress contracts. Add the issue-local module-shape oracle to make the already approved extraction budgets checkable; no new behavior tests for file movement.

**Named mutation:** C1's dependency and off-owner-client mutations are existing-fence checks. No new behavioral fence is introduced in this slice; record that distinction at checkpoint rather than inventing tests for wiring. The new structural oracle's responsibility mutation is discharged with C16 when project decoding exists in S2.

**Complexity/production scale:** N/A — existing loops move unchanged; no new loop or scale characteristic.

**Wall budget/phase:** N/A — no new runtime phase; existing configured logical deadline remains unchanged.

**Files:** `crates/resourcefs-core/src/reference.rs`; new `crates/resourcefs-core/src/reference/jira.rs`; `crates/resourcefs-sources/src/atlassian/jira.rs`; new `crates/resourcefs-sources/src/atlassian/jira/transport.rs`; `crates/resourcefs-sources/src/http/mod.rs`; new `crates/resourcefs-sources/src/http/read.rs`; `.rfs-h212/oracles/module_shape.py`; workflow/tracker artifacts. Public export wiring changes only if required by the move.

**Estimate:** N/A — work is gated by observable outcomes, not an effort estimate.

**Diff estimate:** 2,300 changed lines including extraction churn, evidence probes, module oracle and current workflow artifacts.

**PR increment:** Project preparation.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test jira_reference_contract` → existing direct grammar, canonical encodings and identifier bounds unchanged.
- `cargo test -p resourcefs-sources --features test-support --test jira_wire_contract --test jira_render_contract --test atlassian_jira_adapter_contract` → existing TLS/native controls preserve identity, media, cache and error outcomes.
- `cargo test -p resourcefs-sources --features test-support --test github_adapter_contract pagination_retries_share_the_original_logical_deadline -- --exact` → moved transport retains the same original-deadline behavior.
- `cargo test -p resourcefs-mcp --test architecture_contract` → dependency/egress ownership unchanged.
- `python .rfs-h212/oracles/module_shape.py --stage extraction` → extracted owners and applicable parent-shrink predicates hold; not a claim that browse/cursor modules exist.
- `python scripts/ci-gates.py` → the shared local/CI aggregate, including the Linux extraction-stage placement oracle, remains green. The runner owns the complete command and ignored-row inventory.
- Existing ignored `live_jira_issue_read`, using reader-only fixture environment → moved implementation still agrees with direct native issue/Field identity and representation oracle; record the actual run, then clean owned fixtures when no longer needed.

## S2: Deliver fully functional bounded project browsing

**Claim IDs:** C3, C6, C11, C16. Implement and fence project applications of C2/C4/C5/C9/C10/C12/C13/C14 here; those full multi-family claims discharge in S3, not through a partial PASS here.

**Expected behavior:** Stable project and key-alias reads, project collection offset pages, compact numeric ordering, regex/PCRE2 search and ADR-0006 recovery work end-to-end. Native effective maxima and terminal links drive pagination. The physical-attempt/one-retry budget is active, GET cache semantics remain no-stale, and corrupt pages never become partial successes.

**Oracle:** Native project schema/operation/pagination facts in evidence; hand-authored TLS page/authority/validator fixtures; independent server request ledger and explicit numeric expected order; pre-spill Resource bytes for engine recovery; approved module ledger.

**Stress fixture:** 1,000 shuffled 64-digit-capable IDs with 4 KiB metadata; native maximum 7; empty nonterminal pages with advancing next links and moving totals; duplicate/foreign IDs; exact terminal versus capped nonterminal pages; retry followed by another retry trigger; ETag generation change; metadata absent versus explicit empty/false; forced artifact spill. Expected: numeric page order, no record loss, no more than ten attempts/one retry, correct next offset, and atomic typed errors for invalid cases.

**Regression fence:** Create the named project, wire-project, budget-project and discovery-project cases from the design table in `jira_project_browse_contract.rs`, `jira_browse_wire_contract.rs`, `jira_browse_budget_contract.rs`, and `jira_browse_discovery_contract.rs`; extend core `jira_reference_contract.rs`. Existing shared TLS fixture support is reused or extracted rather than copied. Add a reader-only `live_jira_project_browse` row. Complete the new-capability module-shape predicates and CI hook.

**Named mutation:** Apply the applicable design mutations: cached project alias; ignored project self authority; byte-only numeric comparison; premature short-page termination; recreated fetch budget; partial-success return after later error; ignored cache generation; stripped source selection; omitted continuation; native body in errors; compact decoder moved into core. Each corresponding new fence must go red at its own intended assertion, then green after restoration. C16 uses the now-real `decode_project_page` implementation.

**Complexity/production scale:** At most ten physical native attempts, including retries, and 1,000 records. Strict decoding is O(total bounded response bytes), depth capped by the existing StrictParser. Sorting is O(n log n) for n ≤ 1,000 with ID comparisons bounded at 64 bytes. Rendering is O(output bytes); existing HTTP/session/artifact ceilings remain authoritative. Maximum accepted cost is the ten-attempt ledger, the 1,000-record cap, and existing byte/depth ceilings; no per-record network fetch or full-tenant scan.

**Wall budget/phase:** Always-on native I/O/retry remains inside the original configured logical deadline D, never D per page. Local release-mode 1,000-row/4-KiB-metadata processing must complete within five seconds on this workstation, measured after compilation; this catches accidental repeated document processing without imposing a timing assertion on CI. The permanent fence asserts cardinality/request/byte bounds. Structural-oracle execution is one-off per checkpoint; no runtime wall budget.

**Files:** Existing `reference.rs`, `reference/jira.rs`, `selector.rs`, core export and canonical search identity wiring; new `reference/source_page.rs` for Offset validation (Cursor is added only in S3); `atlassian/mod.rs`, `jira.rs`, `jira/transport.rs`, `http/read.rs`; new `jira/browse.rs`, `wire/collections.rs`, `render/collections.rs`; minimal wire/render parent declarations/shared-helper visibility; the named contract targets and `jira_live_smoke.rs`; existing test-support module as needed; module oracle and CI workflow; behavior documentation and gate evidence. No issue-collection or Query Resource grammar is accepted yet.

**Estimate:** N/A — work is gated by observable outcomes, not an effort estimate.

**Diff estimate:** 2,800 changed lines including tests, fixtures, oracle and behavior records.

**PR increment:** Bounded project browsing.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test jira_reference_contract` → project ID/key/offset examples round-trip; wrong-family selectors and malformed values fail without changing other schemes.
- `cargo test -p resourcefs-sources --features test-support --test jira_project_browse_contract --test jira_browse_wire_contract --test jira_browse_budget_contract --test jira_browse_discovery_contract` → each named design/project case agrees with its explicit identity/order/request/error/recovery oracle.
- Repeat the targeted test under each named mutation → intended assertion red; restore and repeat → original behavior green.
- `cargo test --release -p resourcefs-sources --features test-support --test jira_project_browse_contract production_scale_project_page -- --exact --nocapture` → 1,000 records ordered without narrowing, request cap intact; report measured execution against the five-second local processing budget.
- `python .rfs-h212/oracles/module_shape.py --stage projects` and `cargo test -p resourcefs-mcp --test architecture_contract` → owners, fixture ingress restrictions and all applicable growth/dependency predicates hold; C16 mutation reports C16 and the offending path.
- `cargo test -p resourcefs-sources --all-features --test jira_live_smoke live_jira_project_browse -- --ignored --nocapture` with reader fixture environment → real adapter follows small native offset pages, resolves stable/alias identity, and agrees with known fixture membership without asserting global counts.
- Advance the aggregate's placement stage to `projects` and classify the new ignored live row, then run `python scripts/ci-gates.py`; live rows remain excluded from CI.

## S3: Complete site/project issue browsing and discharge all cross-family claims

**Claim IDs:** C2, C4, C5, C7, C8, C9, C10, C12, C13, C14, C15. Earlier project fences remain active and are extended/reused, not weakened.

**Expected behavior:** Enhanced GET fixed issue enumeration selects canonical compact fields, verifies a project parent when scoped, preserves typed opaque continuation, and sorts the completed logical page numerically. Cursor ownership failures are zero-egress. Every read/search/recovery/error/budget contract now holds for both project and issue collections; h212 is complete without public native JQL or new profile configuration.

**Oracle:** Evidence's populated-project predicate, ID-only default counterexample, native token/terminal semantics and direct-read membership comparison; literal TLS tokens plus server-observed request parameters; independently declared parent/site/origin identities; existing engine recovery oracle.

**Stress fixture:** Nonnumeric and Unicode/space-bearing opaque tokens; same Site ID rebound to another origin; wrong parent/family; omitted/conflicting terminal state; oversized token that cannot form a bounded reference; missing required selected fields; 1,000 issue rows including IDs beyond u64; project-parent lookup plus retry near the ten-attempt boundary; later-page corrupt/foreign/duplicate rows; empty match page with nonterminal continuation; artifact spill plus source cursor. Expected: exact token forwarding only to the configured owner, stable canonical issue links, no extra attempt or retry, complete recovery and atomic typed errors.

**Regression fence:** Create `jira_issue_browse_contract.rs` cases named in design, add issue/cursor cases to the existing core/wire/budget/discovery targets and numeric metadata fence, and add `live_jira_browse` covering both native families. Extend the module oracle's required published owners for the issue stage. Every complete design row now has its full permanent fence.

**Named mutation:** Apply C7 parent-check omission, C8 origin-fingerprint omission and C15 selected-fields omission; apply C2/C4/C5/C9/C10/C12/C13/C14 design mutations to the newly introduced issue/cursor paths as applicable. Every new assertion must fail under its intended bug and pass after restoration; prior project gates remain green.

**Complexity/production scale:** Same n ≤ 1,000 / ten-attempt / 64-digit comparison and bounded-byte/depth costs as S2. Cursor parsing/serialization is O(reference bytes), constrained by the 64-KiB total reference ceiling; oversized native continuation fails rather than allocating an unbounded output or dropping state. Parent validation is one request inside the same operation budget; no fan-out or per-issue fetch. Maximum accepted cost remains the ledger/record/reference/body/depth ceilings.

**Wall budget/phase:** Always-on I/O/retry uses the same original D, including parent verification. Release-mode 1,000-row/4-KiB issue metadata processing must finish within five seconds locally after compilation; permanent CI assertions cover cardinality/request/byte bounds rather than wall-clock wording. Live fixture setup/cleanup and oracle execution are one-off phases; no runtime wall budget.

**Files:** Core Jira/source-page types and contextual parsing plus canonical search identity wiring; `jira/browse.rs`, new `jira/cursor.rs`, `jira/transport.rs`; compact wire/render modules; existing test-support lowering helper; core/source contract targets, new issue browse target, live row, module oracle; behavior documentation and gate evidence. Existing direct issue/Field behavior and unrelated adapters remain unchanged.

**Estimate:** N/A — work is gated by observable outcomes, not an effort estimate.

**Diff estimate:** 2,300 changed lines including issue/cursor regression and live evidence.

**PR increment:** Bounded issue browsing.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test jira_reference_contract` → all published Jira variants/selector families and global reference bounds satisfy C2, including wrong-family constructor inputs.
- `cargo test -p resourcefs-sources --features test-support --test jira_project_browse_contract --test jira_issue_browse_contract --test jira_browse_wire_contract --test jira_browse_budget_contract --test jira_browse_discovery_contract` → every design row's full project/issue input space agrees with its named oracle.
- Run each new targeted fence under its named mutation → claim-local red; restore → green.
- `cargo test --release -p resourcefs-sources --features test-support --test jira_issue_browse_contract production_scale_issue_page -- --exact --nocapture` → 1,000 canonical ordered issue records within request/record bounds and the five-second local processing budget.
- `python .rfs-h212/oracles/module_shape.py --stage issues` plus architecture contracts → final approved owners/growth/egress constraints hold.
- `cargo test -p resourcefs-sources --all-features --test jira_live_smoke live_jira_browse -- --ignored --nocapture` with reader-only fixture environment → real offset and token continuations reach fixture members and match direct native identity/summary checks; absent gate skips cleanly.
- Advance the aggregate's placement stage to `issues` and update its live-row inventory, then run `python scripts/ci-gates.py` → assembled behavior, full release budgets and existing contracts remain green. Run this centrally, not in concurrent writers.

## Execution and self-review

Main owns integration, gates, commits and tracker/artifact state. Independent writers use isolated workspaces with distinct Cargo targets; no concurrent writer mutates the parent checkout. S1 core extraction and source/HTTP extraction are independent ownership slices within one atomic checkpoint. For feature work, core types and native wire/render preparation can proceed concurrently only under an explicit shared type contract; validation runs after integration, never against another writer's half-edit.

Before each slice, use LSP references/code actions and scoped helper/dependency lookup. Record callers in the slice commit. Keep native CLI Rivets updates intentional; never edit JSONL manually. One green checkpoint and commit per slice; after each partition boundary open its draft PR and advance on a stacked branch. Apply upstream drift and actual-size gates before advancing.

Final integration reruns every applicable implementation/oracle comparison and full fence, including live adapter evidence. Clean marker-owned fixtures and temporary worker workspaces when no longer needed. Native JQL remains rfs-nae2; full profile/stdio publication remains rfs-ue44; other Atlassian destination work remains rfs-0zbv. No new anonymous deferrals or risk waivers.

Self-review: every C1–C16 has one final discharge owner; every slice has the mandatory fields; previously introduced project behavior is verified before its independent merge point; no new grammar precedes working behavior; each loop/phase has explicit bounds; partition arithmetic and mergeable definitions are recorded; all named future work cites verified tracker IDs. No slice is declared complete here.
