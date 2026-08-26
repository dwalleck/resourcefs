# Plan: rfs-45ww

## Inputs and partition arithmetic

- Approved design: `.rfs-45ww/design.md`, requester approval `"Approve"`, 2026-08-25, no approved risks.
- Empirical inputs: `.rfs-45ww/evidence.md` and `.rfs-45ww/probe_github_api.py`; P1–P6 PASS.
- Slice diff estimates: 1,050 + 650 + 950 + 1,550 + 1,900 + 950 + 950 + 650 = **8,650 changed lines**.
- Churn margin: **25% = 2,163 lines** (rounded up). Rationale: two exported constructor/interface cutovers, exhaustive address matches across three crates, deterministic HTTP fixtures, and generated-looking wire matrices routinely add integration rows beyond first-pass estimates.
- Projected total: **10,813 changed lines**. This exceeds the 4,000-line review-size gate, so the work is partitioned into four independently mergeable increments.

### PR increment A — Typed foundations

Slices 1–3; projected 2,650 lines + 25% local margin = 3,313. Mergeable definition: core accepts typed GitHub references and every existing exhaustive caller fails closed for the not-yet-mounted variants, the HTTP substrate exposes safe generic metadata, and private wire decoding/config validation compile and pass their direct contracts. No runtime source advertises or mounts `issue://`/`pr://`, so the product remains behaviorally unchanged outside the new parser spellings and explicit unmounted/unsupported outcomes.

### PR increment B — Direct read adapter

Slice 4; projected 1,550 lines + 25% local margin = 1,938. Mergeable definition: a directly constructed `GithubSource` reads one Aggregate/Field/projection through the real bounded substrate and fake TLS upstream. It is not yet compiled/profile-mounted, so ordinary launches remain unchanged; direct tests prove policy, wire, render, projection, and error behavior.

### PR increment C — Bounded state and compiled integration

Slices 5–6; projected 2,850 lines + 25% local margin = 3,563. Mergeable definition: the direct adapter gains collection pagination, session cache, retry, listing, search, and limits, then `CompiledSources` can mount it explicitly in tests/callers. Profile launches still pass `None`, leaving operator configuration unchanged while every compiled/runtime caller is migrated atomically.

### PR increment D — Profile launch and acceptance

Slices 7–8; projected 1,600 lines + 25% local margin = 2,000. Mergeable definition: existing GitHub profile DTOs become a live checked mount with required/degraded behavior, and full direct/profile acceptance closes the issue. No subsequent increment is required.

## Slice 1: Add typed GitHub Path Reference families and canonical identities

**Claim IDs:** C1, C3
**Expected behavior:** Valid repository collections, Aggregates, Fields, PR projection collections/items, diff file indices, and numeric page selectors parse into exhaustive typed variants and round-trip canonically; invalid identities fail `invalid_reference`; canonical item paths use repository-scoped number rather than API object ID; every existing exhaustive caller is migrated and fails closed while GitHub is unmounted; the pre-existing GitHub profile configuration reuses the same validated repository newtype instead of retaining a second raw-string validator.
**Oracle:** Hand-authored canonical/invalid table plus P1 official/OpenAPI and `gh api` number-vs-ID evidence.
**Stress fixture:** ASCII-case variants, max owner/repository lengths, encoded delimiters, zero/overflow numeric IDs, extra segments, and a PR fixture whose issue `id`, PR `id`, and repository number all differ; only the number appears in the canonical path.
**Regression fence:** `crates/resourcefs-core/tests/github_reference_contract.rs`
**Named mutation:** In `crates/resourcefs-core/src/reference.rs`, accept zero numeric IDs or construct a PR path from wire object ID; the zero/identity rows must turn red.
**Complexity/production scale:** Parsing and canonical rendering are O(reference bytes) over the existing 64 KiB Path Reference ceiling; no collection loop. Maximum accepted local cost: 1 ms per 64 KiB reference on the contract fixture, because parsing occurs on every operation and performs only bounded segment/numeric validation.
**Wall budget/phase:** always-on; ≤1 ms per maximum-size reference, measured by the focused contract's repeated boundary row.
**Files:** `crates/resourcefs-core/src/reference.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/src/resource.rs`, `crates/resourcefs-core/src/read.rs`, `crates/resourcefs-core/src/discovery.rs`, `crates/resourcefs-core/src/mutation.rs`, `crates/resourcefs-core/tests/github_reference_contract.rs`, `crates/resourcefs-core/tests/path_reference_contract.rs`, `crates/resourcefs-core/tests/read_engine_contract.rs`, `crates/resourcefs-core/tests/discovery_engine_contract.rs`, `crates/resourcefs-sources/src/compiled.rs`, `crates/resourcefs-sources/src/configuration/github.rs`, `crates/resourcefs-sources/tests/compiled_sources_contract.rs`, `crates/resourcefs-sources/tests/configuration_contract.rs`, `crates/resourcefs-mcp/src/profile/convert.rs`.
**Estimate:** 2 days.
**Diff estimate:** 1,050 lines.
**PR increment:** A — Typed foundations.
**Commands and expected results:**
- `cargo test -p resourcefs-core --test github_reference_contract` → every valid/invalid/canonical row agrees with the independent table.
- Named mutation then the same command → the targeted zero-ID or object-ID row fails; restore then command returns green.
- `cargo test -p resourcefs-core --lib` → every existing family retains its parser/canonical behavior after exhaustive-match migration.
- `cargo test -p resourcefs-core --test github_reference_contract reference_parse_budget -- --exact --ignored` → repeated maximum-size parses average ≤1 ms and every result remains canonical.

## Slice 2: Deepen the bounded HTTP seam with safe headers and typed metadata

**Claim IDs:** C4
**Expected behavior:** Sources can send Accept, User-Agent, API-version, and If-None-Match headers and receive bounded ETag/Link/Retry-After/rate metadata, while authority-bearing/framing headers are rejected and credentials/address/TLS/redirect/body/cancellation behavior remains substrate-owned.
**Oracle:** Server-side request/header log plus existing DNS/address/TLS/redirect fixture counters.
**Stress fixture:** Duplicate/malformed headers; mixed casing; maximum valid values; Authorization, Proxy-Authorization, Cookie, Host, Connection, Transfer-Encoding, Content-Length; redirect to forbidden host/private IP; secret sentinel positive control.
**Regression fence:** `crates/resourcefs-sources/tests/http_substrate_contract.rs` and existing `http_policy_contract.rs`, `http_tls_contract.rs`, `https_credential_contract.rs`.
**Named mutation:** Remove the forbidden-header check in `HttpRequest`; injected Authorization must replace/duplicate the substrate credential in the fake request log and turn the fence red.
**Complexity/production scale:** Request header validation is O(h + bytes) for a hard maximum of 16 retained non-secret headers and 16 KiB total values; response retention examines a fixed header-name set O(1). Maximum accepted local overhead: 1 ms per request, excluding network/body transfer.
**Wall budget/phase:** always-on; ≤1 ms request/response metadata overhead at the accepted header maxima.
**Files:** `crates/resourcefs-sources/src/http/mod.rs`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/tests/http_substrate_contract.rs`, `crates/resourcefs-sources/tests/http_policy_contract.rs`, `crates/resourcefs-sources/tests/https_credential_contract.rs`, `crates/resourcefs-sources/tests/support/tls.rs`.
**Estimate:** 1 day.
**Diff estimate:** 650 lines.
**PR increment:** A — Typed foundations.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test http_substrate_contract --features test-support` → safe headers arrive exactly once; forbidden headers fail before listener accept; typed response metadata matches fixture headers.
- `cargo test -p resourcefs-sources --test http_policy_contract --test http_tls_contract --test https_credential_contract --features test-support` → every existing policy/TLS/credential row remains green.
- Named mutation then focused substrate command → Authorization-override row fails; restore then command returns green.
- `cargo test -p resourcefs-sources --test http_substrate_contract http_metadata_budget --features test-support -- --exact --ignored` → 16 headers/16 KiB metadata processing averages ≤1 ms excluding network.

## Slice 3: Add private typed GitHub wire decoding and validated configuration identities

**Claim IDs:** C6, C17
**Expected behavior:** Source-private serde DTOs decode official issue/PR/comment/review/review-comment/file/error shapes, preserve documented optional/null states, ignore unknown fields, and reject missing required identities with bounded path-aware errors; profile/schema ownership remains MCP-only.
**Oracle:** Pinned official OpenAPI field matrix and architecture dependency table.
**Stress fixture:** Null body/author/commit/review ID, absent submitted_at/patch, unknown nested fields, duplicate IDs, wrong numeric/string types, missing number/title/id, Unicode Markdown, and an 8 MiB malformed JSON tail.
**Regression fence:** `crates/resourcefs-sources/tests/github_wire_contract.rs`, `crates/resourcefs-mcp/tests/architecture_contract.rs`.
**Named mutation:** Make `submitted_at` required or add serde default to required `number`; valid-null or missing-identity row turns red. Add `schemars` or `GithubSourceProfile` to sources; architecture fence turns red.
**Complexity/production scale:** JSON decode/validation is O(response bytes + collection items), bounded by 8 MiB per response and 100 objects per page. Maximum accepted local decode/validate time: 250 ms and retained wire+domain memory ≤24 MiB for one 8 MiB fixture; rationale is a conservative 3× input bound before rendered output replaces DTOs.
**Wall budget/phase:** always-on; ≤250 ms local decode/validate per maximum 8 MiB response, excluding network.
**Files:** `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/src/github/mod.rs`, `crates/resourcefs-sources/src/github/wire.rs`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/tests/github_wire_contract.rs`, `crates/resourcefs-mcp/tests/architecture_contract.rs`.
**Estimate:** 1.5 days.
**Diff estimate:** 950 lines.
**PR increment:** A — Typed foundations.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_wire_contract --features test-support` → every OpenAPI nullable/required identity row matches the hand table; malformed identity returns `source_unavailable`.
- `cargo test -p resourcefs-mcp --test architecture_contract` → serde/serde_json are permitted only as private source-wire dependencies; schemars/profile DTOs remain MCP-owned; core edges stay clean.
- Apply each named mutation and rerun its focused command → targeted row red; restore then green.
- `cargo test -p resourcefs-sources --test github_wire_contract github_wire_decode_budget --features test-support -- --exact --ignored` → 8 MiB decode/validation completes ≤250 ms and reports retained memory ≤24 MiB.

## Slice 4: Implement direct single-resource GitHub reads, projections, rendering, policy, and errors

**Claim IDs:** C2, C7, C8, C11
**Expected behavior:** Direct `GithubSource` reads allowlisted issue/PR Aggregates, title/body, stable comments/reviews/inline comments, unified diff, and indexed diff files through `HttpSubstrate`; output order/metadata/absence/mutability/version tags match spec; unallowlisted repositories make zero requests; status/header errors follow the machine-signal matrix.
**Oracle:** Hand-authored Markdown/type/status tables, fake TLS route/header log, and P2/P3/P5 official/live evidence.
**Stress fixture:** Unallowlisted but existing repo; issue and PR object IDs different from number; null body/author; pending review; nullable commit/review ID; binary file without patch; shuffled comments; Unicode; 401/ordinary403/signaled403/404/410/422/429/500/503/malformed JSON; identical error headers with different prose.
**Regression fence:** `crates/resourcefs-sources/tests/github_adapter_contract.rs` covering `repository_policy_precedes_egress`, `aggregate_render_is_stable_and_navigable`, `pr_projection_kinds_remain_distinct`, and `errors_match_typed_status_header_matrix`.
**Named mutation:** Move allowlist check after fetch; route review-comments through issue comments; sort only by timestamp; classify using `message.contains`; each named row must turn red independently.
**Complexity/production scale:** Endpoint construction O(path bytes); response validation/render O(response bytes); comment/review stable sort O(n log n) for n≤100 per response in this slice. Maximum accepted local work: 250 ms per 8 MiB response and ≤24 MiB transient memory, inherited from Slice 3.
**Wall budget/phase:** always-on; network plus local work remains within the configured ≤30 s HTTP timeout, with ≤250 ms local decode/render at maximum response size.
**Files:** `crates/resourcefs-sources/src/github/mod.rs`, `crates/resourcefs-sources/src/github/render.rs`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/tests/support/mod.rs`, `crates/resourcefs-sources/tests/support/tls.rs`, `crates/resourcefs-sources/tests/support/github.rs`, `crates/resourcefs-sources/tests/github_adapter_contract.rs`.
**Estimate:** 2.5 days.
**Diff estimate:** 1,550 lines.
**PR increment:** B — Direct read adapter.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_adapter_contract --features test-support` → exact route, request-count, render golden, projection, identity, absence, mutability, Version Tag, and error-matrix outcomes agree item-by-item with oracles.
- Apply each named mutation and run the specifically named test → only its behavior row goes red; restore then full direct contract green.
- `cargo test -p resourcefs-sources --test github_adapter_contract github_single_resource_render_budget --features test-support -- --exact --ignored` → maximum response decode/render completes ≤250 ms within the shared logical deadline.

## Slice 5: Add bounded pagination, session cache, one-deadline retry, listing, and layered bounds

**Claim IDs:** C5, C9, C10, C12, C20, C21
**Expected behavior:** Collection/Aggregate fetches follow validated Link targets verbatim for at most 10×100 objects, return numeric source continuations, remain atomic on page failure, implement quota-charged ETag session cache without stale fallback/copies, retry at most once inside one cancellable deadline, and produce correct all-state filtered/sorted/empty listings.
**Oracle:** Fake request/accept/header log, arithmetic `ceil(n/100).min(10)`, hand-sorted listing table, independent session byte/object ledger, paused clock, and P4/P6 Link/ETag evidence.
**Stress fixture:** 0/1/100/101/1,000/1,001 mixed issue/PR rows; timestamp ties; opaque cursor Link; cross-host and same-host-other-repo Links; page failures 2/10; ETag miss/304/changed/no-tag/failure; replacement smaller/larger; exact quota/quota+1; two sessions/disconnect; transient failure; Retry-After 0/equal/over deadline; cancellation during wait/retry; 8 MiB response and 64 MiB logical Resource boundary.
**Regression fence:** `crates/resourcefs-sources/tests/github_cache_contract.rs`, pagination/listing/retry rows in `github_adapter_contract.rs`, and core session cache unit/integration tests.
**Named mutation:** Follow Link without repo validation; loop through page 11; return partial pages; return stale cache on error; refresh retry timeout; fail to filter `pull_request` issue rows; clone cached content on 304. Each corresponding fence must turn red.
**Complexity/production scale:** Pagination O(p+n), p≤10 and n≤1,000 per logical collection; stable sort O(n log n); cache lookup average O(1), put O(content bytes); retained content is bounded by 64 MiB per logical Resource and 256 MiB/1,000 objects per Path Session. Maximum accepted network phase: configured logical deadline ≤30 s total. Maximum local post-network work: 500 ms for 1,000 objects/64 MiB and no full content allocation on 304 hit.
**Wall budget/phase:** always-on; complete first attempt, wait, retry, and pages within configured ≤30 s; local pagination/render/cache overhead ≤500 ms at maxima.
**Files:** `crates/resourcefs-core/src/session.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/tests/session_cache_contract.rs`, `crates/resourcefs-sources/src/github/mod.rs`, `crates/resourcefs-sources/src/github/render.rs`, `crates/resourcefs-sources/tests/github_adapter_contract.rs`, `crates/resourcefs-sources/tests/github_cache_contract.rs`, `crates/resourcefs-sources/tests/support/github.rs`, `crates/resourcefs-sources/tests/support/tls.rs`.
**Estimate:** 3 days.
**Diff estimate:** 1,900 lines.
**PR increment:** C — Bounded state and compiled integration.
**Commands and expected results:**
- `cargo test -p resourcefs-core --test session_cache_contract` → replacement/quota/isolation/disconnect ledger agrees exactly.
- `cargo test -p resourcefs-sources --test github_cache_contract --test github_adapter_contract --features test-support` → pagination counts/continuations/atomicity, Link confinement, cache semantics, retry clock, bounds, listing filter/order/empty outcomes agree with independent oracles.
- Apply each named mutation and rerun its named row → red; restore then both focused binaries green.
- `cargo test -p resourcefs-sources --test github_cache_contract github_pagination_cache_budget --features test-support -- --exact --ignored` → 1,000 objects/64 MiB complete local pagination/render/cache work ≤500 ms, total logical network time ≤30 s, and 304 hits preserve Arc content identity.

## Slice 6: Integrate search, common recovery, compiled dispatch, catalog, and clean cutover

**Claim IDs:** C14, C15, C16
**Expected behavior:** `rfs_search` searches the exact rendered explicit GitHub target through existing regex/PCRE2/case rules and preserves source/artifact continuations; generic read selectors/limits/recovery remain authoritative; mounted compiled sources advertise and route issue/pr, unmounted ones do not, and every GitHub mutation fails `unsupported_mutation`; all constructor callers migrate with no alias.
**Oracle:** Hand-counted rendered-line match table, reconstructed artifact bytes, static compiled registry table, and pre-change snapshots for existing source families.
**Stress fixture:** Aggregate body/comment matches but not title; case variants; PCRE2-only pattern; invalid pattern; no matches; 1,001 source objects; line/raw selectors; lower byte/line/column caps; failed spill; mounted/unmounted catalog; write/edit; every existing address family.
**Regression fence:** `github_adapter_contract::search_uses_rendered_resource_and_preserves_continuation`, `github_adapter_contract::github_uses_common_read_search_recovery`, `compiled_sources_contract::github_mount_dispatch_and_read_only_contract`, plus updated core/MCP contract callers.
**Named mutation:** Search title only; pre-truncate in source; route Issue to HTTPS; permit mutation; keep an old constructor call. Focused search/recovery/dispatch/build diagnostics must turn red.
**Complexity/production scale:** Search is O(rendered bytes × engine cost) under existing pattern limits and a ≤64 MiB logical Resource; compiled dispatch O(1); catalog O(source count), fixed compiled registry. Maximum accepted local search/render time: 1 s for 64 MiB deterministic literal/regex fixtures; PCRE2 retains existing engine limits.
**Wall budget/phase:** always-on; local search ≤1 s at maximum logical Resource, plus any GitHub read inside the Slice 5 ≤30 s deadline.
**Files:** `crates/resourcefs-core/src/discovery.rs`, `crates/resourcefs-sources/src/compiled.rs`, `crates/resourcefs-sources/src/catalog.rs`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/tests/compiled_sources_contract.rs`, `crates/resourcefs-sources/tests/github_adapter_contract.rs`, `crates/resourcefs-sources/tests/support/mod.rs`, `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-sources/tests/filesystem_resource_limits_contract.rs`, `crates/resourcefs-sources/tests/https_credential_contract.rs`, and every LSP-reported `CompiledSources::new` caller.
**Estimate:** 2 days.
**Diff estimate:** 950 lines.
**PR increment:** C — Bounded state and compiled integration.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_adapter_contract --test compiled_sources_contract --features test-support` → match lines/references/continuations, recovery reconstruction, catalog mount state, exhaustive routing, and read-only errors match oracles.
- `cargo test -p resourcefs-mcp --test tool_contract --test cli_contract` → generic tool rendering/descriptions remain truthful and complete after constructor migration.
- Apply each named mutation; targeted test or compile check red; restore then commands green.
- `cargo test -p resourcefs-sources --test github_adapter_contract github_search_budget --features test-support -- --exact --ignored` → a 64 MiB literal/Rust-regex fixture completes local search ≤1 s; PCRE2 retains its existing engine ceiling.

## Slice 7: Mount checked GitHub profiles and prove required/degraded lifecycle

**Claim IDs:** C13, C18
**Expected behavior:** One validated GitHub source resolves its credential at launch, uses default/enterprise/private-network policy, binds cache after Path Session creation, fails required unavailability, visibly degrades optional unavailability, participates in explicit profile check, and performs no ordinary-read lifecycle probe or credential disclosure.
**Oracle:** Strict profile JSON matrix, fake probe/request/header counters, process exit/report, and credential sentinel scan with positive authenticated control.
**Stress fixture:** No GitHub source; one valid source; duplicate source; case-duplicate repo; default and enterprise base path; public/private loopback grant; env/command credential success/failure/non-UTF8/oversize; required/optional unavailable; non-GitHub adapter remains available; normal repeated read probe count unchanged.
**Regression fence:** `crates/resourcefs-mcp/tests/cli_contract.rs`, `crates/resourcefs-mcp/tests/profile_contract.rs`, profile model unit tests, and `crates/resourcefs-sources/tests/github_adapter_contract.rs` mount binding rows.
**Named mutation:** Omit `github` from compiled kinds; bind source before Path Session; accept two source claims; resolve credential in read; probe each read. The corresponding launch/isolation/request-count row must turn red.
**Complexity/production scale:** Profile validation O(repositories), 1≤n≤4,096; one-off credential resolution/probe retains existing command/output/time ceilings. Ordinary mounted read adds O(1) source dispatch only. Maximum static validation: 50 ms for 4,096 repositories; launch probe remains configured ≤30 s network ceiling.
**Wall budget/phase:** profile validation/mount/probe is one-off — no recurring wall budget; ordinary dispatch overhead ≤1 ms.
**Files:** `crates/resourcefs-mcp/src/profile/model.rs`, `crates/resourcefs-mcp/src/profile/mod.rs`, `crates/resourcefs-mcp/src/profile/check.rs`, `crates/resourcefs-mcp/src/launch.rs`, `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-sources/src/github/mod.rs`, `crates/resourcefs-mcp/tests/cli_contract.rs`, `crates/resourcefs-mcp/tests/profile_contract.rs`, profile model unit tests.
**Estimate:** 2 days.
**Diff estimate:** 950 lines.
**PR increment:** D — Profile launch and acceptance.
**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test profile_contract --test cli_contract` → static schema, required/degraded launch, credential redaction, and all non-GitHub source behavior match process/profile oracles.
- `cargo test -p resourcefs-mcp --lib profile::model::tests::github_config_reaches_launch_components -- --exact` → checked GithubConfig survives conversion into launch composition.
- Apply each named mutation and run the named row → red; restore then focused contracts green.
- `cargo test -p resourcefs-mcp --lib profile::model::tests::github_profile_validation_budget -- --exact --ignored` → 4,096 repository entries validate ≤50 ms.

## Slice 8: Close direct-adapter and profile acceptance with deterministic fake upstreams

**Claim IDs:** C19
**Expected behavior:** One deterministic suite exercises the real parser, session, GitHub adapter, HTTP substrate, compiled dispatch, and profile launch for every rfs-45ww acceptance criterion without external DNS/live mutation or policy bypass.
**Oracle:** Fake listener's exact method/path/safe-header/body/request-count ledger plus spec-derived Resource/error goldens; live empirical probe is not called by permanent tests.
**Stress fixture:** Full matrix assembled from prior slices: allowlist denial, Aggregate/Fields, all PR projection kinds, 1,001 pagination, ETag, retry/cancel, limits/recovery/search, required/degraded profile, credential sentinel, malformed upstream, and zero non-GET requests.
**Regression fence:** Complete `crates/resourcefs-sources/tests/github_adapter_contract.rs` plus `crates/resourcefs-mcp/tests/github_profile_contract.rs` acceptance rows.
**Named mutation:** Construct a direct reqwest client in `github.rs` or bypass typed parser with a raw URL; injected resolver/route ledger must miss or observe unauthorized egress and the full acceptance command turns red.
**Complexity/production scale:** N/A — reason: this slice adds deterministic test composition and fixtures, not a production loop; production bounds are owned and measured in Slices 5–7.
**Wall budget/phase:** N/A — reason: one-off test phase; no production wall budget.
**Files:** `crates/resourcefs-sources/tests/github_adapter_contract.rs`, `crates/resourcefs-sources/tests/support/github.rs`, `crates/resourcefs-sources/tests/support/tls.rs`, `crates/resourcefs-mcp/tests/github_profile_contract.rs`, test-only fixtures under `crates/resourcefs-sources/tests/fixtures/github/`.
**Estimate:** 1.5 days.
**Diff estimate:** 650 lines.
**PR increment:** D — Profile launch and acceptance.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_adapter_contract --features test-support` → every direct acceptance row agrees with request/resource/error oracles; listener records only expected GETs and zero external route use.
- `cargo test -p resourcefs-mcp --test github_profile_contract` → profile-launched acceptance agrees with the same fixture truth table and lifecycle outcomes.
- Apply direct-client/parser-bypass mutation → at least one policy/request-ledger row red; restore then both commands green.

## Tracker taxonomy

- Intended future work: rfs-by2z owns all GitHub mutation and Creation Targets; verified open.
- Intended future work: rfs-ww6w owns MCP Resource/template mirroring; verified open.
- Permanent non-goals for this plan: labels/assignees/milestones/reactions/reviewer/check/statistic Aggregate expansion; network glob/completion/cross-repository search; stale/disk cache; more than one retry; message matching; live mutation tests; GraphQL; a second HTTP client/policy layer. Rationale: each is absent from the approved behavior and would add authority, response shape, or failure modes not needed to satisfy rfs-45ww.

## Self-review
- [x] Partition arithmetic is 8,650 + 2,163 = 10,813 lines; four dependency-ordered increments are independently mergeable and each slice names one.
- [x] C1–C21 are each assigned to exactly one slice; every PENDING design falsifier is discharged by its owning slice's exact command/oracle/fence.
- [x] Every slice records all thirteen mandatory fields; conditional fields use `N/A — reason`.
- [x] Every claim's permanent fence and named mutation are created/applied in its owning slice; no fence is postponed and no risk waiver exists.
- [x] Every new loop states asymptotic behavior, production maxima, resulting bounds, and an explicit measurable maximum cost; every always-on phase has a wall budget.
- [x] Partition arithmetic is 8,450 + 2,113 = 10,563 lines; four dependency-ordered increments are independently mergeable and each slice names one.
- [x] Every intended-future-work item cites a verified tracker ID; every permanent non-goal carries rationale.
- [x] No slice is declared complete; `checkpointed-build` exclusively owns slice completion and per-slice gates.
