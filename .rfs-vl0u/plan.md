# Plan: Bounded workspace and artifact discovery

## Approved inputs

- Route: Empirical, `.rfs-vl0u/route.md`.
- Behavior: `.rfs-vl0u/spec.md`, approved verbatim as "Approve amendment" on 2026-08-20.
- Design: `.rfs-vl0u/design.md`, most recently approved verbatim as "Approve C8 correction" on 2026-08-20; C1 is PASS, C2–C18 are PENDING with checkpointed-build owners, and approved risk acceptances are None.
- Evidence/oracles: `.rfs-vl0u/evidence.md`, `probe_pcre2.py`, `oracle_pcre2.py`, `probe_ignore.py`, `oracle_ignore.py`, `probe_globset.py`, and `oracle_globset.py`.
- Completion state: no slice is complete; checkpointed-build exclusively judges completion.

## Integration budget and review increments

| Slice | Diff estimate |
|---|---:|
| Slice 1 — live Artifact catalog | 160 |
| Slice 2 — core validation, engine, recovery, cancellation, and scale | 2,050 |
| Slice 3 — compiled pattern engines | 760 |
| Slice 4 — Artifact discovery | 520 |
| Slice 5 — Workspace discovery | 2,050 |
| Slice 6 — MCP discovery tools, architecture, and lifecycle | 1,870 |
| **Estimated changed lines** | **7,410** |
| **20% churn margin** | **1,482** |
| **Projected total** | **8,892** |

The 20% margin covers unsafe-FFI ownership corrections, native link/reparse fixture variation, generated Cargo lockfile movement, and raw-stdio schema/cancellation fixture churn. Existing capability, Path Session, read-bounding, rendering, and stdio harnesses keep the margin below a greenfield allowance. Because 8,892 is greater than 4,000, four independently mergeable PR increments are mandatory.

### Increment A — Discovery core

- Slices: 1–2; estimated 2,210 changed lines before global churn.
- Mergeable definition: source-neutral request/result types, `DiscoveryAdapter`, `DiscoveryEngine`, stable error categories, deterministic bounds/recovery, cancellation commit fences, and a live ordered Artifact catalog all compile and pass fake-adapter/core contracts. No Source Adapter or MCP tool is exposed, so the increment cannot expose partial discovery behavior.
- Verification without other increments: core contract binaries use only fake adapters and existing Path Session storage fixtures.

### Increment B — Compiled matcher and Artifact discovery

- Slices: 3–4; estimated 1,280 changed lines before global churn.
- Mergeable definition: pinned statically vendored PCRE2, Rust-regex-first matching, glob syntax, and current-session Artifact search/glob pass direct adapter contracts atop Increment A. No MCP tool is exposed.
- Verification without other increments: production matcher output is compared with `pcre2test` and the hand-counted glob oracle; Artifact behavior uses two real Path Sessions and the core engine.

### Increment C — Workspace discovery

- Slices: 5; estimated 2,050 changed lines before global churn.
- Mergeable definition: capability/final-handle Workspace traversal, contained ignore/hidden filtering, search/glob semantics, direct authority-change fencing, and typed `CompiledSources` dispatch pass direct adapter contracts atop Increment B. No MCP tool is exposed.
- Verification without Increment D: the Filesystem, Artifact, and compiled-source contract suites call the source-neutral discovery interface directly.

### Increment D — MCP exposure

- Slices: 6; estimated 1,870 changed lines before global churn.
- Mergeable definition: `rfs_search` and `rfs_glob` are registered only after both adapters are complete; dependency direction, deny-unknown schemas, text/structured rendering, 250 ms cancellation, client-root refresh, and unknown-tool behavior pass against the compiled stdio process.
- Verification without any additional increment: Cargo metadata/source allowlists and raw JSON-RPC contracts verify architecture and invoke all three tools end to end.

The checkout currently has no configured Git remote or discoverable upstream default branch. Checkpointed-build can create local slice commits, but its review-size ship point cannot open Increment A until a review remote exists; it must stop at that boundary rather than collapsing the partition.

## Slice 1: Add the live current-session Artifact catalog

**Claim IDs:** C15

**Expected behavior:** `PathSession::artifact_catalog` returns only the active session's addresses in ascending object-ID order, snapshots exclude post-snapshot retention, and disconnected sessions fail with `source_unavailable`.

**Oracle:** The catalog fixture owns the assigned IDs/session token and expected order before calling production.

**Stress fixture:** Retain 999 session objects with deliberately non-lexical content order and save the catalog, then retain a duplicate and the 1,000th unique object, enumerate the ceiling catalog, and invalidate. Expected: the saved snapshot has 999 ascending unique addresses and excludes object 1,000; the fresh snapshot has exactly 1,000 ascending current-session addresses; duplicate content creates no ID; disconnected lookup fails.

**Regression fence:** `crates/resourcefs-core/tests/path_session_contract.rs::artifact_catalog_is_ordered_and_live`.

**Named mutation:** Iterate the session `HashSet` without sorting and omit the liveness check; the catalog fence reports disconnected success/order drift.

**Complexity/production scale:** Catalog snapshot is $O(A \log A)$ time and $O(A)$ address memory for $A \le 1{,}000$. Maximum accepted cost: 1,000 addresses in at most 250 ms and at most 256 KiB of canonical address payload; the hard session record ceiling makes larger input unreachable, and the budget keeps catalog overhead negligible beside Artifact reads.

**Wall budget/phase:** Always-on for Artifact glob calls: catalog snapshot at 1,000 live objects must finish within 250 ms. Session retention/setup is existing behavior with `N/A — reason: not introduced by this slice`.

**Files:** `crates/resourcefs-core/src/session.rs`, `crates/resourcefs-core/tests/path_session_contract.rs`.

**Estimate:** 2 implementation hours.

**Diff estimate:** 160 changed lines: 30 catalog implementation, 130 catalog fixture.

**PR increment:** Increment A — Discovery core.

**Commands and expected results:**

- `cargo test -p resourcefs-core --features test-support --test path_session_contract artifact_catalog_is_ordered_and_live -- --exact --nocapture` → C15's 1,000 IDs are ascending/current-session-only; the saved snapshot excludes post-snapshot retention; invalidation returns `source_unavailable`; measured catalog time/canonical payload remain within budget.
- Apply the C15 unsorted/no-liveness mutation, rerun the catalog command → red with `C15`; restore, rerun → green.

### Slice 1 checkpoint result — PASS (2026-08-20)

- Impact analysis: language-server analysis found 25 existing `PathSession` references across core, sources, MCP, and contract tests. This slice adds one inherent method without changing an existing signature; no caller migration is required, and Artifact discovery remains the first planned consumer.
- Helper decision: reused the session admission lock, existing `HashSet<ArtifactId>`, `ArtifactAddress` validation, and stable `ResourceError` categories. No dependency, alternate catalog store, or duplicate identity/error helper was added.
- Gate 1 — affected contracts: PASS. The complete 8-case `path_session_contract` suite passed, including existing quota, deduplication, storage-failure, cancellation, and isolation behavior.
- Gate 2 — C15 falsifier and oracle: PASS. The fixture independently owns the expected session token, object IDs, counts, and ordering; the implementation returned the exact 999-object saved snapshot and 1,000-object live catalog, then rejected post-invalidation enumeration with `source_unavailable`.
- Gate 3 — stress and production budget: PASS. The 1,000-object ceiling catalog completed inside the 250 ms assertion and its rendered canonical address payload stayed below 256 KiB; duplicate retention reused object 1 without consuming the final slot.
- Gate 4 — named mutation: PASS. Removing all liveness checks and returning unsorted `HashSet` iteration made the exact C15 fence red with order drift (`659` observed where `999` was required). Restoration returned the fence green.
- Gate 5 — quality: PASS. `cargo fmt --all -- --check` passed; language-server diagnostics reported no errors or warnings in the changed implementation and contract test.

## Slice 2: Implement validated bounded discovery with lossless recovery and cancellation fences

**Claim IDs:** C2, C9, C10, C13

**Expected behavior:** Core rejects a search pattern above 65,536 bytes and every over-ceiling limit before adapter entry; sorts/deduplicates complete source results; returns deterministic lower-only pages; retains every omitted successful record exactly once with valid recovery/continuation references; rejects one byte over 64 MiB without false recovery or eviction; and never retains after operation cancellation/session disconnect. A 100,000-Resource/10,000-match fake source returns 100 initial records plus complete recovery within the measured allocation/wall budget.

**Oracle:** A hand-authored input-boundary table plus atomic fake-adapter entry counter; independently constructed canonical recovery text/digest and record counts; fake gate plus pre/post storage count; arithmetic Resource/match generator plus independent SHA-256 stream that never constructs production result types.

**Stress fixture:** One fake adapter supplies 100,000 canonical Resource candidates with 10,000 matches, duplicate/unsorted rows, ordered diagnostics, and `maxResults=100`; exact 48 KiB/3,000-line/512-column/1,000-record and 64 MiB boundaries are exercised. Expected: 100 records inline, 10,000 unique records in recovery in canonical order, exact independent digest, no object on 64 MiB+1 or cancellation, and peak counted allocation at most 256 MiB.

**Regression fence:** `crates/resourcefs-core/tests/discovery_engine_contract.rs::request_validation_precedes_adapter`; `::lower_only_limits_and_skip`; `::bounded_pages_recover_the_complete_document`; `::cancelled_operations_never_retain`; `::hundred_thousand_resources_remain_bounded_and_recoverable`; existing Path Session quota contracts.

**Named mutation:** C2 — remove the search-pattern ceiling check. C9 — apply `maxResults` before constructing recovery. C10 — remove the liveness check immediately before `PathSession::retain`. C13 — retain only inline records instead of the complete document. Each owning fence reports its claim ID.

**Complexity/production scale:** Request validation is $O(P)$ over at most 65,536 pattern bytes. Normalize/sort/deduplicate is $O((R+D)\log(R+D))$ over returned records/diagnostics; canonical rendering and page selection are $O(B)$ over at most 64 MiB complete text. At 10,000 matches from 100,000 fake Resources, maximum accepted cost is 2 seconds of engine work and 256 MiB counted peak allocation in release mode. The object/session ceilings justify memory below one session's 256 MiB quota; source traversal cost is owned by adapter slices.

**Wall budget/phase:** Always-on per search/glob call: core validation, normalization, rendering, and spill orchestration must remain within 2 seconds at 10,000 records/64 MiB in the release stress fixture. Path Session open is not introduced here; `N/A — reason: existing one-off phase`.

**Files:** `crates/resourcefs-core/src/discovery.rs`, `crates/resourcefs-core/src/error.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/src/read.rs` (hoist shared crate-private bound/continuation helpers), `crates/resourcefs-core/tests/discovery_engine_contract.rs`, `crates/resourcefs-core/tests/path_session_contract.rs` (existing quota fences only).

**Estimate:** 28 implementation hours.

**Diff estimate:** 2,050 changed lines: 850 production, 1,200 tests/generators/counting allocator.

**PR increment:** Increment A — Discovery core.

**Commands and expected results:**

- `cargo test -p resourcefs-core --features test-support --test discovery_engine_contract request_validation_precedes_adapter -- --exact --nocapture` → C2 accepts 65,536 bytes, rejects 65,537 as `limit_exceeded`, rejects every one-over limit, and invalid rows leave the adapter counter zero.
- `cargo test -p resourcefs-core --features test-support --test discovery_engine_contract lower_only_limits_and_skip -- --exact --nocapture && cargo test -p resourcefs-core --features test-support --test discovery_engine_contract bounded_pages_recover_the_complete_document -- --exact --nocapture && cargo test -p resourcefs-core --features test-support --test discovery_engine_contract cancelled_operations_never_retain -- --exact --nocapture` → C9 exact/one-over pages and skips agree with the hand table/digest; C10 cancel/disconnect outcomes add no storage record.
- `cargo test -p resourcefs-core --features test-support --release --test discovery_engine_contract hundred_thousand_resources_remain_bounded_and_recoverable -- --exact --nocapture` → C13 returns exactly 100 inline/10,000 recovered records, independent SHA-256 agrees, peak allocation is at most 256 MiB, and engine wall time is at most 2 seconds.
- `cargo test -p resourcefs-core --features test-support --test path_session_contract` → existing object/session quota atomicity remains green; 64 MiB+1 publishes no object and evicts nothing.
- Apply each C2/C9/C10/C13 named mutation separately and rerun its exact owning command → red with that claim ID; restore after each and rerun → green.

## Slice 3: Add statically pinned Rust-regex, PCRE2, and glob matching

**Claim IDs:** C1, C3

**Expected behavior:** Every non-empty in-bound search pattern compiles with Rust regex first; only Rust syntax rejection invokes non-JIT PCRE2 with Unicode/UCP and explicit match/depth limits; typed invalid/exhaustion categories are stable. Glob matching uses literal `/`, backslash escape, component `*`/`?`/classes, legal `**`, alternation, and ASCII-only case folding on every platform.

**Oracle:** System `pcre2test`, exact pinned upstream source control flow, `.rfs-vl0u/oracle_pcre2.py`, and the independent 20-case `.rfs-vl0u/oracle_globset.py` table. These do not call the safe production wrapper.

**Stress fixture:** Compile a 65,536-byte literal pattern and scan a 64 MiB UTF-8 haystack; run catastrophic match/depth inputs with configured PCRE2 ceilings; run all 20 glob cases including escaped `*`, slash separation, Unicode/ASCII case pairs. Expected: bounded typed outcomes, no JIT, and exact oracle parity.

**Regression fence:** `crates/resourcefs-sources/tests/pattern_contract.rs::rust_first_pcre2_fallback_and_limits`; `::glob_language_table`; `::matcher_selection_unicode_and_work_limits`.

**Named mutation:** C1 — omit `PCRE2_UCP`, one match/depth setter, and `literal_separator(true)` in one mutation; C1 fences report Unicode/limit/glob disagreement. C3 — try PCRE2 before Rust (or enable JIT); the selection/depth fence reports the wrong engine or missing depth exhaustion.

**Complexity/production scale:** Pattern compilation consumes $O(P)$ input with $P \le 65{,}536$ plus library compiler state. Rust matching is linear in a 64 MiB Resource for supported syntax; PCRE2 calls are capped at 100,000 match steps and depth 1,000, preventing input-dependent unbounded backtracking. Glob compilation/match is $O(P+L)$ for path length $L \le 65{,}536$. Maximum accepted release costs: 2 seconds per maximum pattern compilation, 1 second per 64 MiB Rust-regex scan, 250 ms per bounded PCRE2 call, and 100 ms to evaluate the 20-case glob table.

**Wall budget/phase:** Always-on: matcher compilation once per discovery request (2-second maximum), match once per candidate Resource (Rust 1 second at 64 MiB; PCRE2 250 ms at configured work ceilings), and glob match once per path (100 ms for the complete table). Static PCRE2 C compilation is a one-off build phase with `N/A — reason: not production request latency`.

**Files:** `.cargo/config.toml` (`PCRE2_SYS_STATIC=1`, forced), `Cargo.toml`, `Cargo.lock`, `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-sources/src/pattern.rs`, `crates/resourcefs-sources/tests/pattern_contract.rs`.

**Estimate:** 10 implementation hours.

**Diff estimate:** 760 changed lines: 20 configuration/lock/manifest, 390 production wrapper, 350 contract fixtures.

**PR increment:** Increment B — Compiled matcher and Artifact discovery.

**Commands and expected results:**

- `python .rfs-vl0u/probe_pcre2.py && python .rfs-vl0u/oracle_pcre2.py && python .rfs-vl0u/probe_globset.py && python .rfs-vl0u/oracle_globset.py` → C1 evidence remains PASS with typed -47/-53 outcomes and byte-identical 20-case glob JSON.
- `cargo test -p resourcefs-sources --test pattern_contract -- --nocapture` → C1/C3 production matcher agrees item by item with the oracles; the stress rows meet compile/match/glob wall budgets.
- Apply the combined C1 mutation, rerun the pattern command → red with `C1` for Unicode/limit/glob rows; restore, rerun → green.
- Apply the C3 PCRE2-first mutation, rerun `cargo test -p resourcefs-sources --test pattern_contract matcher_selection_unicode_and_work_limits -- --exact --nocapture` → red with `C3`; restore, rerun → green.

## Slice 4: Implement current-session Artifact search and glob

**Claim IDs:** C8

**Expected behavior:** Artifact discovery snapshots only active-session immutable Artifact addresses before any recovery retention, searches selected text, globs canonical `artifact://<id>` identities as `artifact` entries, never exposes another session, and excludes the recovery object created by the same call.

**Oracle:** The fixture creates two independent Path Sessions, records the exact pre-call Artifact IDs, and computes the expected set without calling catalog/discovery code.

**Stress fixture:** Populate one session to 1,000 small unique Artifacts and another with a unique sentinel; force the first session's glob to spill. Expected: exactly the first session's pre-call 1,000 IDs, no second-session sentinel, no just-created recovery ID, stable ascending order, and glob within 250 ms.

**Regression fence:** `crates/resourcefs-sources/tests/artifact_adapter_contract.rs::search_and_glob_are_session_isolated`; `::glob_snapshot_excludes_its_recovery_artifact`.

**Named mutation:** In core `discovery.rs`, retain a provisional Artifact before `DiscoveryAdapter::glob`; the pre-call catalog includes that ID and C8's self-exclusion fence turns red.

**Complexity/production scale:** Catalog/glob is $O(A\log A + A\cdot G)$ for $A\le1{,}000$ and glob cost $G$; search is $O(S+B)$ over catalog selection plus at most 256 MiB aggregate live session text, with one Resource projection capped at 64 MiB. Maximum accepted costs: 250 ms/1 MiB transient allocation for 1,000-entry glob; 10 seconds/96 MiB transient allocation for a 256 MiB aggregate search, excluding core result retention already budgeted in Slice 2.

**Wall budget/phase:** Always-on Artifact glob at 1,000 entries must finish within 250 ms; always-on maximum-session search must finish within 10 seconds. Session creation is existing one-off behavior: `N/A — reason: not introduced by this slice`.

**Files:** `crates/resourcefs-sources/src/artifact.rs`, `crates/resourcefs-sources/tests/artifact_adapter_contract.rs`, `crates/resourcefs-core/src/discovery.rs` only during the temporary named mutation (restored, no permanent diff).

**Estimate:** 6 implementation hours.

**Diff estimate:** 520 changed lines: 150 production, 370 two-session/stress fences.

**PR increment:** Increment B — Compiled matcher and Artifact discovery.

**Commands and expected results:**

- `cargo test -p resourcefs-sources --features test-support --test artifact_adapter_contract search_and_glob_are_session_isolated -- --exact --nocapture && cargo test -p resourcefs-sources --features test-support --test artifact_adapter_contract glob_snapshot_excludes_its_recovery_artifact -- --exact --nocapture` → C8 returns only the exact pre-call active-session set; forced spill recovery is absent from its own result; measured glob/search budgets hold.
- Apply the C8 provisional-retain mutation, rerun the same command → red with `C8` and the extra Artifact ID; restore, rerun → green.

## Slice 5: Implement contained Workspace search, glob, filtering, and authority fencing

**Claim IDs:** C4, C5, C6, C7, C14

**Expected behavior:** `FilesystemSource` walks only capability/final-handle-contained Resources; follows contained file/directory links, deduplicates final identities, stops cycles, reports escapes as permission diagnostics, honors only contained root/nested `.gitignore` plus hidden controls, bypasses filters for exact targets, emits one terminator-free row per matching line with ordered diagnostics, applies the approved glob grammar/kinds/order, and rejects delivery after root authority removal. `CompiledSources` dispatches typed Workspace/Artifact targets without string guessing.

**Oracle:** Outside sentinel/final-native-target fixture; hermetic `git check-ignore --no-index` plus hand hidden table; independently hand-authored line/diagnostic rows from Python-style standard line splitting; 20-case hand glob table and native topology; fixture-controlled root generations/sentinel sets.

**Stress fixture:** Traverse 10,000 actual contained entries with 1,000 matches, nested ignore files, hidden entries, Unicode/spaces, aliases, a contained cycle, and concurrently retargeted escaping links. Expected: exact 1,000 canonical matches, zero outside bytes/duplicates, deterministic five-run output, one vanished-entry diagnostic, peak per-file buffer at most 64 MiB, and release wall time at most 10 seconds. Native Linux/macOS/Windows variants exercise symlink/junction/reparse final-handle functions.

**Regression fence:** `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs::discovery_links_never_escape_or_duplicate`; `::discovery_filters_are_contained_and_explicit`; `::search_groups_lines_and_reports_partial_failures`; `::glob_language_kinds_and_order`; `::root_refresh_fences_discovery_delivery`; `crates/resourcefs-sources/tests/artifact_adapter_contract.rs::searches_selected_artifact_text`; `::glob_lists_session_artifacts`; `crates/resourcefs-sources/tests/compiled_sources_contract.rs` discovery route fence.

**Named mutation:** C4 — replace the capability walker with `ignore::WalkBuilder::follow_links(true)`. C5 — call `GitignoreBuilder::build_global`. C6 — emit one row per occurrence and return the first descendant error. C7 — remove `backslash_escape(true)`. C14 — omit final `validate_read_delivery` for discovery. Each owning fence reports its claim ID.

**Complexity/production scale:** Traversal is $O(E\log D + B + E\cdot I)$: $E$ entries, per-directory lexical sort of at most $D$ names, $B$ scanned UTF-8 bytes, and stacked ignore cost $I$; final-identity/cycle sets are $O(E)$; link resolution is hard-capped at 40 hops. At 10,000 entries/1,000 matches, maximum accepted cost is 10 seconds release wall time, 128 MiB adapter transient memory (one at-most-64 MiB selected file plus directory/dedup/diagnostic state), and no outside byte. Core's 100,000/10,000 result-scale bound remains the product-scale upper orchestration fence.

**Wall budget/phase:** Always-on recursive Workspace search/glob: at most 10 seconds for the 10,000-entry native fixture; exact-file search at 64 MiB: at most 2 seconds. Root refresh remains an existing discrete event with `N/A — reason: one-off authority transition; no recurring wall budget`.

**Files:** `crates/resourcefs-sources/src/filesystem.rs`, `crates/resourcefs-sources/src/artifact.rs` (shared C6/C7 result paths only), `crates/resourcefs-sources/src/compiled.rs`, `crates/resourcefs-sources/src/lib.rs` (test-support gate export), `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`, `crates/resourcefs-sources/tests/artifact_adapter_contract.rs`, `crates/resourcefs-sources/tests/compiled_sources_contract.rs`, `crates/resourcefs-sources/src/pattern.rs` only during C7 mutation (restored).

**Estimate:** 34 implementation hours plus native runner execution.

**Diff estimate:** 2,050 changed lines: 800 production traversal/filter/dispatch, 1,250 direct/native/stress contracts.

**PR increment:** Increment C — Workspace discovery.

**Commands and expected results:**

- `cargo test -p resourcefs-sources --all-features --test filesystem_adapter_contract -- --nocapture` → C4/C5/C6/C7/C14 exact link, ignore, line, glob, diagnostic, concurrency, and 10,000-entry budget rows agree with independent oracles on the native runner.
- `cargo test -p resourcefs-sources --all-features --test artifact_adapter_contract searches_selected_artifact_text -- --exact --nocapture && cargo test -p resourcefs-sources --all-features --test artifact_adapter_contract glob_lists_session_artifacts -- --exact --nocapture` → C6/C7 Artifact line/kind/order rows agree with the hand tables.
- `cargo test -p resourcefs-sources --all-features --test compiled_sources_contract -- --nocapture` → typed Workspace and Artifact targets route to only their owning adapter.
- Run the filesystem contract command on native Linux, macOS, and Windows → C4/C7/C14 platform rows execute (not skip) and return identical canonical slash identities without outside bytes.
- Apply each C4/C5/C6/C7/C14 named mutation separately and rerun its exact named fence → red with that claim ID; restore after each and rerun → green.

## Slice 6: Expose complete discovery tools through MCP with cancellation and root refresh

**Claim IDs:** C11, C12, C16, C17, C18

**Expected behavior:** The compiled process preserves the approved dependency direction, lists exactly `rfs_read`, `rfs_search`, and `rfs_glob`, validates each object-rooted deny-unknown schema against only its own arguments/defaults before root refresh/I/O, returns complete non-empty text equivalent to object-rooted structured records/errors, preserves unknown-tool `invalid_params`, returns `cancelled` within 250 ms without damaging session state, and fences in-flight discovery after client-root removal.

**Oracle:** Cargo metadata and a narrow source allowlist independently compute forbidden dependency edges/raw FFI locations. A raw JSON-RPC client independently owns expected tool/schema tables, default/boundary outcomes, sentinels/root sets, cancellation clock, process gate, and session-storage directory counts.

**Stress fixture:** Execute the full field-presence matrix, exact/one-over pattern/limit boundaries, 1,000-record/48 KiB rendering, both tools behind a held gate, cancellation at each lifecycle seam, and root removal during an in-flight scan. Expected: invalid rows never enter the gate; defaults produce the hand-table rows; text and structured counts/references agree; render is within 25 ms; cancellation response is within 250 ms with no object file after gate release; removed-root discovery never succeeds.

**Regression fence:** `crates/resourcefs-mcp/tests/architecture_contract.rs::discovery_dependencies_stay_in_owning_modules`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::lists_and_calls_discovery_tools`; `::discovery_argument_matrix_precedes_io`; `::cancelled_discovery_returns_promptly_without_late_artifact`; `::root_change_fences_discovery`; `crates/resourcefs-mcp/src/render.rs` unit contracts.

**Named mutation:** C12 — add `rmcp` to core and `pcre2-sys` to mcp. C11 — restore the `request.name != "rfs_read"` guard. C16 — deserialize search `path` with ordinary `Option<String>`, accepting explicit null. C17 — remove the cancellation branch from the biased `tokio::select!`. C18 — omit `refresh_client_roots` for discovery calls.

**Complexity/production scale:** Per-call schema conversion is $O(Q)$ for request JSON $Q$ bounded by the 65,536-byte pattern/path ceiling; MCP mapping/rendering is $O(R+B)$ for at most 1,000 inline records and 48 KiB text, with no sorting/second recovery copy. Maximum accepted costs: 25 ms renderer wall time and 2 MiB transient mapping memory at maximum inline page; dispatch adds at most 25 ms beyond engine/source time. Cancellation response is independently capped at 250 ms because the server races the request token rather than awaiting the blocking worker.

**Wall budget/phase:** Always-on validation/dispatch/render: 25 ms beyond engine/source work at maximum inline page. Always-on cancellation path: response within 250 ms of token notification. Cargo architecture inspection and stdio process startup/initialize are one-off with `N/A — reason: test-only or existing discrete process phases`.

**Files:** `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`, `crates/resourcefs-mcp/tests/architecture_contract.rs`.

**Estimate:** 27 implementation hours.

**Diff estimate:** 1,870 changed lines: 340 server, 460 renderer/unit tests, 1,000 stdio matrices/lifecycle fixtures, 70 architecture fence.

**PR increment:** Increment D — MCP exposure.

**Commands and expected results:**

- `cargo test -p resourcefs-mcp --test architecture_contract discovery_dependencies_stay_in_owning_modules -- --exact --nocapture` → C12 reports no forbidden dependency edge and no raw `pcre2_sys` outside `resourcefs-sources/src/pattern.rs`.
- `cargo test -p resourcefs-mcp --all-features --test stdio_mcp_contract lists_and_calls_discovery_tools -- --exact --nocapture` and `cargo test -p resourcefs-mcp --all-features --lib render -- --nocapture` → C11 lists exactly three object-root tools, calls both discovery tools, preserves unknown-tool failure, and text/structured outputs agree within the 25 ms render budget.
- `cargo test -p resourcefs-mcp --all-features --test stdio_mcp_contract discovery_argument_matrix_precedes_io -- --exact --nocapture` → C16 every invalid table row returns MCP `invalid_params` without gate entry; every valid/default row reaches the released gate and produces its hand-authored outcome.
- `cargo test -p resourcefs-mcp --all-features --test stdio_mcp_contract cancelled_discovery_returns_promptly_without_late_artifact -- --exact --nocapture` → C17 both tools return `cancelled` within 250 ms, a subsequent call succeeds, and storage remains unchanged after gate release.
- `cargo test -p resourcefs-mcp --all-features --test stdio_mcp_contract root_change_fences_discovery -- --exact --nocapture` → C18 removed-root in-flight discovery is a tool error and the new root is authoritative.
- Apply each C11/C12/C16/C17/C18 named mutation separately and rerun its exact owning command → red with that claim ID; restore after each and rerun → green.

## Self-review

- PASS — C1–C18 are each assigned exactly once: Slice 1 C15; Slice 2 C2/C9/C10/C13; Slice 3 C1/C3; Slice 4 C8; Slice 5 C4/C5/C6/C7/C14; Slice 6 C11/C12/C16/C17/C18.
- PASS — every slice has all thirteen mandatory fields; no conditional field is empty.
- PASS — every claim's named permanent fence and named mutation land in its owning slice; no fence uses an approved-risk N/A.
- PASS — every introduced loop states asymptotic cost, production-scale input, resulting bound, maximum accepted cost, and rationale; every always-on phase has a wall budget.
- PASS — 7,410 + 1,482 = 8,892 applies the exact review-size rule and partitions into four independently mergeable increments.
- PASS — no new intended-work item is introduced. Product exclusions and issue-backed adapter/release work remain classified in approved `design.md`; the missing review remote is a current execution prerequisite, not product work.
- PASS — no slice is declared complete; checkpointed-build owns completion.
