# Plan: rfs-pm0y

## Inputs and partition

- Approved design: `.rfs-pm0y/design.md`, requester words “Approve design”, 2026-08-29, no accepted risks.
- Route/evidence: Empirical; `.rfs-pm0y/evidence.md` has P1–P5 `PASS`; C16 is the completed cheapest falsifier and every other claim is assigned below exactly once.
- Projected changed lines: Slice 1 `1,000` + Slice 2 `900` + Slice 3 `1,300` + Slice 4 `1,500` + Slice 5 `2,200` + Slice 6 `300` = `7,200`.
- Churn margin: `25% = 1,800` lines. Rationale: exhaustive `ResourceAddress` migration and deterministic TLS matrices routinely add callsite/test-support rows after the first count.
- Review-size projection: `7,200 + 1,800 = 9,000`, above the 4,000-line gate; use three independently mergeable increments.

### PR increment A — Typed and credential foundations

Slices 1–2. Mergeable definition: core recognizes and rejects the complete direct Jira grammar, Basic credentials compose only at egress, and validated site sets exist behind the not-yet-exported Atlassian module. Core/HTTP/mount contracts pass without native Jira decoding or network reads.

### PR increment B — Native authority and deterministic projections

Slices 3–4. Mergeable definition: private Atlassian wire/render modules strictly decode committed production-shaped fixtures, canonicalize JSON/ADF, and render deterministic Aggregate/index content. They remain behind crate-private interfaces; no Jira namespace is advertised or sent over the network. Native-wire/render contracts verify without Adapter wiring.

### PR increment C — Read Adapter and evidence

Slices 5–6. Mergeable definition: the public multi-site Atlassian Source Adapter, compiled dispatch/catalog, shared HTTP/cache behavior, deterministic TLS matrix, and ignored live row work end to end. It depends only on increments A and B and is the first increment that advertises usable direct Jira references.

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

## Slice 2: Confine Basic credentials and validate multi-site authority

**Claim IDs:** C2, C15

**Expected behavior:** `OriginCredential::basic` validates the account identifier, composes `Basic base64(email:token)` while the token is exposed only inside the credential seam, and remains redacted. A crate-private Atlassian site/mount model accepts one or more distinct Site IDs/canonical HTTPS origins and rejects empty, duplicate, or ambiguous authority before request construction.

**Oracle:** Independent `printf email:token | base64` bytes for Authorization; independently normalized `(scheme, host, effective port)` origin tuples and Site-ID sets for mount validation.

**Stress fixture:** Empty/colon/control/over-ceiling email; empty/valid canary token; one and maximum site count; duplicate IDs, case/port-normalized duplicate origins, same host different ports, userinfo/path/query/fragment/non-HTTPS URLs. Expected: exact Basic header only for valid input, no canary in Debug/errors/output, duplicate authority rejected.

**Regression fence:** `crates/resourcefs-sources/tests/http_substrate_contract.rs::basic_credential_composition`; new source-local `atlassian_mount_contract` test or equivalent integration target.

**Named mutation:** From C2, index mounts by insertion position instead of validated Site ID; duplicate-authority row turns red. From C15, encode token without `email:` in `OriginCredential::basic`; captured-header row turns red. Restore each and confirm green.

**Complexity/production scale:** Site validation is $O(s)$ expected with two sets for `s ≤ MAX_CONFIGURATION_ENTRIES`; Basic composition is $O(e+t)$ once per credential for bounded email/token bytes. Resulting retained authority is linear and capped by existing configuration ceilings. Explicit maximum accepted CPU cost: 5 ms for the maximum site set and 16 KiB credential composition, because this is launch-time work.

**Wall budget/phase:** N/A — one-off mount/credential construction; no wall budget.

**Files:** root `Cargo.toml`/`Cargo.lock` only if a direct base64 dependency is required, `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/src/http/mod.rs`, `crates/resourcefs-sources/src/atlassian/mod.rs`, `crates/resourcefs-sources/src/lib.rs` only when the later public export can remain non-dormant, `crates/resourcefs-sources/tests/http_substrate_contract.rs`, and a focused mount contract test.

**Estimate:** 3–4 hours.

**Diff estimate:** 900 changed lines.

**PR increment:** A — Typed and credential foundations.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test http_substrate_contract basic_credential` → captured Authorization equals the independent base64 oracle; invalid identifiers fail locally; canaries are absent; C15 mutation red/restored green.
- `cargo test -p resourcefs-sources --test atlassian_mount_contract` → distinct sites route by typed ID; empty/duplicate/invalid authority fails; C2 mutation red/restored green.

## Slice 3: Strictly decode Jira authority and canonicalize Field JSON

**Claim IDs:** C4, C5

**Expected behavior:** A private duplicate-preserving recursive JSON decoder admits vendor-optional shapes but validates every ResourceFS authority member after decode; direct issue identity, self URL, present fields, names, and schemas become typed source-local values or one atomic `ResourceError`. Every ordinary field canonicalizes to compact recursively sorted JSON bytes that distinguish all native value kinds.

**Oracle:** A hand-authored required-authority presence table plus a separate test-only canonical writer over fixture literals; the oracle never imports the production decoder/canonicalizer.

**Stress fixture:** Full required/null/wrong-type matrix; duplicate known top-level members; duplicate nested field members; empty/single/multi maps; extra metadata; foreign/wrong-family/wrong-ID self URLs; reordered Unicode-key objects; null/empty/string/array/object/numeric distinctions; 8 MiB near-ceiling nested data. Expected: complete valid shapes become one typed issue, every ambiguity fails atomically, canonical equivalents share bytes/tags, distinct values do not.

**Regression fence:** `crates/resourcefs-sources/tests/jira_wire_contract.rs` with source-local test-support inspection only.

**Named mutation:** From C4, default absent `fields` to an empty map; authority presence matrix turns red. From C5, emit object members in input order; reordered-equivalent row turns red. Restore each and confirm green.

**Complexity/production scale:** Strict decode is $O(b)$ and canonical encoding is $O(b + \sum k_i \log k_i)$ for response bytes `b ≤ MAX_HTTP_FETCH_BYTES = 8 MiB` and object member counts `k_i`; peak retained memory is bounded to decoded authority plus one canonical Field at a time, no more than a small constant multiple of 8 MiB. Explicit maximum accepted CPU cost: 2 seconds for the committed 8 MiB adversarial fixture in release mode, leaving the network deadline dominant while preventing an accidental quadratic writer.

**Wall budget/phase:** always-on for each successful issue response; 2 seconds at the 8 MiB hard ceiling, justified by the existing 30-second logical HTTP ceiling and single-issue scope.

**Files:** `crates/resourcefs-sources/src/atlassian/wire.rs`, `crates/resourcefs-sources/src/atlassian/mod.rs`, `crates/resourcefs-sources/tests/jira_wire_contract.rs`, and committed production-shaped JSON fixtures under `crates/resourcefs-sources/tests/fixtures/atlassian/`.

**Estimate:** 5–7 hours.

**Diff estimate:** 1,300 changed lines.

**PR increment:** B — Native authority and deterministic projections.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test jira_wire_contract authority_presence_matrix` → every presence/null/type/duplicate/identity cell matches the independent table and no partial value escapes; C4 mutation red/restored green.
- `cargo test -p resourcefs-sources --test jira_wire_contract canonical_json_roundtrip_and_distinction_matrix` → equivalent objects agree byte-for-byte; distinct native values remain distinct; C5 mutation red/restored green.
- `cargo test -p resourcefs-sources --release --test jira_wire_contract canonical_json_maximum_fixture` → 8 MiB fixture is lossless and completes at or below 2 seconds on the project verification host.

## Slice 4: Render lossless ADF, issue Aggregates, and Field indexes

**Claim IDs:** C6, C7, C14

**Expected behavior:** Complete ADF Fields retain the canonical whole tree and ADF media type; supported nodes/marks render stable Markdown, well-formed unsupported constructs emit typed source-position markers and warnings with authoritative Field references, and malformed known/root shapes fail atomically. Aggregate and Field index enumerate exactly present fields in stable ID order with names, types, immutability, references; Aggregate also renders readable values and hashes returned Markdown.

**Oracle:** Fixture-side tree walk/source-position list; set difference between raw field keys and rendered references; independently sorted IDs and direct SHA-256 of returned Markdown.

**Stress fixture:** Minimal empty ADF; nested paragraphs/headings/lists/code/blockquote/hard breaks/marks; unknown node and mark with children; malformed supported nodes/root; shuffled fields with duplicate names, extra metadata, empty/single/multi sets, Unicode names, mixed scalar/JSON/ADF values, and warning-bearing fields. Expected: no Field data loss, visible markers/warnings in source order, atomic malformed failure, exact field-set equality and sorted output.

**Regression fence:** `crates/resourcefs-sources/tests/jira_wire_contract.rs::adf_lossless_matrix`; new pure-render tests and the render rows in `atlassian_jira_adapter_contract.rs` if the Adapter fixture is not needed yet.

**Named mutation:** From C6, skip unknown ADF nodes; unsupported marker row turns red. From C7, iterate upstream order; shuffled field-order row turns red. From C14, iterate `names` instead of present `fields`; extra-metadata row turns red. Restore each and confirm green.

**Complexity/production scale:** ADF traversal/rendering is $O(n)$ nodes/bytes; field ordering is $O(f \log f)$ for `f` visible fields contained within the 8 MiB response; canonical Field bytes are produced once and reused. Explicit maximum accepted CPU cost: 2 seconds for an 8 MiB ADF/10,000-field adversarial fixture in release mode, preventing quadratic concatenation while respecting the one-issue operation bound.

**Wall budget/phase:** always-on for Aggregate/index reads; 2 seconds at the 8 MiB/10,000-field stress ceiling, same rationale as the CPU bound.

**Files:** `crates/resourcefs-sources/src/atlassian/render.rs`, `crates/resourcefs-sources/src/atlassian/wire.rs`, focused render tests or `crates/resourcefs-sources/tests/jira_wire_contract.rs`, `crates/resourcefs-sources/tests/atlassian_jira_adapter_contract.rs`, and ADF fixtures.

**Estimate:** 6–8 hours.

**Diff estimate:** 1,500 changed lines.

**PR increment:** B — Native authority and deterministic projections.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test jira_wire_contract adf_lossless_matrix` → complete trees retain every member; malformed rows fail atomically; C6 mutation red/restored green.
- `cargo test -p resourcefs-sources --test atlassian_jira_adapter_contract aggregate_orders_and_links_all_visible_fields` → field set/order/facts/content/tag agree item-by-item with the oracle; C7 mutation red/restored green.
- `cargo test -p resourcefs-sources --test atlassian_jira_adapter_contract field_index_uses_present_values_only` → empty/single/multi/extra-metadata rows match raw present keys; C14 mutation red/restored green.
- `cargo test -p resourcefs-sources --release --test jira_wire_contract adf_maximum_fixture` → adversarial maximum finishes at or below 2 seconds and preserves full authority.

## Slice 5: Wire the compiled read Adapter through shared HTTP and cache semantics

**Claim IDs:** C3, C8, C9, C10, C11, C12, C16

**Expected behavior:** A source-neutral validated `Utf8ContentType` and complete/selected constructors preserve JSON/vendor-JSON/Markdown media and full-resource Version Tags. Public `AtlassianSourceMount::bind` yields one multi-site `AtlassianSource` implementing `SourceAdapter`; `CompiledSources` accepts/routes the optional Adapter and advertises Jira only when mounted. Stable ID/key reads use one direct v3 GET, validate identity, and return stable canonical Aggregate/index/Field Resources. Aggregate/index selectors preserve Markdown/tag; Fields reject selectors. Invalid local inputs are zero-egress. ETag/cache/retry/status/bounds/cancellation/redaction behavior is inherited from shared modules.

**Oracle:** Server-side TLS request/send/flush counters, independently constructed canonical references/status table, direct full-content SHA-256, selector table, cache-generation observations, and architecture dependency/token scanner. C16 additionally compares the request contract to the approved evidence oracle.

**Stress fixture:** Valid stable/key requests; ID/self/site confusion; unknown site; unsupported/dormant path; every invalid identifier; Aggregate/index/Field selectors; first/conditional/304/changed/absent/oversized/orphan cache states; transport retry; delta/date Retry-After; deadline refusal; cancellation phases; body overflow; all mapped status classes; malicious prose and canary email/token; all pre-existing CompiledSources constructor call sites.

**Regression fence:** `crates/resourcefs-core/tests/source_resource_content_type_contract.rs`; `crates/resourcefs-sources/tests/atlassian_jira_adapter_contract.rs`; extended `compiled_sources_contract.rs`; core architecture contract; existing substrate contracts as positive controls.

**Named mutation:** C3 build canonical reference from requested alias; C8 accept orphan 304; C9 bypass `BoundedRead`; C10 resolve site after egress; C11 select a Field as typed JSON and separately label selected Markdown as text/plain; C12 import provider/serde types into core; C16 use a guessed key-specific endpoint. Each named mutation must turn its named deterministic row red, then restore green.

**Complexity/production scale:** Site lookup is $O(1)$ expected; one direct read sends at most two GET attempts and retains at most one 8 MiB response/cache body plus rendered output; selector work is $O(r)$ over rendered bytes. Explicit maximum accepted added CPU cost: 4 seconds for decode+render at the 8 MiB ceiling; network wall remains bounded by the immutable 30-second logical deadline and no Adapter phase may extend it.

**Wall budget/phase:** always-on direct read; 30 seconds total wall-clock maximum from existing `HttpCeilings`, including retry wait and both attempts; Adapter-local CPU sub-budget 4 seconds at maximum body size.

**Files:** `crates/resourcefs-core/src/resource.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/tests/source_resource_content_type_contract.rs`, `crates/resourcefs-sources/src/atlassian/{mod.rs,jira.rs,wire.rs,render.rs}`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/src/compiled.rs`, `crates/resourcefs-sources/src/catalog.rs` if catalog metadata requires the new entry, every `CompiledSources::new` callsite found through LSP references, `crates/resourcefs-sources/tests/atlassian_jira_adapter_contract.rs`, `crates/resourcefs-sources/tests/compiled_sources_contract.rs`, test TLS/support files, and architecture contracts.

**Estimate:** 8–12 hours.

**Diff estimate:** 2,200 changed lines.

**PR increment:** C — Read Adapter and evidence.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test source_resource_content_type_contract` → complete/selected MIME and full-content tags agree with direct byte/SHA-256 oracles; the C11 media mutation turns red and restoration returns green.
- `cargo test -p resourcefs-sources --test atlassian_jira_adapter_contract` → every request/identity/selector/cache/retry/status/bound/cancel/redaction/zero-egress row matches its independent oracle; named mutations C3/C8/C9/C10/C11/C16 each red then restore green.
- `cargo test -p resourcefs-sources --test compiled_sources_contract` → mounted Jira routes and catalog entry work; absent Adapter returns the stable not-configured error; all old source families still route.
- `cargo test -p resourcefs-core --test architecture_contract` → core remains source-neutral; C12 provider-import mutation turns red and restoration returns green.
- `cargo test -p resourcefs-sources --tests` → the completed Source Adapter and all pre-existing source contracts are green together.

## Slice 6: Add the gated real Jira read smoke and final evidence seam

**Claim IDs:** C13

**Expected behavior:** An ignored `live_jira_issue_read` test skips cleanly unless `RFS_LIVE=1` and required Atlassian reader/fixture environment is present; when present it performs GET-only stable-ID, key-alias, Aggregate/index/Field, repeated-read, identity, media, tag, and redaction invariants against a real site without asserting mutable counts or requiring ETag.

**Oracle:** Real upstream stable IDs and returned shape invariants; Field tag recomputed directly from returned bytes; stable/key canonical references compared; deterministic TLS remains the permanent branch oracle; the Rust test harness list independently proves registration without credentials.

**Stress fixture:** Gate absent/partial/present; real fixture with at least ordinary JSON and ADF Fields; repeated read with ETag absent or present. Expected: clean skip without secrets, read-only success when fully configured, stable invariants independent of tenant counts/timestamps/validator presence.

**Regression fence:** `cargo test -p resourcefs-sources --test jira_live_smoke -- --list` must contain `live_jira_issue_read: test`, plus the ignored row, deterministic Adapter suite, and inclusion in `scripts/live-smoke.sh`/`cargo live` discovery.

**Named mutation:** Rename `live_jira_issue_read` to `jira_issue_read`; the harness-list fence must no longer find the required `live_*` row and turn red without credentials. Restore the name and confirm the list/skip are green.

**Complexity/production scale:** Fixed $O(1)$ number of direct issue reads/Fields chosen from one fixture; each response remains under configured 8 MiB and each logical read under 30 seconds. Explicit maximum accepted requests: six GET logical reads, each with at most one bounded retry, because the row proves shapes rather than tenant enumeration.

**Wall budget/phase:** N/A — one-off ignored live evidence; no always-on phase.

**Files:** `crates/resourcefs-sources/tests/jira_live_smoke.rs`, `scripts/live-smoke.sh` only if explicit registration is required, `.cargo/config.toml` only if the existing `cargo live` alias enumerates tests, and `.rfs-pm0y/evidence.md` only when a real run occurs.

**Estimate:** 2–3 hours plus external credential availability for execution.

**Diff estimate:** 300 changed lines.

**PR increment:** C — Read Adapter and evidence.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test jira_live_smoke -- --list` → output contains `live_jira_issue_read: test`; the C13 rename mutation removes that row and turns the fence red, restoration returns green.
- `cargo test -p resourcefs-sources --test jira_live_smoke -- --ignored` with gates absent → test exits successfully after a clean skip and emits no credential/account email.
- `RFS_LIVE=1 ... cargo test -p resourcefs-sources --test jira_live_smoke -- --ignored --nocapture` when the external fixture variables are available → stable/key canonical identity, JSON/ADF media, byte-derived tags, repeated-read invariants, and redaction all pass; record the dated result in `.rfs-pm0y/evidence.md`.
- `cargo test -p resourcefs-sources --tests --no-run` → the ignored live row and deterministic suite compile together without requiring live secrets.

## Tracker taxonomy

- Permanent non-goals: ACLI, a second HTTP client/policy, comment inlining, rendered HTML authority, Aggregate/alias mutation, Markdown-to-ADF conversion. Rationale: each contradicts the approved parent architecture or native-authority contract; no tracker ticket is appropriate.
- Intended future work already has verified owners: browse `rfs-h212`; comments `rfs-9m70`; JQL `rfs-nae2`; Confluence `rfs-wlft`; profile/probe/serve `rfs-ue44`; release proof `rfs-lzbn`; OAuth `rfs-flq5`; replacement `rfs-pdp5`; creation `rfs-ddpt`.

## Self-review

- [x] Every design claim C1–C16 is assigned exactly once; C16 is already `PASS`, and every `PENDING` row is discharged by its owning slice.
- [x] All six slices contain all thirteen mandatory fields and explicit `N/A — reason` values where applicable.
- [x] Every claim receives its permanent fence in the implementing slice and carries the approved named mutation; no fence-less risk exists.
- [x] Every new loop records asymptotic cost, production ceiling, explicit maximum, and rationale; every always-on phase has a wall budget.
- [x] The 9,000-line projection is partitioned into three dependency-ordered, independently mergeable increments.
- [x] Every deferral is a permanent non-goal with rationale or cites a verified future-work ticket.
- [x] No slice is declared complete; `checkpointed-build` exclusively judges completion.
