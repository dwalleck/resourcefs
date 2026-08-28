# Plan: GitHub mutation and Creation Targets

## Inputs and partition arithmetic

Approved inputs: `.rfs-by2z/{route.md,spec.md,evidence.md,design.md}`. Design C1 is already `PASS`; C2–C22 are `PENDING — checkpointed-build` and are each assigned exactly once below. No review re-entry log applies.

| Slice | Diff estimate |
|---|---:|
| S1 — typed Creation Target references | 400 |
| S2 — operation IDs and Path-Session journal | 1,100 |
| S3 — exhaustive mutation target/outcome seam | 1,000 |
| S4 — bounded non-redirecting HTTP mutation transport | 900 |
| S5 — grant-controlled GitHub Field replacement | 1,800 |
| S6 — GitHub Creation Targets and recovery | 2,000 |
| S7 — stdio MCP mutation contract | 700 |
| **Planned subtotal** | **7,900** |
| **25% churn margin** | **1,975** |
| **Projected total** | **9,875** |

The 25% margin covers exhaustive match/caller migration in core, stateful TLS-fixture work, schema/render snapshots, and named-mutation fence adjustments; those are the highest-drift areas in the adjacent rfs-73dz/rfs-45ww changes. The projected total exceeds the 4,000-line review-size gate, so the plan uses three dependency-ordered increments. This checkout has no Git remote or configured upstream/default branch; mergeability is therefore stated relative to the preceding increment rather than a hard-coded branch.

### Increment A — Mutation foundation (S1–S3; 2,500 planned lines)

Mergeable definition: typed Creation Target references parse but remain write-refused; `operationId` is publicly parsed and rejected outside Creation Targets; the core journal and exhaustive target/outcome seam land with every existing mutation adapter/caller migrated. All current authored-text behavior and focused workspace suites remain green without HTTP/GitHub mutation.

Independent verification: core reference/journal/model tests, invalid-operation-ID stdio refusal, complete pre-existing mutation contract binaries, and architecture dependency checks. No later increment is needed.

### Increment B — Existing GitHub Fields (S4–S5; 2,700 planned lines)

Mergeable definition: the single HTTP module supports bounded one-attempt non-redirecting PATCH/POST requests; existing GitHub title/body/conversation-comment Fields become grant-controlled `rfs_write` replacements with authoritative comparison, response tags, status mapping, cache invalidation, and explicit non-target refusal. Creation Targets still reject writes.

Independent verification: HTTP policy/TLS fixtures and direct stateful fake-upstream Field mutation contracts, including concurrent stale contenders. No Creation Target or later MCP acceptance row is needed.

### Increment C — Creation and product seam (S6–S7; 2,700 planned lines)

Mergeable definition: all three Creation Targets, strict documents, exact-byte/session-local deduplication, outcome reconciliation, comprehensive wire validation, and real stdio MCP receipts/errors complete the approved behavior.

Independent verification: direct stateful fake-upstream creation/recovery matrices, deterministic wire route/payload fences, the three read-only empirical scripts, and the real stdio MCP process contract. This is the terminal increment.

## Slice 1: Add typed write-only GitHub Creation Target references

**Claim IDs:** C2

**Expected behavior:** `issue://owner/repo/new`, `issue://owner/repo/N/comments/new`, `pr://owner/repo/new`, and `pr://owner/repo/N/comments/new` parse into explicit variants, render byte-canonically, reject malformed roots/components/numeric/selector shapes, and return `unsupported_projection` with zero network requests when read.

**Oracle:** Hand-authored canonical/error table covering roots, encoded repository components, numeric boundaries, `/new`, `comments/new`, and selector variants; the table never calls the production parser.

**Stress fixture:** A 64 KiB-boundary Path Reference corpus containing percent-encoded delimiters, mixed-case owner/repository spelling, `u64::MAX`, zero/negative/overflow IDs, page/raw/line/multi-range selectors, and extra segments. Expected: exact valid canonical outputs; invalid rows fail in their named category; Creation Target reads emit zero fixture requests.

**Regression fence:** `crates/resourcefs-core/tests/github_reference_contract.rs::creation_targets_round_trip_and_are_write_only`; `crates/resourcefs-sources/tests/github_adapter_contract.rs::creation_target_reads_are_unsupported` — created/extended in this slice with `[C2]` failure context.

**Named mutation:** In `crates/resourcefs-core/src/reference.rs`, render `CommentsNew` as `comments/0`; the C2 round-trip fence must fail red, then green after restoration.

**Complexity/production scale:** Reference parsing/rendering remains $O(n)$ in at most `MAX_PATH_REFERENCE_BYTES = 64 KiB`; four new terminal variants add constant match work and no collection. Maximum accepted added cost: 5 ms for the 64 KiB stress corpus row in release mode, because ordinary GitHub references are under 256 bytes and this preserves the existing bounded parser shape.

**Wall budget/phase:** Always-on for every GitHub reference parse/read dispatch. Budget: 5 ms at the 64 KiB ceiling excluding network (there is no network for Creation Target reads), measured by the stress fence in release mode.

**Files:** `crates/resourcefs-core/src/reference.rs`; `crates/resourcefs-core/src/resource.rs`; `crates/resourcefs-core/tests/github_reference_contract.rs`; `crates/resourcefs-sources/src/github/mod.rs`; `crates/resourcefs-sources/tests/github_adapter_contract.rs`.

**Estimate:** 4–6 hours.

**Diff estimate:** 400 changed lines.

**PR increment:** Increment A — Mutation foundation.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test github_reference_contract creation_targets_round_trip_and_are_write_only` → every valid row round-trips exactly; invalid/boundary rows return the table's category; under the named rendering mutation the command fails with `[C2]`, then passes after restoration.
- `cargo test -p resourcefs-sources --test github_adapter_contract creation_target_reads_are_unsupported` → all four reads return `unsupported_projection` and the fixture request count remains zero.

## Slice 2: Add validated operation IDs and the exact-byte Path-Session journal

**Claim IDs:** C3, C4

**Expected behavior:** `OperationId` validates the exact 1–128 ASCII token language; `rfs_write.operationId` is rejected before dispatch outside Creation Targets; one Path Session binds an ID to one canonical target and exact shared content bytes, coalesces identical in-flight calls, models success/conclusive/unknown transitions, rejects changed meaning, enforces 10,000 entries plus the shared 256 MiB byte ceiling, never evicts entries, and isolates disconnect/new sessions.

**Oracle:** Test-local exact-byte `BTreeMap` transition model plus literal token/metadata tables. The model stores no production fingerprint and receives the same traces as the real asynchronous journal.

**Stress fixture:** (1) Every ASCII byte at positions 1 and 128, Unicode/control/space/forbidden punctuation, and 129 bytes; (2) 10,000 one-byte entries then a 10,001st; (3) exact 256 MiB retained-byte accounting and one byte over; (4) same/different-length conflicting content; (5) forced equal cached fingerprint with distinct exact bytes; (6) 64 concurrent identical calls and two conflicting calls; (7) authoritative admission into a 1,000-entry cache under a 32 KiB session ceiling. Expected: one owner attempt, 63 shared results, conflict for changed bytes/target, exact ceiling success, one-over `limit_exceeded`, no cross-session reuse, and sufficient LRU cache eviction within 25 ms.

**Regression fence:** `crates/resourcefs-core/tests/mutation_engine_contract.rs::{operation_id_validation_matrix,operation_id_is_rejected_before_resolution,operation_journal_trace_matches_model,operation_journal_concurrent_repeat_coalesces,operation_journal_forced_fingerprint_collision_conflicts,operation_journal_count_and_byte_ceilings,operation_journal_budget}`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::invalid_operation_id_precedes_dispatch` — created in this slice with `[C3]`/`[C4]` context.

**Named mutation:** C3 — move the non-Creation-Target `operationId` rejection in `crates/resourcefs-core/src/mutation.rs` after `MutationAdapter::resolve`; the zero-resolver-call fence fails. C4 — remove final exact-byte equality in `crates/resourcefs-core/src/session.rs` and trust fingerprint equality; the forced-collision fence returns a repeat instead of `version_conflict`.

**Complexity/production scale:** Token validation is $O(i)$ for $i \le 128$. Journal lookup is expected $O(1)$ by ID; exact comparison is $O(c)$ for $c \le 64 MiB$ only after target/fingerprint equality; entry counting and byte accounting are $O(1)$. Authoritative journal admission may reuse the existing LRU cache-yield loop, which is $O(k^2)$ over at most `MAX_SESSION_ARTIFACTS = 1,000` cache objects (at most 1,000,000 key comparisons); it never scans the 10,000-entry journal. Maximum accepted CPU cost: 50 ms for one forced 64 MiB fingerprint/exact comparison and 25 ms for either a 10,000-entry journal trace or worst-case cache yield in release mode.

**Wall budget/phase:** Always-on for every `rfs_write` carrying metadata and every Creation Target journal transition. Budgets: 1 ms for token validation/ordinary (<64 KiB) lookup; 50 ms at the 64 MiB comparison ceiling; waits on an in-flight owner inherit that owner's later adapter deadline and perform no busy loop.

**Files:** `crates/resourcefs-core/src/mutation.rs`; `crates/resourcefs-core/src/session.rs`; `crates/resourcefs-core/src/lib.rs`; `crates/resourcefs-core/tests/mutation_engine_contract.rs`; `crates/resourcefs-mcp/src/server.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 1–1.5 days.

**Diff estimate:** 1,100 changed lines.

**PR increment:** Increment A — Mutation foundation.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test mutation_engine_contract operation_id` → C3 valid tokens and C4 exact-byte/count/byte-ceiling traces agree item-by-item with the literal/model oracle; both named mutations produce their named red result and restore green.
- `cargo test -p resourcefs-core --test mutation_engine_contract operation_journal` → identical concurrency has one owner; conflict/certainty/disconnect/quota rows match the exact-byte model.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract invalid_operation_id_precedes_dispatch` → invalid and non-Creation-Target IDs return `invalid_reference` with zero resolver/adapter/network observations.

## Slice 3: Deepen the common mutation seam with exhaustive target and outcome states

**Claim IDs:** C7, C17

**Expected behavior:** `MutationTargetMode::{AuthoredText,AuthoritativeText,CreationTarget}`, typed commit successes, and typed conclusive/unknown failures replace the universal authored-text assumption. Authored mode retains the existing pre-commit seen reservation, input tag/coverage, locks, cancellation masking, and all filesystem/local/edit/delete/move behavior; illegal mode/outcome pairs fail instead of fabricating receipts.

**Oracle:** Exhaustive legal/illegal target-outcome table in a core fake adapter plus the complete focused mutation contract binaries that predate rfs-by2z.

**Stress fixture:** Fake adapter emits all $3 \times 3$ target/outcome combinations and both failure certainty variants, including cancellation before/after commit entry and a failed authored commit with a reserved seen snapshot. Expected: only legal cells produce their mode-specific receipt; reservations release on failure; existing move/edit concurrency remains unchanged.

**Regression fence:** `crates/resourcefs-core/tests/mutation_engine_contract.rs::target_mode_outcome_matrix` plus complete pre-existing test binaries `mutation_engine_contract`, `filesystem_mutation_contract`, `local_mutation_contract`, and `stdio_mcp_contract` — migrated in this slice with `[C7]`/`[C17]` context for new assertions.

**Named mutation:** C7 — accept `CreationCommitted` for `AuthoritativeText`; the exhaustive matrix fails red. C17 — skip seen reservation for every mode; existing `failed_commit_releases_seen_reservation`/stdio edit-after-write behavior fails red.

**Complexity/production scale:** Exhaustive mode/outcome dispatch adds only constant-time matches. Authored content remains one shared immutable buffer; no new content loop or copy is introduced. Maximum accepted overhead: 1 ms excluding existing filesystem/local I/O for a 64 MiB write, because the slice adds type dispatch only.

**Wall budget/phase:** Always-on for every mutation. Budget: 1 ms of core orchestration overhead excluding adapter I/O, measured with fake adapters across all target/outcome cells; existing adapter budgets remain unchanged.

**Files:** `crates/resourcefs-core/src/mutation.rs`; `crates/resourcefs-core/src/lib.rs`; `crates/resourcefs-core/tests/mutation_engine_contract.rs`; `crates/resourcefs-sources/src/filesystem_mutation.rs`; `crates/resourcefs-sources/src/local.rs`; `crates/resourcefs-sources/src/compiled.rs`; `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`; `crates/resourcefs-sources/tests/local_mutation_contract.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 1–1.5 days.

**Diff estimate:** 1,000 changed lines.

**PR increment:** Increment A — Mutation foundation.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test mutation_engine_contract target_mode_outcome_matrix` → every legal cell returns the table's receipt, every illegal cell fails, and the named mutation fails with `[C7]` before restoration.
- `cargo test -p resourcefs-core --test mutation_engine_contract && cargo test -p resourcefs-sources --test filesystem_mutation_contract && cargo test -p resourcefs-sources --test local_mutation_contract && cargo test -p resourcefs-mcp --test stdio_mcp_contract` → all pre-existing authored create/replace/edit/delete/move/snapshot/lock/cancellation outcomes remain unchanged; the C17 named mutation turns an existing row red and restoration returns green.

## Slice 4: Extend the bounded HTTP seam for one-attempt non-redirecting mutation

**Claim IDs:** C15

**Expected behavior:** `HttpRequest` carries a private typed GET/POST/PATCH method, optional bounded JSON body, and redirect mode. `HttpSubstrate` owns both redirecting-read and non-redirecting-mutation reqwest clients built from the same resolver/allowlist/credential/TLS/timeout configuration; mutation sends once, never follows Location, streams bounded responses, and exposes no reqwest type/source bypass.

**Oracle:** Socket/TLS fixture independently records method, target, headers, complete bounded body, connections, and redirect follow-ups; existing architecture scan independently finds reqwest token/dependency placement.

**Stress fixture:** 64 MiB exact/one-over body, 16-header and 16 KiB header ceilings, public/private/literal-host policy rows, credentialed same/cross-origin redirects, TLS failure, timeout/cancellation, and a redirect loop. Expected: exact-limit request admitted; one-over rejected before socket; one PATCH/POST connection; zero Location follow-up; all existing GET redirect rows unchanged.

**Regression fence:** `crates/resourcefs-sources/tests/http_substrate_contract.rs::mutation_request_is_bounded_and_non_redirecting`; existing HTTP policy/TLS contracts; `crates/resourcefs-mcp/tests/architecture_contract.rs::single_http_client_module` — extended in this slice with `[C15]` context.

**Named mutation:** Instantiate `reqwest::Client` in `crates/resourcefs-sources/src/github/mutation.rs` or select the redirecting client for mutation; architecture or redirect-count fence fails red.

**Complexity/production scale:** Request validation is $O(h+b)$ for at most 16 headers/16 KiB header bytes and body $b \le 64 MiB$; response streaming stays $O(r)$ under existing `MAX_HTTP_FETCH_BYTES`; two clients add a second private connection pool but no per-request construction. Maximum accepted local-fixture overhead: 100 ms to validate/send/read an exact 64 MiB loopback body excluding scheduler noise; ordinary payload overhead under 5 ms.

**Wall budget/phase:** Always-on for network operations. Mutation inherits `HttpCeilings`' 30,000 ms per-request maximum; local policy/body processing budget is 100 ms at 64 MiB and 5 ms for ordinary payloads. Client construction is one-off at source mount: N/A — one-off phase; no wall budget beyond existing startup checks.

**Files:** `crates/resourcefs-sources/src/http/mod.rs`; `crates/resourcefs-sources/src/lib.rs`; `crates/resourcefs-sources/tests/support/tls.rs`; `crates/resourcefs-sources/tests/http_substrate_contract.rs`; applicable existing HTTP policy/TLS contract tests; `crates/resourcefs-mcp/tests/architecture_contract.rs`.

**Estimate:** 1 day.

**Diff estimate:** 900 changed lines.

**PR increment:** Increment B — Existing GitHub Fields.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test http_substrate_contract mutation_request_is_bounded_and_non_redirecting` → method/body bytes match the socket oracle, one-over is pre-socket, one request/zero follow-ups observed, and redirect-client mutation turns `[C15]` red.
- `cargo test -p resourcefs-sources --test http_policy_contract && cargo test -p resourcefs-sources --test http_tls_contract && cargo test -p resourcefs-mcp --test architecture_contract single_http_client_module` → all existing GET redirect/address/TLS behavior remains green and a source-owned client mutation fails the architecture fence.

## Slice 5: Implement grant-controlled GitHub Field replacement

**Claim IDs:** C8, C9, C12, C13, C14, C21, C22

**Expected behavior:** Only allowlisted, repository-update-granted issue/PR title/body/stable conversation-comment Fields resolve as `AuthoritativeText`; `rfs_write` performs one uncached authoritative GET and one field-minimal PATCH under the canonical lock, compares the current Version Tag, validates response identity/field, returns a response-derived tag with no seen coverage, classifies/redacts errors, and clears the entire session `github-http` namespace before commit returns. Aggregates/collections/reviews/inline/diff/delete/edit/unsupported paths produce zero mutation egress. Profile grants stay create/update-only and architecture locality holds.

**Oracle:** Stateful fake upstream owns authoritative objects independently, transforms selected response fields, logs ordered requests/minimal JSON, and exposes cache/request inventories; hard-coded Python-hashlib Version Tag vectors, literal status/projection tables, existing schema golden, and Cargo architecture scan provide independent comparisons.

**Stress fixture:** Title/body/comment current/stale/missing tags; empty/blank/Unicode/max bodies; two same-Field contenders and distinct-Field concurrency; stale ETag cache; response normalization; every approved status with body/credential sentinel; 10,000 GitHub cache keys plus non-GitHub collision prefixes; all non-mutable paths; source/repository grant subsets/delete. Expected: one winner/one stale conflict, exact one-field PATCH, response-vector tag, no seen entry, cache empty before adapter return, no secret/body leak, and zero egress for denials.

**Regression fence:** `crates/resourcefs-sources/tests/github_mutation_contract.rs::{field_replace_compares_uncached_authority,concurrent_field_replacements_serialize_and_reject_stale,field_receipt_tags_response_without_seen_coverage,mutation_status_matrix_is_stable_and_redacted,success_invalidates_github_cache_before_commit_returns,unsupported_mutation_matrix_has_zero_egress}`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::github_non_target_mutation_is_refused`; `configuration_contract::nested_grants_are_subsets`; `profile_contract::schema_matches_deserializer`; architecture contracts — created/extended in this slice with corresponding claim IDs.

**Named mutation:** C8 key locks by requested alias/use cached load; concurrent/stale rows fail. C9 publish authored coverage/input tag; response-vector/seen row fails. C12 map 410 through fallback/include upstream body; status/redaction row fails. C13 spawn cache clearing after commit return or match a loose namespace prefix; ordering/isolation row fails. C14 route Aggregate to update; zero-egress row fails. C21 permit GitHub delete; grant/schema rows fail. C22 add reqwest/rmcp dependency to core or GitHub orchestration to MCP; architecture rows fail.

**Complexity/production scale:** Field classification/status/identity checks are $O(1)$ plus response JSON decode $O(r)$ under the HTTP fetch ceiling. Cache namespace removal is $O(g)$ using a namespace index for $g$ GitHub entries, not $O(all\ session\ entries)$; stress scale is 10,000 GitHub plus 10,000 non-GitHub entries. Maximum accepted local cost: 50 ms to remove/verify 10,000 GitHub entries and 100 ms to validate a maximum bounded response; no nested collection scan.

**Wall budget/phase:** Always-on replacement performs at most one uncached GET plus one PATCH, each bounded by 30,000 ms, for a maximum 60,000 ms network wall time; local compare/decode/cache work budget 150 ms at production ceilings. This explicit two-request maximum follows immediate compare-before-write and no remote atomic condition.

**Files:** create `crates/resourcefs-sources/src/github/mutation.rs` and `crates/resourcefs-sources/tests/github_mutation_contract.rs`; modify `crates/resourcefs-sources/src/github/{mod.rs,wire.rs}`; `crates/resourcefs-sources/src/compiled.rs`; `crates/resourcefs-core/src/session.rs`; `crates/resourcefs-sources/tests/support/tls.rs`; `crates/resourcefs-sources/tests/github_adapter_contract.rs`; `crates/resourcefs-sources/tests/configuration_contract.rs`; `crates/resourcefs-mcp/tests/{profile_contract.rs,stdio_mcp_contract.rs,architecture_contract.rs}`; `crates/resourcefs-mcp/tests/fixtures/server-profile-v1.schema.json` only if generated schema bytes change.

**Estimate:** 2–2.5 days.

**Diff estimate:** 1,800 changed lines.

**PR increment:** Increment B — Existing GitHub Fields.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_mutation_contract field_` → C8/C9 current/stale/concurrent/normalized rows match fake state, request order, hard-coded tags, and no-seen oracle; each named mutation produces its claim-local red output before restoration.
- `cargo test -p resourcefs-sources --test github_mutation_contract mutation_status_matrix_is_stable_and_redacted` → every approved status maps exactly and sentinel body/credential text is absent.
- `cargo test -p resourcefs-sources --test github_mutation_contract success_invalidates_github_cache_before_commit_returns` → direct commit return observes zero GitHub keys, all non-GitHub keys, and a failed commit leaves both; ordering/prefix mutations fail red.
- `cargo test -p resourcefs-sources --test github_mutation_contract unsupported_mutation_matrix_has_zero_egress && cargo test -p resourcefs-mcp --test stdio_mcp_contract github_non_target_mutation_is_refused` → every non-target returns the table category with zero mutating requests.
- `cargo test -p resourcefs-sources --test configuration_contract nested_grants_are_subsets && cargo test -p resourcefs-mcp --test profile_contract schema_matches_deserializer && cargo test -p resourcefs-mcp --test architecture_contract` → C21/C22 grants, schema, and dependency/module ownership remain exact; named mutations fail locally.

## Slice 6: Implement strict GitHub Creation Targets and reconciliation

**Claim IDs:** C1, C5, C6, C10, C11, C18, C19, C20

**Expected behavior:** Creation Target metadata/grants validate before network; strict issue/PR documents and non-blank comment Markdown produce only approved JSON keys; the six total mutation routes match empirical evidence; POST success validates repository/parent/ID and returns canonical reference/null tag/no coverage; identical calls coalesce/repeat, conclusive 3xx/4xx permits identical caller retry, 5xx/transport/drop/wrong-or-invalid success becomes unknown and blocks, no mutation retries/redirects, and invalid responses never fabricate receipts.

**Oracle:** Hand-authored grammar/payload/reference/certainty tables; exact expected body bytes; sequential journal model; socket-level state/request log; published go-github/OpenAPI oracles and docs probe for the six routes/statuses/types.

**Stress fixture:** Full strict parser accept/reject matrix; blank/Unicode/exact-64-MiB comments and issue/PR bodies; draft presence matrix; 64 concurrent identical calls; changed target/content conflicts; conclusive retry; every unknown trigger; malformed/missing/wrong parent/number/ID/field responses; truncated/oversize JSON; redirect/Retry-After/5xx; separate Path Sessions. Expected: exact JSON, one request/no follow-up, canonical response identity, null tag/coverage, model-matching journal state, and reconciliation text.

**Regression fence:** `github_wire_contract::mutation_routes_match_empirical_contract`; `github_mutation_contract::{metadata_grant_target_matrix_precedes_egress,creation_document_strict_matrix,creation_receipts_use_returned_identity_without_seen_coverage,mutation_never_retries_or_redirects,operation_recovery_state_matrix,mutation_response_validation_matrix,mutation_payloads_are_minimal_and_exact}` — created/extended in this slice with `[C1,C5,C6,C10,C11,C18,C19,C20]` output context.

**Named mutation:** C1 use pull-comment route for conversation comment. C5 authorize source grant without child repo/create metadata. C6 trim body/accept a forbidden YAML form. C10 keep `/new` or publish coverage. C11 call retrying/redirecting fetch. C18 mark Unknown retryable. C19 deserialize missing ID as zero or accept wrong parent. C20 serialize read DTOs/excluded fields. Each corresponding fence must turn red, restore, then green.

**Complexity/production scale:** Frontmatter scan/JSON serialization and exact journal comparison are $O(c)$ for content $c \le 64 MiB$; journal lookup expected $O(1)$; response validation $O(r)$ under the HTTP ceiling; concurrent repeats use notification/wait, not polling. Maximum accepted local cost: 100 ms for maximum document parse/serialization, 50 ms exact comparison, and 150 ms combined excluding network; 10,000-entry lookup trace remains under 25 ms as set in S2.

**Wall budget/phase:** Always-on creation sends one POST bounded by 30,000 ms; local validation/journal/serialization budget 150 ms at ceilings. Identical waiters share the owner's 30,000 ms attempt and add no second deadline/request. Conclusive caller retries are discrete later operations; unknown repeats return locally within 5 ms.

**Files:** `crates/resourcefs-sources/src/github/{mutation.rs,mod.rs,wire.rs}`; `crates/resourcefs-sources/src/compiled.rs`; `crates/resourcefs-sources/tests/{github_mutation_contract.rs,github_wire_contract.rs}`; `crates/resourcefs-sources/tests/support/tls.rs`; `crates/resourcefs-core/src/{mutation.rs,session.rs}` only for journal integration corrections; `.rfs-by2z/probe_github_mutation_docs.py`; `.rfs-by2z/probe_go_github_oracle.py`; `.rfs-by2z/probe_openapi_types_oracle.py` (retained evidence, no production import).

**Estimate:** 2.5–3 days.

**Diff estimate:** 2,000 changed lines.

**PR increment:** Increment C — Creation and product seam.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_mutation_contract creation_` → strict documents/comments, exact payloads, response identities, null tag/coverage, concurrency, conflict, quota, certainty, and cross-session rows match independent tables/model; every named mutation fails with its claim ID then restores green.
- `cargo test -p resourcefs-sources --test github_mutation_contract operation_recovery_state_matrix` → received 3xx/4xx is conclusive; every approved unknown trigger blocks; identical conclusive retry alone retransmits; changed meaning conflicts.
- `cargo test -p resourcefs-sources --test github_mutation_contract mutation_never_retries_or_redirects && cargo test -p resourcefs-sources --test github_mutation_contract mutation_response_validation_matrix` → one socket request/zero follow-up and no fabricated receipt across fault matrix.
- `cargo test -p resourcefs-sources --test github_wire_contract mutation_routes_match_empirical_contract` → all six deterministic method/path/key/response-type rows match the evidence table; route mutation turns C1 red.
- `python3 .rfs-by2z/probe_github_mutation_docs.py | jq -e '.claim == "C1"' && python3 .rfs-by2z/probe_go_github_oracle.py | jq -e '.claim == "C1"' && python3 .rfs-by2z/probe_openapi_types_oracle.py | jq -e '.claim == "C1"'` → all three read-only external instruments complete and identify C1; comparison remains PASS.

## Slice 7: Complete the public stdio MCP GitHub mutation contract

**Claim IDs:** C16

**Expected behavior:** `rfs_write` schema/tool description exposes optional `operationId` with Creation-Target-only semantics; real stdio calls produce non-empty text and object-root structured receipts/errors agreeing on operation, response-derived canonical reference, null creation/non-null Field tag, no coverage where specified, repeat result, and stable category/reconciliation guidance.

**Oracle:** Checked JSON schema fixture plus a test-local plain-text receipt parser and literal semantic row table, independent of production render functions.

**Stress fixture:** Real binary/profile/fake-upstream session calls Field replace and all three Creation Targets, identical/conflicting/unknown repeats, invalid/non-target operation IDs, stale tags, denied grants, and every non-mutable projection. Expected: text/structured semantic equality, one upstream mutation where applicable, object-root schema, non-empty TextContent, and no protocol errors for operational failures.

**Regression fence:** `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::github_mutation_schema_and_receipts_match` plus existing tool-list/object-root and versioned-write rows — created/extended in this slice with `[C16]` context.

**Named mutation:** Remove `operation_id` from `WriteInput` or render Creation Target receipt using requested `/new`; the schema/call or text/structured canonical-reference row fails red.

**Complexity/production scale:** MCP deserialization/rendering is $O(c)$ in input/error/receipt text bounded by 64 MiB input and standard text-output ceilings; normal receipts are constant-sized. Maximum accepted local protocol overhead: 10 ms per ordinary call and 100 ms for a maximum rejected input, excluding adapter/network time.

**Wall budget/phase:** Always-on for every `rfs_write`. Local MCP overhead budget 10 ms ordinary/100 ms at maximum input; total network-backed budgets remain S5 (60,000 ms Field) and S6 (30,000 ms Creation).

**Files:** `crates/resourcefs-mcp/src/server.rs`; `crates/resourcefs-mcp/src/render.rs` only if common receipt rendering needs a legal mode-aware adjustment; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`; tool schema snapshots/fixtures already owned by that test surface if generated bytes change; `scripts/live-smoke.sh` and read-only live rows intentionally unchanged because repository policy forbids live upstream mutation and the direct fake is the approved mutation oracle.

**Estimate:** 1 day.

**Diff estimate:** 700 changed lines.

**PR increment:** Increment C — Creation and product seam.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract github_mutation_schema_and_receipts_match` → schema contains optional operationId; each text/structured pair agrees item-by-item; Field/creation/repeat/error rows match the literal oracle; named mutations fail `[C16]` then restore green.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract versioned_write_receipts_and_catalog_policy_work_over_stdio` → existing authored-text stdio behavior remains unchanged after the public input addition.
- Launch the actual `resourcefs` process through the stdio contract fixture against the stateful local TLS upstream → title replacement and comment creation change fake authoritative state once, the returned canonical reference reads back correctly in the same Path Session, and unknown repeat returns reconciliation guidance without a second request.

## Tracker taxonomy

Permanent non-goals: the exact design non-goals remain permanent for the same approved rationale—no labels/assignees/status/milestones/reactions/reviews/inline-review mutation, delete, GitHub `rfs_edit`, bulk mutation, GraphQL, live upstream writes, automatic retry/redirect, cross-session deduplication, external-writer atomicity claim, full YAML, source-owned HTTP client, or GitHub-specific MCP tools. No plan step defers required behavior.

Intended future work: none. No trigger-conditioned follow-up is introduced, so no tracker issue is required.

## Self-review

- Every design claim C1–C22 is assigned exactly once: S1 `{C2}`; S2 `{C3,C4}`; S3 `{C7,C17}`; S4 `{C15}`; S5 `{C8,C9,C12,C13,C14,C21,C22}`; S6 `{C1,C5,C6,C10,C11,C18,C19,C20}`; S7 `{C16}`.
- All seven slices contain all thirteen mandatory fields; every conditional field is populated, and no approved-risk `N/A` exists because design approval records no risk acceptances.
- Every slice creates or extends every assigned permanent fence and carries each assigned named mutation; claim-local `[C#]` output is required.
- Every new loop has an asymptotic/scale bound and explicit maximum accepted cost; every always-on phase has a wall budget and rationale.
- Partition arithmetic is exact: 7,900 + 1,975 = 9,875; all slices name one of three independently mergeable increments, each below the 4,000-line review threshold before churn.
- Tracker taxonomy is applied; there is no intended future work.
- No slice is declared complete. Slice completion and actual cumulative diff belong exclusively to checkpointed-build.
