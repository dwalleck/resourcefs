# Plan: rfs-r31i

Date: 2026-09-09. Inputs: `route.md` (Empirical), `design.md` (approved 2026-09-09, verbatim "Approve as designed"), `evidence.md` + `probe-result.json`/`oracle-result.json`/`comparison.json`. Base branch: `fix/pr10-review` = 9f147b5086335d76318d543bc13c35a87a80ce70 (default/upstream branch discovered per contract: `origin/main`, base commit 340d08b).

## Module growth ledger

| Module | Baseline production lines | Projected final lines | Responsibility change | Interface change | Protected-parent rule |
|---|---:|---:|---|---|---|
| `crates/resourcefs-core/src/reference/pull.rs` | 51 | 85–100 | add four PR fact variants | `PullRequestFact` sum grows; cursor gate widens | N/A |
| `crates/resourcefs-core/src/reference.rs` | 1,961 | 2,010–2,040 | canonical + parse arms for four spellings | `parse_pull_request_address` accepts four new paths | N/A |
| `crates/resourcefs-sources/src/github/facts.rs` | 627 | 650–665 | four dispatch arms; selector guard widened | none | declarations + dispatch arms only; no family body |
| `crates/resourcefs-sources/src/github/facts/family.rs` | 0 | 160–220 | create: family set + dispatch | new private `Native`/`Record`/`RecordView` sums | repository stage `review` |
| `crates/resourcefs-sources/src/github/facts/parent.rs` | 0 | 80–130 | create: PR parent acquisition/validation/projection moved from `comment.rs` | `parent_facts`, `fetch_parent`, `validate_parent`, `ParentFacts` | repository stage `review` |
| `crates/resourcefs-sources/src/github/facts/collection.rs` | 767 | 800–840 | parameterize over a collection family; delegate family work | `read(..., CollectionFamily, ...)` | no family-module imports; no family validation |
| `crates/resourcefs-sources/src/github/facts/comment.rs` | 298 | 235–250 | delete parent ownership; keep conversation family | loses `ParentFacts`/`fetch_parent`; record protocol renamed `ValidatedComment`/`validate_comment`/`read_comment_item` | repository stage `review` |
| `crates/resourcefs-sources/src/github/facts/review.rs` | 0 | 260–330 | create: review family | `ValidatedReview`, `validate_review`, `read_review_item`, `ReviewRecord` | repository stage `review` |
| `crates/resourcefs-sources/src/github/facts/inline.rs` | 0 | 340–420 | create: inline family | `ValidatedInline`, `validate_inline`, `read_inline_item`, `InlineCommentRecord` | repository stage `review` |
| `crates/resourcefs-sources/src/github/facts/identity.rs` | 459 | 510–560 | extend web-fragment recognition; add reusable object-link/id predicates | `expected_object_url`, `require_object_link`, `validate_optional_object_link`, `validate_optional_web_link`, `require_observed_id` | N/A |
| `crates/resourcefs-sources/src/github/mod.rs` | 1,170 | 1,170–1,172 | catalog grammar string | none | grammar line only |
| `crates/resourcefs-sources/src/github/facts/continuation.rs` | 253 | 253–256 | none (module doc only) | none | N/A |
| `crates/resourcefs-sources/src/github/facts/pull.rs` | 264 | 264 | none (unchanged; recorded as touched by the ledger review) | none | N/A |
| `crates/resourcefs-mcp/src/server.rs` | 1,003 | 1,003 | none | none | retain |

## Partition arithmetic

| Slice | Diff estimate (implementation + tests + fixtures) |
|---|---:|
| 1 — Addressable review and inline facts end to end | 1,850 |
| 2 — Bounded collection outcomes for the new families | 620 |
| 3 — Integration surfaces and live proof | 800 |
| Sum | 3,270 |
| Churn margin (25%) | 818 |
| Total | 4,088 |

Churn margin rationale: a new-family feature with hand-authored native fixtures and a moved responsibility; the conversation-facts slice (rfs-bfwa) drifted ~20% above its projection on fixtures alone, and this plan adds two families.

**Partition.** The projected total exceeds 4,000, so the plan is partitioned into two independently mergeable PR increments:

| Increment | Slices | Mergeable definition | Verifies without the later increment |
|---|---|---|---|
| A — Review and inline fact families | 1, 2 | Core grammar, both families, shared collection engine, and every bounded-outcome fence land together | Core grammar tests + fixture-driven adapter contract tests + the module-shape fence all pass against deterministic fixtures; no MCP/catalog/live dependency |
| B — Integration and live proof | 3 | MCP recovery, catalog, egress oracle, Version Tag, and gated live smokes | Increment A's behavior is already proven; B adds consumer/recovery/live surfaces only |

Ship point: increment A opens its PR before slice 3 starts (step 10 of `checkpointed-build`). Actual cumulative lines `> 4,000` with placement unchanged returns to this plan; a placement change returns to `design.md`.

## Slice 1: Addressable review and inline facts end to end

**Claim IDs:** C1, C2, C3, C4, C5, C6, C7, C8, C9, C10, C11, C12, C13, C14, C15, C16, C28, C29

**Expected behavior:** `pr://<owner>/<repo>/<n>/reviews/facts`, `…/reviews/<id>/facts`, `…/review-comments/facts`, `…/review-comments/<id>/facts` parse to distinct `PullRequestFact` variants, accept the `:cursor:` selector only for the three collections, and read complete owned JSON through `GithubSource` that preserves every supplied native field (omitted/null/present) and rejects wrong parents/ids.

**Oracle:** Recorded public provider observation `probe-result.json` transcribed into expected documents for the deterministic fixtures (production code never reads it); pre-change parser census `falsifiers/grammar_probe.rs` output for the four human routes.

**Stress fixture:** Two-page inline collection whose page 2 record names a different PR in `pull_request_url`, plus an item read served with a different native `id`; expected: `upstream_identity_mismatch` rejection and no emitted document. Inline record carrying `line: null`, `original_line: 2741`, `in_reply_to_id` omitted, `side: "RIGHT"`, `start_side: null`, `subject_type: "line"`; expected: nulls emitted as null, `replyToId` omitted, `side` verbatim.

**Regression fence:** `crates/resourcefs-core` grammar tests (`review_and_inline_facts_routes_round_trip`, `cursor_requires_a_discussion_collection`); `crates/resourcefs-sources/tests/github_review_facts_contract.rs`; `crates/resourcefs-sources/tests/github_inline_facts_contract.rs`; `.rfs-r31i/oracles/shape_fence.py`.

**Named mutation:** Delete `commit_sha` from `ReviewRecord` (review fence red); substitute `line: &self.0.original_line` in `inline::project` (inline fence red); remove the `validate_review_links`/`validate_review_comment_links` call (identity fence red); widen the cursor gate to item facts (grammar fence red); add `use super::review::NativeReview;` to `collection.rs` (shape fence red).

**Complexity/production scale:** Per-record projection is O(fields) with a fixed field count (≤ 24 for inline), so O(1) per record; the collection loop is the inherited O(pages × page size) bounded at 1,000 records. Maximum accepted cost for the slice: the inherited 1,000-record / 16 MiB serialized / 30 s deadline envelope, with no new asymptotic term; rationale: this slice adds families to an existing bounded engine and adds no unbounded work.

**Wall budget/phase:** always-on — per-read acquisition deadline, inherited bound 30 s for the whole read (send/body/wait shared); rationale: the new families add no phase of their own, and the specification's 30 s ceiling already covers them.

**Module shape:** adds `family.rs`, `parent.rs`, `review.rs`, `inline.rs`; moves parent acquisition and its validation policy from `comment.rs`; widens `collection.rs` interface to take a `CollectionFamily`; protected parents: `facts.rs` (declarations + 4 arms only), `github/mod.rs` (grammar line only), `collection.rs` (no family imports; `read(source, repository, number, CollectionFamily, cursor, ctx)`). Owner after slice: per the approved module ledger. Shape fence: `python .rfs-r31i/oracles/shape_fence.py` → `PASS` with every ledger rule reported by claim ID.

**Files:** `crates/resourcefs-core/src/reference/pull.rs`, `crates/resourcefs-core/src/reference.rs`, `crates/resourcefs-core/tests/github_reference_contract.rs`, `crates/resourcefs-sources/src/github/facts.rs`, `…/facts/family.rs` (new), `…/facts/parent.rs` (new), `…/facts/collection.rs`, `…/facts/comment.rs`, `…/facts/review.rs` (new), `…/facts/inline.rs` (new), `…/facts/identity.rs`, `crates/resourcefs-sources/tests/github_facts_contract.rs`, `.rfs-r31i/oracles/shape_fence.py` (new).

*Correction (2026-09-09, checkpointed-build slice 1):* the adapter contract fences landed in the existing `crates/resourcefs-sources/tests/github_facts_contract.rs` instead of new `github_review_facts_contract.rs` / `github_inline_facts_contract.rs` files, because that file owns the TLS fake-upstream harness these fences need; duplicating the harness into two new files would have created a second verification location for one responsibility. Fence names, claims, oracles, and mutations are unchanged.

**Estimate:** 1.5–2.5 days.

**Diff estimate:** 1,850.

**PR increment:** A.

**Commands and expected results:**
- `cargo test -p resourcefs-core` → grammar rows pass; the four human routes and three existing facts routes keep their parsed variants and canonical strings (C1–C4).
- `cargo test -p resourcefs-sources --test github_review_facts_contract --test github_inline_facts_contract` → collection and item reads emit the expected documents, omitted/null/present distinctions survive, unknown native values verbatim, wrong parent/id rejected (C5–C15).
- `python .rfs-r31i/oracles/shape_fence.py` → `PASS`; under the named mutation it reports the claim ID and exact path/symbol (C16, C29).
- `cargo test -p resourcefs-sources` → existing human projection and conversation-facts tests unchanged (C28).

## Slice 2: Bounded collection outcomes for the new families

**Claim IDs:** C17, C18, C19, C20, C21, C22, C23

**Expected behavior:** The inherited collection engine produces the same honest outcomes for review and inline collections: duplicate/repeated pagination → unknown coverage; atomic 1,000-record and serialized-ceiling admission with verified-prefix retention and continuation to the rejected page; typed first failure vs retained later-page prefix; cancellation/authority rejection; bound continuations; one shared attempt/deadline budget across parent and pages.

**Oracle:** Independent measurement of the emitted document (`content.len()` and JSON array length) and the loopback request log; specification §5 outcome table as the external expectation list.

**Stress fixture:** Inline collection sized to cross the serialized ceiling by one byte on the final page; a 1,001-record page; a page repeating a native id; a next link repeated verbatim; a cursor replayed against a different resource; a cancelled read after page 1. Expected: prefix retained with `incomplete` + continuation naming the rejected page; 1,001 never partially admitted; `unknown` with `duplicate_record_id`; `unknown` with `repeated_pagination`; cursor refused before egress; typed cancellation with no document.

**Regression fence:** `crates/resourcefs-sources/tests/github_review_facts_contract.rs` and `…/github_inline_facts_contract.rs` outcome rows (`record_ceiling_is_atomic`, `representation_ceiling_retains_prefix`, `first_failure_is_typed_later_failure_keeps_prefix`, `continuation_is_resource_and_family_bound`).

**Named mutation:** Admit `page_records` before the ceiling check; truncate the serialized document instead of rolling back the page; treat cancellation like an ordinary later failure; skip the resource/origin check in `continuation::decode`.

**Complexity/production scale:** no new loop; the inherited loop's bound is unchanged (≤ 1,000 records, ≤ 16 MiB serialized, ≤ 10 attempts, 30 s). Maximum accepted cost for the slice: the same envelope, with per-page admission measurement O(1) in the number of pages beyond the existing serialization pass; rationale: fixtures exercise the inherited bound, they do not add one.

**Wall budget/phase:** always-on — inherited 30 s per-read deadline; rationale: fixtures exercise the existing phase, no new phase is introduced.

**Module shape:** no responsibility or interface change; the engine stays the single owner. Protected parents unchanged. Shape fence: `python .rfs-r31i/oracles/shape_fence.py` → `PASS` with the same ledger.

**Files:** `crates/resourcefs-sources/tests/github_facts_contract.rs` (outcome fences reuse the existing fake-upstream harness; see the slice-1 correction).

**Estimate:** 1–1.5 days.

**Diff estimate:** 620.

**PR increment:** A.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_review_facts_contract --test github_inline_facts_contract` → every outcome fixture produces the recorded expected state and failure facts (C17–C23).
- `python .rfs-r31i/oracles/shape_fence.py` → `PASS`.

## Slice 3: Integration surfaces and live proof

**Claim IDs:** C24, C25, C26, C27

**Expected behavior:** All four routes produce only authorized GETs with zero forbidden egress and zero remote writes; oversized review/inline documents survive MCP display via exact Recovery References; source continuation remains a separate acquisition; the Version Tag tracks representation bytes; the catalog advertises the four routes; gated live read-only smokes exercise all four routes and the anchor invariants.

**Oracle:** Independent loopback listener request log (egress/writes/attempts); MCP reconstruction byte comparison against the emitted document; Version Tag recomputed over emitted bytes; recorded public observation for live-row invariants.

**Stress fixture:** Loopback server with a denied origin and a redirect target; MCP stdio read whose document exceeds the inline page; a mutated supplied native fact with an unchanged Git identity. Expected: zero requests to denied/redirect targets and only GETs to the configured origin; recovery reconstructs the exact bytes and parses only after reconstruction; the Version Tag changes with the native fact and not with Git identity.

**Regression fence:** `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs` rows for review/inline recovery and continuation separation; `crates/resourcefs-sources/tests/github_live_smoke.rs` and `crates/resourcefs-mcp/tests/stdio_live_smoke.rs` ignored `live_*` rows; catalog discovery test.

**Named mutation:** Emit a truncated document instead of an artifact continuation; hash the Git commit id in `finish_facts`; follow a `pull_request_url` to another origin for the parent lookup; rename a route in the catalog string.

**Complexity/production scale:** no new loop; MCP recovery is the inherited artifact path. Maximum accepted cost: the inherited 16 MiB representation bound and recovery round-trip; rationale: integration adds no new unbounded work.

**Wall budget/phase:** one-off — `N/A — reason: one-off phase; no wall budget` for the live smoke rows; the per-read 30 s deadline is inherited and unchanged.

**Module shape:** no production placement change; `github/mod.rs` gains the grammar string only; MCP retains its responsibility. Shape fence: `python .rfs-r31i/oracles/shape_fence.py` → `PASS`.

**Files:** `crates/resourcefs-sources/src/github/mod.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`, `crates/resourcefs-sources/tests/github_live_smoke.rs`, `crates/resourcefs-mcp/tests/stdio_live_smoke.rs`, `.rfs-r31i/evidence.md` (executed live rows and exact revision).

**Estimate:** 1–1.5 days.

**Diff estimate:** 800.

**PR increment:** B.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract` → recovery reconstructs exact bytes for review and inline documents; source continuation is separate (C25).
- `RFS_LIVE=1 GITHUB_TOKEN=$(gh auth token) cargo test -p resourcefs-sources --test github_live_smoke -- --ignored` → all four routes read public read-only data; anchors match the independent observation; absent gate skips cleanly (acceptance row 8).
- `RFS_LIVE=1 GITHUB_TOKEN=$(gh auth token) cargo test -p resourcefs-mcp --test stdio_live_smoke -- --ignored` → real stdio recovery and continuation rows pass (C24, C25).
- `python scripts/ci-gates.py` → `All repository gates passed.` (final integration).

## Self-review

1. Every design row C1–C29 is assigned to exactly one slice; every `PENDING` falsifier is discharged by its slice's Commands and expected results field. ✔
2. Every slice records all fourteen mandatory fields with `N/A — reason` where conditional. ✔
3. Every claim's fence is created in the slice implementing it; every fence carries the design's named mutation; no claim uses `N/A — approved risk`. ✔
4. Every new loop states complexity, production-scale cost, and an explicit maximum accepted cost with rationale; the always-on phase records the inherited 30 s wall budget. ✔
5. Module shape and the growth ledger cover every touched module and protected parent; no slice crosses an approved seam with unrelated responsibilities. ✔
6. Partition arithmetic is recorded with a 25% churn margin and two independently mergeable increments. ✔
7. Tracker taxonomy: deferrals are classified in `design.md`'s non-goals; no new deferral phrase appears here. ✔
8. The plan declares no slice complete; completion is `checkpointed-build`'s judgment. ✔
