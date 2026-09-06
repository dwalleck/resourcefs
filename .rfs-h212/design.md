# Design: rfs-h212

Revision 1 — approved by the requester on 2026-09-06 UTC.

## Route and inputs

- Route: Empirical, from `route.md`. The complete seven given/when/then behaviors are `route.md#givenwhenthen-behavior-contract`; all seven are retained.
- `spec.md`: N/A — explicit behavior is fixed by rfs-h212 and rfs-lcuh/rfs-qsyk/rfs-lbps. The requester approved doing this prerequisite first, not this design.
- `evidence.md`: P1–P3 PASS; N1/N2 classify specified behavior and existing substrate. `probe_openapi.py`, `probe_live.py`, and `probe_pages.py` are evidence instruments, not implementation seeds.
- Evidence constrains this design: project search rejects numeric-ID ordering; actual project status metadata may be missing; enhanced search defaults to ID-only fields; `id > 0` is not complete enumeration; `project IS NOT EMPTY` is documented and tenant-proven; project offsets and enhanced tokens have different terminal contracts.
- Existing seams: `SourceAdapter`, `DiscoveryAdapter`, `SourceResource::with_continuation`, `SearchSourceResult::with_source_continuation`, and ADR-0006. Full issue/Field decoding and rendering remain intact.
- Code inventory found a necessary addition to the existing transport: `BoundedRead::fetch` currently gives each fetch a fresh retry allowance. rfs-lbps instead requires one retry for the entire Atlassian logical operation. The new collection budget counts physical attempts, including retries and parent reads; it is not a ten-page loop mislabeled as ten requests.
- This is additive behavior with a responsibility-preserving extraction. No identity, authority, ordering, permission, deadline, no-stale, or atomic-result invariant is removed.

### Observable decisions

1. Add `jira://<site>/projects`, `/projects/<project-id>`, `/project-keys/<project-key>`, `/issues`, and `/projects/<project-id>/issues`. The direct issue/Field grammar does not change. Bare product/site catalog publication belongs to rfs-ue44.
2. A direct project Resource shows its canonical identity, compact project metadata, and canonical project-issues navigation. It is not an invented project Field Map or a dump of every native field.
3. Fixed collections use the endpoints' normal visible, live-project defaults. This design does not force archived/trash search filters or claim tenant completeness. Direct ID reads retain native access semantics. This default-scope choice is included in the approval request.
4. Project requests start at 100 records; an explicitly returned lower effective maximum lowers subsequent requests in that logical read. Enhanced issue requests use 100 and explicitly select `key,summary,status,project`; fewer returned rows alone do not prove a new maximum.
5. Fixed issue queries are `project IS NOT EMPTY ORDER BY key ASC` for the site and `project = <validated-project-id> ORDER BY key ASC` for a project. These are adapter-owned predicates, not caller-authored JQL. A project collection verifies its stable parent with a direct project read first; a missing/hidden parent therefore produces the existing direct-read error rather than a fabricated empty collection.
6. Sort each completed bounded logical page by canonical decimal ID, comparing digit count then bytes without allocation or changing existing public ID `Ord` semantics. This does not claim globally numeric ordering across separate native cursor pages; native project search cannot supply that guarantee. Do not scan an entire tenant to manufacture it.
7. Stop before another native fetch at 1,000 records, ten physical attempts, or authoritative termination. Parent lookup and a retry consume the same ten-attempt budget. Thus a project-issues page can stop below 1,000 records. A failed fetch remains an error, not a successful budget stop.
8. Aliases never become cache, search-record, or continuation identity. GET responses use the existing conditional Path Session cache; alias lookup is uncached. Atomicity means no partial outward Resource/search result, not rollback of independently retained HTTP cache entries.

The live harness may lower native page size, logical record count and request count through a `test-support`-only helper; it cannot raise production ceilings or be selected through CLI/profile/environment configuration. A one-record logical limit and two-attempt limit allow a project-parent lookup plus one native issue page, exposing real typed continuations with small fixtures. Normal construction keeps the fixed production limits.

## Input shapes

Each row names distinct reachable shapes; malformed values are not normalized into valid ones.

| Shape | Inputs and cases | Coverage |
|---|---|---|
| S1 | Existing Issue aggregate/Fields/Field and issue-key alias; new Project, project-key alias, Projects, Issues, ProjectIssues | C2, C3, C12 |
| S2 | Valid configured Site ID; missing mount; degraded mount; same local Site ID rebound to a different allowed origin | C2, C8, C14 |
| S3 | Stable ID: 1, 9/10, values beyond u64, 64 digits; empty, zero, leading zero, negative, non-digit, 65 digits | C2, C5 |
| S4 | Key/segment: safe ASCII and Unicode, spaces, reserved punctuation requiring canonical encoding; empty, dot segments, `new`, separators, controls, excessive length, invalid UTF-8/noncanonical escapes | C2, C3 |
| S5 | No selector; raw/line selector; project offset; issue cursor; zero/overflow/noncanonical offset; malformed/padded/noncanonical base64url; wrong family; stacked source/text selectors | C2, C8, C12, C14 |
| S6 | Row arrays: empty, one, distinct multiple, duplicate stable IDs within/across native pages; reordered keys and IDs | C4, C5, C10 |
| S7 | Required project id/key/name/self and issue id/key/self/fields/summary/project identity: present correctly typed versus absent, null, wrong type, invalid identity | C4, C7 |
| S8 | Optional project type/archived/deleted and issue status metadata: every combination of absent/null/present; explicit false/true, empty versus nonempty text; wrong types; status object with or without name | C4, C5 |
| S9 | Native JSON: valid unknown fields, duplicate keys at any depth, invalid/trailing JSON, excessive depth/body size | C4, C10 |
| S10 | Returned self/parent/page URL: correct origin/family/identity versus foreign site, credentials, query/fragment on object self, wrong object kind/ID, redirect, changed page query or parent | C4, C6, C7, C8, C14 |
| S11 | Project page: correct/current startAt, positive effective max, last/nonlast; missing/wrong markers, smaller max, absent/present nextPage, advancing/repeated/backward offsets, changing/absent totals, empty nonterminal page | C6, C9 |
| S12 | Enhanced page: empty/populated issues, absent/present opaque token, absent/true/false isLast, conflicting terminal/token state, empty/repeated token, oversized token | C7, C8, C10 |
| S13 | Logical limits: below/exact/over 100 native rows and 1,000 logical records; fewer than/exact ten attempts; parent verification; one retry versus another retry trigger; cancellation and deadline exhaustion | C9, C10, C14 |
| S14 | Cache: no validator, valid ETag/304, absent/corrupt/unusable validator, generation change, alias reuse, later-page failure after earlier cache retention | C3, C10, C11 |
| S15 | Text: empty/nonempty metadata, Unicode/newlines/Markdown punctuation; fit/spill/unretainable output; selected read; regex/PCRE2 match/no match; next page and recovery together | C5, C10, C12, C13 |
| S16 | HTTP success, 401/403/rate-signaled 403, 404/410, 413, 429/503 with valid/invalid retry guidance, other 5xx, transport failure; secret canaries | C9, C14 |
| S17 | Production defaults versus test-only lower page limits; absent live gate/credentials versus reader-enabled fixture run | C15, C16 |

Native caller-authored queries, mutations, project Field Maps, archive-filter expansion, and product/site catalog/profile publication are classified in Non-goals below. They are not silently accepted input shapes.

## Placement

### Interfaces and ownership

| Capability | Owner and interface | New seam / choice | Forbidden |
|---|---|---|---|
| Jira identifiers/address grammar | Private `core/src/reference/jira.rs`, composed by existing `PathReference`; existing crate exports remain the public interface | Existing parsing seam. Move existing Jira responsibility, add Project ID/key and address variants there. Keep shared Site ID/URI helpers in the parent. | Native JSON/HTTP, tenant lookup, and transport cursor interpretation in core; duplicate Jira parsers; dormant query variants. |
| Source selector syntax | `core/src/reference/source_page.rs` validates `SourceOffset` and canonical encoded `SourceCursor`; `ProjectionSelector` gains distinct Offset/Cursor kinds | Existing selector interface. Dedicated accessors; legacy `page_offset()` remains only legacy Page. Jira contextual splitting admits the new kinds only for matching collection families. | Global marker changes that reinterpret workspace/HTTPS paths; treating source offsets as artifact byte offsets; silently applying them as text selection. |
| Logical read budget | `sources/src/http/read.rs` owns existing `BoundedRead` retry/deadline implementation plus `HttpReadBudget` and `fetch_with_budget` | Choose a caller-owned mutable budget passed to the existing bounded transport over a source-local retry loop. One implementation enforces retries/deadline/attempt charging; old non-budgeted callers retain their behavior. | Atlassian retry sleeps, direct HTTP-client construction, bypassing egress checks, or resetting the collection retry permit per fetch. |
| Jira GET/cache/status work | `sources/src/atlassian/jira/transport.rs`; move existing helpers, add private `JiraRead` context carrying the optional collection budget | Existing fetch/cache seam, not a new trait. Direct reads and collection reads actually differ in budget scope; both reuse the same GET/cache machinery. | Alias cache keys, stale success, independent retry policy, or cache-transaction machinery invented for outward page atomicity. |
| Fixed browsing | `sources/src/atlassian/jira/browse.rs`; one read entry behind the existing adapter dispatch | Existing SourceAdapter seam. Private endpoint-specific loops share the context and consume typed rows. | Native page loops in core, compiled dispatch, the Jira facade, or the renderer; generic pagination that guesses one provider-wide terminal rule. |
| Continuation ownership | `sources/src/atlassian/jira/cursor.rs`; encode/decode typed issue continuation for an expected collection and mount | Choose a self-contained strict envelope over a Path Session cursor map. Owner is the canonical unselected collection reference; an origin fingerprint binds the configured mount; token stays opaque. No raw native next URL is followed. | Session-hidden state, alias owners, re-parsing caller text after the typed owner is established, or claiming the envelope is a cryptographic authorization capability. |
| Compact native decoding | `sources/src/atlassian/wire/collections.rs`; project/issue row and endpoint page decoders | Existing strict-wire seam. Reuse StrictParser and shared self-URL authority checks; keep direct IssueBean requirements intact. | Relaxing full IssueBean decoding to accept compact rows; defaulting missing metadata; native DTOs outside the provider module. |
| Compact Markdown | `sources/src/atlassian/render/collections.rs`; project/issue collection and project Resource rendering | Existing rendering seam. Reuse quoting/Markdown escaping; assembler supplies already ordered rows. | HTTP/continuation validation, native JSON parsing, sorting inside the reusable renderer, or duplicating ADF rendering. |
| Read/search integration | Existing Jira adapter and core canonical search identity helper | Existing interfaces; preserve source-page-bearing search references and ADR-0006. | New tools, a JQL meaning for regex patterns, native wire logic in core/MCP, or alias SearchRecord identities. |

`JiraAddress` remains a typed sum with separate Project/ProjectKeyAlias/Projects/Issues/ProjectIssues variants. Page state is a source selector, not an added field on every Resource or a raw string passed through the engine. Public constructors still reparse through the family-aware PathReference interface.

### Page and cursor rules

- A project page requires typed `values`, matching `startAt`, a usable positive `maxResults`, and an authoritative `isLast`. Nonterminal pages require a validated next link; terminal/next-link contradictions fail. The next link supplies the advancing offset after origin, endpoint and fixed-query validation. Totals do not determine completion. Rebuild requests from validated state rather than following raw URLs.
- Enhanced search uses token absence as the documented terminal signal. Optional `isLast`, when supplied, must agree. Missing token with `isLast:false`, token with `isLast:true`, empty token, or repeated token fails atomically. The provider token is not parsed as an offset or ID.
- Every issue row validates its own stable identity/self URL and its embedded project identity/self URL. Project-scoped rows must belong to the requested stable parent. Duplicate stable IDs in a logical fixed collection are rejected rather than silently deduplicated or nondeterministically rendered.
- Continuation decoding checks strict envelope shape, canonical owner, endpoint family, Site ID, origin fingerprint, and parent before network activity. A structurally valid envelope is not a MAC: a caller can construct inputs, but cannot redirect credentials away from the configured origin or bypass returned authority checks.
- Source offsets are positive canonical u64 offsets; the first native offset zero is represented by the bare collection reference. IDs remain bounded decimal strings, never u64.
- The existing single-projection grammar remains: a source-page selector cannot be stacked with a text selector. Bare collection pages support ordinary raw/line reads; source selectors are consumed before generic text projection, which rejects unconsumed source selectors explicitly.

### Protected parents and mechanical placement fence

Comparison origin was discovered from the checkout's upstream before branching: `origin/main` at `0b81d31`. It is a captured baseline, not an assumed branch name in the oracle. Budgets count whole files, including unchanged test/observation regions. Production-region baselines are 2,064 lines for reference.rs (25 inline-test lines), 1,620 for http/mod.rs (139 inline-test lines), 511 for wire.rs and 403 for render.rs (the remainder is feature-gated observation support). Jira and discovery have no inline test region. This accounting clarification does not change any approved budget.

| Parent | Baseline lines | Approved final tripwire | Allowed changes |
|---|---:|---:|---|
| `core/src/reference.rs` | 2,089 | at most 1,989 | Net shrink of at least 100; generic selector additions, imports/exports and dispatch only; Jira grammar/model moves to child. |
| `core/src/discovery.rs` | 1,342 | at most 1,367 | Canonical source-page search identity wiring only. |
| `sources/src/atlassian/jira.rs` | 467 | at most 317 | Net shrink of at least 150; direct issue behavior, adapter dispatch/search and catalog grammar wiring; transport moves out. |
| `sources/src/atlassian/wire.rs` | 571 | at most 601 | Child declaration/shared strict authority helper visibility; no compact row/page decoder bodies. |
| `sources/src/atlassian/render.rs` | 480 | at most 488 | Child declaration/shared helper visibility; no collection-render loops. |
| `sources/src/http/mod.rs` | 1,759 | at most 1,679 | Net shrink of at least 80; read module export and construction wiring; retry implementation moves out. |

New production-file tripwires: Jira reference child 650; source-page syntax 160; HTTP read 350; Jira transport 400; browse 600; cursor 250; compact wire 600; compact render 350. These stop a slice for placement review; they are not permission to split code cosmetically. Exceeding an approved placement/budget requires revising and re-approving this design.

The issue-local `oracles/module_shape.py` checks responsibility placement, protected-parent growth, dependency direction and new-file tripwires against the captured upstream baseline. Run it at checkpoints/final integration and wire its stable placement checks into CI. Behavioral tests live in focused integration targets or owning child modules, not enlarged parent files. Existing dependency/egress contracts remain active.

## Claims

- C1: Core remains source-neutral and all network egress remains in the existing bounded HTTP substrate.
- C2: Jira project/collection references parse once into validating domain types without changing other schemes' selector interpretation.
- C3: Project-key reads publish stable project identity and never acquire alias cache/search/continuation authority.
- C4: Compact decoding rejects malformed or foreign authority while preserving legitimate optional metadata.
- C5: A fixed logical page has numeric stable-ID order and faithful compact metadata, independent of native key order.
- C6: Project paging follows validated response-driven offsets and never invents terminal state from totals or requested size.
- C7: Fixed issue paging preserves opaque continuation semantics and validates every issue's project membership.
- C8: Replayed or malformed continuation ownership fails before egress, including rebinding a local Site ID to another origin.
- C9: One collection operation makes at most ten physical attempts and one retry while retaining the 100/1,000 and original-deadline bounds.
- C10: Corrupt, failed, or unrepresentable logical pages produce no partial outward success or discarded continuation.
- C11: New canonical GET representations obey the existing no-stale, generation-safe ETag cache contract.
- C12: Read projection and regex/PCRE2 search operate on the selected rendered collection page with canonical source-page identities.
- C13: Artifact recovery retains the entire rendered logical page while the next native page remains independently reachable.
- C14: New routes retain typed safe errors, read-only behavior, cancellation, origin confinement and diagnostic redaction.
- C15: An ignored reader-only live row exercises real project and issue continuation, selected metadata and parent identity without asserting tenant counts.
- C16: New responsibilities stay in the approved deep modules rather than growing protected facades or creating bypass paths.

## Falsification

All PENDING rows are discharged by checkpointed-build at the implementing slice assigned in plan.md. No risk waiver is requested.

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Source-neutral core/one egress | Placement | Run owning architecture contracts; forbidden dependencies or egress locations are red. | Cargo metadata and independent source ownership scan | Add serde_json dependency/import to core; separately construct a reqwest client in Jira browse. The corresponding ownership fence must fail. | Existing `enforces_dependency_direction` and `single_http_client_module` | Existing cached local checks | PASS — both checks ran before approval. |
| C2 | Typed contextual references | S1–S5 | Round-trip valid project/collection references; reject invalid IDs/encoding/selector families; legacy path interpretations must stay unchanged. | Explicit grammar examples and invalid-form table, not production parsing | Admit Offset for Issue in `reference/jira.rs`; wrong-family case must become red. | `jira_reference_contract::browse_reference_identity_and_selector_families` | Local Rust target | PENDING — checkpointed-build/core slice |
| C3 | Alias authority | S1, S4, S14 | Change a key's returned ID between reads; both results must use returned stable identity with no conditional alias request. | TLS server's ID transition and observed request headers | Route ProjectKeyAlias through cached fetch in `jira/browse.rs`; alias transition/header assertion must fail. | `jira_project_browse_contract::project_aliases_never_own_cached_identity` | Local TLS target | PENDING — checkpointed-build/project slice |
| C4 | Strict compact authority | S6–S10 | Valid controls pass; missing/foreign/wrong-type/duplicate-key pages fail without a Resource; optional presence matrix remains representable. | Hand-authored native JSON and expected authority/error categories | Skip project self-URL check in `wire/collections.rs`; foreign-parent case must fail its expected error. | `jira_browse_wire_contract::compact_authority_and_presence_matrix` | Local Rust target | PENDING — checkpointed-build/wire slice |
| C5 | Numeric order/fidelity | S3, S6, S8, S15 | Project and issue IDs 10,2,large-64-digit must render numerically; absent versus explicit empty/false metadata stays distinct. | Explicit numeric order and paired native metadata inputs | Replace digit-count comparison with byte-only comparison in `jira/browse.rs`; 2/10 order assertion must fail. | `numeric_order_and_optional_metadata_survive_rendering` in project and issue browse contract targets | Local TLS targets | PENDING — checkpointed-build/browse slices |
| C6 | Native offset semantics | S10–S11 | Traverse clamped, empty-nonterminal and changing-total pages; wrong/nonadvancing links fail; exact terminal page has no invented continuation. | Server-authored next offsets and captured requests | Stop on `rows.len() < requested` in `jira/browse.rs`; empty nonterminal fixture must lose expected later row. | `jira_project_browse_contract::offset_pages_follow_authoritative_state` | Local TLS target | PENDING — checkpointed-build/project slice |
| C7 | Opaque issue pages/parent | S7, S10, S12 | Follow a nonnumeric token, observe exact next token sent, and reject a row from another project; missing parent fails direct lookup. | Native token literal, server request log, separately defined project IDs | Remove project-ID equality check in `jira/browse.rs`; cross-parent row must be wrongly accepted and turn fence red. | `jira_issue_browse_contract::opaque_pages_preserve_parent_membership` | Local TLS target | PENDING — checkpointed-build/issue slice |
| C8 | Cursor ownership | S2, S5, S10, S12 | Replay a cursor across site/origin/parent/family; expect invalid_reference and zero destination requests. | Separate server counters and requested canonical owners | Omit origin-fingerprint comparison in `jira/cursor.rs`; same-Site-ID/different-origin replay must turn red. | `jira_issue_browse_contract::cursor_replay_is_rejected_before_egress` | Local TLS target | PENDING — checkpointed-build/cursor slice |
| C9 | Operation-wide bounds | S11–S13, S16 | Exercise exact 1,000, small native max, parent lookup, a retry then another retry trigger; observe at most ten attempts and one retry. | TLS attempt ledger plus independent record totals | Recreate HttpReadBudget per fetch in `jira/transport.rs`; second retry or eleventh-attempt assertion must fail. | `jira_browse_budget_contract::logical_budget_counts_attempts_and_one_retry` | Local TLS target | PENDING — checkpointed-build/transport slice |
| C10 | Atomic failure/losslessness | S6, S9, S12–S15 | A later corrupt page, duplicate ID, over-limit body/rows, or unencodable continuation must return the stable error, never successful partial output. | Fixture failure position and explicit hard ceilings | Return accumulated rows after a later fetch error in `jira/browse.rs`; expected error assertion must fail. | `jira_browse_budget_contract::logical_page_failure_never_becomes_partial_success` | Local TLS target | PENDING — checkpointed-build/browse slice |
| C11 | Revalidated cache | S14 | Canonical project/page GET revalidates; valid 304 preserves bytes, invalid generation/304 cannot succeed, alias lookup stays unconditional. | Server validators/headers and Path Session generation | Remove generation comparison in `jira/transport.rs`; invalidated-304 case must become red. | `jira_project_browse_contract::project_cache_revalidates_without_stale_success` | Local TLS target | PENDING — checkpointed-build/project slice |
| C12 | Selected-page search | S1, S5, S15 | Follow a source page and regex/PCRE2-search its metadata; match references retain that source page and no JQL is derived from the pattern. | Hand-placed unique metadata and captured fixed-query requests | Strip source selector in Jira search before read; second-page-only match must disappear. | `jira_browse_discovery_contract::search_keeps_selected_page_identity` | Local TLS/engine target | PENDING — checkpointed-build/integration slice |
| C13 | Dual continuation | S15 | Force a nonterminal logical page to spill; walk retained artifact bytes and separately follow source continuation to the next native page. | Pre-spill SourceResource bytes for recovery; server's next-page input for navigation | Omit `with_continuation` in browse finalization; next-native-page navigation must fail. | `jira_browse_discovery_contract::artifact_recovery_and_source_pages_remain_independent` | Local TLS/engine target | PENDING — checkpointed-build/integration slice |
| C14 | Safe read-only errors | S2, S5, S10, S13, S16 | Invalid Jira selectors/glob/mutation issue no requests; status/deadline/cancel categories hold; error canaries never escape. | Typed status table and destination request counters | Include the native response body in `jira/transport.rs::classify_status` errors; the response-canary assertion must fail. | `jira_browse_budget_contract::browse_errors_are_typed_redacted_and_read_only` | Local TLS target | PENDING — checkpointed-build/integration slice |
| C15 | Real provider proof | S17 | Reader live row forces small pages and follows typed continuations to marker-owned fixtures; assert identities/shape, not totals. | Separate fixture setup identities and direct reads; published endpoint contract | Remove selected `fields` from enhanced GET in `jira/browse.rs`; native/default-ID-only control must fail. | `live_jira_browse` plus `jira_issue_browse_contract::selected_fields_enable_canonical_rows` | Disposable reader tenant and local TLS target | PENDING — checkpointed-build/live gate |
| C16 | Locality/placement | Placement, S17 | Run module_shape and existing dependency/egress checks; any forbidden owner, facade growth, or production fixture-limit ingress is red. | Captured upstream structure and approved owner/path ledger | Move `decode_project_page` into `core/src/reference.rs`; shape oracle must identify C16 and the wrong owner. | `.rfs-h212/oracles/module_shape.py` plus owning architecture contracts | Local structural checks | PENDING — checkpointed-build/every slice |

Metadata presence cases use behavior/metamorphic checks, not snapshots pinning incidental wording. Existing tests broken only because they pin newly valid grammar cases are migrated; wording-only assertions are removed rather than re-pinned. New tests defend the named plausible bugs, not wiring or mock echoes.

## Non-goals and future work

- Native caller-authored enhanced JQL Query Resources are rfs-nae2, which remains claimed and blocked by this issue. Do not add unused query variants, dormant routing, POST behavior, or a JQL parser here.
- Full Atlassian Server Profile and root/site catalog publication is rfs-ue44. Existing mounted Jira adapter catalog grammar must accurately describe implemented browsing; this does not advertise unsupported profile configuration.
- Jira comment/subtask/link and Confluence behavior are owned by the Atlassian destination rfs-0zbv, including its comment and Confluence browse work. They are not this prerequisite; no placeholders are introduced here.
- Permanent non-goals: Atlassian glob and mutation through these read-only Resources; cross-site fan-out; inventing project Fields; overriding native archive/trash defaults; globally re-sorting an entire tenant beyond the bounded page; treating cursors as authenticated capabilities; transactional rollback of successful HTTP cache entries; changing unrelated adapters' retry semantics.
- Delivery is split into independently green project-browse and issue-browse review increments. Each increment introduces only executable grammar and behavior. `plan.md` will apply the 4,000-line review-size gate including extraction churn, assign every pending falsifier, and keep each increment independently mergeable before rfs-nae2 starts.
- No risk acceptance without a fence is requested.

## Falsifier run log

Cheapest existing falsifier, C1, ran before design approval:

```text
env -u CARGO_TARGET_DIR cargo test -p resourcefs-mcp --test architecture_contract enforces_dependency_direction -- --exact
running 1 test
test enforces_dependency_direction ... ok
test result: ok. 1 passed; 0 failed; 6 filtered out
```

The dependency check took 6.69 seconds. The second C1 check also ran before approval:

```text
env -u CARGO_TARGET_DIR cargo test -p resourcefs-mcp --test architecture_contract single_http_client_module -- --exact
running 1 test
test single_http_client_module ... ok
test result: ok. 1 passed; 0 failed; 6 filtered out
```

The egress check took 0.37 seconds. These prove the current architecture baseline only; they do not claim new browsing code exists. Both rerun at implementation checkpoints, alongside the pending feature fences. No production code or feature tests were changed for this design.

## Approval

Requester approved revision 1 on 2026-09-06 UTC with the verbatim words: “Approve revision 1”.

Approved risk acceptances: None.
