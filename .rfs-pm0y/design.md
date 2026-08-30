# Design: rfs-pm0y

## Route and inputs

- Route: **Empirical**, from `.rfs-pm0y/route.md`.
- Behavior source: `.rfs-pm0y/route.md` T4; `spec.md` is `N/A — behavior was fully explicit in the ticket and parent decisions`.
- Complete behavior set: validated canonical Site Mount/issue-ID/issue-key/field-ID references; stable-ID and alias reads resolving one validated issue and publishing stable-ID identity; deterministic issue Aggregate and Field index ordered by field ID; exact canonical JSON or complete ADF Fields with decided media types and content-derived Version Tags; typed unsupported-ADF projection warnings; atomic malformed-authority failure; stable status/error/redaction/bounds/cancellation/retry/cache behavior; validation before egress.
- Empirical input: `.rfs-pm0y/evidence.md` and `.rfs-pm0y/probe_openapi.py`.
- Empirical premises: P1 ID/key lookup and no redirect; P2 optional `IssueBean` identity/fields/names/schema shapes; P3 complete ADF root grammar; P4 no promised issue-read ETag; P5 API-token Basic authentication as `base64(email:token)`. All are `PASS` against independent vendor-document oracles.
- Edge-case source: `rfs-pm0y`, parent `rfs-0zbv`, and closed decisions `rfs-lcuh`, `rfs-nrjp`, `rfs-lbps`; no separate interrogated spec applies.

## Input shapes

| ID | Production-reachable shape | Status |
|----|----------------------------|--------|
| I1 | Site ID: empty; 1 and 63 bytes; over 63; lowercase alphanumeric endpoints; internal hyphen; uppercase, Unicode, control, underscore, leading/trailing hyphen. | Covered by C1. |
| I2 | Jira issue ID: empty; zero; leading zero; one digit; long nonzero decimal at the ceiling; over ceiling; sign, whitespace, Unicode digit, alphabetic, encoded separator. | Covered by C1. |
| I3 | Issue-key alias and field ID: empty; ASCII; Unicode; embedded spaces; dot segments; reserved `new`; controls; literal/encoded `/` or `\\`; malformed/lowercase percent escapes; boundary and over-boundary values. | Covered by C1. |
| I4 | Address sum: stable issue Aggregate; issue-key alias; stable issue Field index; stable issue Field; supported aggregate selector; Field selector; unrelated ResourceAddress variant. | Covered by C1, C11, and C12. |
| I5 | Site mounts: empty, one, multiple distinct, duplicate Site ID, duplicate canonical origin, same host with different port/path, non-HTTPS/userinfo/query/fragment origin. | Covered by C2 and C15. |
| I6 | Issue response presence matrix: each of `id`, `key`, `self`, `fields`, `names`, `schema` absent/present/null/wrong type; empty/single/multi maps; duplicate JSON member names; extra top-level and metadata members. | Covered by C4. |
| I7 | Identity: stable-ID request match/mismatch; alias returning same/different ID; returned key changed by move/case; self URL correct, foreign origin, wrong API family, wrong ID, userinfo/query/fragment. | Covered by C3 and C4. |
| I8 | Field value: null, false/true, integer/float/negative zero, empty/non-empty string, empty/non-empty array, empty/non-empty object, reordered object, Unicode keys/values, nested duplicate member, unknown native members. | Covered by C5. |
| I9 | Field metadata: name empty/non-empty/Unicode; schema type absent/empty/non-empty; system/custom/items/configuration absent/present; metadata entry missing for a present field; extra metadata for an absent field. | Covered by C4, C7, and C14. |
| I10 | ADF: minimal empty doc; supported block/inline/mark nodes; nested supported content; well-formed unknown node/mark with and without children; known node with missing/wrong required member; doc with wrong/missing type/version/content; unknown members. | Covered by C6. |
| I11 | HTTP/cache: 200 with absent/usable/oversized/unreadable ETag; 304 with matching/missing/wrong-generation entry; changed ETag; response below/above body ceiling; transport failure; 401, ordinary/rate-signaled 403, 404/410, 413, 429, 5xx, unknown status; cancellation before send/during body/during retry wait. | Covered by C8 and C9. |
| I12 | Authentication: empty/valid account email; colon/control/over-ceiling email; empty/valid token; canary credentials in captured request, errors, Debug, logs, cache metadata, and rendered output. | Covered by C15. |
| I13 | Render set: zero, one, and several fields; field IDs in upstream/random order; equal display names; scalar, structured JSON, ADF, and warning-bearing values; aggregate and index selection. | Covered by C7, C11, and C14. |
| I14 | Version inputs: semantically identical JSON with different whitespace/object order; distinct null/empty/string/array/object/number values; identical/different ADF bytes after canonicalization; identical/different Aggregate Markdown. | Covered by C5, C6, and C7. |
| I15 | Product roots, project browse, issue collections, comments, JQL, Confluence, OAuth, mutations, and creation. | N/A — intended future work is already covered by `rfs-h212`, `rfs-9m70`, `rfs-nae2`, `rfs-wlft`, `rfs-flq5`, `rfs-pdp5`, and `rfs-ddpt`; this ticket must not advertise dormant paths. |

Core move: purely additive. It adds a new typed address family and Adapter; it removes no serialization, validation, ordering, uniqueness, or precondition invariant.

## Placement

### Typed Jira identity and references

- **Owner:** `resourcefs-core::reference`; Path References and exhaustive address sums already live there, and source code must receive parsed types rather than reparse strings.
- **New seam:** no new seam. Add `AtlassianSiteId`, `JiraIssueId`, `JiraIssueKey`, `JiraFieldId`, `JiraAddress`, and `JiraIssueResource` behind the existing `PathReference` interface. Only direct issue, alias, Field-index, and Field variants enter now; dormant browse/comment/query variants do not.
- **Forbidden:** no raw `String` identifiers after parsing; no Jira regex guesses for aliases/field IDs; no Jira parsing in MCP or the Source Adapter; no encoded/literal separator or noncanonical percent spelling.

### Complete UTF-8 media types

- **Owner:** `resourcefs-core::resource`; it owns the canonical `SourceResource` envelope and Version Tag association.
- **New seam:** two alternatives were considered. (A) add ADF-specific constructors to core, which leaks provider vocabulary inward; (B) add a source-neutral validated `Utf8ContentType` newtype plus complete/selected UTF-8 projection constructors. Choose **B**: it preserves the small `SourceResource` interface while allowing JSON/vendor-JSON Fields and Markdown selections without source-specific core concepts.
- **Forbidden:** MCP must not rewrite media types; sources must not directly construct `SourceResource` fields; selected JSON/ADF fragments must not be labelled as complete JSON, so Jira Fields reject selectors.

### API-token credential composition

- **Owner:** `resourcefs-sources::http::OriginCredential`; this is the only egress point that may expose a `Secret` and attach Authorization.
- **New seam:** two alternatives were considered. (A) compose `email:token` and base64 in the Atlassian module before calling the existing generic constructor; (B) add `OriginCredential::basic`, accepting a validated username and `&Secret`, composing and wrapping the encoded credential internally. Choose **B**: the secret-bearing allocation and `expose` remain at the audited credential seam, reusable by any Basic-auth source.
- **Forbidden:** no email/token/base64 string in Jira config Debug, errors, logs, cache keys, artifacts, or Resource content; no Authorization assembly in request call sites.

### Atlassian multi-site Adapter

- **Owner:** new `resourcefs-sources::atlassian` module, with private `wire`, `render`, and Jira request/cache implementation modules. Provider DTOs, serde, canonical JSON, ADF rendering, status mapping, and URL identity validation stay in the Source Adapter layer.
- **New seam:** two alternatives were considered. (A) add a Jira-only Adapter now and wrap it later; (B) introduce one `AtlassianSourceMount`/`AtlassianSource` holding validated site entries, with only Jira direct-read capability implemented. Choose **B** because the parent contract fixes one multi-site Atlassian Adapter, while its public interface remains the existing `SourceAdapter::read` plus a small mount/bind interface.
- **Forbidden:** no separate HTTP client, ACLI process, source-specific egress policy, provider DTO in core, raw cursor/query type, comment fetch, browse/JQL implementation, or credential-based identity/cache key.

### Native wire and canonical content

- **Owner:** private `resourcefs-sources::atlassian::wire`.
- **New seam:** no external seam. A private strict decoder returns one validated `JiraIssue` domain value containing typed identity and a sorted map of validated `JiraField` values. A private recursive canonical-JSON writer sorts object members deterministically and uses compact JSON scalar encoding; it rejects duplicate members at every nesting depth before a `Value` can erase ambiguity.
- **Forbidden:** no serde defaults for authority-bearing members; no `filter_map(Result::ok)`, `.ok()`, or lossy fallback; no raw upstream prose in errors; no reliance on `serde_json::Map` insertion order.

### Deterministic projections

- **Owner:** private `resourcefs-sources::atlassian::render`.
- **New seam:** no external seam. Pure functions render a validated issue Aggregate, Field index, and supported ADF subset; they return Markdown plus structured projection warnings before any `SourceResource` is built.
- **Forbidden:** no remote renderer, rendered HTML authority, Markdown-to-ADF conversion, comment inlining, partial output after malformed known ADF, or Field bytes with ResourceFS frontmatter.

### Shared HTTP, cache, retry, and compiled dispatch

- **Owner:** existing `HttpSubstrate`/`BoundedRead` and `PathSession` cache; `CompiledSources` owns exhaustive address dispatch.
- **New seam:** no transport/cache seam. The Atlassian Adapter constructs `HttpRequest::get`, uses `BoundedRead`, and stores exact successful body plus bounded ETag metadata under namespace `atlassian.jira.<site>` and an issue representation key. `CompiledSources` gains an optional Atlassian Adapter and routes `ResourceAddress::Jira`; all constructor call sites migrate atomically.
- **Forbidden:** no retry loop, deadline, resolver, redirect, cache, or cancellation implementation inside Atlassian; no cache entry without an echoable ETag; no 304 reuse without matching generation; no Jira catalog advertisement when no Atlassian Adapter is mounted.

## Claims

- **C1.** Jira direct-read Path References have one canonical typed spelling and reject every unsafe, ambiguous, over-ceiling, dormant, or noncanonical input before source dispatch.
- **C2.** A mounted Jira request resolves exactly one validated Site ID to one canonical HTTPS origin, and duplicate Site IDs or origins are unrepresentable in an `AtlassianSourceMount`.
- **C3.** Stable-ID and issue-key-alias reads call the documented direct endpoint, validate the returned stable identity, and emit only `jira://<site>/issues/<id>` as canonical identity.
- **C4.** Native decode accepts optional/nullable vendor schema shapes but atomically rejects every missing, null, wrong-type, duplicate, cross-origin, wrong-family, or identity-confused authority member needed by the requested Resource.
- **C5.** Every ordinary Field is exact compact canonical JSON with deterministic recursively sorted object members, JSON media type, no frontmatter, and a Version Tag over those exposed bytes.
- **C6.** Every complete ADF Field preserves the whole canonical JSON tree under the ADF media type; supported nodes render deterministically, well-formed unsupported nodes render typed positional markers plus warnings, and malformed ADF fails before any partial Aggregate escapes.
- **C7.** The issue Aggregate renders every visible field in field-ID order with name, native type, `Mutable: false`, canonical Field reference, readable value, warnings, and a Version Tag over the returned Markdown.
- **C8.** Session caching is opportunistic: usable ETags revalidate, valid matching-generation 304 reuses exact bytes, unusable/absent validators fetch unconditionally, and inconsistent 304 fails `source_unavailable`.
- **C9.** All Jira egress inherits the one body ceiling, logical deadline, at-most-one bounded idempotent retry, cancellation, redirect refusal, status mapping, and diagnostic redaction of `HttpSubstrate`/`BoundedRead`.
- **C10.** Invalid references, unknown Site IDs, unsupported Jira paths, and Field selectors produce typed local errors with zero Atlassian egress.
- **C11.** Aggregate/Field-index text selectors preserve Markdown media type and whole-resource Version Tag, while complete JSON/ADF Fields reject projection selectors as `unsupported_projection`.
- **C12.** Jira provider code remains source-local and reaches core/MCP only through typed `PathReference`, `SourceAdapter`, `SourceResource`, `ResourceError`, and existing compiled-source interfaces.
- **C13.** An ignored `live_jira_issue_read` row exercises a real read-only site when gated, while deterministic local TLS remains the permanent contract for every request/error branch.
- **C14.** `jira://<site>/issues/<id>/fields` is a deterministic read-only index over exactly the fields present on the validated issue, with the same ordering/name/type/mutability/reference facts as the Aggregate and without inventing fields from extra metadata.
- **C15.** API-token requests send exactly `Authorization: Basic base64(email:token)` through `OriginCredential`, and no credential/account-email representation reaches any observable channel.
- **C16.** The current Jira REST v3 direct-issue contract accepts either issue ID or key, performs moved/case fallback without redirect, and returns the found issue identity required for canonical alias resolution.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|-------|-------------|-----------|--------|----------------|------------------|------|--------|
| C16 | Vendor direct-read contract supports canonical alias resolution. | I7 | Run the pinned OpenAPI/document probe; absence of the ID-or-key parameter, moved/case fallback, returned found key, or no-redirect contract falsifies C16. | Separately rendered Atlassian operation prose manually compared in `.rfs-pm0y/evidence.md`, independent of probe traversal and production Rust. | In `atlassian/jira.rs`, replace the one `issueIdOrKey` route with a key-specific guessed endpoint; `stable_and_alias_reads_share_canonical_identity` turns red against the fake and live positive control. | `atlassian_jira_adapter_contract::stable_and_alias_reads_share_canonical_identity` plus `jira_live_smoke::live_jira_issue_read`. | seconds | PASS |
| C1 | Canonical typed Jira references. | I1–I4 | Parse the full valid/invalid grammar table; any accepted noncanonical spelling, rejected valid boundary, raw separator, dot segment, zero/leading-zero ID, or dormant path falsifies C1. | Hand table derived from `rfs-lcuh`, independently enumerated before parser implementation. | In `core/reference.rs`, remove the canonical re-encode comparison; lowercase `%2f` becomes green and `jira_reference_contract::rejects_noncanonical_segments` turns red. | `resourcefs-core/tests/jira_reference_contract.rs` grammar matrix plus existing exhaustive path-reference contract. | minutes | PENDING — checkpointed-build, core-reference slice. |
| C2 | One Site ID maps to one canonical origin. | I5 | Construct empty/distinct/duplicate/invalid mount sets and capture destination; acceptance of duplicate ID/origin or routing to another site falsifies C2. | Independently normalized `url::Url` origin tuple table in the test fixture. | In `sources/atlassian/mod.rs`, key sites only by insertion index; duplicate-ID case stops failing and `mount_rejects_duplicate_authority` turns red. | `atlassian_jira_adapter_contract::mount_validation_and_site_routing`. | minutes | PENDING — checkpointed-build, mount/auth slice. |
| C3 | ID/key aliases resolve one stable identity. | I4, I7 | Fake TLS returns the same issue for ID/key then returns a mismatched ID/self; differing canonical references or accepted confusion falsifies C3. | Server-side request log plus independently constructed stable canonical reference. | In `atlassian/jira.rs`, build canonical reference from requested alias; `alias_returns_stable_canonical_identity` turns red. | `atlassian_jira_adapter_contract::stable_and_alias_reads_share_canonical_identity` and confusion rows. | minutes | PENDING — checkpointed-build, Adapter integration slice. |
| C4 | Native authority is strict and atomic. | I6, I7, I9 | Feed a nullable/required presence matrix, duplicate keys, malformed types, foreign self URLs, and positive controls; any defaulted authority or partial success falsifies C4. | Hand-authored table of required ResourceFS authority fields, separate from serde DTO definitions. | In `atlassian/wire.rs`, add `#[serde(default)]` to `fields`; missing-fields row returns success and `jira_wire_contract::authority_presence_matrix` turns red. | `resourcefs-sources/tests/jira_wire_contract.rs` native-wire matrix. | minutes | PENDING — checkpointed-build, wire slice. |
| C5 | Ordinary Fields are deterministic canonical JSON authority. | I8, I14 | Encode reordered/whitespace-varied equivalents and distinct null/empty/number/Unicode cases; unequal equivalents or equal distinct values/tags falsify C5. | A small test-only independent canonical writer over fixture literals, not the production recursive writer. | In `atlassian/wire.rs`, serialize the parsed map in insertion order; reordered-object row changes bytes and `canonical_json_is_order_independent` turns red. | `jira_wire_contract::canonical_json_roundtrip_and_distinction_matrix`. | minutes | PENDING — checkpointed-build, wire/render slice. |
| C6 | Complete ADF remains lossless and projection failures are typed/atomic. | I10, I14 | Compare Field bytes to independent canonical JSON; render supported/unknown/malformed trees; lost member, missing marker/warning, or partial malformed output falsifies C6. | Fixture-side JSON tree walk and expected source-position list, separate from renderer traversal. | In `atlassian/render.rs`, skip unknown nodes instead of emitting markers; `unsupported_adf_nodes_are_visible_and_lossless` turns red. | `jira_wire_contract::adf_lossless_matrix` and `atlassian_jira_adapter_contract::adf_projection_contract`. | minutes | PENDING — checkpointed-build, wire/render slice. |
| C7 | Aggregate is complete, ordered, readable, immutable, and content-tagged. | I9, I13, I14 | Return shuffled fields with duplicate names and mixed values; compare ordered field IDs/facts and recomputed SHA-256; omission/order/tag mismatch falsifies C7. | Test-side sort of captured raw field IDs and direct SHA-256 of returned Markdown. | In `atlassian/render.rs`, iterate upstream map order; `aggregate_orders_every_field_by_id` turns red. | `atlassian_jira_adapter_contract::aggregate_orders_and_links_all_visible_fields`. | minutes | PENDING — checkpointed-build, render slice. |
| C8 | ETag caching never serves unvalidated or wrong-generation bytes. | I11 | Run first fetch, conditional repeat, valid 304, changed ETag, absent/oversized validator, and orphan 304; stale success or missed unconditional request falsifies C8. | Fake server request/response transcript plus cache-generation observations from existing test support. | In `atlassian/jira.rs`, accept 304 without cached metadata; `orphan_304_is_source_unavailable` turns red. | `atlassian_jira_adapter_contract::etag_revalidation_matrix`. | minutes | PENDING — checkpointed-build, HTTP/cache slice. |
| C9 | Shared bounds/retry/status/cancellation/redaction govern Jira. | I11, I12 | Deterministic TLS drives transport retry, 429/503 Retry-After, ordinary/rate 403, body overflow, malicious prose, canary secrets, and cancellation; extra send, wrong category, leak, or late completion falsifies C9. | Server-side send/flush counters and direct stable-category table independent of Adapter internals. | In `atlassian/jira.rs`, call `HttpSubstrate::execute` instead of `BoundedRead`; HTTP-date retry row sends the wrong count and turns red. | `atlassian_jira_adapter_contract::status_retry_bound_cancel_redaction_matrix`; existing substrate contracts remain positive controls. | minutes | PENDING — checkpointed-build, HTTP/cache slice. |
| C10 | Local refusals are zero-egress. | I1–I5, I12 | Pair every invalid/unknown/unsupported input with a viable TLS endpoint and assert zero accepted connections; any connection falsifies C10. | Server accept counter is independent of parser/dispatch. | In `atlassian/mod.rs`, resolve site after constructing/sending request; `invalid_inputs_are_zero_egress` turns red. | `atlassian_jira_adapter_contract::invalid_inputs_are_zero_egress`. | minutes | PENDING — checkpointed-build, integration slice. |
| C11 | Selectors preserve truthful media/tag semantics. | I4, I13 | Select Aggregate/index lines and attempt Field selectors; wrong MIME/tag or accepted partial JSON falsifies C11. | Direct full-content SHA-256 and selector-range table from core selector contract. | In `atlassian/mod.rs`, route Field selection through `selected_utf8`; partial bytes report JSON and `field_selectors_are_rejected` turns red. | Core `source_resource_content_type_contract` plus `atlassian_jira_adapter_contract::selector_media_contract`. | minutes | PENDING — checkpointed-build, core-resource/integration slices. |
| C12 | Provider implementation stays behind source-neutral seams. | I4 | Run architecture checks and dependency inspection; serde/provider types in core or rmcp/source internals crossing seams falsifies C12. | Cargo metadata and source-token architecture scanner, independent of Rust module visibility. | In `core/reference.rs`, import `serde_json::Value`; `architecture_contract::core_remains_source_neutral` turns red. | Existing `resourcefs-core/tests/architecture_contract.rs`, extended for Atlassian provider tokens/dependencies. | minutes | PENDING — checkpointed-build, every slice. |
| C13 | Live and deterministic evidence both exist. | I7, I9–I12 | Compile ignored live row with gates absent, list the registered `live_jira_issue_read` row, run deterministic TLS, and when credentials are present run the real stable-ID/key/Field invariants; a missing row/clean skip or invariant failure falsifies C13. | Real Atlassian response compared by stable IDs/shapes only; deterministic server transcript is the permanent branch oracle; the test harness list independently proves row registration. | Rename `live_jira_issue_read` to `jira_issue_read`; the harness-list fence no longer finds the required `live_*` row and turns red without credentials. | `cargo test -p resourcefs-sources --test jira_live_smoke -- --list` must contain `live_jira_issue_read: test`, plus the ignored live row and deterministic Adapter suite. | credentials for live; seconds for registration; minutes deterministic | PENDING — checkpointed-build, final slice; live execution recorded only when gate data exists. |
| C14 | Field index exactly reflects present fields. | I6, I9, I13 | Return empty/single/shuffled fields plus extra/missing metadata; invented/omitted/out-of-order index entries or non-atomic missing metadata falsifies C14. | Test-side set difference between raw `fields` keys and rendered canonical references. | In `atlassian/render.rs`, iterate `names` instead of `fields`; extra metadata creates an entry and `field_index_uses_present_values_only` turns red. | `atlassian_jira_adapter_contract::field_index_uses_present_values_only`. | minutes | PENDING — checkpointed-build, render slice. |
| C15 | Basic credentials are exact and non-observable. | I5, I12 | Capture the TLS request for a known email/token and scan Debug/errors/logs/cache/output with canaries; wrong header or any leak falsifies C15. | Independent `printf email:token | base64` expected header and byte-level canary scan. | In `http/mod.rs`, compose Basic with token alone; captured header differs and `basic_api_token_is_exact_and_redacted` turns red. | `http_substrate_contract::basic_credential_composition` and Jira TLS canary row. | minutes | PENDING — checkpointed-build, mount/auth slice. |

## Non-goals and future work

- Permanent non-goal: ACLI, a Jira-only transport Adapter, a second HTTP client, source-specific egress policy, rendered HTML authority, comment inlining, mutation of Aggregates/aliases, or Markdown-to-ADF conversion. These contradict the accepted parent architecture and authority model.
- Intended future work: project/site issue browsing (`rfs-h212`), Jira comments (`rfs-9m70`), enhanced JQL (`rfs-nae2`), Confluence reads (`rfs-wlft` and dependents), profile/probe/serve integration (`rfs-ue44`), release proof/publication (`rfs-lzbn`), OAuth (`rfs-flq5`), Field replacement (`rfs-pdp5`), and Creation Targets (`rfs-ddpt`). All IDs were verified in the local tracker.
- This ticket does not advertise product/site roots, collections, comments, queries, Confluence, OAuth, mutation, or creation before their owning tickets make those paths usable.

## Falsifier run log

- `2026-08-29 | python .rfs-pm0y/probe_openapi.py | PASS` — C16's cheapest falsifier confirmed the current endpoint accepts ID/key, performs moved/case fallback without redirect, and returns the found key; `.rfs-pm0y/evidence.md` records the independent rendered-document oracle. The same run also reconfirmed P2–P5.

## Approval

- Requester approval: “Approve revised design”
- Date: 2026-08-29
- Approved risk acceptances: None.
