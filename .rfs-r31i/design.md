# Design: rfs-r31i

Date: 2026-09-09. Source state: fix/pr10-review = 9f147b5086335d76318d543cb13c35a87a80ce70. Ticket: rfs-r31i (in_progress, omp).

## Route and inputs

- Route: Empirical (`.rfs-r31i/route.md`), because native review/inline endpoint premises were unverified.
- Behavior set: `route.md` T4 rows 1–13 (complete given/when/then contract from rfs-r31i and approved specification stories 23–25, 28–43). `spec.md` is `N/A — behavior fully explicit`; the T4 set is the behavior source.
- Empirical premises: `.rfs-r31i/evidence.md` P1 (selected REST 2022-11-28 review/inline records supply native identity, parent and anchor facts) and P2 (list next links and item endpoints permit parent-confined traversal), both `PASS` against public rust-lang/rust PR 159232 with an independent `gh`/JavaScript oracle (`probe-result.json`, `oracle-result.json`, `comparison.json` structural equality).
- Observed selected-version shapes (evidence, not assumption): review keys `_links, author_association, body, commit_id, html_url, id, node_id, pull_request_url, state, submitted_at, user` (no top-level `url` in the sampled list); inline keys include `commit_id, diff_hunk, in_reply_to_id, line, original_commit_id, original_line, original_position, original_start_line, path, position, pull_request_id, pull_request_url, side, start_line, start_side, subject_type, updated_at, url`; `line`, `start_line`, `start_side`, `original_start_line` are supplied **null** while `in_reply_to_id` is **omitted** for non-replies. The tri-state `Presence` decoder in `facts.rs` is the only vocabulary that keeps those distinct.
- Codebase extraction (`.rfs-r31i/route.md` T2 plus seam mapping `agent://ReviewSeams`): core owns `pr://` grammar (`reference.rs:1300-1380`, `reference/pull.rs`); `GithubSource` owns endpoint mapping (`github/mod.rs:613-678`) and facts dispatch (`facts.rs:549-621`); `facts/collection.rs` owns bounded traversal/admission/coverage for conversation comments only; `facts/comment.rs` owns conversation decoding, validation, projection **and** the shared PR parent (`ParentFacts`, `parent_facts`, `fetch_parent`); `facts/identity.rs` owns PR identity and supplied-link authority; `facts/continuation.rs` owns the authenticated cursor; `fetch.rs`/`http/read.rs` own transport and budgets; MCP `rfs_read` plus artifact recovery already handle any facts document.
- Human projections (`PullRequestResource::Reviews/Review/ReviewComments/ReviewComment`) and their wire DTOs stay untouched: `wire.rs:79-103` cannot carry the required anchors, which is exactly why a separate owned family is required.

## Input shapes

| # | Shape | Status |
|---|---|---|
| S1 | Four proposed `/facts` spellings, canonical round-trip | C1, C2 |
| S2 | Cursor selector on the three collection facts; refused on item facts and `Pull` | C3 |
| S3 | Existing human routes and existing conversation facts routes | C4, C28 |
| S4 | Review record: body/author/submittedAt/commitSha/nodeId/url present, absent, or null | C5 |
| S5 | Review collection: empty, single, multi; item/list parity | C6 |
| S6 | Inline record: every supplied anchor field present, absent, or null | C7 |
| S7 | Unknown native enum-ish values (`side`, `state`, `subject_type`) | C8 |
| S8 | Outdated discussion (current vs original commit/line/position) | C9 |
| S9 | File-level (`subject_type: "file"`, null line) and null-line anchors | C10 |
| S10 | Multiline anchors (`start_side`/`start_line`) | C11 |
| S11 | Reply/thread relationship (`in_reply_to_id` present vs omitted), `reviewId` null | C12 |
| S12 | Wrong parent PR in supplied `pull_request_url` | C13 |
| S13 | Item read whose native id differs from the requested id | C14 |
| S14 | Inline item read through the repository-wide endpoint under a PR that does not own it | C15 |
| S15 | Page with duplicate native ids; repeated pagination URL | C17 |
| S16 | Record count 1,000 exact and 1,001; oversized first page | C18 |
| S17 | Representation bytes exact and one byte over, with prefix retention and continuation | C19 |
| S18 | First-page failure with no usable record vs later-page failure with a verified prefix | C20 |
| S19 | Cancellation and authority/identity violation mid-collection | C21 |
| S20 | Valid, unrepresentable, wrong-family and wrong-origin continuation | C22 |
| S21 | Physical attempts and one shared deadline across parent + pages | C23 |
| S22 | Denied origin/repository/redirect and mutation verbs on all four routes | C24 |
| S23 | Oversized facts document spilled through MCP display | C25 |
| S24 | Representation bytes vs Git identity in the Version Tag | C26 |
| S25 | Catalog grammar text | C27 |
| S26 | Module placement: family dispatch, protected parents, cross-family imports | C29 |
| S27 | Numeric zero/negative/overflow child ids in the grammar | C2 (refusal half) |
| S28 | Unknown/unsupplied contract fields → `unavailableFacts` | C5, C7 |
| S29 | Provider caps (reviews/comments endpoints) | C18 |
| S30 | Non-UTF-8 or non-object page body | C20 |
| S31 | Regular-file decoded-byte bound (4 MiB) | `N/A — no regular-file path in this change` |
| S32 | Comparison/tree/checks families | `N/A — owned by rfs-nchb, rfs-iktl, rfs-e5cv, rfs-3r0s, rfs-jrz7` |
| S33 | Corporate `ghe.com`, native Windows, Cyril retention/UI | `N/A — separate Cyril acceptance gate (spec §7)` |

## Placement

| Capability | Owner | New seam | Forbidden |
|---|---|---|---|
| `pr://…/reviews/facts`, `/reviews/<id>/facts`, `/review-comments/facts`, `/review-comments/<id>/facts` grammar and cursor eligibility | `resourcefs-core::reference` (`reference.rs`, `reference/pull.rs`) | Extends the existing `PullRequestFact` sum and `parse_pull_request_reference` cursor gate | Provider decoding, endpoint knowledge, HTTP |
| Review-submission native decode, validation, projection, item read | `crates/resourcefs-sources/src/github/facts/review.rs` (new) | Private family module, mirroring `comment.rs`; owns its singular `pulls/<n>/reviews/<id>` endpoint exactly as `comment.rs` owns `issues/comments/<id>` | Traversal/admission, parent acquisition, collection endpoint suffixes |
| Inline-comment native decode, validation, projection, item read | `crates/resourcefs-sources/src/github/facts/inline.rs` (new) | Private family module; owns its singular `pulls/comments/<id>` endpoint | Traversal/admission, parent acquisition, collection endpoint suffixes |
| The set of discussion fact families and their decode/validate/id/project/kind/suffix dispatch | `crates/resourcefs-sources/src/github/facts/family.rs` (new) | Private `Native`/`Record`/`RecordView` sum types, `CollectionFamily::{from_fact, kind, suffix, decode}`, and `validate`/`id`/`unavailable`/`project` | Traversal/coverage, HTTP, provider auth |
| PR parent acquisition, validation policy and projection shared by all families | `crates/resourcefs-sources/src/github/facts/parent.rs` (new; moved out of `comment.rs`) | `parent::fetch_parent` (acquire, decode, establish generation), `parent::validate_parent` (composes `identity::validate` against the endpoint/API/web authorities), `parent::parent_facts`, `ParentFacts` | Family record knowledge |
| Bounded traversal, whole-page admission, coverage, continuation | `crates/resourcefs-sources/src/github/facts/collection.rs` | `read(source, repository, number, CollectionFamily, cursor, ctx)` | Family decoding/validation bodies, parent acquisition, family-to-collection conversion |
| Supplied-link authority: reusable object-link/id predicates and web-fragment recognition | `crates/resourcefs-sources/src/github/facts/identity.rs` | Extends `web_route` fragment recognition and adds `expected_object_url`, `require_object_link`, `validate_optional_object_link`, `validate_optional_web_link`, `require_observed_id`; each family composes its own parent/item predicate from them | Family record projection |
| Family dispatch from `PullRequestFact` | `crates/resourcefs-sources/src/github/facts.rs` | Four dispatch arms + guard extension | New responsibility bodies |
| Catalog grammar text | `crates/resourcefs-sources/src/github/mod.rs` | One grammar string | New handler logic |

## Module shape

T2 records a module-shape change (new families, a moved responsibility, a new dispatch module). Procedure from `references/module-shape.md`:

### Inventory (baseline production lines, `wc -l`)

| Module | Lines | Current responsibility | Change |
|---|---:|---|---|
| `core/src/reference.rs` | 1,961 | all `pr://`/`issue://` grammar | deepen (declarations only) |
| `core/src/reference/pull.rs` | 51 | PR fact sum + cursor gate | deepen |
| `sources/src/github/facts.rs` | 627 | envelope, presence primitives, serialization, family dispatch, read orchestration | deepen (protected parent) |
| `sources/src/github/facts/collection.rs` | 767 | bounded traversal/admission/coverage (conversation only) | deepen |
| `sources/src/github/facts/comment.rs` | 298 | conversation decode/validate/project **and** shared PR parent | deepen (loses parent ownership) |
| `sources/src/github/facts/identity.rs` | 459 | PR identity + supplied-link authority | deepen |
| `sources/src/github/facts/continuation.rs` | 253 | authenticated cursor | retain |
| `sources/src/github/mod.rs` | 1,170 | endpoints, human routes, catalog | deepen (grammar string only) |
| `sources/src/github/fetch.rs` | 628 | transport, confinement, budgets | retain |

### Seam alternatives (three materially different shapes)

1. **Generic `FactFamily` trait (GAT).** `collection.rs` becomes `read<F: FactFamily>`; each family implements `type Native`, `type Validated`, `type Record<'a>: Serialize`, `KIND`, `suffix`, `decode`, `validate`, `id`, `unavailable`, `project`. Maximum compile-time separation; cost is generic plumbing through `Records`/`CollectionBody`/`CandidatePage` plus `serde(bound = …)`/GAT friction, and a private seam with three monomorphizations that never vary at runtime.
2. **Concrete family dispatch module (selected).** `collection.rs` keeps one concrete state machine over `family::Record`/`family::Native` sums; `family.rs` owns the single exhaustive match per family; family modules own all behavior. No generics, no wrappers beyond the sums, one verification location per responsibility, and adding a family is a compile error until every match is handled.
3. **Per-family collection modules.** Copy the traversal/admission/coverage engine into `review_collection.rs` and `inline_collection.rs`. Rejected by the deletion test: deleting `collection.rs` would make ~600 lines of budget/coverage logic reappear in three callers, and the three copies would drift on exactly the failure/rollback semantics the ticket demands.

**Selected: 2.** Reason: it keeps one concrete private module for traversal (adapter test: no generic seam is justified for a set that varies only by exhaustive match), preserves the existing single verification location, and confines per-family knowledge to `family.rs` + one family module. Alternative 1 buys compile-time separation this private seam does not need; alternative 3 fails locality.

### Module ledger

| Module/path | Interface | Owns | Hides/reuses | Must not own | Adapters | Tests through | Change |
|---|---|---|---|---|---|---|---|
| `core/src/reference/pull.rs` | `PullRequestFact` variants; `parse_pull_request_reference` cursor gate | PR fact-family identity | `ProjectionSelector` parsing | provider facts | `N/A — no seam` | core grammar tests | deepen |
| `core/src/reference.rs` | `parse_pull_request_address`, `canonical_reference` | canonical `pr://` grammar | escaping/segment rules | provider behavior | `N/A` | core grammar tests | deepen |
| `sources/.../facts.rs` | `read_facts`/`facts_resource` dispatch | envelope, `Presence`, serialization, acquisition context | `CappedWriter`, `serialized_len` | family decoding | `N/A` | facts contract tests | deepen |
| `sources/.../facts/family.rs` | `Native`, `Record`, `RecordView`, `CollectionFamily::{from_fact, kind, suffix, decode}`, `validate_native`, `record_id`, `record_unavailable`, `project_record` | the set of discussion families and their dispatch | family types | traversal, HTTP | `N/A — private concrete dispatch` | collection/item contract tests | create |
| `sources/.../facts/parent.rs` | `ParentFacts`, `parent_facts`, `fetch_parent`, `validate_parent` | PR parent acquisition, validation policy + projection | `identity::validate` | family records | `N/A` | collection/item contract tests | create |
| `sources/.../facts/collection.rs` | `read(source, repo, number, CollectionFamily, cursor, ctx)` | bounded traversal, admission, coverage, continuation | budget/`fetch_controlled` | family decode/validate, parent fetch | `N/A` | collection contract tests | deepen |
| `sources/.../facts/continuation.rs` | unchanged authenticated cursor envelope | one cursor for every discussion collection | HMAC envelope, binding checks | family records | `N/A` | collection contract tests | retain (module doc names all discussion collections) |
| `sources/.../facts/comment.rs` | `NativeComment`, `validate_comment`, `read_comment_item`, `ValidatedComment` | conversation family | `Presence` | parent acquisition | `N/A` | contract tests | deepen |
| `sources/.../facts/review.rs` | `NativeReview`, `ValidatedReview`, `validate_review`, `read_review_item`, `ReviewRecord` | review-submission family | `Presence`, `parent` | traversal, parent fetch | `N/A` | contract tests | create |
| `sources/.../facts/inline.rs` | `NativeInlineComment`, `ValidatedInline`, `validate_inline`, `read_inline_item`, `InlineCommentRecord` | inline-comment family | `Presence`, `parent` | traversal, parent fetch | `N/A` | contract tests | create |
| `sources/.../facts/identity.rs` | `validate`, `expected_object_url`, `require_object_link`, `validate_optional_object_link`, `validate_optional_web_link`, `require_observed_id`, `WebFragment`, `WebObjectRoute`, `clean_authority` | PR identity + reusable supplied-link authority; conversation keeps its older `validate_comment_links` here (same cluster, pre-existing) | `api_route`/`web_route` | record projection, family endpoint knowledge | `N/A` | contract tests | deepen |
| `sources/src/github/mod.rs` | catalog grammar string | source metadata | — | facts behavior | `N/A` | catalog test | deepen |
| `sources/.../facts/pull.rs` | `NativePull`, `Branch`, `read` | singular PR native schema and PR facts | `identity::validate` | review/inline native schema | `N/A` | PR facts contract tests | retain |
| `resourcefs-mcp` read/recovery | existing `rfs_read` + artifacts | unchanged | — | — | `N/A` | stdio contracts | retain |

### Protected parents

| Protected parent | Baseline responsibilities | Allowed change | Forbidden change | Exit condition |
|---|---|---|---|---|
| `sources/.../facts.rs` | envelope, presence, serialization, dispatch, read orchestration | `mod` declarations; four dispatch arms; cursor/selector guard extension | any family decode/validate/project body | no `NativeReview`/`NativeInlineComment`/`validate_record` symbols |
| `sources/src/github/mod.rs` | endpoints, human routes, catalog | catalog grammar string | new handler or facts logic | production diff limited to the grammar line |
| `sources/.../facts/collection.rs` | traversal/admission/coverage | fact-parameterization and calls into `family`/`parent` | naming `comment::`/`review::`/`inline::`; owning family validation | zero family-module imports |

### Shape claim

Every ledger rule above becomes C29, discharged by `.<change-slug>/oracles/shape_fence.py`: required/forbidden paths, forbidden cross-module symbols, protected-parent production deltas, and required test locations. The delta baseline is this change's own base, discovered from the commit that first added `facts/family.rs` (falling back to `HEAD` before that commit exists), never a hard-coded branch: `origin/main` predates the stacked conversation-facts slice, so a default-branch diff would charge this change with inherited edits that are not its protected-parent growth. Named mutation: move `review::validate_record`'s body into `facts.rs` (or add `use super::review::NativeReview;` to `collection.rs`) → fence reports the claim ID and exact path/symbol; restoring returns green.

*Repository placement policy (2026-09-09):* the checked-in permanent gate `scripts/module_shape.py` + `scripts/module-ledger.json` gained a `review` stage that registers `facts/family.rs`, `facts/parent.rs`, `facts/review.rs` and `facts/inline.rs` as owners, moves the conversation record-protocol symbols to unique family names, and records the new `identity.rs` entry points. `scripts/module_shape_base.py` raises the `core/src/reference.rs` tripwire from 1,989 to 2,010 lines for the four added grammar arms. `python3 scripts/module_shape.py` reports `C02 PASS stage=review`.

*Bounded repair (2026-09-09):* `identity.rs` gained the private `clean_authority` rule (origin match plus no username/password) so every authority comparison rejects credential-bearing supplied URLs, including a recognized `html_url`. Approved behavior, ownership, interfaces and risk are unchanged; spec §6 and the conversation validator determine the outcome.

*Ledger corrections (2026-09-09, isolated design-conformance review):* the Placement and ledger rows above were rewritten to match the implemented interface names (`parent::fetch`/`validate`/`facts`, `CollectionFamily` methods, `collection::read(..., CollectionFamily, ...)`) and to state the link-authority split precisely: `identity.rs` owns reusable predicates and route recognition, each family composes its own parent/item predicate from them, and `parent::validate` owns the parent-validation policy for every discussion family. `facts/pull.rs` was missing and is now recorded as `retain`. The "endpoint mapping" prohibition was narrowed to collection endpoint suffixes: a family module owning its own singular endpoint is the existing `comment.rs` precedent and the behavior, ownership, and risk accepted at approval are unchanged.

## Claims

- C1 The four proposed `/facts` spellings are unclaimed today and adding them leaves the four human routes' meaning unchanged.
- C2 Each new spelling parses to its own `PullRequestFact` variant and round-trips to the same canonical string; zero/negative/overflow child ids and the `facts`-as-id alias are refused.
- C3 The `:cursor:` selector is accepted on exactly `Comments`, `Reviews`, `ReviewComments` and refused on `Pull`, `Comment`, `Review`, `ReviewComment`.
- C4 Every pre-existing `pr://` reference spelling keeps its parsed variant and canonical string.
- C5 A review record preserves native `id`, `nodeId`, parent PR identity/links, `body`, `author`, `state`, `commitSha`, `submittedAt` and original links, distinguishing omitted, null and present.
- C6 A review collection emits `kind: "github.review_submission_collection"`, parent observation, per-page `upstream`, honest `collection`, and an item read of the same id yields the same native facts as the list record.
- C7 An inline record preserves `id`, `nodeId`, parent, `reviewId`, `replyToId`, `body`, `author`, times, `path`, `diffHunk`, `commitSha`, `originalCommitSha`, `side`, `line`, `startSide`, `startLine`, `originalLine`, `originalStartLine`, `position`, `originalPosition`, `subjectType` and links, each as omitted, null or present.
- C8 Unknown or unrecognized native values in `side`, `state` and `subjectType` are emitted verbatim and never coerced to a known variant.
- C9 Current and original commit/line/position facts are emitted separately and unchanged; no line is recomputed from `diffHunk` and no anchor is retargeted to the current head.
- C10 File-level and null-line anchors keep `subjectType` and explicit nulls instead of a fabricated line.
- C11 Multiline anchors keep `startSide`/`startLine` alongside `side`/`line`.
- C12 `reviewId` and `replyToId` keep the supplied relationships, including null `reviewId` and omitted `replyToId` for a thread root.
- C13 A supplied parent link that does not name the addressed PR rejects the whole current component, including already-acquired pages.
- C14 An item read whose native id differs from the requested id rejects with an identity mismatch.
- C15 An inline item read through the repository-wide endpoint under a PR that does not own the comment rejects.
- C16 All three discussion collections run through the single `collection.rs` engine; no family owns traversal, admission or coverage.
- C17 A duplicate native id within a page or across pages, and a repeated pagination URL, produce unknown coverage with the matching inconsistency reason rather than a fabricated complete result.
- C18 Record admission is atomic against the 1,000-record ceiling: exactly 1,000 admits, 1,001 does not, and an oversized first page is a typed limit error.
- C19 Representation admission is atomic against the serialized ceiling: a candidate page that would exceed it is not partially admitted; the verified prefix is retained with a continuation to the rejected page.
- C20 A first-page failure with no usable record is a typed `ResourceError`; an ordinary later-page failure retains the verified prefix with honest incomplete coverage.
- C21 Cancellation and authority/identity violations reject the current component even after pages were acquired.
- C22 Continuations are opaque, credential-free, Path-Session-scoped and bound to source/resource/operand; wrong-family or wrong-origin continuation targets are refused before egress, and an unrepresentable next link yields unknown coverage, never a guessed cursor.
- C23 One attempt/deadline/accepted-body budget is shared across the parent lookup and every page of a read.
- C24 The four routes produce only authorized GET requests to the configured origin with zero remote mutations and zero requests to denied origins/repositories.
- C25 An oversized review/inline facts document survives MCP display limits: Recovery References reconstruct exactly the acquired bytes before parsing, and source continuation remains a separate acquisition.
- C26 The Version Tag hashes the emitted representation bytes and changes when only a supplied native fact changes, independent of Git object ids.
- C27 The catalog grammar advertises the four new routes without renaming existing ones.
- C28 Existing human `reviews`/`review-comments` projections and their sorted listing output are unchanged.
- C29 Production placement matches the approved module ledger, protected-parent deltas, and forbidden cross-family symbols.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Four proposed spellings unclaimed; human routes unchanged | S1, S3 | Parse the 12 references through the public parser: any proposed spelling that already parses, or any human route whose parsed variant/canonical string changed, falsifies | Pre-change parser census recorded in `falsifiers/grammar_probe.rs` output (four `PROPOSED-FREE`, four `HUMAN-OK`, three `EXISTING-OK`) | `N/A — census claim about the pre-change system` | `crates/resourcefs-core` grammar tests retain the four human routes and three existing facts routes | minutes | PASS — see run log |
| C2 | New spellings parse to distinct variants and round-trip; malformed ids refused | S1, S27 | `PathReference::parse` on each spelling yields the expected variant and `requested()`; `…/reviews/facts`, `…/reviews/0/facts`, `…/reviews/-1/facts`, `…/review-comments/99999999999999999999999/facts` are `invalid_reference` | Approved specification address table (spec §2) as an external expectation list; not production code | Map `"review-comments"` to `PullRequestResource::ReviewComments` facts in `canonical_reference` but parse it back as `Review`, or accept `"facts"` as a child id | `core` grammar test `review_and_inline_facts_routes_round_trip` | minutes | PENDING — checkpointed-build, slice 1 |
| C3 | Cursor accepted on the three collections, refused elsewhere | S2 | Parse `…/reviews/facts:cursor:<opaque>` (accepted) and the same selector on `facts`, `reviews/<id>/facts`, `review-comments/<id>/facts` (refused) | Specification §5 continuation rules; refusal categories compared against the pre-change census | Widen the gate to accept `Comments`, `Reviews`, `ReviewComments`, `Review` and `ReviewComment` | `core` grammar test `cursor_requires_a_discussion_collection` | minutes | PENDING — slice 1 |
| C4 | Pre-existing spellings unchanged | S3 | Re-run the C1 census plus the existing facts grammar tests | Recorded pre-change census output | Rewrite `[owner, repo, number, resource]` arms without preserving `"reviews"`/`"review-comments"` | Existing core reference tests + `github_*` grammar tests | minutes | PENDING — slice 1 |
| C5 | Review record preserves supplied identity/parent/nullable fields | S4, S28 | Feed controlled review pages (present/null/absent permutations, mismatched parent) and compare emitted JSON fields to the expected document; a fabricated, dropped or coerced field falsifies | Expected document transcribed from the recorded public observation `probe-result.json` (review 4988827796: `commit_id` present, `submitted_at` present, `pull_request_url` names PR 159232); production code never reads the probe output | Delete `commit_sha` from `ReviewRecord`, or emit `submitted_at` as `""` when absent | `crates/resourcefs-sources/tests/github_review_facts_contract.rs` | hours | PENDING — slice 2 |
| C6 | Review collection kind/envelope and item/list parity | S5 | Read the collection and each item from the same fixture; assert `kind`, `collection.state`, per-record facts and item/collection record equality | Expected `kind`/`state` from the approved schema table; parity compared against the fixture's own native rows via the item path | Emit `github.conversation_comment_collection` for reviews | Same contract test | hours | PENDING — slice 2 |
| C7 | Inline record preserves every supplied anchor field | S6, S28 | Controlled inline pages with each field present/null/absent; compare emitted JSON to the expected document | Expected document transcribed from `probe-result.json` inline rows (e.g. id 3826494362: `line: null`, `original_line: 2741`, `side: "RIGHT"`, `in_reply_to_id` absent) | Drop `original_start_line`, or serialize null as omitted (`skip_serializing_if` on a wrong field) | `crates/resourcefs-sources/tests/github_inline_facts_contract.rs` | hours | PENDING — slice 3 |
| C8 | Unknown native values preserved verbatim | S7 | Feed `side: "BOTH"`, `state: "FUTURE_STATE"`, `subject_type: "unknown"`; assert verbatim emission and no rejection | Recorded observation plus the spec's "unknown native values remain explicit" rule | `match side { "LEFT" => …, "RIGHT" => …, _ => "RIGHT" }` | Same contract test | minutes | PENDING — slice 3 |
| C9 | No retargeting or diff reconstruction | S8 | Feed an outdated record (current null/current != original); assert current and original fields unchanged and that `diffHunk` is echoed byte-for-byte | Recorded observation of an outdated row; `diffHunk` digest compared to the probe digest | `line: &self.0.original_line` in `project`, or compute `line` by counting `diffHunk` lines | Same contract test + `probe-result.json` digest fixture | hours | PENDING — slice 3 |
| C10 | File-level/null-line anchors preserved | S9 | Feed `subject_type: "file"`, `line: null`; assert `subjectType`, null `line`, and no substituted coordinate | Recorded native shape + spec inline row | Fall back to `original_line` when `line` is null | Same contract test | minutes | PENDING — slice 3 |
| C11 | Multiline anchors preserved | S10 | Feed `start_side`/`start_line` with `side`/`line`; assert all four present | Recorded native shape | Swap `startLine` and `line` in the projection | Same contract test | minutes | PENDING — slice 3 |
| C12 | Reply/review relationships preserved | S11 | Feed a thread root (no `in_reply_to_id`) and a reply; assert `replyToId` omitted vs present and `reviewId` null preserved | Recorded observation (3 of 6 rows carry `in_reply_to_id`) | Default `replyToId` to the review id when absent | Same contract test | minutes | PENDING — slice 3 |
| C13 | Wrong parent rejects the component including pages | S12 | Two-page collection whose second page names another PR in `pull_request_url`; assert `upstream_identity_mismatch` and no emitted document | Specification §5 override rule; loopback oracle records that no further page request occurs | Remove the `validate_review_links`/`validate_review_comment_links` call from the family validator | Contract test `wrong_parent_rejects_acquired_pages` | hours | PENDING — slice 4 |
| C14 | Item id mismatch rejects | S13 | Request review id A while the endpoint serves id B; assert identity mismatch | Existing `validate_comment_links` precedent for conversation comments | Delete the `review_id.is_some_and(...)` comparison | Contract test | minutes | PENDING — slice 4 |
| C15 | Inline item parent verification | S14 | Request `pr://o/r/7/review-comments/<id>/facts` where the repository-wide record belongs to PR 9; assert rejection | Specification §2 item route + existing human `review_comment` parent check | Trust the requested number without comparing `pull_request_url` | Contract test | hours | PENDING — slice 4 |
| C16 | One traversal engine for three families | S26 | `shape_fence.py` asserts `collection.rs` names no family module and that each family module lacks traversal symbols | Approved module ledger (this file) | Add `use super::review::NativeReview;` to `collection.rs` | `oracles/shape_fence.py` | minutes | PENDING — slice 4 |
| C17 | Duplicate/repeated pagination yields unknown coverage | S15 | Fixture with a repeated id and a repeated next URL; assert `collection.state: "unknown"` and the exact `inconsistency.reason` | Specification §5 inconsistency rule | Delete the `seen_ids`/`seen_urls` guard | Contract test | hours | PENDING — slice 4 |
| C18 | Atomic 1,000-record admission | S16, S29 | Fixture at exactly 1,000 (admitted, `complete`) and 1,001 (no partial page, honest limit facts); oversized first page is a typed limit error | Independent record count computed from the emitted JSON array length, not from production counters | Admit `page_records` before the ceiling check | Contract test `record_ceiling_is_atomic` | hours | PENDING — slice 5 |
| C19 | Atomic representation admission with prefix retention | S17 | Fixture whose last page crosses the serialized ceiling by one byte; assert the prefix is retained, the page is absent, and a continuation names it | Independent byte measurement of the emitted document (`content.len()`) vs the configured cap | Admit the page then truncate JSON, or drop records from a page | Contract test `representation_ceiling_retains_prefix` | hours | PENDING — slice 5 |
| C20 | First failure typed; later failure prefix retained | S18, S30 | Truncated/malformed first page → typed error; malformed second page → prefix with `incomplete` and no continuation; non-object body rejected | Specification §5 outcome rules | Return `Ok` with an empty collection on first-page failure | Contract test `first_failure_is_typed_later_failure_keeps_prefix` | hours | PENDING — slice 5 |
| C21 | Cancellation/authority violation rejects the component | S19 | Cancel after page 1; assert typed cancellation and no document. Unauthorized repository on page 2 → rejection | Specification §5 override rule | Treat cancellation like an ordinary later-page failure | Contract test | hours | PENDING — slice 5 |
| C22 | Continuation binding and refusal | S20 | Valid next link → cursor round-trips; unrepresentable link → unknown coverage; cursor replayed with a different resource/origin/family → refused before egress | Loopback oracle observes zero requests for refused cursors; specification §5 binding rules | Skip the resource/origin check in `continuation::decode` | Contract test `continuation_is_resource_and_family_bound` | hours | PENDING — slice 5 |
| C23 | Shared attempt/deadline budget | S21 | Loopback oracle counts parent + page requests and the elapsed window; assert ≤10 attempts and one deadline | Independent request log from the loopback server | Give each page a fresh budget | Contract test | hours | PENDING — slice 5 |
| C24 | Zero forbidden egress and remote writes | S22 | Loopback oracle on all four routes: only GETs to the configured origin; denied origin/repository and redirect targets see zero requests | Independent listener (different failure mechanism than the adapter) | Follow a `pull_request_url` to another origin for the parent lookup | Contract test `no_forbidden_egress_on_review_and_inline_routes` | hours | PENDING — slice 6 |
| C25 | MCP recovery reconstructs exact bytes | S23 | Real stdio read of an oversized review/inline document; follow Recovery References and parse only after reconstruction; assert byte equality and a separate source continuation | Existing MCP recovery contract precedent (bfwa `github_comment_facts_recover_without_reacquisition`) plus independent byte comparison | Emit a truncated document instead of an artifact continuation | `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs` rows | hours | PENDING — slice 6 |
| C26 | Version Tag hashes representation bytes | S24 | Two reads differing only in a supplied native fact produce different Version Tags; two identical reads produce the same tag | Version Tag recomputed independently over the emitted content bytes | Hash the Git commit id instead of the representation | Contract test | hours | PENDING — slice 6 |
| C27 | Catalog advertises the four routes | S25 | Catalog text contains the four spellings and still contains every existing one | Approved specification address table | Rename `reviews/facts` to `review/facts` in the grammar string | Catalog discovery test | minutes | PENDING — slice 6 |
| C28 | Human projections unchanged | S3 | Existing human `reviews`/`review-comments` listing and item tests re-run unchanged | Recorded pre-change test expectations | Reuse the facts projection in the human renderer | Existing human projection tests | minutes | PENDING — slice 6 |
| C29 | Placement matches the ledger | S26 | `shape_fence.py` checks required/forbidden paths, forbidden symbols, protected-parent deltas and required test locations | Approved module ledger (this file) | Move family validation into `facts.rs` | `oracles/shape_fence.py` | minutes | PENDING — slice 6 |

## Non-goals and future work

Permanent non-goals (rationale recorded; no tracker issue):

- No new public provider trait, generic fact framework, second HTTP client, or private inspector: the approved seam is the existing `SourceAdapter`/`SourceResource` (spec §1, §Testing Decisions).
- No anchor reconstruction, retargeting, diff-offset arithmetic or output-line substitution: native anchors are supplied facts; freshness and representability decisions stay in Cyril (spec §3, ticket placement).
- No provider-prose parsing or hidden-404-to-empty inference: `ResourceError` keeps stable categories and bounded details (spec §6).
- No issue-evidence family: `issue://` grammar is unchanged (spec §2).

Intended future work (verified tracker IDs):

- Checks and commit statuses: `rfs-nchb`.
- PR commit and native file listings: `rfs-iktl`.
- Exact commit/source facts: `rfs-jrz7`.
- Comparisons (direct-tree and merge-base): `rfs-e5cv`, `rfs-3r0s`.
- Corporate `ghe.com`, native Windows, credential attachment and Cyril retention/UI acceptance: separate gate recorded in spec §7 and ticket acceptance; not ResourceFS-acceptable by this work.

## Falsifier run log

- Cheapest falsifier: C1.
- Command: `cargo build -p resourcefs-core && rustc --edition 2024 -L dependency=target/debug/deps --extern resourcefs_core=target/debug/libresourcefs_core.rlib .rfs-r31i/falsifiers/grammar_probe.rs -o /tmp/grammar_probe && /tmp/grammar_probe` (2026-09-09).
- Result: `PASS` — `PROPOSED-FREE` for all four proposed spellings (`InvalidReference`), `HUMAN-OK` for all four human routes with unchanged canonical strings, `EXISTING-OK` for `…/facts`, `…/comments/facts`, `…/comments/<id>/facts`.
- Learning: the first run used `target/debug/libresourcefs_core.rlib` from 2026-08-27, predating the conversation-facts slice, and reported the *existing* facts routes as broken. The instrument was wrong, not the design: rebuild the crate before linking a scratch probe against it. No production code or recorded provider output was edited.

## Approval

Approved 2026-09-09. Requester's verbatim approval: "Approve as designed" — selected for the question "Approve the rfs-r31i design as written so implementation can start (plan → checkpointed build)?", whose option text was: "Spellings /reviews/facts, /reviews/<id>/facts, /review-comments/facts, /review-comments/<id>/facts; kinds github.review_submission and github.review_comment; enum-dispatch family seam with new facts/family.rs + facts/parent.rs + facts/review.rs + facts/inline.rs."

Risk acceptances approved: None.
