# Plan: rfs-pm0y

## Inputs and partition

- Approved design: `.rfs-pm0y/design.md`, requester words “Approve revised design”, 2026-08-29, no accepted risks.
- Route/evidence: Empirical; `.rfs-pm0y/evidence.md` has P1–P5 `PASS`; C16 is the completed cheapest falsifier. Initial slices assign every claim exactly once; review-fix Slice 5 traces accepted findings back to their existing claims.
- Projected changed lines: Slice 1 `1,000` + Slice 2 `500` + Slice 3 `5,400` + Slice 4 `300` + Slice 5 `600` = `7,800`.
- Churn margin: `25% = 1,950` lines. Rationale: exhaustive `ResourceAddress` migration, deterministic TLS matrices, and accepted review corrections add callsite/test-support rows after the first count.
- Review-size projection: `7,800 + 1,950 = 9,750`, above the 4,000-line gate; retain three independently mergeable increments.

### PR increment A — Typed and credential foundations

Slices 1–2. Mergeable definition: core recognizes and rejects the complete direct Jira grammar, and bounded Basic credentials compose only at the audited egress seam. Core/HTTP contracts pass without native Jira decoding, mount state, or network-backed Jira reads.

### PR increment B — Complete direct Jira Source Adapter

Slice 3. Mergeable definition: the public multi-site Atlassian Source Adapter strictly decodes native authority, canonicalizes JSON/ADF Fields, renders deterministic Aggregate/index Markdown, uses shared HTTP/cache/retry/cancellation, supports direct search, and is routed/cataloged by `CompiledSources`. Deterministic native-wire/TLS/oracle contracts verify it without profile launch integration.

### PR increment C — Live evidence and review corrections

Slices 4–5. Mergeable definition: the ignored real Jira row proves the complete Adapter against a configured read-only tenant, and accepted post-build review findings harden exact numeric authority, projection scope, stable search identity, and redirect/redaction behavior. This increment depends only on increment B.

## Slice 1: Add canonical Jira reference types

**Claim IDs:** C1

**Expected behavior:** `PathReference` parses only canonical direct issue, issue-key alias, Field-index, and Field Jira references into exported validated newtypes/sums; every exhaustive core caller handles `ResourceAddress::Jira`.

**Oracle:** The `rfs-lcuh` grammar table is independently hand-enumerated in the test.

**Stress fixture:** 65,536-byte overall references; 63-byte Site IDs; boundary/over-boundary issue IDs, Unicode aliases/field IDs, spaces, backslashes, raw/encoded separators, malformed/lowercase escapes, dot segments, reserved `new`, and every existing ResourceAddress family. Expected: one canonical spelling for valid rows, `invalid_reference`/`limit_exceeded` for invalid rows, unchanged behavior for old families.

**Regression fence:** `crates/resourcefs-core/tests/jira_reference_contract.rs` plus migrated existing path/read/discovery/resource contracts.

**Named mutation:** From C1, remove canonical segment re-encode comparison in `crates/resourcefs-core/src/reference.rs`; `jira_reference_contract::rejects_noncanonical_segments` must turn red, then restoration must return green.

**Complexity/production scale:** Parsing/encoding is $O(n)$ over at most `MAX_PATH_REFERENCE_BYTES = 65,536`; global admission scans at most 64 KiB and valid Jira component encoding performs one additional bounded allocation/comparison. Explicit maximum accepted CPU cost: 2 ms per maximum valid Jira reference in release mode, because parsing is a local pre-egress admission step and must be negligible beside network I/O.

**Wall budget/phase:** always-on for Jira parse; 2 ms at the maximum valid Jira component ceilings, same rationale as the CPU maximum.

**Files:** `crates/resourcefs-core/src/reference.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/src/read.rs`, `crates/resourcefs-core/src/discovery.rs`, `crates/resourcefs-core/src/resource.rs` only for exhaustive canonical-identity handling, `crates/resourcefs-core/src/mutation.rs` only where exhaustive handling requires an explicit unsupported arm, `crates/resourcefs-core/tests/jira_reference_contract.rs`, and existing exhaustive core contracts identified by compiler/LSP references.

**Estimate:** 3–5 hours.

**Diff estimate:** 1,000 changed lines.

**PR increment:** A — Typed and credential foundations.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test jira_reference_contract` → every valid row round-trips to one canonical reference; invalid/dormant rows return the expected stable category; the C1 named mutation turns the noncanonical-segment row red and restoration returns green.
- `cargo test -p resourcefs-core --tests` → all old address families and exhaustive engines retain their observable behavior with explicit Jira arms.

## Slice 2: Confine Basic credential composition to egress

**Claim IDs:** C15

**Expected behavior:** `OriginCredential::basic` validates the account identifier, composes `Basic base64(email:token)` while the token is exposed only inside the credential seam, bounds the final header value, and remains redacted.

**Oracle:** Independent `printf email:token | base64` bytes for Authorization and a byte-level canary scan of Debug/errors/captured output.

**Stress fixture:** Empty/colon/control email; valid canary email/token; a credential whose encoded header is exactly at/beyond the 16 KiB source-header ceiling. Expected: exact Basic header only for valid bounded input, `invalid_reference` or `limit_exceeded` before egress otherwise, and no canary in Debug/errors/output.

**Regression fence:** `crates/resourcefs-sources/tests/http_substrate_contract.rs::basic_credential_composition_is_exact_and_redacted`.

**Named mutation:** From C15, encode token without `email:` in `OriginCredential::basic`; the captured-header row turns red. Restore and confirm green.

**Complexity/production scale:** Basic composition is $O(e+t)$ once per credential, with the final value capped by `MAX_SOURCE_REQUEST_HEADER_BYTES = 16 KiB`; the raw pair and encoded value are each allocated once and the temporary raw buffer is overwritten before return. Explicit maximum accepted CPU cost: 5 ms at the 16 KiB ceiling because this is launch-time work.

**Wall budget/phase:** N/A — one-off credential construction; no wall budget.

**Files:** root `Cargo.toml`/`Cargo.lock`, `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/src/http/mod.rs`, and `crates/resourcefs-sources/tests/http_substrate_contract.rs`.

**Estimate:** 2–3 hours.

**Diff estimate:** 500 changed lines.

**PR increment:** A — Typed and credential foundations.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test http_substrate_contract basic_credential` → captured Authorization equals the independent base64 oracle; invalid/over-limit identifiers fail locally; canaries are absent; C15 mutation red/restored green.

## Slice 3: Implement the complete direct Jira Source Adapter

**Claim IDs:** C2, C3, C4, C5, C6, C7, C8, C9, C10, C11, C12, C14, C16

**Expected behavior:** A source-neutral UTF-8 envelope preserves JSON/vendor-JSON/Markdown media and whole-resource tags. One validated multi-site `AtlassianSourceMount` binds the shared substrate and yields `AtlassianSource`; stable-ID/key reads use Jira REST v3, validate returned authority, and return stable canonical Aggregate/index/Field Resources. Strict recursive JSON rejects duplicates/missing authority and produces deterministic compact Field bytes. Complete ADF remains lossless; supported nodes render Markdown, unsupported well-formed nodes/marks emit typed positional warnings, malformed ADF fails atomically. Aggregate/index facts are complete and field-ID ordered. ETag/cache/retry/status/bounds/cancellation/redaction/zero-egress behavior uses shared seams. `CompiledSources` routes/searches/catalogs mounted Jira and preserves honest unmounted errors.

**Oracle:** Hand-authored authority/status/grammar tables; independent compact-JSON fixture writer; fixture-side ADF tree walk/source-position list; raw-field set difference and sorted IDs; direct SHA-256; normalized Site-ID/origin sets; TLS request/send/flush transcript; session cache-generation observations; evidence oracle for the vendor endpoint; Cargo metadata/architecture scanner.

**Stress fixture:** Empty/one/maximum site sets; duplicate IDs/origins and non-origin URLs; full required/null/type/duplicate JSON matrix; empty/single/multi/reordered/Unicode/native-value distinctions; foreign/wrong-family/wrong-ID self URLs; minimal/supported/unknown/malformed ADF; shuffled fields/duplicate names/extra metadata; 8 MiB JSON and ADF; stable/key identity; unknown site/dormant path/Field selectors; ETag absent/valid/changed/oversized/orphan; transport/rate/status/body/cancellation branches; malicious prose/canaries; every `CompiledSources::new` callsite. Expected outcomes are the claim table’s exact categories/content/media/tags/request counts and zero-egress refusals.

**Regression fence:** `crates/resourcefs-core/tests/source_resource_content_type_contract.rs`; `crates/resourcefs-sources/tests/jira_wire_contract.rs`; `jira_render_contract.rs`; `atlassian_jira_adapter_contract.rs`; extended `compiled_sources_contract.rs`; `resourcefs-core/tests/architecture_contract.rs`; existing substrate contracts as positive controls.

**Named mutation:** C2 route an unknown Site ID to the first mount; C3 build canonical identity from the requested alias; C4 default absent `fields`; C5 reverse canonical object order; C6 skip unsupported ADF nodes; C7 reverse rendered field order; C8 accept orphan 304; C9 bypass `BoundedRead`; C10 let unknown sites reach the first origin; C11 accept Field selectors and label selected Markdown text/plain; C12 import provider/serde types into core; C14 synthesize fields from extra `names`; C16 use a guessed key-specific endpoint. Every mutation must turn its named row red, then restore green.

**Complexity/production scale:** Mount validation is $O(s)$ expected for `s ≤ MAX_CONFIGURATION_ENTRIES`, once; site lookup is $O(1)$ expected. Strict decode/ADF render is $O(b)$ plus $O(\\sum k_i \\log k_i)$ object sorting and $O(f \\log f)$ field ordering for `b ≤ 8 MiB`; one logical read retains a bounded response/cache/projection constant multiple and sends at most two GET attempts. Explicit maxima: mount validation ≤5 ms; 8 MiB canonical JSON ≤2 s release; 8 MiB ADF ≤2 s release; assembled Adapter-local CPU ≤4 s; network wall ≤30 s immutable logical deadline.

**Wall budget/phase:** mount validation is one-off with no wall budget; direct read/search is always-on with 30 seconds total from `HttpCeilings`, including retry wait/attempts, and 4 seconds Adapter-local CPU at the body ceiling.

**CI measurement follow-up (`rfs-a3ag`, 2026-09-07):** Windows run `34082162966` exceeded the five-millisecond guard while duplicate run `34082163611` passed identical source. The fixture now captures elapsed construction time before dropping the returned mount and its sole-owned HTTP substrate; teardown was incorrectly inside the measurement. The failure log did not capture elapsed time, so this is not claimed as the proven Windows cause. Per the requester's instruction to raise unstable budgets and investigate, the release CI wall-clock guard is 50 ms (debug remains 100 ms), with elapsed/budget diagnostics and unchanged maximum cardinality. The original five-millisecond CPU objective remains under investigation; no production deadline or validation behavior changes.

Local verification: Rust 1.98.0 formatting and strict Clippy passed; the complete assertion-disabled release `atlassian_jira_adapter_contract` target passed all 12 cases. Maximum Site Mount construction measured 1.077304 ms against the 50 ms CI guard (artifact 973). The isolated build target is disk-backed; native CI must pass on the pushed revision before merge.

**Files:** `crates/resourcefs-core/src/{resource.rs,lib.rs}`; `crates/resourcefs-core/tests/source_resource_content_type_contract.rs`; `crates/resourcefs-sources/src/atlassian/{mod.rs,jira.rs,wire.rs,render.rs}`; `crates/resourcefs-sources/src/{lib.rs,compiled.rs}`; all LSP-enumerated `CompiledSources::new` callsites in sources/MCP; `crates/resourcefs-sources/tests/{jira_wire_contract.rs,jira_render_contract.rs,atlassian_jira_adapter_contract.rs,compiled_sources_contract.rs}`; TLS/session support as required; architecture contracts.

**Estimate:** 20–30 hours.

**Diff estimate:** 5,400 changed lines.

**PR increment:** B — Complete direct Jira Source Adapter.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test source_resource_content_type_contract` → complete/selected MIME and full tags match direct byte/SHA-256 oracles; C11 media mutations red/restored green.
- `cargo test -p resourcefs-sources --test jira_wire_contract` → authority/duplicate/identity/canonical-value matrices agree item-by-item; C4/C5/C14 mutations red/restored green.
- `cargo test -p resourcefs-sources --test jira_render_contract` → ADF losslessness/warnings/atomic failures and Aggregate/index order/set facts agree; C6/C7/C14 mutations red/restored green.
- `cargo test -p resourcefs-sources --test atlassian_jira_adapter_contract` → mount, stable/key identity, media/tags, cache, retry, bounds, cancellation, redaction, zero-egress, compiled routing/catalog/search all agree; C2/C3/C8/C9/C10/C11/C16 mutations red/restored green.
- `cargo test -p resourcefs-sources --release --test jira_wire_contract canonical_json_maximum_fixture` and `cargo test -p resourcefs-sources --release --test jira_render_contract adf_maximum_fixture` → each 8 MiB fixture completes ≤2 seconds and remains lossless.
- `cargo test -p resourcefs-mcp --test architecture_contract` → core remains source-neutral and serde remains confined to provider wire modules; C12 core-provider mutation red/restored green.
- `cargo test -p resourcefs-sources --tests` and `cargo test -p resourcefs-mcp --tests` → all migrated constructor callsites and existing source/tool behavior remain green.

## Slice 4: Add the gated real Jira read smoke and final evidence seam

**Claim IDs:** C13

**Expected behavior:** An ignored `live_jira_issue_read` skips cleanly unless `RFS_LIVE=1` and required reader/fixture environment is present; when present it performs GET-only stable-ID, key-alias, Aggregate/index/Field, repeated-read, identity, media, tag, and redaction invariants without asserting moving counts or ETag presence.

**Oracle:** Real upstream stable IDs/shapes; direct Field-byte tag recomputation; stable/key canonical comparison; deterministic TLS permanent branch oracle; Rust test-harness list independently proves registration without credentials.

**Stress fixture:** Gate absent/partial/present; real fixture with ordinary JSON and ADF Fields; repeated read with ETag absent or present. Expected: clean secret-free skip, read-only success when configured, tenant-state-independent invariants.

**Regression fence:** `cargo test -p resourcefs-sources --test jira_live_smoke -- --list` contains `live_jira_issue_read: test`, plus the ignored row, deterministic Adapter suite, and `scripts/live-smoke.sh`/`cargo live` registration.

**Named mutation:** Rename `live_jira_issue_read` to `jira_issue_read`; harness-list fence loses the required `live_*` row and turns red without credentials; restore green.

**Complexity/production scale:** Fixed $O(1)$ direct reads/Fields; each response ≤8 MiB and logical read ≤30 seconds. Maximum six logical GET reads, each with at most one bounded retry, because the row proves shapes rather than tenant enumeration.

**Wall budget/phase:** N/A — one-off ignored live evidence; no always-on phase.

**Files:** `crates/resourcefs-sources/tests/jira_live_smoke.rs`, `scripts/live-smoke.sh` if explicit registration is required, `.cargo/config.toml` if the alias enumerates targets, `.rfs-pm0y/evidence.md` only when a real run occurs.

**Estimate:** 2–3 hours plus external credential availability.

**Diff estimate:** 300 changed lines.

**PR increment:** C — Live evidence.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test jira_live_smoke -- --list` → contains `live_jira_issue_read: test`; rename mutation removes it/red, restoration green.
- `cargo test -p resourcefs-sources --test jira_live_smoke -- --ignored` with gates absent → clean successful skip with no credential/account email.
- `RFS_LIVE=1 ... cargo test -p resourcefs-sources --test jira_live_smoke -- --ignored --nocapture` when external variables exist → stable/key identity, JSON/ADF media, byte tags, repeated-read, redaction invariants pass; record dated run in evidence.
- `cargo test -p resourcefs-sources --tests --no-run` → ignored live and deterministic suites compile without secrets.

## Slice 5: Apply accepted Jira review corrections

**Claim IDs:** C3, C4, C5, C6, C9, C10, C12, C14

**Expected behavior:** `JiraCodeReview-P1`/`JiraSpecReview-P5` preserve arbitrary-precision JSON numbers through canonicalization; `JiraCodeReview-P2`/`JiraSpecReview-P6` scope ADF rendering to Aggregate or the requested ADF Field so unrelated malformed ADF cannot poison Field/index reads; the post-fix depth review caps recursive JSON at 128 containers; `JiraSpecReview-P1` uses resolved stable identity for SearchRecords; `JiraSpecReview-P3` rejects every direct-issue redirect; `JiraSpecReview-P4` redacts redirected/policy URLs. The proposed 1.2.0 bump is rejected here because `rfs-lzbn` owns publication after profile/Confluence read release; the premature version claim is removed.

**Oracle:** Exact decimal normalization table over original number lexemes; explicit 512-level nesting refusal; direct Field/index vs Aggregate malformed-ADF matrix; SearchRecord stable-reference assertion; TLS same-origin/out-of-origin redirect transcript and canary scan; verified future release ticket `rfs-lzbn`.

**Stress fixture:** High-precision decimal, exponent, negative zero, numeric equivalence; 512 nested arrays; malformed unrelated ADF beside valid ordinary Field; alias search; same-origin and forbidden-origin redirect carrying canary path. Expected: lossless normalized bytes, bounded `source_unavailable` depth refusal without stack overflow, Field/index success with Aggregate atomic failure, stable SearchRecords, `source_unavailable` for unexpected same-origin redirect, fixed redacted `permission_denied` for egress refusal.

**Regression fence:** `jira_wire_contract::{canonical_json_preserves_arbitrary_precision_numbers,excessive_json_depth_fails_without_stack_overflow}`; `atlassian_jira_adapter_contract::{ordinary_field_read_ignores_unrelated_malformed_adf_projection,compiled_registry_routes_and_advertises_mounted_jira,unexpected_redirects_fail_without_url_disclosure}` plus full Jira/source suites.

**Named mutation:** Restore f64 number parsing; disable the JSON depth guard; render the entire issue before resource selection; pass the requested alias to `search_document`; omit final-URL comparison; inject/preserve full URL error prose. Each must turn its corresponding review regression row red, then restore green.

**Complexity/production scale:** Lossless number parsing/canonicalization remains $O(b)$ within the existing 8 MiB body bound, caps recursive descent at 128, and avoids exponent-driven output expansion by scientific notation outside fixed decimal thresholds. Projection scoping performs no more work than Slice 3; redirects/search remain $O(1)$ identity checks. Existing 2-second wire, 4-second Adapter CPU, and 30-second logical wall maxima remain unchanged.

**Wall budget/phase:** always-on decode/read/search paths retain Slice 3’s 4-second Adapter-local CPU and 30-second total wall limits; no new phase.

**Files:** `crates/resourcefs-sources/src/atlassian/{wire.rs,render.rs,jira.rs}`, `crates/resourcefs-sources/src/compiled.rs`, `crates/resourcefs-sources/tests/{jira_wire_contract.rs,atlassian_jira_adapter_contract.rs}`, `.rfs-pm0y/plan.md`.

**Estimate:** 3–5 hours.

**Diff estimate:** 600 changed lines.

**PR increment:** C — Live evidence and review corrections.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test jira_wire_contract canonical_json_preserves_arbitrary_precision_numbers` → exact decimal table agrees; f64 mutation red/restored green.
- `cargo test -p resourcefs-sources --test jira_wire_contract excessive_json_depth_fails_without_stack_overflow` → 512-level input fails safely; disabled-depth mutation red/restored green.
- `cargo test -p resourcefs-sources --test atlassian_jira_adapter_contract ordinary_field_read_ignores_unrelated_malformed_adf_projection` → Field/index succeed and Aggregate fails atomically; unconditional-render mutation red/restored green.
- `cargo test -p resourcefs-sources --test atlassian_jira_adapter_contract compiled_registry_routes_and_advertises_mounted_jira` → alias search records use stable reference; alias-record mutation red/restored green.
- `cargo test -p resourcefs-sources --test atlassian_jira_adapter_contract unexpected_redirects_fail_without_url_disclosure` → same-origin redirect fails and forbidden redirect remains redacted; final-URL/error-prose mutations red/restored green.
- `cargo test --workspace --all-features` → assembled implementation and every migrated caller remain green.

## Tracker taxonomy

- Permanent non-goals: ACLI, a second HTTP client/policy, comment inlining, rendered HTML authority, Aggregate/alias mutation, Markdown-to-ADF conversion. Rationale: each contradicts the approved parent architecture or native-authority contract; no tracker ticket is appropriate.
- Intended future work already has verified owners: browse `rfs-h212`; comments `rfs-9m70`; JQL `rfs-nae2`; Confluence `rfs-wlft`; profile/probe/serve `rfs-ue44`; release proof `rfs-lzbn`; OAuth `rfs-flq5`; replacement `rfs-pdp5`; creation `rfs-ddpt`.

## Self-review

- [x] Initial slices assign every design claim C1–C16 exactly once; review-fix Slice 5 maps accepted findings to existing claims without changing the approved design.
- [x] All five slices contain all thirteen mandatory fields and explicit `N/A — reason` values where applicable.
- [x] Every claim receives its permanent fence in the implementing slice; review regressions add narrower fences and named mutations.
- [x] Every new loop records asymptotic cost, production ceiling, explicit maximum, and rationale; every always-on phase has a wall budget.
- [x] The 9,750-line projection is partitioned into three dependency-ordered, independently mergeable increments.
- [x] Every deferral is a permanent non-goal with rationale or cites a verified future-work ticket.
- [x] No slice is declared complete; `checkpointed-build` exclusively judges completion.
