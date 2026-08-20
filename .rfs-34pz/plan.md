# Plan: bounded reads and Path Session artifacts

## Approved inputs

- Route: Empirical (`route.md`).
- Behavior: requester-approved `spec.md`.
- Design: requester-approved `design.md` on 2026-08-19 with no accepted risks.
- Every design falsifier row is mechanically specific; C1 is PASS, C2–C16 name `checkpointed-build` slices as discharge owners, and no row is FAIL.

## Integration budget

| Slice | Diff estimate |
|---|---:|
| 1. Core selector contract, workflow evidence, and tracker state | 2,293 actual |
| 2. Streaming filesystem projection | 389 actual |
| 3. Path Session and durable artifact storage | 1,803 actual |
| 4. Artifact adapter and bounded read engine | 1,530 actual |
| 5. MCP rendering and lifecycle | 1,696 actual |
| **Sum** | **7,711** |
| **Churn margin** | **0 (implementation complete)** |
| **Actual total** | **7,711** |

Committed/staged reality replaces all five slices with 2,293, 389, 1,803, 1,530, and 1,696 changed lines respectively, including their required workflow records. Because 7,711 exceeds the exact 4,000-line threshold, the work remains partitioned into three independently mergeable PR increments.

### PR increment: core-selector-contract

Slice 1. Mergeable definition: typed workspace/artifact selector grammar and exact bounded-memory selection are complete and directly fenced, while every compiled Source Adapter keeps its prior output behavior and filesystem selector candidates remain unsupported. No unbounded selected output reaches MCP.

### PR increment: bounded-recovery-engine

Slices 2–4, stacked on `core-selector-contract`. Mergeable definition: filesystem selection, Path Session identity/quota/durable storage, Artifact Source Adapter, and common bounding/recovery engine land together; direct core/source fences prove every selected result is bounded before any MCP caller is wired.

### PR increment: recovery-mcp

Slice 5, stacked on `bounded-recovery-engine`. Mergeable definition: compiled-source routing, strict MCP schemas/rendering, and disconnect/cancellation fencing expose the complete issue behavior and compiled-process acceptance.

## Slice 1: Define exact typed selectors and bounded selection in core

**Claim IDs:** C1, C2, C3

**Expected behavior:** `PathReference` parses workspace/artifact addresses and the approved raw/line/artifact-page selector variants without losing requested spelling; exact UTF-8 line selection preserves LF/CRLF, terminators, order, and duplicates; core retains inward dependency direction.

**Oracle:** `.rfs-34pz/oracle_selectors.py` independently evaluates byte spans with Python `splitlines(keepends=True)` and validates the committed case matrix; Cargo metadata independently reports dependency direction.

**Stress fixture:** `crates/resourcefs-core/tests/fixtures/bounded_read_cases.json` includes empty text, CRLF, unterminated final lines, Unicode scalar boundaries, selector-looking path components, duplicate/out-of-order ranges, all EOF boundaries, 512/513-column lines, and 1,000 range atoms. Expected output is exact byte equality with the oracle and atomic failure for any invalid atom.

**Regression fence:** `crates/resourcefs-core/tests/path_reference_contract.rs` and `crates/resourcefs-core/tests/selector_golden_contract.rs` — created/extended in this slice.

**Named mutation:** Treat `:page:` as source-neutral or split a literal path before filesystem probing; normalize CRLF or deduplicate ranges. The corresponding fence must turn red, then green after restoration.

**Complexity/production scale:** Reference parsing is O(reference bytes). Selection performs two O(source bytes) validation/hash scans plus O(emitted bytes) seekable span reads, retaining at most 64 MiB selected content and fixed buffers/range state. The second scan rejects ordinary source-state changes rather than pairing selected bytes with a stale Version Tag. Production fixture: 256 MiB source, 64 MiB selected projection, up to 1,000 range atoms. Maximum accepted cost: at most 70 MiB retained selector memory and no second complete projection; rationale is the 64 MiB artifact-object ceiling plus bounded decoder/range bookkeeping.

**Wall budget/phase:** Always-on parse/selection phase: at most 10 seconds for the 256 MiB release stress source, at most 5 ms for a 4 KiB production-shaped reference, and at most 50 ms for the 64 KiB binary-max reference; rationale is bounded linear local work with no network I/O.

**Files:** `crates/resourcefs-core/src/reference.rs`, `crates/resourcefs-core/src/resource.rs`, `crates/resourcefs-core/src/source.rs`, `crates/resourcefs-core/src/selector.rs` (new), `crates/resourcefs-core/src/version.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/tests/path_reference_contract.rs`, `crates/resourcefs-core/tests/selector_golden_contract.rs` (new), `crates/resourcefs-core/tests/fixtures/bounded_read_cases.json` (new), `.rfs-34pz/oracle_selectors.py` (new).

**Estimate:** 1.5 days.

**Diff estimate:** 2,293 changed lines actual: 960 executable/test changes plus 1,333 required workflow evidence and tracker lines.

**PR increment:** core-selector-contract.

**Commands and expected results:**
- `cargo metadata --no-deps --format-version 1 | jq -e '([.packages[] | select(.name == "resourcefs-core") | .dependencies[].name] | all(. != "resourcefs-sources" and . != "resourcefs-mcp"))'` → `true`; adding a core→sources dependency changes it to false.
- `python .rfs-34pz/oracle_selectors.py crates/resourcefs-core/tests/fixtures/bounded_read_cases.json` → every case reports exact expected bytes/category and no mismatch.
- `cargo test -p resourcefs-core --test path_reference_contract --test selector_golden_contract` → literal precedence, grammar, and exact line bytes agree item-by-item with the oracle; each named mutation makes its owning test fail and restoration makes it pass.
- `cargo test --release -p resourcefs-core --test selector_golden_contract stress_selection_budget -- --exact` → 256 MiB scan finishes within 10 seconds and reports retained selected capacity no greater than 70 MiB.

### Slice 1 checkpoint result — PASS (2026-08-19)

- Impact analysis: `PathReference` had 30 references across core exports/resources/tests, `FilesystemSource`, adapter tests, MCP parsing/render tests, and `SourceAdapter`; all removed `literal()` callers migrated to typed `ResourceAddress`/`workspace_address()`. `SourceAdapter` itself had six references and its signature did not change in this slice.
- Helper decision: reused existing percent/path parsing, `sha2`, `url`, `std::io::{Read, Seek}`, and `ResourceError`; no new dependency or duplicate encoding/path/error helper was added. A seekable two-scan selector replaced the planned one-scan description because out-of-order/duplicate exact ranges cannot combine single-pass source stability with a 64 MiB memory bound; the plan budget was corrected before the checkpoint.
- Symmetry audit: workspace literal-first resolution remains primary and delays valid/malformed selector candidates until literal `not_found`; artifact references execute typed selectors directly. Both paths preserve `invalid_reference` for malformed grammar and `unsupported_projection` for page execution outside the read engine.
- Gate 1 — affected tests: PASS. `cargo test -p resourcefs-core --test path_reference_contract --test selector_golden_contract` passed all parser/selector cases; `cargo check --workspace --all-targets`, strict core Clippy, formatting, diagnostics, and Rust 1.88 checks passed.
- Gate 2 — PENDING falsifiers: PASS for C2/C3; the typed artifact/workspace grammar, literal-first page candidate, exact bytes, invalid UTF-8, past-EOF atomicity, order, and duplicates passed. C1: N/A — reason: its cheapest dependency-direction falsifier was already PASS in `design.md`.
- Gate 3 — stress fixture: PASS. The 256 MiB virtual source selected the exact 1 MiB final line; exact 64 MiB content succeeded and 64 MiB + 1 failed with narrowing guidance.
- Gate 4 — independent oracle: PASS. `oracle_selectors.py` matched all 21 committed success/error/syntax rows byte-for-byte/category-for-category.
- Gate 5 — production budget: PASS. Release 256 MiB selection completed in 0.99 seconds against the 10-second limit; 4 KiB and 64 KiB parser timing fences passed their 5 ms/50 ms limits; retained result content stayed beneath the 70 MiB bound.
- Gate 6 — regression fences: PASS. Core dependency metadata, `path_reference_contract`, and `selector_golden_contract` were green.
- Gate 7 — named mutations: PASS. Adding a core→sources dependency made the metadata predicate false; treating `:page:` as ordinary text made the page fence red; sorting/deduplicating spans made the ordered-duplicate fence red.
- Gate 8 — restored fences: PASS. All three mutations were restored and the dependency predicate plus selector suite returned green.

## Slice 2: Stream and version filesystem projections without weakening authority

**Claim IDs:** C6

**Expected behavior:** `FilesystemSource` streams an authorized source to compute the authoritative whole-file Version Tag while retaining only the exact selected projection, supports a narrow selection from a source above 64 MiB, and rejects output if root authority/generation changes before delivery.

**Oracle:** OS file SHA-256 and direct byte slicing compute full-source identity and selected content independently; the recorded pre/post root generations determine whether delivery is legal.

**Stress fixture:** A generated 256 MiB UTF-8 file has a unique CRLF/Unicode target near EOF. Selecting that range must return exact target bytes and the whole-file tag without retaining the whole source. A test gate starts a read, changes the root set, and releases delivery; expected outcome is `source_unavailable` and no content.

**Regression fence:** `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs` additions `filesystem_streams_narrow_large_range` and `stale_stream_delivery_is_rejected` — created in this slice.

**Named mutation:** Restore `.take(MAX_TEXT_BYTES + 1)` on source input or remove the final generation validation. The respective large-range/stale-delivery fence must turn red, then green after restoration.

**Complexity/production scale:** O(source bytes + selected bytes): two sequential validation/hash scans plus selected seekable span reads, maximum 256 MiB stress source and 64 MiB selected output. Maximum accepted cost: at most 70 MiB retained heap attributable to projection plus fixed I/O buffers; rationale is that source size must not determine memory and ordinary mid-read source changes must not pair stale selected bytes with a new Version Tag.

**Wall budget/phase:** Always-on local-filesystem read phase: at most 10 seconds for the 256 MiB release stress fixture; rationale is bounded local sequential I/O with no network work.

**Files:** `crates/resourcefs-core/src/resource.rs`, `crates/resourcefs-core/src/selector.rs`, `crates/resourcefs-core/tests/selector_golden_contract.rs`, `crates/resourcefs-core/tests/version_tag_contract.rs`, `crates/resourcefs-sources/src/filesystem.rs`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`.

**Estimate:** 1 day.

**Diff estimate:** 389 changed lines actual: 329 executable/test changes plus 60 workflow-plan lines.

**PR increment:** bounded-recovery-engine.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --all-features --test filesystem_adapter_contract` → narrow large-source selection equals direct fixture bytes, Version Tag equals independent whole-file hash, and a root-generation race returns no stale result.
- `cargo test --release -p resourcefs-sources --test filesystem_adapter_contract filesystem_streams_narrow_large_range -- --exact` → 256 MiB fixture completes within 10 seconds and its retained projection capacity stays within 70 MiB; restoring the old complete-file cap makes it fail, restoration makes it pass.
- `cargo test -p resourcefs-sources --all-features --test filesystem_adapter_contract stale_stream_delivery_is_rejected -- --exact` → removing final generation validation makes stale content observable and the test red; restoration makes it pass.

### Slice 2 checkpoint result — PASS (2026-08-19)

- Impact analysis: `SourceAdapter::read` had four language-server references (filesystem implementation, MCP server, direct source test, trait declaration) and kept its signature. `ReadResource::text` had seven references and remains the authoritative-complete-content constructor; the new tested `text_projection` constructor carries a whole-Resource Version Tag without copying the selected `String`.
- Helper decision: reused the core `select_utf8` engine, existing capability-open/final-path containment, `validate_read_delivery`, `ReadResource`, and `VersionTag`; no second selector/hash/path/error implementation was added. The dormant process-global environment gate was replaced with a feature-gated, per-source, one-shot test gate so parallel integration tests cannot block unrelated Sources.
- Symmetry audit: relative, canonical, absolute, and local-file workspace addresses all enter the same literal-first `read_address` path. Only literal `not_found` activates the parsed base plus selector; malformed candidates keep `invalid_reference`, workspace page candidates return `unsupported_projection`, and a concurrent authority change returns `source_unavailable` with no content.
- Gate 1 — affected tests: PASS. Core selector/version suites passed 16 tests; the all-feature filesystem adapter suite passed 24 tests; workspace all-target checking, strict Clippy, formatting, diagnostics, Rust 1.88, Windows GNU, and macOS cross-target checks passed.
- Gate 2 — PENDING falsifier C6: PASS. A narrow line from an exact 256 MiB source returned exact LF/CRLF/Unicode bytes and the authoritative whole-source tag; the deterministic delivery gate rejected a result after root replacement.
- Gate 3 — stress fixture: PASS. The sparse 256 MiB UTF-8 source retained only the 18-byte target plus fixed selector buffers; a complete read still failed at the pre-existing 48 KiB inline source limit.
- Gate 4 — independent oracle: PASS. Python `hashlib` independently computed `sha256:7340d5b10ba225d1a0dd49203cbdae47afc05a4937227033b728f881b32d4f5e` for the deterministic 268,435,456-byte fixture; Rust returned that exact tag and the directly specified target bytes.
- Gate 5 — production budget: PASS. The restored release fence completed the full 256 MiB fixture/oracle/selection test in 0.49 seconds against the 10-second limit; selected content was below the 70 MiB bound and no source-sized allocation exists in the projection branch.
- Gate 6 — regression fences: PASS. Core ownership/version contracts, all 24 direct filesystem adapter contracts, the complete workspace check, and cross-target builds were green.
- Gate 7 — named mutations: PASS. Restoring the full-source 48 KiB cap made `filesystem_streams_narrow_large_range` red; removing final `validate_read_delivery` made stale `"beta\n"` content observable and `stale_stream_delivery_is_rejected` red.
- Gate 8 — restored fences: PASS. Both mutations were restored; the exact release large-source fence and exact all-feature stale-delivery fence returned green.

## Slice 3: Add linearizable Path Session admission and lease-safe durable storage

**Claim IDs:** C7, C8, C9, C11

**Expected behavior:** one live Path Session owns opaque token-plus-counter artifact identities, immutable byte-identical deduplication, exact 64 MiB object/256 MiB session quotas, atomic durable writes, immediate invalidation, indistinguishable foreign/unknown/inactive lookup failure, and cleanup only for unlocked backing state aged at least 86,400 seconds.

**Oracle:** A sequential Python/session model computes content equality, quota, and identity outcomes; fake-storage call logs plus before/after directory SHA-256 establish publication atomicity; controlled timestamps and the independent platform-lock evidence in `evidence.md` establish cleanup outcomes.

**Stress fixture:** Barrier-started equal and distinct 64 MiB admissions at exact and one-byte-over quotas, a forced same-digest/different-bytes collision, two sessions both allocating object 1, every injected storage failure, TTL-1/exact/TTL+1 directories, a concurrently held cross-process lease, and an owner killed without a tombstone. Expected state is one charge for equal bytes, no oversubscription/eviction/partial record, no cross-session alias, and deletion only after both unlocked and old enough.

**Regression fence:** `crates/resourcefs-core/tests/path_session_contract.rs` (new) and `crates/resourcefs-sources/tests/session_storage_contract.rs` (new) — created in this slice.

**Named mutation:** Release the admission mutex before write; charge identical content twice; publish the final filename before write/sync; compare object ID without token; check age before lease acquisition; use `age > TTL` instead of `age >= TTL`. Each owning fence must turn red, then green after restoration.

**Complexity/production scale:** Artifact lookup/admission metadata is O(1) average. Normal dedupe is O(content bytes) only after digest match; synthetic collision comparisons are bounded by same-digest candidates. Atomic write is O(content bytes), capped at 64 MiB; session state is capped at 256 MiB and 1,000 records. Cleanup is O(session directories) once per startup/maintenance event. Maximum accepted cost: no more than 256 MiB authoritative stored bytes, 64 MiB + one page per admission buffer, and 1,000 record entries; rationale is the accepted binary quota contract.

**Wall budget/phase:** Admission is always-on: at most 5 seconds for one 64 MiB durable local write, including sync, on the release fixture. Heartbeat is always-on: each O(1) update at most 100 ms and never under the admission mutex. Cleanup is one-off at startup/explicit maintenance: N/A — reason: one-off phase; no wall budget.

**Files:** `crates/resourcefs-core/src/session.rs` (new), `crates/resourcefs-core/src/error.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/Cargo.toml`, `crates/resourcefs-core/tests/path_session_contract.rs` (new), `crates/resourcefs-sources/src/session_storage.rs` (new), `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/tests/session_storage_contract.rs` (new), `Cargo.toml`, `Cargo.lock`.

**Estimate:** 2 days.

**Diff estimate:** 800 changed lines: 430 implementation, 330 tests, 40 manifests.

**PR increment:** bounded-recovery-engine.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test path_session_contract` → concurrent admission matches the sequential model; equal content reuses identity, distinct content charges once, quota never exceeds 256 MiB, foreign/inactive IDs are `not_found`, and one-byte-over content makes no store call.
- `cargo test -p resourcefs-sources --test session_storage_contract` → failure injection leaves directory hashes/quota unchanged, cleanup outcomes match TTL/lease table, and no live lease is removed.
- `python .rfs-34pz/probe_session_lease.py` → native Linux and Windows/Wine observations still show contention while owner lives and acquisition after owner death; the published macOS compile/API oracle remains true.
- Under each named mutation, `cargo test -p resourcefs-core --test path_session_contract` or `cargo test -p resourcefs-sources --test session_storage_contract` → owning case fails; after restoration the same command agrees with the independent model/directory snapshot.

### Slice 3 checkpoint result — PASS (2026-08-19)

- Impact analysis: the new `SessionStorage` trait remains dependency-inverted in `resourcefs-core`; `PathSession` owns token authority, admission, deduplication, quota, and liveness while `DiskSessionStorage` owns cache paths, atomic files, platform leases, timestamps, and cleanup. No existing exported signature changed in this slice.
- Gate 1 — core contract: PASS. `cargo test -p resourcefs-core --all-features --test path_session_contract` passed 6 cases covering strict tokens/IDs, forced digest collision byte comparison, byte-identical reuse, exact concurrent quota, object/record ceilings before storage I/O, cancellation, and indistinguishable foreign/unknown/inactive lookup.
- Gate 2 — durable storage contract: PASS. `cargo test -p resourcefs-sources --all-features --test session_storage_contract` passed 6 cases covering every injected open/write/sync/persist/remove failure, unchanged directory SHA-256 snapshots and prior objects, heartbeat, exact TTL boundary, markerless live lease ordering, symlink/malformed entry containment, abnormal abandonment, and a 64 MiB durable write.
- Gate 3 — independent oracle: PASS. `.rfs-34pz/oracle_session.py` independently produced IDs `[1,2,1]` for a forced-collision alpha/beta/alpha sequence, 9 charged bytes and 2 stores, IDs `[1,2,3,4]` at the exact 268,435,456-byte quota, uniform `not_found` lookup outcomes, and the TTL-1/exact/TTL+1 plus locked table.
- Gate 4 — production budgets: PASS. The exact release 64 MiB durable-admission fence completed in 0.16 seconds under the 5-second bound; the exact release heartbeat fence completed in under the test runner's 0.01-second resolution under the 100 ms bound.
- Gate 5 — lease/platform evidence: PASS. `.rfs-34pz/probe_session_lease.py` again observed live-owner contention and post-kill acquisition on native Linux and Windows/Wine; macOS lock API cross-compilation remained true. All-feature source/session code cross-checked for `x86_64-pc-windows-gnu` and `x86_64-apple-darwin`.
- Gate 6 — named mutations: PASS. Charging identical content twice, token-only lookup, releasing admission before write, publishing before sync, and checking age before lease each turned its owning exact contract red; the `age > TTL` fence is the exact-boundary cleanup assertion. Every mutation was restored.
- Gate 7 — quality: PASS. `cargo fmt --all -- --check` and strict all-target/all-feature Clippy for core and sources passed; the session/storage contracts returned green after restoration.

## Slice 4: Route immutable artifacts through one bounded read engine

**Claim IDs:** C5, C12, C15, C16

**Expected behavior:** `ReadEngine` bounds workspace/artifact text under all effective limits, stores one complete selected projection when first output omits bytes, returns separate immutable recovery and progressing continuation references, reuses an existing artifact root for direct artifact pages, and routes typed addresses to exactly one adapter. Concatenated pages reproduce the selected projection byte-for-byte.

**Oracle:** `.rfs-34pz/oracle_selectors.py` computes selected bytes; an independent reconstruction driver follows returned continuations and compares concatenated structured content by length and SHA-256; a hand route table maps workspace forms only to filesystem and artifact forms only to artifact.

**Stress fixture:** Boundary/intersection pages for empty, 49,151/49,152/49,153-byte, 2,999/3,000/3,001-line, 512/513-column, Unicode-midline, CRLF, multi-range, and 64 MiB projections. Expected cursors always advance at UTF-8 scalar boundaries, root identity remains stable, the final page has no continuation, and direct artifact paging consumes no additional quota.

**Regression fence:** `crates/resourcefs-core/tests/read_engine_contract.rs` (new), `crates/resourcefs-sources/tests/artifact_adapter_contract.rs` (new), and `crates/resourcefs-sources/tests/compiled_sources_contract.rs` (new) — created in this slice.

**Named mutation:** Advance cursors by displayed characters rather than UTF-8 byte offsets; insert a newline between pages; return a page hash/cache path; expose an artifact mutation method; route raw selectors specially; collect/store one byte above the object ceiling. Each owning fence must turn red, then green after restoration.

**Complexity/production scale:** First workspace spill is O(complete selected bytes), capped at 64 MiB, plus O(page bytes), capped at 49,152 bytes/3,000 lines/512 scalars per displayed line. Direct artifact paging is bounded by one 64 MiB object read plus one page under the approved storage seam. Registry routing is O(1). Maximum accepted cost: at most 70 MiB retained read state and 5 seconds for a 64 MiB artifact page operation; rationale is the accepted object ceiling and the approved simple storage interface.

**Wall budget/phase:** Always-on read/paging phase: at most 5 seconds for a 64 MiB source/artifact in release tests and at most 25 ms for an inline 49 KiB artifact; rationale is one capped projection pass plus one bounded page pass.

**Files:** `crates/resourcefs-core/src/read.rs`, `crates/resourcefs-core/src/resource.rs`, `crates/resourcefs-core/src/session.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/tests/read_engine_contract.rs` (new), `crates/resourcefs-sources/src/artifact.rs` (new), `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/tests/artifact_adapter_contract.rs` (new), `crates/resourcefs-sources/tests/compiled_sources_contract.rs` (new).

**Estimate:** 1.5 days.

**Diff estimate:** 800 changed lines: 420 implementation, 380 tests.

**PR increment:** bounded-recovery-engine.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test read_engine_contract` → every boundary/intersection page stays within all limits, continuations make progress, concatenation length/SHA-256 equals the oracle projection, failed spill returns no reference, and repeated identical spill reuses identity/quota.
- `cargo test -p resourcefs-sources --test artifact_adapter_contract --test compiled_sources_contract` → artifact root/range/raw/page bytes match backing bytes/oracle, cache paths never appear, no mutation API exists, and exactly the address-selected adapter is called.
- `cargo test --release -p resourcefs-core --test read_engine_contract artifact_page_production_budget -- --exact` → a 64 MiB artifact page stays within 70 MiB retained state and 5 seconds; each named paging mutation breaks reconstruction and restoration returns exact equality.

### Slice 4 checkpoint result — PASS (2026-08-19)

- Impact analysis: `SourceAdapter::read` now returns complete authoritative `SourceResource`; the new `ReadEngine` alone owns selection paging, limit accounting, immutable spill/recovery creation, and post-source liveness. `CompiledSources` routes only on typed `ResourceAddress`, and the filesystem adapter now constructs source results without a second paging implementation.
- Gate 1 — bounded engine contract: PASS. `cargo test -p resourcefs-core --all-features --test read_engine_contract` passed 6 cases covering exact byte/line/column boundaries, CRLF and UTF-8 cursors, ordered and duplicate ranges, byte-identical reconstruction, immutable recovery roots, root reuse without quota growth, one-byte-over failure before storage, and inline/64 MiB budgets.
- Gate 2 — source adapters: PASS. The direct artifact and typed-registry suites passed 4 cases covering root/raw/range/page byte equality, whole-Resource Version Tags on every page, disconnect invalidation, nondisclosing malformed/foreign failures, absence of backing paths, and exactly-one address-family dispatch. The compile-fail doctest proves `ArtifactSource` has no mutation method.
- Gate 3 — independent oracle: PASS. `.rfs-34pz/oracle_selectors.py crates/resourcefs-core/tests/fixtures/bounded_read_cases.json` returned `{"checked": 21, "status": "ok"}`; contract reconstruction independently compared selected length and SHA-256 across every returned page.
- Gate 4 — production budgets: PASS. The exact release 64 MiB artifact-page fence completed in 0.13 seconds under the 5-second/70 MiB bounds; the all-feature inline maximum-page fence remained under 25 ms.
- Gate 5 — named mutations: PASS. Character-count cursors, inserted inter-page newlines, selected-page hashes, a public artifact write method, selector-based registry routing, and accepting one byte above the object ceiling each turned its owning exact contract red; every mutation was restored.
- Gate 6 — quality: PASS. Strict all-target/all-feature Clippy for core and sources, `cargo fmt --all -- --check`, the artifact doctest, and all focused recovery/adapter contracts passed after restoration.

## Slice 5: Wire strict MCP output and disconnect-safe Path Session lifecycle

**Claim IDs:** C4, C10, C13, C14

**Expected behavior:** the compiled stdio server exposes strict object-root `rfs_read` input/output schemas; rejects zero, malformed, unknown, or above-ceiling limits before source I/O; renders complete non-empty TextContent equivalent to structured content for success, empty, bounded, and operational-error cases; returns malformed inputs as MCP invalid params; and invalidates the Path Session on request cancellation or transport EOF/drop before any late result/artifact commit.

**Oracle:** An independent JSON-RPC driver parses schemas/results and reconciles structured fields with TextContent. A gated lifecycle event log proves `cancel/invalidate < release < attempted commit`; post-run cache snapshots and quota establish that no late artifact became reachable.

**Stress fixture:** Compiled-process calls cover unbounded, bounded first/middle/final page, empty success, every valid limits-presence and malformed/zero/above-ceiling limits shape, invalid UTF-8, invalid selector, quota/storage failures, a request canceled while storage is gated, and input EOF while a handler outlives rmcp's drain timeout. Expected malformed input is protocol invalid params with zero source calls; operational failures are complete non-empty tool errors; canceled/disconnected operations publish nothing.

**Regression fence:** `crates/resourcefs-mcp/tests/stdio_mcp.rs` additions for bounded output/schema/error/lifecycle, `crates/resourcefs-core/tests/text_limits_contract.rs` (new), and `crates/resourcefs-core/tests/path_session_contract.rs` cancellation additions — created in this slice.

**Named mutation:** Clamp an above-ceiling limit; omit continuation from TextContent; render an empty success as empty text; send malformed limits through the tool-error renderer; remove the final session/operation check before record insertion. Each owning compiled-process/direct lifecycle fence must turn red, then green after restoration.

**Complexity/production scale:** Rendering/serialization is O(page bytes), capped at 49,152 content bytes plus fixed metadata. Transport invalidation is O(1); each in-flight operation performs O(1) liveness checks around existing source/store work. Maximum accepted cost: at most 64 KiB additional render allocation and 25 ms render time for a maximum page; rationale is the fixed page ceiling and complete text fallback.

**Wall budget/phase:** MCP render and liveness checks are always-on: at most 25 ms per maximum page and 1 ms for invalidation itself in release fixtures. Startup cleanup/session creation is one-off: N/A — reason: one-off phase; Slice 3 owns its lease-gated scan.

**Files:** `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-mcp/src/lib.rs`, `crates/resourcefs-mcp/src/cli.rs`, `crates/resourcefs-mcp/Cargo.toml`, `crates/resourcefs-mcp/tests/stdio_mcp.rs`, `crates/resourcefs-core/src/read.rs`, `crates/resourcefs-core/src/session.rs`, `crates/resourcefs-core/tests/text_limits_contract.rs` (new), `crates/resourcefs-core/tests/path_session_contract.rs`, `Cargo.lock`.

**Estimate:** 1.5 days.

**Diff estimate:** 600 changed lines: 300 implementation, 300 tests.

**PR increment:** recovery-mcp.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test text_limits_contract` → all valid field-presence combinations preserve their exact values; zero, wrong-type, unknown, and above-ceiling members fail, and the clamp mutation makes the fence red before restoration.
- `cargo test -p resourcefs-mcp --test stdio_mcp` → independent driver finds object-root schemas, exact structured/text agreement, separate recovery/continuation references, invalid params for malformed limits, complete text for operational errors, and no late commit after EOF/cancel.
- `python .rfs-34pz/probe_rmcp_disconnect.py` → still demonstrates rmcp's bounded-drain behavior, while the compiled server lifecycle fence proves ResourceFS invalidates before late publication.
- `cargo test -p resourcefs-core --test path_session_contract request_cancel_prevents_commit -- --exact` → gated event order matches the oracle and no artifact/quota remains; removing the final liveness check makes it red, restoration makes it pass.
- `cargo run --quiet -p resourcefs-mcp --bin resourcefs -- --root tests/fixtures/workspace` driven by the stdio contract client → real process reads, spills, follows recovery/continuation, and returns byte-identical content with protocol stdout free of diagnostics.

### Slice 5 checkpoint result — PASS (2026-08-20)

- Impact analysis: `rfs_read` now validates a strict optional lower-limit object before root/source I/O, routes complete source projections through the common `ReadEngine`, renders recovery and continuation references in both output forms, and owns one persisted heartbeat-backed Path Session whose EOF/drop path invalidates synchronously before durable disconnect recording.
- Gate 1 — direct contracts: PASS. The 2-case `text_limits_contract`, 7-case `path_session_contract`, 25-case filesystem adapter suite, and 6-case session-storage suite cover the complete field-presence matrix, exact ceilings, cancellation cleanup order, complete selected source projections through the 64 MiB object boundary, stale-root rejection, and atomic storage failures.
- Gate 2 — compiled MCP seam: PASS. All 17 real-stdio cases passed, covering both required protocol revisions, object-root schemas, exact success/empty/error text, first/middle/final recovery pages, byte/line/column and UTF-8 reconstruction, 18 malformed/non-lower limit shapes rejected as MCP invalid params before a gated source read, storage failure, request cancellation with a live follow-up, EOF before gated release, and observable disconnect-marker failure.
- Gate 3 — lifecycle ordering: PASS. The compiled EOF case observed the durable `disconnected` marker while the spill-candidate read remained gated and found no object before or after release; the cancellation case retained connection authority for a follow-up read and published no artifact. `DisconnectState` uses one cancellation-safe `OnceCell` result for concurrent receive/close/finalization callers, and the always-on heartbeat is joined on every service terminal path.
- Gate 4 — independent premises: PASS. `.rfs-34pz/oracle_selectors.py` independently accepted all 21 selector rows. `.rfs-34pz/probe_rmcp_disconnect.py` reconfirmed rmcp 3.1.3 returns `Closed` after 5,002 ms before the gated handler completes and cancels its request context afterward; the ResourceFS transport fence supplies the earlier session invalidation.
- Gate 5 — production bounds: PASS. The maximum-page renderer stayed under its 25 ms fence with fixed page-scale allocations; the 256 MiB narrow-range filesystem stress case returned exact bytes without collecting the source; complete unselected projections cap at 64 MiB plus one sentinel byte; heartbeat and invalidation perform constant work outside artifact admission.
- Gate 6 — named mutations: PASS. Clamping one-over limits, omitting continuation from TextContent, clearing empty-success TextContent, accepting an empty-array limits shape, and removing the post-write commit guard each turned its owning exact contract red; restoration returned every focused and workspace contract to green.
- Gate 7 — quality and review: PASS. `cargo test --workspace --all-targets --all-features` passed 96 tests in 16 suites; `cargo fmt --all -- --check` and strict workspace/all-target/all-feature Clippy passed. Final review reported no P0–P2 correctness or security findings.

## Tracker taxonomy

- Strict profile-configurable lower ceilings/TTL are intended future work carried by verified issue `rfs-r9m6` and its rfs-34pz tracker note.
- Search/glob artifact reuse is intended future work carried by verified issue `rfs-vl0u`.
- Structural Summary selector consumers are intended future work carried by verified issues `rfs-m739`, `rfs-7eqb`, `rfs-n22g`, and `rfs-h7iq`.
- Session Scratch reuse is intended future work carried by verified issue `rfs-60g1`.
- MCP Resource mirroring is intended future work carried by verified issue `rfs-ww6w`.
- A global artifact registry, cross-session resolution, a persistent cleanup daemon, workspace `:page:`, artifact mutation, and arbitrary binary mutation are permanent non-goals for the rationales in approved `design.md`.

## Self-review

- [x] C1–C16 are assigned exactly once; each slice uses existing Claim IDs and owns every mapped PENDING falsifier.
- [x] Every slice has all thirteen mandatory fields; no conditional field is omitted.
- [x] Every claim has a fence in its implementing slice and every fence has its approved named mutation.
- [x] Every new loop states asymptotic cost, production scale, resulting bound, maximum accepted cost, and rationale; every always-on phase has a wall budget.
- [x] Arithmetic is 7,711 + 0 = 7,711; the exact >4,000 rule produces three independently mergeable increments, and every slice names one.
- [x] Every deferral phrase is classified and every intended-future item cites a verified tracker ID.
- [x] No slice is declared complete; `checkpointed-build` exclusively judges completion.
