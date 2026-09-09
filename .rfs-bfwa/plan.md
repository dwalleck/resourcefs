# Plan: rfs-bfwa resumable conversation-comment facts

## Inputs, ownership and approval

Approved design: `design.md`, requester **"Approve and implement"**, 2026-09-08; no risk waivers. Route is Empirical; `evidence.md` owns P1/P2 (fresh) and P3 (retained). No production code exists yet. This plan declares no slice complete; `checkpointed-build` exclusively judges completion.

Main owns integration, checkpoints and evidence. Writers use separate verified Git worktrees and skip validation while concurrent; Main applies isolated patches in slice order, then runs each owning focused checkpoint. No shared target directory and no direct edits to another writer's checkout. Source baseline: `feat/rfs-0n97` = `084d136e681179eb33a31f6b84a0fb17abf47daa`. Observed remote symbolic default: `refs/remotes/origin/HEAD` targets `refs/remotes/origin/main` (discovered with `git for-each-ref --format='%(refname:short) %(symref)' refs/remotes`). Increments below are an implementation/review partition, not publication authorization: no push, PR, merge or tracker closure is authorized by this plan.

## Cross-writer contracts

Incidental Rust spellings implementing the approved interfaces, not new ownership decisions.

- `PullRequestResource::Facts(PullRequestFact)`, `PullRequestFact::{Pull, Comments, Comment(ConversationCommentId)}` in core `reference.rs`; `parse_pull_request_reference(input) -> Result<(PullRequestAddress, Option<ProjectionSelector>), ResourceError>` mirrors `parse_jira_reference` and is the only cursor-splitting site.
- `MAX_COLLECTION_RECORDS: usize = 1_000` in core `lib.rs`; no new `ReadAcquisitionLimits` dimension.
- `fetch.rs`: `pub(super) struct PageResponse { pub(super) body: Arc<[u8]>, pub(super) observation: BodyObservation, pub(super) revalidation: Option<BodyObservation>, pub(super) cache_generation: u64, pub(super) next: Option<Url> }` and `pub(super) async fn facts_page(&self, url: Url, operation: BoundedRead<'_>, budget: &mut HttpReadBudget) -> Result<PageResponse, ResourceError>`, reusing `fetch_controlled` and `next_link`.
- `facts.rs`: `struct Facts<'a, B: Serialize>` with `#[serde(flatten)] body: B`, `read_facts(reference, fact, operation, acquisition)`, and the shared `Presence`/`NativeId`/`native!` primitives stay here; `finish_facts` keeps the capped writer.
- `facts/pull.rs`: `pub(super) async fn read(source, repository, number, ctx) -> Result<SourceResource, ResourceError>` owning the singular PR fetch, identity validation, projection and publication through the shared `FactsRead` context; JSON output is byte-identical to the pre-slice implementation.
- `facts/comment.rs`: presence-aware `NativeComment` decoders, `pub(super) fn record(...)` and the singular `read_item`/`fetch_parent` used by the collection; parent linkage goes through the existing `validate_parent` rules.
- `facts/collection.rs`: `pub(super) async fn read(...) -> Result<SourceResource, ResourceError>` owning the page loop, admission, coverage, continuation decision and publication through the shared `FactsRead` context (revised placement, approved "Approve revised placement").
- `facts/continuation.rs`: `CursorOwner::{new, decode, continuation}` following `atlassian/jira/cursor.rs`; envelope `{"v":1,"r":…,"o":…,"s":…,"n":…}` with SHA-256 origin and session bindings, base64url no padding.
- No new trait, no second HTTP client, no provider type in core/MCP, no MCP production change.

## Module growth ledger

Baseline production lines are physical counts at `084d136`; ranges are drift tripwires, not limits.

| Module | Baseline P | Projected final P | Responsibility change | Interface change | Protected-parent rule |
|---|---:|---:|---|---|---|
| `crates/resourcefs-core/src/reference.rs` | 1946 | 1990–2045 | add fact-family sum and `:cursor:` split | `PullRequestFact`, `parse_pull_request_reference` | body changes limited to `parse`, `parse_pull_request_reference`, `canonical_reference`, `parse_pull_request_address` |
| `crates/resourcefs-core/src/lib.rs` | 78 | 80–86 | re-export the new constant and limit kind | re-exports | wiring only |
| `crates/resourcefs-core/src/reference/pull.rs` | 0 | 40–70 | create: pull-fact sum and `:cursor:` split | private parser | private grammar child |
| `crates/resourcefs-core/src/resource.rs` | 710 (685 production) | 720 (690 production) | add `MAX_COLLECTION_RECORDS` | new public constant | source-neutral only |
| `crates/resourcefs-core/src/error.rs` | 286 | 290–296 | add `AcquisitionLimitKind::CollectionRecords` | new limit kind | source-neutral vocabulary |
| `crates/resourcefs-sources/src/github/facts/identity.rs` | 250 | 250–255 | retain identity validation | unchanged `validate` | private; no HTTP/cache/clock |
| `crates/resourcefs-sources/src/github/facts.rs` | 748 | 470–620 | envelope + dispatch only; PR projection moves out | generic `Facts<B>`, `read_facts(fact)` | net shrink; no family bodies. Tripwire raised to 760 for the rebase: origin/main's reviewed file is 748 lines and the family dispatch lands in S1 before the S2 extraction returns it to 569 |
| `crates/resourcefs-sources/src/github/facts/pull.rs` | 0 | 240–340 | create: singular PR fetch/validate/project/publish | private `read` entry | private; no second HTTP client/cursor |
| `crates/resourcefs-sources/src/github/facts/comment.rs` | 0 | 220–330 | create: comment decoding, record projection, singular read | private decoders/record/read | private; no second HTTP client/coverage/cursor |
| `crates/resourcefs-sources/src/github/facts/collection.rs` | 0 | 420–620 | create: page loop, atomic admission, coverage, continuation, publication | private `read` entry | private; no HTTP stack of its own/cursor encoding |
| `crates/resourcefs-sources/src/github/facts/continuation.rs` | 0 | 180–270 | create: cursor codec/binding | private `CursorOwner` | private; no network/credentials |
| `crates/resourcefs-sources/src/github/fetch.rs` | 565 | 600–690 | add one controlled page seam | `facts_page`, `PageResponse` | no fact schema/coverage policy |
| `crates/resourcefs-sources/src/github/mod.rs` | 1136 | 1140–1160 | dispatch arm + catalog string only | `Facts(fact)` arm | protected: no new responsibility body |
| `crates/resourcefs-sources/src/github/mutation.rs` | 1319 | 1320–1328 | refusal arm | `Facts(_)` | no new write behavior |
| `crates/resourcefs-sources/tests/github_facts_contract.rs` | 1495 | 2500–3300 | new collection/comment rows | public `SourceAdapter` reads | tests through the public seam only |
| `crates/resourcefs-core/tests/github_reference_contract.rs` | — | +120–200 | new route/cursor rows | parser | no production reach-through |
| `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs` | — | +150–250 | recovery/catalog rows | real stdio | no production change |
| `crates/resourcefs-sources/tests/github_mutation_contract.rs` | — | +40–80 | refusal matrix rows | mutation surface | no production change |

## Partition arithmetic

| Slice | Changed-line estimate |
|---|---:|
| S0 successor placement fence | 2,642 (measured at commit `0baf162`) |
| S1 core route grammar and cursor scope | 450 |
| S2 shared envelope and singular comment read | 1100 |
| S3 bounded collection read | 1300 |
| S4 opaque session continuation | 700 |
| S5 MCP recovery, catalog, mutation, docs | 550 |
| S6 live read-only proof | 350 |

Sum 7,092; churn margin 20% = 1,418 (fixture/schema/caller migration, structural-oracle repairs, and the one-time workflow evidence this ticket carries); total **8,510**, above 4,000, so partition is mandatory.

S0 plan correction (2026-09-08): the original 800-line estimate counted only the fence, ledger and runner. The measured commit is 2,642 lines, dominated by one-time workflow evidence — route/evidence/design/plan markdown (553), retained probe and oracle result JSON (1,636, less the probe lock removed as build noise), fence and probe scripts (439), and the rest — none of which recurs in later slices. The remaining slice estimates are unchanged, the partition policy is unchanged, and the tripwire basis is now the measured value.

| Increment | Slices | Mergeable definition | What verifies it without later increments |
|---|---|---|---|
| A — placement and grammar | S0, S1 | Successor shape fence governs the tree; both new routes parse and canonicalize; cursor scope is route-scoped | `cargo test -p resourcefs-core --test github_reference_contract` and `python scripts/module_shape_bfwa.py --stage grammar` |
| B — singular comment read | S2 | `pr://o/r/N/comments/<id>/facts` returns owned JSON through the public source seam | `cargo test -p resourcefs-sources --test github_facts_contract` |
| C — collection read | S3 | The collection route returns atomically admitted records with honest coverage and one shared budget | `cargo test -p resourcefs-sources --test github_facts_contract --test http_bounds_contract` |
| D — continuation | S4 | A truncated collection names an opaque session-bound continuation and resumes the next native page | `cargo test -p resourcefs-sources --test github_facts_contract` continuation rows |
| E — MCP and docs | S5 | Real stdio recovery, catalog and mutation refusal; docs describe the contract | `cargo test -p resourcefs-mcp --test stdio_mcp_contract` and `--test github_mutation_contract` |
| F — live proof | S6 | Credential-gated read-only live rows for adapter and stdio | `RFS_LIVE=1 GITHUB_TOKEN=… cargo test … -- --ignored` |

## Slice S0: Successor placement fence

**Claim IDs:** C18.
**Expected behavior:** `scripts/module_shape_bfwa.py` accepts the unchanged tree at stage `baseline` and rejects an unlisted new production file, a new parent responsibility body, and a premature staged owner, each with its own path/symbol; `scripts/ci-gates.py` invokes the successor and the placement gate passes.
**Oracle:** independent Git baseline/diff/symbol census plus disposable mutation trees whose verdicts are authored separately from the checker.
**Stress fixture:** disposable copies with an unlisted `crates/*/src` Rust file, a new serializer body in `github/mod.rs`, and a premature staged owner file; each must name its exact violation while the unchanged tree passes.
**Regression fence:** `scripts/module_shape_bfwa.py`, `scripts/module-ledger-bfwa.json`, `scripts/ci-gates.py`.
**Named mutation:** add a serializer responsibility body to `github/mod.rs`; add an unlisted `crates/resourcefs-sources/src/github/extra.rs`; add a `trait` to a staged-but-absent owner. Each distinct mutation must fail with its own path, and restoration must pass.
**Complexity/production scale:** one-off source census with bounded lexical scans, `O(total scanned source + Σ file bytes × declarations)`; maximum accepted cost 10 s on the local checkout, rationale: a non-runtime gate must stay cheaper than compilation.
**Wall budget/phase:** N/A — one-off verification phase; no runtime wall budget.
**Module shape:** no production ownership yet; the merged ledger stages the new owners and approves the `reference.rs` body changes. `python scripts/module_shape_bfwa.py --stage baseline` → PASS with inherited h212/nae2/0n97 assertions retained.
**Files:** `scripts/module_shape_bfwa.py`, `scripts/module-ledger-bfwa.json`, `scripts/ci-gates.py`.
**Estimate:** one bounded oracle implementation/review pass.
**Diff estimate:** 2,642 measured (see partition arithmetic; one-time workflow evidence dominates).
**PR increment:** A — placement and grammar.
**Commands and expected results:**
- `python scripts/module_shape_bfwa.py --stage baseline` → unchanged tree accepted, inherited checks retained.
- The same command against each disposable `--root` mutation tree → the exact offending path/symbol, and the restored tree passes.


### Checkpoint S0 (2026-09-08)

Commit `0baf162`. Gate: 1 `N/A` — no executable production change; 2 `PASS` — unchanged tree accepted, three disposable mutations localized; 3 `PASS` — unlisted owner / parent serializer body / premature owner each red, restored tree green; 4 `PASS` — checker verdicts match pre-written expectations; 5 `PASS` — inherited h212/nae2/0n97 assertions retained at stage `facts`; 6 `PASS` — 6.3–6.9 s vs 10 s; 7 `PASS` — `python .rfs-bfwa/oracles/module_shape.py`; 8 `PASS` — all three mutations red; 9 `PASS` — restoration green. Plan correction: S0 measured 2,642 lines, not 800 (one-time workflow evidence).

## Slice S1: Core route grammar and cursor scope

**Claim IDs:** C1, C2.
**Expected behavior:** `pr://o/r/N/comments/facts` and `pr://o/r/N/comments/<id>/facts` parse and canonicalize exactly; a `:cursor:` selector is split and accepted only on the collection facts route; every previously accepted/refused spelling keeps its verdict.
**Oracle:** hand-written expected spelling/verdict table independent of the parser, plus the falsifier's positive controls.
**Stress fixture:** `comments/facts/1`, `comments/new/facts`, `comments/0/facts`, non-canonical and non-numeric ids, `:cursor:` on PR facts and on the item route, `:raw`/`:1-2`/`:page:2`/`:offset:1` on the collection route, two selectors.
**Regression fence:** `github_reference_contract::conversation_comment_facts_routes_and_cursor_scope`.
**Named mutation:** map `"comments","facts"` to `Comment` in `parse_pull_request_address`; or delete the `:cursor:` split so a canonical cursor parses as an address segment.
**Complexity/production scale:** `O(reference bytes)` parse with no new loop; the existing `reference_parse_budget` row covers cost, and the two added arms add no measurable work. Maximum accepted cost: existing budget unchanged.
**Wall budget/phase:** always-on parse path; the existing reference-parse budget is the wall-clock bound.
**Module shape:** `reference.rs` deepen (grammar + split); owner after slice unchanged; protected parent `reference.rs` body changes limited to `parse`, `parse_pull_request_reference`, `canonical_reference`, `parse_pull_request_address`; `python scripts/module_shape_bfwa.py --stage grammar` → PASS.
**Files:** `crates/resourcefs-core/src/reference.rs`, `crates/resourcefs-core/tests/github_reference_contract.rs`, `scripts/module-ledger-bfwa.json`.
**Estimate:** one grammar pass plus contract rows.
**Diff estimate:** 450.
**PR increment:** A — placement and grammar.
**Commands and expected results:**
- `cargo test -p resourcefs-core --test github_reference_contract` → new spellings round-trip, refusals preserved, cursor scope exact.
- `cargo test -p resourcefs-core --test selector_golden_contract` → selector grammar unchanged elsewhere.
- `python scripts/module_shape_bfwa.py --stage grammar` → PASS.


### Checkpoint S1 (2026-09-08)

Commit `fca1d6a`. Gate: 1 `PASS` — core suite, GitHub facts/adapter/mutation suites, workspace all-target/all-feature check; 2 `PASS` — C1/C2 corpus; 3 `PASS` — malformed/canonical/cursor-scope fixture; 4 `PASS` — hand-written spelling/verdict table; 5 `PASS` — fence at stage `grammar` (reference.rs 1964 ≤ 1989, `reference/pull.rs` 51 ≤ 650); 6 `PASS` — release `reference_parse_budget` under 1 ms average; 7 `PASS` — `conversation_comment_facts_routes_and_cursor_scope`; 8 `PASS` — removed `comments/facts` arm and removed `:cursor:` split each red at their own assertion; 9 `PASS` — both restored green. Placement correction: the inherited 1989-line parent cap forced the new sum/parser into the private `reference/pull.rs` child (design.md records it); `cargo fmt` repair of `core/lib.rs` carried into the S2 commit.

## Slice S2: Shared envelope and singular comment read

**Claim IDs:** C10, C11, C12, C17.
**Expected behavior:** `pr://o/r/N/comments/<id>/facts` returns the shared envelope with singular comment facts — native kind/id/nodeId, verified parent identity/links, nullable body/author distinct from absent, supplied times and links — and rejects a wrong parent before publishing; existing PR facts output is byte-identical.
**Oracle:** independent expected JSON key order and presence matrix derived from `evidence.md` P1 plus the fixture.
**Stress fixture:** `body` null vs absent, `user` null vs absent, missing `nodeId`, id above 2^53, duplicate recognized keys, non-object record, empty and Unicode bodies, `issue_url` naming another repository or pull request, an issue number addressed as a PR.
**Regression fence:** `github_facts_contract::comment_records_preserve_native_presence`, `::wrong_parent_rejects_whole_component`, `::single_comment_facts_shape`, `::envelope_serializes_shared_fields_in_order`.
**Named mutation:** decode `body` as `Option<String>` so null and absent collapse; emit ids as JSON numbers; skip `validate_parent`; serialize an empty `collection` for singular reads; replace `Presence` with `Option`.
**Complexity/production scale:** `O(record bytes)` decode and serialize over a response ≤8 MiB and a document ≤16 MiB; no new loop. Maximum accepted local processing ≤1 s release at production-size fixture, rationale: serialization dominates and stays far below the 30 s logical deadline.
**Wall budget/phase:** always-on per read; the existing ≤30 s logical deadline covers it; local processing target above.
**Module shape:** create private `facts/pull.rs` and `facts/comment.rs`; `facts.rs` becomes envelope + dispatch and nets smaller; `github/mod.rs` gains one dispatch arm and no new responsibility body. `python scripts/module_shape_bfwa.py --stage item` → PASS.
**Files:** `crates/resourcefs-sources/src/github/facts.rs`, `facts/pull.rs`, `facts/comment.rs`, `facts_tests.rs`, `crates/resourcefs-sources/src/github/mod.rs`, `crates/resourcefs-sources/tests/github_facts_contract.rs`, `scripts/module-ledger-bfwa.json`.
**Estimate:** one extraction plus one read path and its contract rows.
**Diff estimate:** 1100.
**PR increment:** B — singular comment read.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_facts_contract` → presence matrix, parent rejection, singular shape and envelope order exact; existing PR facts rows unchanged.
- `cargo test -p resourcefs-core --test github_reference_contract` → grammar rows still pass.
- `python scripts/module_shape_bfwa.py --stage item` → PASS.


### Checkpoint S2 (2026-09-08)

Gate: 1 `PASS` — facts (24), adapter (24), mutation (15), MCP architecture (7) and stdio (43) suites; workspace all-target/all-feature check; 2 `PASS` — four named fences; 3 `PASS` — presence matrix (null vs absent vs empty vs unknown key vs duplicate key vs non-object vs >2^53 vs Unicode) and wrong-parent corpus; 4 `PASS` — independent expected key order/presence table; 5 `PASS` — fence at stage `item` (facts.rs 541 ≤ 750, pull.rs 269 ≤ 400, comment.rs 260 ≤ 400); 6 `PASS` — release `github_facts_production_budget` 27.7 ms / 32.8 MiB vs 1 s / 96 MiB; 7 `PASS` — the four fences; 8 `PASS` — ids-as-numbers, skipped `validate_parent`, empty `collection` on singular reads and `Option`-collapsed body each red; 9 `PASS` — all restored green. Known issue: pre-existing intermittent `facts_inflight_cancel_and_deadline_refuse_barrier_delayed_response` on the base branch, filed as `rfs-kpgy`; it reproduces without this slice's changes and a clean rerun passed. `cargo fmt` repaired S1's `core/lib.rs` ordering drift.

## Slice S3: Bounded collection read

**Claim IDs:** C5, C6, C7, C8, C14, C15, C19.
**Expected behavior:** `pr://o/r/N/comments/facts` returns conversation-comment records with atomic page admission, honest coverage, one shared budget, partial retention and a bounded final document; no continuation yet (a truncated collection reports `incomplete` + `localLimit` without inventing one).
**Oracle:** independent record/byte counting plus a request/byte/timestamp log from the loopback fixture.
**Stress fixture:** 600+400 accepted; 600 then 401 retains page one; 1,001-record first page is `limit_exceeded`; empty page with no `rel="next"` is complete/zero; 404/403 is a typed error; page two transport/malformed/deadline/limit failure retains page one; cancellation and parent mismatch after page one reject all pages; response 8 MiB exact/+1; accepted total 16 MiB exact/+1; representation exact/+1; `maxAttempts: 5` stops the sixth physical request.
**Regression fence:** `github_facts_contract::collection_admits_whole_pages_atomically`, `::empty_complete_is_not_inaccessible`, `::partial_retention_and_rejection_precedence`, `::collection_coverage_vocabulary_is_honest`, `::one_budget_covers_parent_and_pages`, `::representation_ceiling_includes_outcome_overhead`, `architecture_contract::enforces_dependency_direction`.
**Named mutation:** extend the record set before measuring the page; check size only after building the whole document; return complete-empty on 404; drop earlier pages on later failure; retain pages after cancellation; emit `providerCap: 1000` from the local ceiling; mark complete after a local stop; construct a fresh `HttpReadBudget` per page; import `serde_json` or a provider type into core.
**Complexity/production scale:** page loop `O(pages × records)` with ≤10 attempts, ≤1,000 records, ≤16 MiB accepted bodies and ≤16 MiB representation; per-page admission adds one measurement pass over the page bytes plus `O(1)` bookkeeping. Maximum accepted local processing ≤1 s release at a 16 MiB document and ≤4 KiB bookkeeping per read, rationale: double serialization is linear and must stay far below the deadline without claiming a peak-memory bound.
**Wall budget/phase:** always-on per collection read; one ≤30 s logical deadline shared by parent, pages, retries and revalidation; local processing target above.
**Module shape:** create private `facts/collection.rs`; `fetch.rs` gains `facts_page`/`PageResponse`; core `lib.rs` gains `MAX_COLLECTION_RECORDS`; `github/mod.rs` dispatch arm only. `python scripts/module_shape_bfwa.py --stage collection` → PASS.
**Files:** `crates/resourcefs-sources/src/github/facts/collection.rs`, `facts.rs`, `fetch.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-sources/tests/github_facts_contract.rs`, `crates/resourcefs-sources/tests/http_bounds_contract.rs`, `scripts/module-ledger-bfwa.json`.
**Estimate:** one acquisition/admission pass plus adversarial fixture rows.
**Diff estimate:** 1300.
**PR increment:** C — collection read.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_facts_contract` → every admission, coverage, retention, budget and ceiling row exact.
- `cargo test -p resourcefs-sources --test http_bounds_contract --test http_substrate_contract` → shared budget behaviour unchanged.
- `cargo test -p resourcefs-mcp --test architecture_contract enforces_dependency_direction -- --exact` → core stays source-neutral.
- `python scripts/module_shape_bfwa.py --stage collection` → PASS.


### Checkpoint S3 (2026-09-08)

Gate: 1 `PASS` — facts (30 + 2 ignored budget rows), adapter (24), mutation (15) suites; 2 `PASS` — seven named fences; 3 `PASS` — 600+400, 600+401, 1,001 first page, empty complete, 404/403, malformed later page, malformed first page, cancellation after a page, repeated pagination, attempt-budget exhaustion, representation boundary; 4 `PASS` — independent record/byte counting and the measured-cap derivation in the representation fence; 5 `PASS` — fence at stage `collection` (collection.rs 455 ≤ 700, fetch.rs 598 ≤ 650); 6 `PASS` — release `github_collection_production_budget` 32.7 ms at 900 records / 6.4 MB vs 1 s, plus retained PR-facts budget; 7 `PASS` — the seven fences; 8 `PASS` — per-page record counting, complete-empty on error, dropped earlier pages, local stop marked complete, fresh budget per page, admission without envelope bytes and a forbidden core dependency each red; 9 `PASS` — all restored green. Technical corrections recorded: `AcquisitionLimitKind::CollectionRecords` added for the approved record ceiling (DESIGN.md updated in the same commit), and the admission reserve is now a measured envelope plus a bounded outcome/page reserve. Reachable maximum with the default budget is nine data pages plus the parent lookup (900 records at per_page=100); the 1,000-record ceiling remains the local admission limit. Known flake `rfs-kpgy` unchanged.

## Slice S4: Opaque session continuation

**Claim IDs:** C3, C4, C9.
**Expected behavior:** a truncated collection names a `:cursor:` continuation whose envelope binds canonical resource, API origin and Path Session and carries the provider's native next link; resuming acquires exactly the next native page; malformed, foreign, cross-session, cross-resource or off-authority cursors acquire nothing and fail typed; a repeated or stalled target stops with `unknown` coverage plus an `inconsistency` fact.
**Oracle:** independent listener request log and expected binding table.
**Stress fixture:** cursor from session A presented by session B; cursor for another resource; cursor with another API origin; cursor whose native link points to another host or endpoint family; repeated next target; malformed base64url; padded encoding; native link near the 64 KiB ceiling.
**Regression fence:** `github_facts_contract::continuation_encoding_is_canonical_and_bounded`, `::continuation_is_session_and_authority_bound`, `::repeated_pagination_is_explicit`.
**Named mutation:** skip the session-hash comparison; skip the origin/resource comparison; accept any next URL without confinement; follow a repeated target until the attempt budget is exhausted; replace base64url-no-pad with padded base64.
**Complexity/production scale:** encode/decode `O(link bytes)` bounded by the 64 KiB reference ceiling and one SHA-256 per binding; no new loop. Maximum accepted cost ≤1 ms per cursor construction/validation at the ceiling, rationale: scalar hashing plus base64 must be negligible versus network work.
**Wall budget/phase:** always-on per collection read that emits or consumes a cursor; covered by the existing ≤30 s logical deadline.
**Module shape:** create private `facts/continuation.rs`; `facts.rs` accepts exactly the cursor selector on the collection route and refuses it elsewhere; `facts/collection.rs` issues and follows. `python scripts/module_shape_bfwa.py --stage continuation` → PASS.
**Files:** `crates/resourcefs-sources/src/github/facts/continuation.rs`, `facts/collection.rs`, `facts.rs`, `crates/resourcefs-sources/tests/github_facts_contract.rs`, `scripts/module-ledger-bfwa.json`.
**Estimate:** one codec/binding pass plus adversarial rows.
**Diff estimate:** 700.
**PR increment:** D — continuation.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_facts_contract` → cursor round-trip, binding refusals with zero requests, and inconsistency outcomes exact.
- `cargo test -p resourcefs-core --test github_reference_contract` → cursor grammar rows still pass.
- `python scripts/module_shape_bfwa.py --stage continuation` → PASS.


### Checkpoint S4 (2026-09-08)

Gate: 1 `PASS` — facts (33 + 2 ignored budgets), adapter (24), mutation (15); 2 `PASS` — C3/C4/C9 fences; 3 `PASS` — resumed page, foreign session, foreign resource, tampered origin, non-canonical handle, repeated target; 4 `PASS` — envelope decoded independently in the test with a test-local base64url codec and compared field by field; 5 `PASS` — fence at stage `continuation` (continuation.rs 130 ≤ 320); 6 `PASS` — retained collection/PR release budgets; 7 `PASS` — the three fences; 8 `PASS` — padded-handle encoder, dropped session binding and undetected repeated target each red; 9 `PASS` — all restored green. Notes: the handle is issued only for a page never fetched (local limit or retryable attempt/deadline exhaustion), never for a fetched-but-malformed page; when naming the handle would push an otherwise admitted document over the representation ceiling it is dropped and the honest pages are published alone. Two mutation attempts were rejected as non-decisive (padded base64 is byte-identical for this payload; a cancelled read is still rejected by the final acceptance check) and were replaced with observable equivalents.

## Slice S5: MCP recovery, catalog, mutation refusal, docs

**Claim IDs:** C13, C16.
**Expected behavior:** a real MCP client recovers an oversized collection document byte-exactly before parsing with exactly one upstream acquisition for the page and a separately surfaced source continuation; the source catalog advertises both new routes; mutation is refused with zero egress; `DESIGN.md` and `docs/operating.md` document the contract.
**Oracle:** independent byte hash of the acquired document plus the real child-process request log and an egress counter.
**Stress fixture:** oversized collection with and without a next page; denied repository; field and creation mutation attempts on both routes; catalog listing.
**Regression fence:** `stdio_mcp_contract::github_comment_facts_recover_without_reacquisition`, `github_mutation_contract::facts_routes_are_read_only_with_zero_egress`, `stdio_mcp_contract::catalog_advertises_comment_facts`.
**Named mutation:** re-acquire upstream on recovery; drop the source continuation when the page overflows; add `Facts(_)` to `GithubFieldTarget::parse` as a mutable target.
**Complexity/production scale:** existing recovery is linear in output bytes; no new production loop; catalog and mutation arms are `O(1)`. Maximum accepted cost: existing recovery budget unchanged.
**Wall budget/phase:** N/A — reuses existing always-on read/recovery phases; no new phase.
**Module shape:** `github/mod.rs` catalog string only (protected parent, no new responsibility body); `mutation.rs` refusal arm; no MCP production change. `python scripts/module_shape_bfwa.py --stage mcp` → PASS.
**Files:** `crates/resourcefs-sources/src/github/mod.rs`, `crates/resourcefs-sources/src/github/mutation.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`, `crates/resourcefs-sources/tests/github_mutation_contract.rs`, `DESIGN.md`, `docs/operating.md`, `scripts/module-ledger-bfwa.json`.
**Estimate:** one MCP/mutation/docs pass.
**Diff estimate:** 550.
**PR increment:** E — MCP and docs.
**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract` → recovery reconstructs exact bytes, one upstream acquisition, source continuation separate, catalog lists both routes.
- `cargo test -p resourcefs-sources --test github_mutation_contract` → both routes refused with zero egress.
- `python scripts/module_shape_bfwa.py --stage mcp` → PASS.


### Checkpoint S5 (2026-09-08)

Gate: 1 `PASS` — stdio (45), architecture (7), profile (19), schema (4), mutation (15); 2 `PASS` — C13/C16 fences; 3 `PASS` — oversized collection recovered across artifact pages, denied collection, field/creation mutation attempts on both routes, catalog listing; 4 `PASS` — the fence parses the reconstructed bytes and compares record identity/parent to the acquired fixture while the child process request log proves no reacquisition; 5 `PASS` — fence at stage `exposure` (`github/mod.rs` catalog string only); 6 `N/A` — no new loop or phase, retained recovery budget unchanged; 7 `PASS` — the two fences plus the extended mutation matrix; 8 `PASS` — facts route made a mutable field target and the rendered recovery reference dropped each red; 9 `PASS` — both restored green. Test-support change: the TLS fixture now compares a request path without its query, so collection reads (which carry `per_page`/`page`) resolve by endpoint.

## Slice S6: Live read-only proof

**Claim IDs:** C20.
**Expected behavior:** with `RFS_LIVE=1` and a nonempty `GITHUB_TOKEN`, the adapter row reads a real public PR's conversation-comment collection and single comment and the stdio row drives the same through the real `rfs serve`, each asserting shapes/invariants against an independent `gh api` observation; with the gate absent both rows skip cleanly before any credential use.
**Oracle:** independent `gh api` observation of the same public upstream, plus the real stdio process log.
**Stress fixture:** N/A — live upstream; assertions are shape/invariant based, never moving counts.
**Regression fence:** `github_live_smoke::live_github_conversation_comment_facts_hold_up`, `stdio_live_smoke::live_stdio_github_comment_facts_match_native_observation`.
**Named mutation:** N/A — no fence mutation for a live row; absence of the gate is a skip, never a pass.
**Complexity/production scale:** one-off credential-gated network phase bounded by the existing acquisition budgets; no production loop.
**Wall budget/phase:** N/A — one-off phase; no wall budget.
**Module shape:** N/A — no production ownership change; `scripts/ci-gates.py` classification of the new ignored rows only.
**Files:** `crates/resourcefs-sources/tests/github_live_smoke.rs`, `crates/resourcefs-mcp/tests/stdio_live_smoke.rs`, `scripts/ci-gates.py`, `.rfs-bfwa/evidence.md`.
**Estimate:** one live-proof pass plus evidence recording.
**Diff estimate:** 350.
**PR increment:** F — live proof.
**Commands and expected results:**
- `RFS_LIVE=1 GITHUB_TOKEN=$(gh auth token) cargo test -p resourcefs-sources --test github_live_smoke -- --ignored` → new collection/comment rows pass against a public PR with independently observed shapes.
- `RFS_LIVE=1 GITHUB_TOKEN=$(gh auth token) cargo test -p resourcefs-mcp --test stdio_live_smoke -- --ignored` → the real binary path passes.
- With the gate absent → both rows skip explicitly and are never counted as passes.
- `python scripts/ci-gates.py` → complete repository gate on the final tree.

### Checkpoint S6 (2026-09-08)

Gate: 1 `PASS` — the live rows compile and skip cleanly without the gate; with the gate they run; 2 `PASS` — C20 rows executed against real GitHub; 3 `N/A` — live upstream, assertions are shape/invariant based and never compare moving counts; 4 `PASS` — independent `gh api` observation on the same public PR; 5 `N/A` — no production ownership change; 6 `N/A` — one-off credential-gated phase; 7 `PASS` — both live rows; 8 `N/A` — approved risk: no fence mutation for a live row (absence of the gate is a skip, never a pass); 9 `N/A` — approved risk: no fence to restore. Evidence recorded in `evidence.md` (S6 live read-only proof) with the exact commands and observations.

### Final integration check (2026-09-08)

Assembled tree `e409054`; rebased onto `origin/main` = `340d08b` (PR #9) with the upstream review fixes ported into the family layout, then re-gated. Complete gate `python scripts/ci-gates.py` → `All repository gates passed.` (exit 0; 1,692 s pre-rebase, 1,438 s post-rebase; zero failing test results). Every slice's falsifier, fence, mutation and restoration evidence is recorded above; the isolated design-conformance review's findings are resolved (see `design.md` placement revision) and the live read-only proof is recorded in `evidence.md`. No `FAIL` remains in any recorded gate. No push, PR, merge or tracker closure.

## Self-review

1. Every design row is assigned: C1,C2→S1; C3,C4,C9→S4; C5,C6,C7,C8,C14,C15,C19→S3; C10,C11,C12,C17→S2; C13,C16→S5; C18→S0; C20→S6. Every `PENDING` falsifier is discharged by its claim's slice.
2. Every slice records all fourteen fields, with `N/A — reason` where conditional.
3. Every fence is created in its claim's slice; every fence carries its design mutation; the one fence-less claim (C20) copies the design row's exact `Named mutation: N/A — no fence mutation for a live row`.
4. Every new loop states complexity, production-scale inputs and an explicit maximum accepted cost; every always-on phase records a wall budget.
5. The module growth ledger covers every touched module and protected parent; no slice crosses an approved seam with unrelated responsibilities.
6. Partition arithmetic is recorded (5,250 + 1,050 = 6,300 > 4,000) and every slice names its increment; every increment has a mergeable definition.
7. Tracker taxonomy: the plan adds no deferral; successors are cited by verified IDs (`rfs-r31i`, `rfs-nchb`, `rfs-iktl`, `rfs-e5cv`, `rfs-3r0s`, `rfs-jrz7`).
8. This plan declares no slice complete; `checkpointed-build` judges completion.

## PR10 approved review repair plan amendment

Approval and corrected claims: `design.md` section "Approved PR10 review repair
amendment". Per-finding scope, rejected claims and checkpoint ownership:
`review-decisions.md`. Base is `70c6063`; source equals the assessed `acd02a9`.
Earlier checkpoint receipts above remain historical, not evidence for changed code.

Main integrates patches from five disjoint writer worktrees. Writers run no build,
formatter, linter or test and make no commits. Main owns red/green, mutation/restoration,
production budgets, final gates and commits. This prohibition transfers proof to Main;
it does not waive any check.

| Repair | Inherited slices / claims | Atomic outcome | Estimated changed lines | Local increment |
|---|---|---|---:|---|
| R1 | S2–S4; C3–C15/C17 and reviewed obligations | Authenticated cursors, typed comment identity, exact whole-page admission, scoped outcomes, read-wide validity, source regression/fixture repairs and matching contract | 3,000 | Review-source |
| R2 | S5/S6; C13/C16/C20 | Correct recovery consumers, strict request-target fixtures, native live-helper reuse | 600 | Review-consumers |
| R3 | S0; C18 | One permanent placement command/ledger, inherited checks removed from frozen evidence dependency | 600 | Review-gate |

Estimate 4,200 plus 20% churn allowance (840) = 5,040: review increments are required.
Source repair owns the cursor/fetch consumer seam atomically; cursor and collection
patches cannot be treated as independently green while PageResponse changes. Consumer
and permanent-gate changes are separate local commits/increments. No push, PR, merge or
tracker closure is authorized by the request; local review partition does not imply
publication. If actual source repair exceeds its review-size boundary, split at a
verified interface-compatible seam before committing, not by arbitrary file count.

Expected module ownership remains the design's revised ledger. Facts identity and
collection owners can grow to implement their existing invariants; helpers must stay
within those responsibilities, not become pass-through files to satisfy line counts.
The only protected github/mod.rs changes are private key state/constructor wiring and
catalog grammar. Cursor cryptography stays in continuation.rs. Existing HTTP request
authority validation rejects userinfo. Core record policy and grammar remain unchanged.
The permanent gate owner records exact measured deltas and retained restrictions in its
single ledger; unchanged obligations are inherited, not silently widened.

R1 expected observable checks: addressed and observed IDs agree including >2^53; wrong
parent/link authority rejects the whole read; ordinary later malformed records retain
the prefix without a cursor; duplicates never claim complete; tampered targets and URL
userinfo acquire nothing; source clones resume and foreign contexts cannot; 600+401
resumes the unadmitted page; nested pretty bytes, long metadata and full failure details
fit exact ceilings; resumed scope is explicit; actual expiry and generation invalidation
reject publication. Existing source production tests retain their release limits,
including <=1s collection processing. Per-page work may revisit a bounded prefix only
within the ten-attempt ledger, never perform full serialization once per record.

R2 expected checks: one JSON document reconstructed under both inline and spilled
partial outputs, no artifact-recovery egress, separate source-cursor traversal, wrong
query target rejected, missing gh follows established skip behavior. The existing
1024-byte continuation helper remains the small-page fence.

R3 expected checks: permanent import chain works without frozen oracle directories;
old forbidden dependencies and protected-parent restrictions still fail; new unauthorized
owners still fail; invalid stage is a usage error; restored legitimate tree passes.

Main records exact commands, observed outcomes, independent fixture comparisons and
compilable named-mutation red/restored-green results in review-decisions.md. Final
assembly requires source/HTTP/MCP affected tests, existing production budgets,
credential-gated adapter and real stdio live smoke, python scripts/ci-gates.py, and
independent design/security review. No checkpoint is declared complete by this plan.

### Verified integration partition

Implementation exposed an ordering dependency: the final placement policy requires
the new source symbols, while the old checker rejects their responsibility changes.
The coherent local commit order is therefore **R3 → R1 → R2 → final receipts**:

- R3 transports the existing `module-ledger-bfwa.json` policy unchanged into the
  permanent ledger and keeps the original `validate` identity entry point. The
  permanent-only checker passes against an archived `70c6063` source tree with no
  frozen ticket directories. This checkpoint changes gate machinery, not source
  behavior or its accepted ownership map.
- R1 changes source behavior and its contract together, including the exact
  corresponding required-symbol/API policy. It carries the source regression
  corpus and the approved design/plan amendment. Source, tests and contract
  changes total 3,739 changed lines before that small policy delta, below the
  4,000-line review boundary; the final evidence ledger is a separate receipt.
- R2 carries MCP recovery/strict fixtures and live-oracle changes (865 changed
  lines measured before final receipts).
- The final receipt commit records completed gates and the resulting commit
  identities. It contains no production behavior.

The source corpus now passes 39 tests (2 ignored), including the requested
shared-route differential, hundred-record byte boundaries, full outcome checks,
all cursor fields and actual public-token/hash forgery attempts. Compilable
source/MCP/gate mutations and actual adapter/stdio continuation runs are recorded
in `review-decisions.md` and `evidence.md`. Final gate completion remains required
before these local commits.

Final assembled gate: **PASS**, exit 0 / `All repository gates passed`.
R1's original MCP recovery fixture, with only its required HTML-anchor
consistency correction, also passes without the R2 helper changes. R3's old
source checkpoint and the final strict policy both pass their permanent gate.
