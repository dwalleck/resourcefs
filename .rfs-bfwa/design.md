# Design: rfs-bfwa resumable conversation-comment facts

Date: 2026-09-08. Route: Empirical (`route.md`). Evidence: `evidence.md` (P1/P2 fresh PASS, P3 retained PASS). No production code has been written.

## Route and inputs

- Route: Empirical. `route.md` T1 names two unverified provider premises; `evidence.md` discharges both (P1 comment record shape, P2 pagination/coverage) and retains P3 (MCP recovery) with its applicability reason.
- Behavior set: `route.md` **T4 observable behavior contract**, triples 1–11. `spec.md` is `N/A — behavior fully explicit`; the ticket and the user-approved contract specification (`resourcefs-wt-github-resource-contract/docs/github-resource-contract-spec.md`, stories 23–24 and 28–38, decisions 2/3/5/6) are the behavior source. That specification file is not present in this worktree; its content was read from the sibling documentation worktree.
- Empirical premises and oracle: `evidence.md` Premise checklist and Oracle sections (Python `urllib` probe versus Go `gh api` + `jq`, compared in `probe-oracle-comparison.json`, `all_match: true`).
- Base: `feat/rfs-0n97` = `084d136`; existing owners reused are `github/facts.rs`, `github/facts/identity.rs`, `github/fetch.rs`, core `reference.rs`, core `acquisition.rs`, MCP `server/read.rs`/`render/error.rs`.

## Input shapes

| # | Shape | Status |
|---|---|---|
| S1 | Route `pr://o/r/N/comments/facts` (canonical, non-canonical `N`, zero `N`, issue number, `comments/new/facts`, `comments/facts/1`) | C1 |
| S2 | Route `pr://o/r/N/comments/<id>/facts` (`<id>` canonical, zero, non-numeric, Unicode, wrong parent) | C1, C11, C12 |
| S3 | Selector: absent, `:cursor:<valid>`, `:cursor:<malformed/foreign/expired>`, `:raw`, `:1-2`, `:page:2`, `:offset:1`, two selectors | C2, C3, C4 |
| S4 | Collection size 0, 1, 100, 101, 600+400, 600+401, 1001 first page | C5, C6, C7 |
| S5 | Page `Link`: absent, `rel="next"` present, `rel="next"`+`last`, malformed, off-origin, repeated target, non-numeric page, empty page with `Link` but no `next` | C4, C8, C9 |
| S6 | Record presence matrix: `body` present/null/absent, `user` present/null/absent, `node_id` absent, extra unknown keys, duplicate keys, `id` 0/negative/above 2^53, non-object record, empty/Unicode body | C10 |
| S7 | HTTP: 200, 304 revalidation, 401, 403 rate, 404, 429, 5xx, truncated body, invalid UTF-8, non-JSON | C7, C13, C14 |
| S8 | Budgets: attempts 1..10, deadline expiry, response 8 MiB exact/+1, accepted total 16 MiB exact/+1, representation 16 MiB exact/+1, zero/above-hard caller controls | C5, C14, C15 |
| S9 | Cancellation: mid-page, after admitted pages, at final acceptance; authority/identity violation | C7 |
| S10 | Session: same session, different session, principal change, expired session | C4 |
| S11 | Concurrency: cache generation change mid-read; two reads sharing the cache | C14 |
| S12 | Deployment: public GitHub, matched ghe pair, custom base with/without `webOrigin` | C13 |
| S13 | MCP display limits small/large, numbered, artifact recovery chain | C16 |
| S14 | Mutation attempts (field and creation targets) on the new routes | C13 |
| S15 | Source-neutral record ceiling and provider-cap absence | C8, C19 |

## Placement

| Capability | Owner | New seam | Forbidden |
|---|---|---|---|
| Route grammar + canonical spelling for the two facts routes | core `reference.rs` (deepen) | `PullRequestResource::Facts(PullRequestFact)`; `PullRequestFact::{Pull, Comments, Comment(ConversationCommentId)}` | No provider vocabulary, no HTTP/serde, no new address family |
| `:cursor:` selector split and route-scoped acceptance for `pr://` | core `reference.rs` (deepen) | private `parse_pull_request_reference(input) -> Result<(PullRequestAddress, Option<ProjectionSelector>), ResourceError>`, mirroring `parse_jira_reference` | No provider vocabulary; no cursor acceptance on any route but the comments collection |
| Source-neutral collection record ceiling | core `lib.rs` constant `MAX_COLLECTION_RECORDS` (deepen) | public constant | No provider names, no per-family policy |
| One controlled page fetch with confined next-link resolution | sources `github/fetch.rs` (deepen) | `pub(super) async fn facts_page(...) -> Result<PageResponse, ResourceError>`; `PageResponse { body, observation, revalidation, cache_generation, next: Option<Url> }` | No fact schema, no decoding, no coverage policy |
| Facts dispatch and shared envelope | sources `github/facts.rs` (deepen, net shrink) | `read_facts(reference, fact, operation, acquisition)`; generic `Facts<'a, B: Serialize>` with `#[serde(flatten)] body: B` | No per-family decoding, no HTTP call sites, no collection loop |
| Singular PR facts read | sources `github/facts/pull.rs` (create, moved from `facts.rs`) | private `pub(super) async fn read(...)` owning fetch, identity validation, projection and publication through the shared `FactsRead` context | No second HTTP client, no cursor, no collection loop |
| Conversation-comment decoding, record projection and singular read | sources `github/facts/comment.rs` (create) | private `pub(super)` decoders/record plus the singular read and parent fetch used by the collection | No second HTTP client, no coverage policy, no cursor encoding |
| Bounded collection acquisition, atomic admission, coverage outcome | sources `github/facts/collection.rs` (create) | private `pub(super) async fn read(...)` owning the page loop, admission, coverage, continuation decision and final publication through the shared context | No HTTP stack of its own, no second cache, no provider JSON leakage to core/MCP, no generic continuation list |
| Opaque session-scoped continuation | sources `github/facts/continuation.rs` (create) | private `CursorOwner::{new, decode, continuation}` following the Jira `CursorOwner` shape | No credentials, no authorization grant, no network |
| Mutation refusal for the new routes | sources `github/mutation.rs` (deepen) | existing refusal arm gains `Facts(_)` | No new write behavior |
| Catalog grammar | sources `github/mod.rs` (deepen) | updated `catalog_entries` string | No new responsibility body |
| Docs | `DESIGN.md`, `docs/operating.md` | — | No schema/profile change (no new tool input) |

## Module shape

### Current cluster inventory

| Module | Production lines | Interface | Responsibility clusters | Change |
|---|---:|---|---|---|
| `crates/resourcefs-core/src/reference.rs` | 1946 | `PathReference`, `PullRequestAddress`, `PullRequestResource` | address parsing, canonical spelling, selector typing, per-family selector split | deepen |
| `crates/resourcefs-core/src/reference/pull.rs` | 0 | private pull-fact sum + cursor split | grammar child of `reference.rs` | create (technical correction, S1) |
| `crates/resourcefs-core/src/lib.rs` | 78 | crate exports and public constants | wiring | deepen (one constant) |
| `crates/resourcefs-sources/src/github/mod.rs` | 1136 | `GithubSource`, `SourceAdapter`, `SourceCatalogMetadata` | route dispatch, human rendering, parent/identity validation, catalog | deepen (dispatch arm + catalog string only) |
| `crates/resourcefs-sources/src/github/fetch.rs` | 565 | private fetch/cache/pagination | conditional fetch, cache, Link confinement | deepen (one page seam) |
| `crates/resourcefs-sources/src/github/facts.rs` | 741 | private facts entry | presence decoding primitives, envelope, PR projection, admission | split |
| `crates/resourcefs-sources/src/github/facts/identity.rs` | 250 | private `validate` | PR identity/link validation | retain |
| `crates/resourcefs-sources/src/github/mutation.rs` | 1319 | `GithubFieldTarget`, `GithubCreationTarget` | write targets, mutability | deepen (one arm) |
| `crates/resourcefs-mcp/src/server/read.rs`, `render/error.rs` | 145 / existing | `rfs_read` input and error output | protocol boundary | retain (no change) |

### Seam tests

1. **Deletion test** — deleting `facts/collection.rs` would force cache/conditional/Link/coverage/admission logic back into `facts.rs` and `mod.rs`; deleting `fetch.rs::facts_page` would force the collection to reimplement cache keying, conditional revalidation, generation checks and Link confinement. Both pass.
2. **Interface test** — callers and tests reach the collection only through `SourceAdapter::read` and the real MCP `rfs_read`; no test needs the private modules. Passes.
3. **Adapter test** — no new trait or generic seam is introduced. `facts_page` is a method on the existing concrete `GithubSource` fetch owner with one caller and is justified by locality (one transport/cache/Link owner), so the two-adapter rule does not apply.
4. **Locality test** — transport/cache in `fetch.rs`; decoding/projection in `facts/*`; grammar in core; protocol in MCP. One owner per responsibility. Passes.

### Three alternatives for the disputed decisions

**A. How fact families are grouped in the grammar.**
1. *Minimal interface:* flat variants `Facts`, `CommentsFacts`, `CommentFacts(id)`. Smallest diff; the enum grows one arm per family per successor ticket (six more routes already planned).
2. *Common caller:* nested `Facts(PullRequestFact)` with `PullRequestFact::{Pull, Comments, Comment(id)}`. One arm per family, existing `Facts` unit becomes a tuple, exhaustive matches force the three production call sites; successors add one enum arm inside one owner.
3. *Extension flexibility:* a generic `Facts(FactRoute)` trait object. Rejected: an unapproved generic seam and provider-shaped polymorphism.
Selected: **2** — the fact families share one dispatch and one envelope, and successors (`rfs-r31i`, `rfs-iktb`) add families, so a nested sum keeps the growth inside one owner instead of widening the route enum.

**B. Where collection orchestration lives.**
1. Inside `facts.rs`: no new file, but 741/750 physical lines plus envelope generics plus admission exceeds the approved tripwire and concentrates three responsibilities.
2. New private child `facts/collection.rs` under the existing facts owner, with `facts/pull.rs` extracted so `facts.rs` keeps only shared primitives, envelope and dispatch.
3. New top-level `github/collection.rs` beside `facts.rs`: splits the facts owner across two parents and duplicates access to the private `Presence`/decoder primitives.
Selected: **2** — one parent (`facts.rs`), private children, shared primitives stay with their owner, and `facts.rs` nets smaller.

**C. How the envelope is shared across families.**
1. Duplicate the envelope field list per family: three copies drift independently; violates the specification's "reuse the first slice's reviewed envelope".
2. One generic `Facts<'a, B: Serialize>` with `#[serde(flatten)] body: B`; family bodies own their `request`/`observed`/`upstream`/`data`/`collection` sections.
3. One non-generic `Facts` with an untagged `data` enum and every family section as `Option`.
Selected: **2** — one envelope owner, one serialization order, family sections isolated; the flatten ordering behaviour is itself a falsifiable claim (C17).

### Approved module ledger

| Module/path | Interface | Owns | Hides/reuses | Must not own | Adapters | Tests through | Change |
|---|---|---|---|---|---|---|---|
| `crates/resourcefs-core/src/reference.rs` | `PathReference`, `PullRequestResource`, `PullRequestFact` | route grammar, canonical spelling, typed child ids, per-family `:cursor:` split | existing address/selector types | provider behavior, HTTP | N/A | `github_reference_contract` | deepen |
| `crates/resourcefs-core/src/reference/pull.rs` | private `PullRequestFact`, `parse_pull_request_reference` | pull-fact sum, `:cursor:` split and route scope | `super` grammar helpers | provider behavior, HTTP, serde | N/A | `github_reference_contract` | create |
| `crates/resourcefs-core/src/resource.rs` | public constants | `MAX_COLLECTION_RECORDS` | — | provider names, family policy | N/A | core contracts | deepen |
| `crates/resourcefs-core/src/error.rs` | public error/limit vocabulary | `AcquisitionLimitKind::CollectionRecords` | existing details | provider names | N/A | core contracts | deepen |
| `crates/resourcefs-core/src/lib.rs` | re-exports | the new constant and variant | — | provider names | N/A | core contracts | retain |
| `crates/resourcefs-sources/src/github/mod.rs` | `GithubSource` | dispatch, catalog string, parent/identity validation | all private owners | new fact bodies | N/A | adapter contracts | deepen |
| `crates/resourcefs-sources/src/github/fetch.rs` | private fetch/cache/pagination | conditional fetch, cache, Link confinement, one page seam | `BoundedHttpResponse`, `HttpReadBudget` | fact schema, coverage policy | N/A | HTTP/GitHub contracts | deepen |
| `crates/resourcefs-sources/src/github/facts/identity.rs` | private `ValidatedIdentity`, `validate` | PR/repository/link/SHA identity validation | `pull` DTOs and `super` primitives | HTTP, cache, clock, serialization | N/A | facts contracts | retain |
| `crates/resourcefs-sources/src/github/facts.rs` | private facts entry | shared presence/decoder primitives, envelope, dispatch, capped serialization | serde, `BoundedRead`, `HttpReadBudget` | per-family bodies, HTTP clients | N/A | facts contracts | split |
| `crates/resourcefs-sources/src/github/facts/pull.rs` | private `read` | singular PR fetch/validate/project/publish | `super` primitives, `fetch`/budget | second HTTP client, cursor, collection loop | N/A | facts contracts | create |
| `crates/resourcefs-sources/src/github/facts/comment.rs` | private decoders/record/`read_item`/`fetch_parent` | comment decoding, record projection, singular read | `super` primitives, `fetch`/budget | second HTTP client, coverage, cursor | N/A | facts contracts | create |
| `crates/resourcefs-sources/src/github/facts/collection.rs` | private `read` | page loop, atomic admission, coverage outcome, continuation issuance, publication | `fetch::facts_page`, `HttpReadBudget`, shared envelope | HTTP stack of its own, second cache, cursor encoding | N/A | facts contracts | create |
| `crates/resourcefs-sources/src/github/facts/continuation.rs` | private | cursor envelope encode/decode/validate | `SourceCursor`, base64url, sha2 | network, authorization grants, credentials | N/A | facts contracts | create |
| `crates/resourcefs-sources/src/github/mutation.rs` | `GithubFieldTarget` | write-target parsing | existing refusal | new write behavior | N/A | mutation contract | deepen |

### Protected parents

| Protected parent | Baseline responsibilities | Allowed change | Forbidden change | Exit condition |
|---|---|---|---|---|
| `crates/resourcefs-sources/src/github/mod.rs` | route dispatch, human rendering, validation helpers, catalog | one `Facts(fact)` dispatch arm, catalog string, no new helper body beyond wiring | new fact projection/admission/decoding bodies | production lines ≤ 1136 and no new responsibility declaration (the implemented dispatch arm and catalog string net to the baseline count) |
| `crates/resourcefs-mcp/src/server.rs` | tool registration/dispatch | none | any change | byte-identical to base |
| `crates/resourcefs-core/src/reference.rs` | grammar | two parse arms, one canonical arm, the `Facts(PullRequestFact)` variant, `mod pull;` and the `PULL_REQUEST_PREFIX` branch delegating to the child | provider or transport vocabulary | production lines ≤ the inherited 1989 ceiling, and the shape fence approves exactly `parse`, `canonical_reference` and `parse_pull_request_address` body changes |

### Shape fence

`scripts/module_shape_bfwa.py` (standalone, private, non-production) imports the inherited `scripts/module_shape.py` (which in turn inherits `scripts/module_shape_base.py`) and calls its `check()` with a merged ledger `scripts/module-ledger-bfwa.json` = the rfs-0n97 ledger plus the new owners, a `collection` stage, `reference.rs` body-change approvals (`parse`, `parse_pull_request_reference`, `canonical_reference`, `parse_pull_request_address`), and the new `MAX_COLLECTION_RECORDS` constant. `scripts/ci-gates.py` invokes the successor instead of the rfs-0n97 script. It reports claim IDs and exact paths/symbols and discovers the default branch through Git, never a hard-coded branch name.

### Placement revision (S2–S4, 2026-09-08)

The isolated design-conformance review reconstructed the production map before reading this document and reported five ownership deviations from the original ledger. They are resolved by revising the ledger to the implemented placement rather than by moving the code, because the implemented shape is the deeper one:

- Each fact family owns its own end-to-end read (`pull::read`, `comment::read_item`, `collection::read`) and shares acquisition through one `FactsRead` context, `fetch::facts_page` and one `HttpReadBudget`. The alternative — `facts.rs` owning every HTTP call site and the family modules being pure projections — would put three fetch/validate/publish sequences and their error handling in one orchestrator and push `facts.rs` toward its growth tripwire.
- `facts.rs` still owns the shared envelope, presence primitives, the acquisition constructor, sanitized serialization and dispatch; the family modules own only their own family's fetch, validation and projection.
- The collection publishes its own document (rather than returning a `CollectionOutcome` the parent serializes) for the same reason: its admission accounting and continuation fallback need the final serialization, which lives behind `finish_facts`.
- The private dependency graph inside `facts/` is recorded: `identity` validates `pull::NativePull` while `pull` calls `identity::validate` (reciprocal); `comment` consumes `pull` DTOs, `identity` and the shared primitives; `collection` consumes `comment`, `identity`, `continuation` and the shared primitives. These are sibling-private dependencies inside one parent that keep one identity validator and one decoder owner instead of duplicating either; nothing outside `facts/` sees these types.
- Ledger accounting corrections: `MAX_COLLECTION_RECORDS` is defined in `core/resource.rs` (re-exported by `lib.rs`); `core/error.rs` owns `AcquisitionLimitKind::CollectionRecords`; the protected-parent exit condition is `≤ 1136` production lines because the dispatch arm and catalog string net to the baseline.
- The reviewer's remaining notes are not defects: no pass-through module, no provider payload in core/MCP, no protected parent responsibility body, and the pre-existing `facts_tests.rs` private reach-through is unchanged from the base.

Duplicate acquisition construction was removed after the review: `collection.rs` now calls the single `acquisition_at` constructor owned by `facts.rs`.

## Claims

- **C1** `pr://o/r/N/comments/facts` and `pr://o/r/N/comments/<id>/facts` parse to `PullRequestResource::Facts(PullRequestFact::Comments | Comment(id))` and canonicalize to exactly those spellings; every previously accepted and previously refused `pr://` spelling keeps its verdict except `comments/facts`, which becomes a route instead of a malformed child id.
- **C2** The `pr://` parser splits a `:cursor:` selector before address parsing, mirroring the Jira pattern, and accepts a cursor only on the comments collection facts route; every other `pr://` route keeps its current verdict and every other selector on any facts route is refused with `unsupported_projection`.
- **C3** The continuation encoding is canonical unpadded base64url that `SourceCursor` and `ProjectionSelector` accept, carrying the provider's next link plus canonical-resource, API-origin and Path-Session bindings, and it stays below the 64 KiB reference ceiling for every link this endpoint can emit; padded or otherwise non-canonical encodings are rejected.
- **C4** Continuation validation happens before any upstream request; a cursor that is malformed, names another resource, another API origin, another Path Session, or an off-authority link acquires no data and fails with a typed error.
- **C5** A page is admitted only when its records and the resulting final serialized representation — including envelope, coverage, failure and continuation facts — fit `MAX_COLLECTION_RECORDS` (1,000) and the representation ceiling; 600+400 records are accepted, 600 followed by 401 retains the first page, and a 1,001-record first page is `limit_exceeded`.
- **C6** A successfully read collection with no records is `complete` with count zero; a denied, hidden or not-found collection is a typed error and is never reported as an empty complete collection.
- **C7** An ordinary later transport/malformed/deadline/limit failure retains every verified earlier page with `incomplete` coverage and bounded failure facts; a first failure with no usable data is a typed error; cancellation or an authority/requested-identity violation rejects every page of that read.
- **C8** Coverage is reported with `state` ∈ {`complete`, `incomplete`, `unknown`} plus `acceptedCount`, and with `localLimit`/`failure`/`continuation`/`inconsistency` only when each is true; `providerCap` and `reportedTotal` are absent for this family because the provider supplies neither, and no local limit is ever presented as a provider cap.
- **C9** A next target already followed in the same read, or one equal to the page just fetched, stops traversal with `state: unknown` and an `inconsistency` fact rather than looping, claiming completeness, or claiming a snapshot.
- **C10** Each record preserves native kind, decimal-string `id`, `nodeId`, verified parent PR identity and links, nullable `body`/`author` distinct from absent, supplied `createdAt`/`updatedAt` and original links; unknown extra native keys are ignored and unknown native enum values are preserved.
- **C11** A record whose `issue_url` names another repository or pull request, or a collection addressed under a number that is not a pull request, rejects the entire component before any record is published.
- **C12** The singular comment read verifies the parent PR, returns the shared envelope with singular comment facts and no `collection` object, and refuses every selector.
- **C13** Every facts route is read-only (zero remote writes, mutation refused with zero egress), accepts `acquisition` controls, and continues to reject those controls on non-facts resources; the new routes appear in the source catalog grammar.
- **C14** The parent lookup, every page fetch, every retry and every revalidation charge one `HttpReadBudget` (≤10 physical attempts, ≤16 MiB accepted bodies, one ≤30 s deadline); exhaustion produces a typed limit/deadline outcome that preserves already admitted pages.
- **C15** The final document, including outcome and continuation overhead, is at most the representation ceiling; admission reserves that overhead before extending the record set, and serialization is capped and never truncated or lossy.
- **C16** A collection document larger than the inline display limit is reconstructed byte-exactly through MCP artifact recovery before parsing, with exactly one upstream acquisition for the page; the source continuation is surfaced separately and acquires new upstream data.
- **C17** The shared envelope is one generic `Facts<B>` with a flattened family body that serializes the shared fields in the documented order and preserves `Presence` omitted/null/present distinctions.
- **C18** New production ownership is exactly the approved ledger: private `facts/{pull,comment,collection,continuation}.rs` children, `facts.rs` net smaller, `fetch.rs` one page seam, no new trait, no second HTTP client, no provider type in core or MCP.
- **C19** Core stays source-neutral: the record ceiling is a plain `usize` constant, and the existing architecture dependency fence still passes.
- **C20** Ignored read-only `live_*` rows exercise the collection and single-comment reads through the real adapter and the real `rfs serve` stdio path, gated on `RFS_LIVE=1` and `GITHUB_TOKEN`, and skip cleanly when absent.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | New route grammar and canonical spellings | S1, S2 | Parse/canonicalize corpus: new spellings round-trip; `comments/facts/1`, `comments/new/facts`, `comments/0/facts`, `comments/<id>/facts` with non-canonical id refused; all pre-existing rows unchanged | Hand-written expected spelling/verdict table independent of the parser | Map `"comments","facts"` to `Comment` (number parse) in `parse_pull_request_address`; or drop the six-segment arm | `github_reference_contract::conversation_comment_facts_routes_and_cursor_scope` | minutes | PENDING — checkpointed-build, per-slice gate |
| C2 | `:cursor:` split and route-scoped acceptance | S3 | Parse corpus: `.../comments/facts:cursor:<valid>` accepted; `:cursor:` on every other `pr://` route, and `:raw`/`:1-2`/`:page:2`/`:offset:1` on facts, refused; `:page:`/line selectors on facts keep their current refusal verdict | Hand-written verdict table plus the falsifier's positive controls proving `pr://` selector support exists today | Delete the `:cursor:` split so a canonical cursor is parsed as an address segment; or accept a cursor on the PR facts route | `github_reference_contract::conversation_comment_facts_routes_and_cursor_scope` | minutes | PENDING — checkpointed-build, per-slice gate |
| C3 | Cursor encoding is canonical and bounded | S3, S5 | Build the envelope for the observed native next link and a synthetic 2 KiB link; assert canonical unpadded base64url accepted by `SourceCursor`/`ProjectionSelector` and total length ≤ 65536; assert a padded encoding is rejected | Independent Python base64url/length arithmetic and canonical-alphabet check against the rules read from `source_page.rs` | Replace base64url-no-pad with padded base64 in the encoder | `github_facts_contract::continuation_encoding_is_canonical_and_bounded` | seconds | PASS — see Falsifier run log |
| C4 | Continuation validated before egress | S3, S10 | TLS fixture with a request counter: cursor from session A used by session B, cursor for another resource, cursor with another API origin, cursor with an off-authority link → typed failure with zero requests | Independent listener request log | Skip the session-hash comparison in `continuation.rs` | `github_facts_contract::continuation_is_session_and_authority_bound` | minutes | PENDING — checkpointed-build, per-slice gate |
| C5 | Atomic page admission | S4, S8 | Fixture pages 600+400 (accepted), 600 then 401 (first page only + continuation), 1,001 first page (`limit_exceeded`), representation-boundary page (not admitted) | Independent record and byte counting over the fixture | Extend the record set before measuring the page, or check size only after building the whole document | `github_facts_contract::collection_admits_whole_pages_atomically` | minutes | PENDING — checkpointed-build, per-slice gate |
| C6 | Empty complete versus inaccessible | S4, S7 | Empty page with no `rel="next"` → complete/0; 404/403 collection → typed error, no empty success | Independent status/link table | Return complete-empty on 404 | `github_facts_contract::empty_complete_is_not_inaccessible` | minutes | PENDING — checkpointed-build, per-slice gate |
| C7 | Partial retention and rejection precedence | S4, S7, S9 | Page 1 ok, page 2 transport/malformed/deadline/limit failure → page 1 retained + `incomplete`; first-page failure → typed error; cancellation and parent mismatch after page 1 → no pages | Independent fixture sequencing and page-presence assertions | Drop earlier pages on later failure; or retain pages after cancellation | `github_facts_contract::partial_retention_and_rejection_precedence` | minutes | PENDING — checkpointed-build, per-slice gate |
| C8 | Coverage vocabulary and no fabricated caps | S4, S5, S15 | Assert `state`/`acceptedCount` and that `providerCap`/`reportedTotal` are absent; local stop reports `localLimit`, never `providerCap`; complete only after a page with no `next` | Independent expected-coverage table | Emit `providerCap: 1000` from the local ceiling; or mark complete after a local stop | `github_facts_contract::collection_coverage_vocabulary_is_honest` | minutes | PENDING — checkpointed-build, per-slice gate |
| C9 | Repeated/stalled pagination is explicit | S5 | Fixture whose next link repeats the current page → one stop, `state: unknown`, `inconsistency` present, bounded request count | Independent request-count and outcome table | Follow the repeated target until the attempt budget is exhausted | `github_facts_contract::repeated_pagination_is_explicit` | minutes | PENDING — checkpointed-build, per-slice gate |
| C10 | Record presence and identity fidelity | S6 | Presence matrix: `body` null vs absent, `user` null vs absent, missing `nodeId`, id above 2^53, unknown keys, duplicate keys, non-object record, Unicode body | Independent JSON shape expectations from `evidence.md` P1 plus the fixture | Decode `body` as `Option<String>` and serialize `null` for both null and absent; emit ids as JSON numbers | `github_facts_contract::comment_records_preserve_native_presence` | minutes | PENDING — checkpointed-build, per-slice gate |
| C11 | Wrong parent rejects the component | S1, S2 | Comment whose `issue_url` names another PR/repo, and a collection under an issue number → typed error before any record | Independent parent-URL table | Skip `validate_parent` for collection records | `github_facts_contract::wrong_parent_rejects_whole_component` | minutes | PENDING — checkpointed-build, per-slice gate |
| C12 | Singular comment read shape | S2, S3 | Singular read returns envelope + data, no `collection`; cursor and other selectors refused | Independent expected-JSON shape | Serialize an empty `collection` object for singular reads | `github_facts_contract::single_comment_facts_shape` | minutes | PENDING — checkpointed-build, per-slice gate |
| C13 | Read-only, catalog, controls | S7, S12, S14 | Mutation matrix on both routes → refused with zero requests; acquisition accepted on facts, refused elsewhere; catalog string contains both spellings | Independent egress counter and catalog assertion | Add `Facts(_)` to `GithubFieldTarget::parse` as a mutable target | `github_mutation_contract::facts_routes_are_read_only_with_zero_egress`, `stdio_mcp_contract::catalog_advertises_comment_facts` | minutes | PENDING — checkpointed-build, per-slice gate |
| C14 | One shared budget across parent and pages | S8, S11 | Fixture counting physical requests: parent + 10 pages with `maxAttempts: 5` stops at 5; two 8 MiB pages + one byte fails accepted-total; expired deadline stops with `deadline_exceeded` | Independent request/byte/timestamp log | Construct a fresh `HttpReadBudget` per page | `github_facts_contract::one_budget_covers_parent_and_pages` | minutes | PENDING — checkpointed-build, per-slice gate |
| C15 | Representation ceiling with reserved overhead | S8 | Exact-cap success and cap-minus-one refusal for the final document including continuation overhead; oversized page not admitted | Independent byte length of the serialized document | Measure only record bytes and omit envelope/outcome overhead from admission | `github_facts_contract::representation_ceiling_includes_outcome_overhead` | minutes | PENDING — checkpointed-build, per-slice gate |
| C16 | MCP recovery of an oversized collection | S13 | Real stdio: small collection inline; oversized collection recovered across artifact pages, reconstructed bytes equal acquired JSON, exactly one upstream request, `sourceContinuationReference` present when a next page exists | Independent byte hash of the acquired document and the process request log | Re-acquire on recovery; or drop the source continuation when the page overflows | `stdio_mcp_contract::github_comment_facts_recover_without_reacquisition` | minutes | PENDING — checkpointed-build, per-slice gate |
| C17 | Shared envelope serialization order and presence | S6 | Serialize each family body and assert the exact top-level key order and omitted/null/present handling | Independent expected key sequence | Replace `Presence` with `Option` for `body` so omitted and null collapse | `github_facts_contract::envelope_serializes_shared_fields_in_order` | minutes | PENDING — checkpointed-build, per-slice gate |
| C18 | Approved module placement | S15 | `python scripts/module_shape_bfwa.py` accepts the tree; unlisted file, new parent body, widened `facts.rs`, or a new `trait` fails with its own path/symbol | Independent Git baseline/diff/symbol census | Add a serializer body to `github/mod.rs`; add an unlisted `crates/*/src` Rust file; add a `trait` to `facts/collection.rs` | `scripts/module_shape_bfwa.py` | minutes | PENDING — checkpointed-build, per-slice gate |
| C19 | Core stays source-neutral | S15 | `cargo test -p resourcefs-mcp --test architecture_contract enforces_dependency_direction -- --exact` plus core source scan | Independent Cargo/dependency census | Import `serde_json` or a provider type into core | `architecture_contract::enforces_dependency_direction` | minutes | PENDING — checkpointed-build, per-slice gate |
| C20 | Live read-only proof | S1, S2 | `RFS_LIVE=1 GITHUB_TOKEN=… cargo test -p resourcefs-sources --test github_live_smoke -- --ignored` and the stdio live row; without the gate both skip cleanly | Independent `gh api` observation of the same public collection | N/A — no fence mutation for a live row; absence of the gate is a skip, never a pass | `github_live_smoke::live_github_conversation_comment_facts_hold_up`, `stdio_live_smoke::live_stdio_github_comment_facts_match_native_observation` | hours (network) | PENDING — checkpointed-build, live phase |

## Non-goals and future work

Permanent non-goals (rationale recorded, no tracker issue):

- Reviews, review submissions and inline review anchors — distinct families with their own anchors, owned by `rfs-r31i`.
- Checks/statuses (`rfs-nchb`), PR commits/files (`rfs-iktl`), comparisons (`rfs-e5cv`, `rfs-3r0s`), exact commit/source (`rfs-jrz7`).
- A generic continuation list, a public provider trait, a second HTTP client, a Cyril capture coordinator, or whole-PR capture orchestration.
- Issue-comment facts under `issue://`: the approved specification deliberately adds no issue-evidence family.
- Atomic snapshot guarantees across pages or components; the continuation is best-effort traversal.
- Provider-native rename claims, alternate-endpoint exhaustive fallback, or invented continuations at a provider cap.

Intended future work: none beyond the verified successor tickets already named above; this design adds no new deferral.

## Falsifier run log

Cheapest falsifier (C3), run 2026-09-08 before approval:

```
cargo build --release   # /tmp/rfs-bfwa-falsifier, path dep on the real resourcefs-core
python3 .rfs-bfwa/falsifier_cursor.py
```

Result: `PASS` — `.rfs-bfwa/falsifier-cursor-result.json`, `all_pass: true`. Probe: a throwaway crate at `/tmp/rfs-bfwa-falsifier` depending on this worktree's `resourcefs-core` (no production edit). Oracle: independent Python base64url canonicality/length arithmetic. Five rows agreed:

| Case | Oracle canonical | Length ≤ 64 KiB | `PathReference::parse` today |
|---|---|---|---|
| observed native next link (386-byte cursor) | yes | yes | `InvalidReference` |
| synthetic 2 KiB next link (3,146-byte cursor) | yes | yes | `InvalidReference` |
| padded-base64 mutation | no | yes | `InvalidReference` |
| positive control `:page:2` | n/a | n/a | accepted (`selector_kind: page`) |
| positive control `:2-4` | n/a | n/a | accepted (`selector_kind: lines`) |

**Technical placement correction (S1, 2026-09-08).** The build found the inherited h212 placement census caps `crates/resourcefs-core/src/reference.rs` at 1989 production lines; the new grammar pushed it to 2004 and the fence rejected both the overrun and a new parent declaration. The approved owner and interface are unchanged, so this is a technical correction determined by an existing obligation: `PullRequestFact` and `parse_pull_request_reference` moved into a private `crates/resourcefs-core/src/reference/pull.rs` child, mirroring the existing `reference/jira.rs` and `reference/source_page.rs` children, and `reference.rs` re-exports `PullRequestFact` from the same public path. `reference.rs` is now 1964 lines, the child is 51, ownership and the public interface are identical, and the successor fence passes at stage `grammar`.

**Design correction from this run (first attempt falsified the design).** The original C3 asserted that `PathReference::parse` already accepts a canonical cursor on a `pr://` facts reference. It does not: `projection_candidate_split` only recognizes `:raw`, `:page:` and trailing line ranges, and `pr://` has no scheme-specific split, so `pr://…/facts:cursor:<x>` is parsed as an address segment and fails with `InvalidReference`. Jira's `parse_jira_reference` is the existing pattern that solves exactly this. The design now adds `parse_pull_request_reference` (C2) as a required core change, narrows C3 to the encoding/canonicality/ceiling claim, and records the `reference.rs` body-change approval in the protected-parent table. The positive controls prove selector support already exists on this family, so the gap is isolated to the missing `:cursor:` split and not a parser quirk. Re-run after the correction: `PASS`.

## Approval

Requester approval (verbatim): "Approve and implement"
Date: 2026-09-08
Approved risk acceptances: None — every claim carries a deterministic regression fence and a named mutation; no `N/A — approved risk` row exists.

Placement revision approval (verbatim): "Approve revised placement"
Date: 2026-09-08

## Approved PR10 review repair amendment

Requester approval (verbatim): "Okay, lets address all the issues following your recommendations"

This approves the per-finding assessment and remediation recommendations recorded in
`review-decisions.md`, not the original review's refuted claims. Main owns integration.
The existing C1–C20 obligations remain except for the explicit corrections below.
No new risk waiver, publication authorization, or unrelated adapter redesign is implied.

- C3/C4: replace editable URL envelopes with authenticated cursors using an independent,
  private random key owned by a session-bound GithubSource and shared by its clones.
  The public artifact SessionToken is not signing material. Bind source, API base,
  canonical resource and native target; verify before parent acquisition and retain
  confinement. Use the already-resolved aws-lc-rs HMAC primitive rather than bespoke
  cryptography. Cursor format cutover rejects the previous unsigned format.
- C5/C15: one actual final-context pretty serializer governs whole-page admission and
  publication. Include actual provenance, revalidation, availability and continuation
  metadata. A verified but unadmitted page retains its own target; a fetched malformed
  page remains nonresumable. Numeric observed sizes must be measured, not estimated.
  Final-outcome growth may roll back whole verified pages to a nonempty prefix;
  recompute retained provenance/availability and resume at the first removed page.
  Malformed/inconsistent traversal, authority, generation and expired acceptance
  cannot be converted into a new recovery cursor by rollback.
- C7/C14: preserve the original logical deadline through final acceptance. Actual
  expiry rejects publication; an early acquisition stop may retain a prefix only while
  final acceptance still succeeds. No fresh deadline or cancellation bypass. Parent
  and child requests continue to consume separate attempts in one shared ledger.
- C8: collection coverage explicitly identifies initial versus continuation traversal;
  complete describes that segment, not a snapshot or proof of a full conversation on
  resumed reads. Preserve the canonical collection identity. Collection schema minor
  version becomes 1 for the explicit scope and lossless typed failure details.
- C10/C11/C12: requested comment identity, recognizable link authority/identity and
  per-read duplicate IDs are verified before publication. Empty actor fields remain
  native facts. Authority/requested-identity contradictions reject the entire read;
  ordinary malformed later records retain only the previously verified prefix.
- C14: one parent-response generation baseline covers every participating response and
  final serialization. Invalidation rejects the assembled read rather than blessing a
  prefix with stale parent/record evidence.
- C18: existing facts identity, projection, collection, continuation and fetch owners
  remain. Private helper extraction may separate collection outcome/serialization
  responsibilities under facts; no second HTTP client, core serde dependency, generic
  cursor framework, or responsibility body in github/mod.rs. Its source key field,
  constructor wiring and catalog correction are permitted. Common HTTP URL validation
  owns rejection of implicit URL credentials. Permanent scripts own the placement gate,
  with one ledger retaining old and new checks; frozen ticket evidence is not executable
  gate infrastructure.

Repair falsifiers are consumer-visible wrong-ID/off-origin/duplicate-page refusals;
cursor target tampering and URL userinfo with zero egress; exact-cap nested documents
with long metadata; local-stop continuation resuming the unadmitted page; malformed
versus wrong-parent retention precedence; generation invalidation between acquisitions
and before publication; scoped resumed coverage; byte-exact artifact recovery without
source acquisition; and strict fixture query matching. Existing release budgets remain
unchanged. Main must establish new/changed regression red and restored-green evidence,
run the complete gate and real credential-gated live paths, and retain an independent
assembled design/security review before declaring these repairs complete.
