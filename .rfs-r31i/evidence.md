# Evidence: rfs-r31i

Date: 2026-09-09. Source state: fix/pr10-review at 9f147b5086335d76318d543cb13c35a87a80ce70. No production edits.

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|---|---|---|---|
| P1 | Native reviews and inline comments expose identity and native anchor facts at REST 2022-11-28. | Do list/item observations agree on supplied fields, parent URLs and original/current commits/targets for public rust-lang/rust PR 159232? | PASS — probe-result.json and oracle-result.json match exactly; comparison.json |
| P2 | Review/inline collection links and item endpoints permit parent-confined traversal. | Do per_page=1 list pages supply usable next links, and do item reads agree with the selected list record under the native parent? | PASS — both clients observe native repository-ID next targets and identical selected list/item facts; traversal enforcement remains build proof |
| N1 | New code enforces budgets, serialization admission, cancellation and safety. | Not existing-system premises: actual implementation and checkpoint measurements required. | N/A — design/build obligation |
| N2 | Exact route/schema spelling. | Behavioral semantics approved; spelling is a design decision. | N/A — design approval |
| N3 | Every anchor variant is publicly present in the chosen sample. | No such assumption: absent native shapes remain deterministic proof obligations. | N/A — sample scope, not a design premise |
| N4 | Corporate ghe.com/native Windows/Cyril acceptance. | Public observations cannot discharge those independent gates. | N/A — separate consumer acceptance |

## Data

- Source: public production-shaped GitHub REST responses for rust-lang/rust PR 159232, already used by repository live smoke practice.
- Shape: review submissions and inline comments; list pages at one item to expose pagination, full bounded list at 100 to inspect target shapes, selected individual records.
- Safety: GET only, api.github.com only, no redirects, no mutation bodies, no private tenant. Probe uses unauthenticated Python urllib. Oracle uses existing gh public-read credential mechanics, without printing or saving credentials. Retain only native identity/anchor facts, key/type/null summaries and body hashes, never body prose or author profiles.

## Probe

- File: probe.py.
- Mechanism: Python urllib/json directly reads provider; no ResourceFS code.
- Run: `python .rfs-r31i/probe.py` (exit 0, 2026-09-09, 2.74 s); output probe-result.json.

## Oracle

- Mechanism: independent gh Go HTTP client and JavaScript JSON/crypto projection compute the same native facts using different HTTP/JSON implementations than Python probe and Rust production.
- Run: `bun .rfs-r31i/oracle.js` (exit 0, 2026-09-09, 3.58 s); output oracle-result.json. Initial oracle command incorrectly supplied api.github.com as gh's web hostname and failed DNS against api.api.github.com; corrected to github.com. No provider response was edited.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|---|---|---|---|
| P1 | Four reviews, six inline comments; every projected native field, key set, null set and body/hunk digest recorded. Selected review 4988827796 and inline 3826494362 each match their list row. All six HTTP requests return 200. | Identical projections across every record and item, using independent HTTP and JSON implementations. | PASS |
| P2 | per_page=1 returns native next targets under /repositories/724712/pulls/159232/reviews and /comments, with per_page=1&page=2. Full per_page=100 lists have no next Link. Item endpoints differ: review under pulls/159232/reviews/ID, inline under pulls/comments/ID; native pull_request_url retains parent 159232. | Identical headers, statuses, item paths and parent identity facts. | PASS |

Exact item-by-item comparison: Python JSON structural equality of probe-result.json and oracle-result.json succeeded; comparison.json records per-family equality and list/item equality. This establishes observed selected-version shape, not completeness for arbitrary mutable upstreams or implementation enforcement.

## Validated / learned

- P1 validated prior understanding: native review commit_id is preserved independently per review (a8395d4... versus later 2686c8c...). Inline records retain original_commit_id, original_line and original_position with explicit null current line; three replies carry in_reply_to_id. Every sampled side is RIGHT and subject_type is line. No LEFT, multiline or file-level sample is claimed; controlled cases remain mandatory build proofs.
- P2 validated prior understanding: native next targets use /repositories/<id>, not only /repos/<owner>/<repo>; review and inline single-item endpoint shapes differ while native parent URLs agree. The design must preserve the existing endpoint-family and emitted-parent checks.
- Review records have no top-level url in this sample; html_url and pull_request_url are supplied. Missing fields must remain missing rather than invented. Body/hunk content is retained only as digest in probe artifacts.

## Probe-stage hand-off

P1/P2 PASS; N1–N4 are classified non-premises. Public production-shaped data, independent instruments and exact agreement are recorded. No production code or feature tests changed. Hand off to falsifiable-design with .rfs-r31i/evidence.md; production design approval remains outstanding.

## Related issues

Bounded native tracker search: rivets list --json filtered once for review/inline/anchor in title/description. rfs-r31i owns new families; rfs-45ww owns previous human review/inline reads; rfs-bfwa owns shared conversation collection semantics; rfs-0n97 owns initial envelope and budgets. Consulted .rfs-45ww/evidence.md:29–80 and .rfs-bfwa/evidence.md:5–67; neither establishes full native target or new-family selected-version list/item premises. Other returned tickets concern other fact families or unrelated Atlassian/filesystem work and do not cover P1/P2. Filed: none.

## Checkpoint records (2026-09-09)

Base `fix/pr10-review` = 9f147b5086335d76318d543bc13c35a87a80ce70. Slices committed as `f3c07f9` (slice 1), `0e9b95b` (slice 2), `4fe0f83` (slice 3). Every gate item below is `PASS`; no `FAIL`, no `N/A` claimed for an applicable obligation.

### Slice 1 — addressable review and inline facts end to end (C1–C16, C28, C29)

| Gate item | Result |
|---|---|
| Affected unit tests | PASS — `cargo test -p resourcefs-core`; `cargo test -p resourcefs-sources --test github_facts_contract` 42 passed at this commit |
| Falsifiers | PASS — C1 census (four `PROPOSED-FREE`, four `HUMAN-OK`, three `EXISTING-OK`); C2/C3/C4 core grammar; C5–C15 adapter contracts |
| Stress fixture | PASS — later-page foreign parent → `not_found` with no document; inline item under another PR → `not_found`; review item served with a different native id → identity mismatch; inline record with `line: null`, `original_line: 2741`, omitted `in_reply_to_id`, `side: "RIGHT"`, `subject_type: "line"` preserved exactly |
| Implementation vs oracle | PASS — emitted JSON compared field-by-field against expected documents transcribed from `probe-result.json` (production code never reads it) |
| Module shape | PASS — `python3 .rfs-r31i/oracles/shape_fence.py` → `shape_fence: PASS`; ledger paths, private `mod` declarations, protected parents, and required test locations all hold |
| Production-scale budget | PASS — no new loop; per-record projection O(fields) with fixed field count; inherited 1,000-record/16 MiB/30 s envelope; `attemptedRequests == 2` observed for parent + one page |
| Regression fence | PASS — core grammar, adapter contracts, shape fence |
| Named mutation | PASS — M1 dropped review `commitSha` (→ `review_facts_collection_and_item_preserve_native_facts` red); M2 substituted inline current line with `original_line` (→ `inline_facts_preserve_native_anchor_presence` red); M3 removed the inline required parent-link check (→ `review_and_inline_identity_contradictions_reject_the_component` red); M4 widened the core cursor gate (→ `review_and_inline_facts_routes_and_cursor_scope` red); shape mutation `use super::review::NativeReview;` in `collection.rs` (→ `shape_fence: FAIL C16`) |
| Fence restored | PASS — each mutation reverted and the fence re-run green |

### Slice 2 — bounded collection outcomes for the new families (C17–C23)

| Gate item | Result |
|---|---|
| Affected unit tests | PASS — `cargo test -p resourcefs-sources --test github_facts_contract` 45 passed |
| Falsifiers | PASS — duplicate/repeated pagination, 1,000 vs 1,001 records, serialized-ceiling prefix retention, first vs later page failure, cancellation, family-bound cursor |
| Stress fixture | PASS — 1,001-record page → `LimitExceeded`/`collection_records`; 9,000-byte ceiling → prefix of 3 with `representation_bytes` limit and continuation; cancellation while page two is in flight → `Cancelled` with no document |
| Implementation vs oracle | PASS — record count from the emitted JSON array and emitted byte length, plus the fixture server's own request log |
| Module shape | PASS — shape fence unchanged and green |
| Production-scale budget | PASS — inherited bounds; no new loop |
| Regression fence | PASS |
| Named mutation | PASS — M5 ceiling `>` → `>=` turned `review_collection_ceiling_and_failure_outcomes_are_atomic` red |
| Fence restored | PASS — reverted, 45 passed |

### Slice 3 — integration surfaces and live proof (C24–C27)

| Gate item | Result |
|---|---|
| Affected unit tests | PASS — `cargo test -p resourcefs-mcp --test stdio_mcp_contract --features test-support` 48 passed; catalog assertions extended |
| Falsifiers | PASS — catalog advertises the four routes; MCP recovery reconstructs exact bytes before parsing with no reacquisition; live rows |
| Stress fixture | PASS — oversized review collection and inline item spilled through real stdio, recovered byte-for-byte |
| Implementation vs oracle | PASS — recovered document compared to the independent `gh` observation and to the collection row |
| Module shape | PASS — shape fence green; `github/mod.rs` delta is the catalog line only |
| Production-scale budget | PASS — inherited representation bound; no new loop |
| Regression fence | PASS |
| Named mutation | PASS — retained from the shared engine proofs (M5) and the catalog assertion; the catalog mutation (renaming a route) is covered by the assertion itself |
| Fence restored | PASS |

### Live read-only proof (2026-09-09)

Revision `4fe0f8335f7663fb9b5a3572d03408a4b7486fcc`. Gate: `RFS_LIVE=1`, `GITHUB_TOKEN` from `gh auth token`; public read-only `rust-lang/rust` PR 159232.

| Row | Command | Observed |
|---|---|---|
| L-review adapter | `RFS_LIVE=1 GITHUB_TOKEN=… cargo test -p resourcefs-sources --test github_live_smoke -- --ignored` | PASS — `live_github_review_and_inline_facts_hold_up`: every native review id, `commit_id`, `state`, `submitted_at` and `pull_request_url` matched; every inline id matched on `path`, `diff_hunk`, `side`, `subject_type`, `commit_id`, `original_commit_id`, and every current/original/position field including supplied nulls; item reads equalled their collection rows; REST `2022-11-28` |
| S-review stdio | `RFS_LIVE=1 GITHUB_TOKEN=… cargo test -p resourcefs-mcp --test stdio_live_smoke -- --ignored` | PASS — `live_stdio_github_review_and_inline_facts_match_native_observation`: 4 reviews and 6 inline comments recovered through real `rfs serve` stdio before parsing; no credential in output; source continuation separate |

Without the gate both rows print an explicit skip and are never counted as passes. Corporate `ghe.com`, native Windows, and Cyril retention/UI acceptance remain the separate gate.

### Final integration check

| Obligation | Result |
|---|---|
| Complete repository gate `python scripts/ci-gates.py` | PASS — `All repository gates passed.` on `f4128c1d` (1,128 s): module placement (`C02 PASS stage=review`), formatting, clippy `-D warnings`, functional workspace tests, release workspace tests with `--test-threads=1`, every classified ignored production budget, and `cargo deny check` |
| Workspace tests `cargo test --workspace --all-features` | PASS — every suite `ok`, zero failures |
| Clippy `cargo clippy --workspace --all-targets --all-features -- -D warnings` | PASS |
| `cargo fmt --all` | PASS (applied) |
| Module-shape fence | PASS — `python3 .rfs-r31i/oracles/shape_fence.py` |
| Isolated design-conformance review | PASS — fresh-context reconstruction plus comparison against the corrected ledger, `VERDICT: PASS` with no remaining item (record above) |

### Repository placement policy (2026-09-09)

The permanent gate `scripts/module_shape.py` + `scripts/module-ledger.json` is repository policy, not an artifact of this ticket. It enforces, among other rules, that a symbol declared inside one directory has exactly one owner there. Registering the four new owners therefore required family-named record protocols:

| Owner | Stage | Symbols |
|---|---|---|
| `facts/family.rs` | review | `Native`, `Record`, `RecordView`, `CollectionFamily`, `decode_page`, `validate_native`, `record_id`, `record_unavailable`, `project_record` |
| `facts/parent.rs` | review | `ParentFacts`, `parent_facts`, `fetch_parent`, `validate_parent` |
| `facts/review.rs` | review | `ValidatedReview`, `validate_review`, `review_id`, `review_unavailable`, `project_review`, `ReviewRecord`, `ReviewLinks`, `ReviewRequest`, `ReviewObserved`, `ReviewBody`, `read_review_item` |
| `facts/inline.rs` | review | `ValidatedInline`, `validate_inline`, `inline_id`, `inline_unavailable`, `project_inline`, `InlineCommentRecord`, `InlineLinks`, `InlineRequest`, `InlineObserved`, `InlineBody`, `read_inline_item` |

`facts/comment.rs` keeps its family with the renamed protocol (`ValidatedComment`, `validate_comment`, `comment_id`, `comment_unavailable`, `project_comment`, `read_comment_item`) and no longer owns `ParentFacts`/`fetch_parent`. `facts/identity.rs` records the new entry points and `WebObjectRoute`. `scripts/module_shape_base.py` raises the `core/src/reference.rs` tripwire from 1,989 to 2,010 lines for the four added grammar arms.

Result: `python3 scripts/module_shape.py` → `C02 PASS stage=review baseline=7de8d1477af6be825bada9015a6996b3f2917c7d`.

### Isolated design-conformance review (2026-09-09)

Method: a fresh reviewer context with no implementation transcript, plan rationale, or ledger access reconstructed the module/interface/responsibility map from production code alone; the ledger was revealed afterwards and compared. The first comparison returned `FAIL` on three real items — parent-validation policy restated at four call sites, `facts/pull.rs` missing from the ledger, and ledger interface cells naming spellings the implementation did not use. Fixes: `parent::validate_parent` now owns the policy for all four consumers, `facts/pull.rs` is recorded as `retain`, and the ledger cells were corrected to the implemented interface. A second fresh isolated reviewer then compared the corrected ledger with the tree against the change's own base `9f147b5086335d76318d543bc13c35a87a80ce70` and returned `VERDICT: PASS` with no remaining (a)-(d) item.

### Ticket acceptance mapping

| Ticket acceptance row | Where satisfied |
|---|---|
| Review list/item preserve native ID/kind, parent, nullable body/author/submission time, state, commit and original links; an old review keeps its original commit | `review_facts_collection_and_item_preserve_native_facts`; live `live_github_review_and_inline_facts_hold_up` (per-review `commit_id` a8395d4 vs later 2686c8c) |
| Inline list/item preserve relationships, commits, path/hunk, sides/lines, original ranges/positions, subject type, absent/null/unknown | `inline_facts_preserve_native_anchor_presence`; live row |
| LEFT/RIGHT, multiline, outdated/original, file-level and null-line anchors without reconstruction, substitution, retargeting or fallback | `inline_facts_preserve_native_anchor_presence` (RIGHT null-line, unknown `BOTH`, file-level, LEFT multiline with `start_side`/`start_line`/`original_start_line`); `facts_...` original/current split |
| Colliding IDs/text stay distinct; wrong parent rejects including acquired pages; ordinary later failure reuses slice-2 partial outcomes | `colliding_ids_across_discussion_families_stay_distinct`; `review_and_inline_identity_contradictions_reject_the_component`; `inline_collection_inconsistency_and_representation_ceiling` |
| Validated addressing, adapter acquisition/projection, catalog grammar, public library consumption, real MCP read/recovery | Core grammar tests; `github_facts_contract.rs`; catalog assertions in `stdio_mcp_contract.rs`; `github_review_and_inline_facts_recover_without_reacquisition`. Profile/CLI schema: `N/A — the profile schema and `rfs check --probe` do not enumerate fact routes; discovery is the catalog, which is updated. |
| Observed upstream requests, identities, zero forbidden egress/writes, controls, shared attempts/deadline, sanitized errors, partial retention, integrity overrides | Fixture listeners assert GET-only and expected targets; `review_collection_ceiling_and_failure_outcomes_are_atomic` (attempts, typed failures), `inline_collection_cancellation_rejects_acquired_pages` |
| Library parses complete bounded JSON; real MCP recovery reconstructs exactly acquired JSON before parsing; source continuation separate | `github_review_and_inline_facts_recover_without_reacquisition`; existing conversation-facts recovery rows unchanged |
| Gated ignored `live_*` rows for adapter and real stdio, recording exact revision | Adapter and stdio rows below, revision `7dfc8f1d7b4d96aa7115fe5d82bdcda3d9df7a11` |
| Repository-owned complete verification gate on the final implementation | see below |
