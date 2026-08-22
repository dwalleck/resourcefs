# Plan: Self-describing namespace catalogs

## Inputs

- Route: Structural (`.rfs-ewh2/route.md`).
- Approved behavior: `.rfs-ewh2/spec.md`, revised and approved 2026-08-22.
- Approved design: `.rfs-ewh2/design.md`, revised and approved 2026-08-22; C7 cheapest falsifier is `PASS`, no row is `FAIL`, all other rows name checkpointed-build as discharge owner, and risk acceptances are `None`.
- Empirical evidence/probes: N/A — Structural route.

## Review-size budget and PR increments

| Slice | Diff estimate |
|---|---:|
| 1. Make rendered Workspace Roots valid teaching references | 430 lines |
| 2. Implement catalog rendering and authority snapshots | 690 lines |
| 3. Wire typed catalog routing and discovery redirects | 500 lines |
| 4. Fence the common MCP shape and limits | 260 lines |
| 5. Advertise the namespace discovery entry point | 180 lines |
| **Projected subtotal** | **2,060 lines** |
| **Churn margin (30%)** | **618 lines** |
| **Projected total** | **2,678 lines** |

The 30% margin covers exhaustive `ResourceAddress` match migration, golden-reference fixtures, raw MCP contract setup, and root-directory behavior added by the approved revision after the first draft. The projected total is below the 4,000-line review-size gate, so the plan has one PR increment.

### PR increment: Namespace discovery

Slices 1–5. Mergeable definition: exact catalog and root-directory references parse canonically; direct and MCP reads expose registry/root state through the common read contract; unsupported discovery operations redirect; initialization and all visible tools teach `rfs_read` of `rfs://`. The increment verifies through core grammar contracts, direct compiled-source contracts, and raw stdio MCP contracts without relying on any later issue. rfs-hwlm and rfs-73dz remain separately tracked consumers of the typed references.

## Slice 1: Make rendered Workspace Roots valid teaching references

**Claim IDs:** C13

**Expected behavior:** `rfs://workspace/<root>/` and its slash-less alias parse as one canonical Workspace root-directory address that renders with the trailing slash. Relative empty/`.` and `file://` inputs cannot construct the root sentinel. `rfs_read` of the canonical root returns `unsupported_projection` with the exact directory teaching message naming `rfs_glob` and rfs-hwlm, never `invalid_reference`.

**Oracle:** Literal expected root strings derived from fixture IDs plus a literal expected error category/message; the oracle does not call `WorkspacePath` rendering or the future catalog renderer.

**Stress fixture:** A multi-root fixture with IDs `alpha`, `root.with-dots`, and `z-last`; parse trailing/slash-less forms, reject relative `""`/`.` construction, preserve a contained `file://` root directory as a filesystem input rather than the root sentinel, and read `rfs://workspace/root.with-dots/`. Expected: one `WorkspaceAddress::Canonical` root value, canonical trailing-slash rendering, and the exact teaching error.

**Regression fence:** `crates/resourcefs-core/tests/path_reference_contract.rs::catalog_root_lines_parse_and_teach`; `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs::root_directory_references_are_valid_and_teach`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::catalog_root_lines_parse_and_teach` — created in this slice.

**Named mutation:** From C13: `crates/resourcefs-core/src/reference.rs` restores the missing-root/path rejection for an empty canonical remainder. The three fences must turn red because the rendered root returns `invalid_reference`; restore the root sentinel and all three return green.

**Complexity/production scale:** N/A — no new loop. `WorkspacePath::root()` / `is_root()` and the directory teaching branch are constant-time additions to existing parse/read phases; traversal and containment algorithms are unchanged.

**Wall budget/phase:** N/A — no new runtime phase. The constant-time branches run inside existing per-request parsing and filesystem lookup.

**Files:** `crates/resourcefs-core/src/reference.rs`; `crates/resourcefs-core/tests/path_reference_contract.rs`; `tests/fixtures/workspace_references.json`; `crates/resourcefs-sources/src/filesystem.rs`; `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 2–3 hours.

**Diff estimate:** 430 changed lines: 150 implementation, 280 tests/fixtures.

**PR increment:** Namespace discovery.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test path_reference_contract catalog_root_lines_parse_and_teach` → trailing and slash-less root spellings agree with the literal canonical oracle; relative empty/`.` do not gain authority. Under the named mutation this command is red on the missing-root error, then green after restoration.
- `cargo test -p resourcefs-sources --test filesystem_adapter_contract root_directory_references_are_valid_and_teach` → a contained root directory resolves through existing containment and returns the exact `unsupported_projection` teaching message; ordinary file reads and directory traversal behavior remain unchanged.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract catalog_root_lines_parse_and_teach` → raw MCP text and structured errors agree on `unsupported_projection`, preserve the requested canonical root reference, and name `rfs_glob` plus rfs-hwlm.

## Slice 2: Implement catalog rendering and authority snapshots

**Claim IDs:** C2, C3, C4, C10, C14

**Expected behavior:** A private `NamespaceCatalog` validates and renders the compiled adapters' live self-descriptions and an immutable current `WorkspaceRootSet` snapshot. Source entries are unique, sorted, bounded, syntax-valid, and active/degraded; workspace entries cover every 0–256-root shape. Content and Version Tags are deterministic, and the source header chains to `rfs://workspace` while teaching the selector grammar exactly once. This slice exercises the renderer directly; typed Path Reference routing lands in Slice 3.

**Oracle:** C2 hand-authored registry boundary table; C3/C4/C14 byte-literal documents; C10 equality/inequality relations against literal content. None calls production sorting, formatting, or VersionTag construction.

**Stress fixture:** 256 metadata rows supplied in reverse order with spaces/Unicode, 4,096-byte boundary fields, one degraded reason, duplicate and 257th-entry failures, and a valid example per row; plus empty, single, selected/unselected multi-root, and 256-root snapshots. Expected: exact sorted documents, explicit validation errors without omission, stable same-state content/tags, and changed content/tags only when a supplied snapshot changes.

**Regression fence:** Private `catalog::tests::{catalog_metadata_rejects_duplicate_and_invalid_entries,catalog_metadata_accepts_exact_boundaries,source_catalog_marks_degraded_and_omits_unmounted,workspace_catalog_covers_empty_primary_and_max_root_sets,catalog_content_and_tags_are_snapshot_deterministic,source_catalog_header_chains_and_teaches_selectors,catalog_production_budget}` — created in this slice.

**Named mutation:** C2 silently deduplicates duplicate schemes; C3 renders degraded as active; C4 marks the first sorted root primary; C10 appends a read counter; C14 removes the selector-grammar header line. Checkpointed-build applies each mutation separately, confirms its named fence red, restores, and confirms green.

**Complexity/production scale:** Source rendering is `O(S log S + B)` for `S <= 256` and validated text `B <= 256 × 4 × 4,096 bytes` (about 4 MiB); maximum accepted renderer cost is 250 ms in release, scaled from the existing 5-second/64-MiB read budget. Workspace rendering is `O(R log R + B)` for `R <= 256`; production IDs are at most 128 bytes or fixed client hashes, keeping output around 49 KiB; maximum accepted renderer cost is 25 ms in release.

**Wall budget/phase:** Always-on when the renderer is invoked: source rendering <= 250 ms at 256 entries/4 MiB; workspace rendering <= 25 ms at 256 production roots. No I/O or probe runs in either phase.

**Files:** New `crates/resourcefs-sources/src/catalog.rs`; `crates/resourcefs-sources/src/lib.rs`; `crates/resourcefs-sources/src/compiled.rs`; `crates/resourcefs-sources/src/filesystem.rs`; `crates/resourcefs-sources/src/artifact.rs`; `crates/resourcefs-sources/tests/compiled_sources_contract.rs`; `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`; `crates/resourcefs-sources/tests/filesystem_resource_limits_contract.rs`; `crates/resourcefs-mcp/src/server.rs`; `crates/resourcefs-mcp/src/render.rs`.

**Estimate:** 4–6 hours.

**Diff estimate:** 690 changed lines: 300 implementation, 390 tests.

**PR increment:** Namespace discovery.

**Commands and expected results:**
- `cargo test -p resourcefs-sources catalog::tests::` → count/field/example/state/root/header/determinism fixtures agree byte-for-byte with independent literals. C2, C3, C4, C10, and C14 mutations each make the named test red; restoration is green.
- `cargo test --release -p resourcefs-sources catalog_production_budget` → the 256-entry/4-MiB renderer is <= 250 ms and the 256-root renderer <= 25 ms while content still equals the boundary oracle.
- `cargo test -p resourcefs-sources compiled::tests::catalog_metadata_is_the_compiled_registry` → filesystem and artifact metadata are registered once through the compiled composite, with no central per-scheme table.

## Slice 3: Wire typed catalog routing and discovery redirects

**Claim IDs:** C1, C5, C8, C11, C12

**Expected behavior:** `rfs://`, `rfs://workspace`, and alias `rfs://workspace/` parse as typed catalog addresses and route through `CompiledSources` into Slice 2's renderer. Catalog reads are immutable and track one active authority generation after client-root replacement. Search/glob of either exact catalog fails before root/source I/O with the approved same-reference `unsupported_projection` redirect.

**Oracle:** C1 literal grammar/canonicalization table; C5 client-supplied roots plus canonical prefix from an ordinary file read; C8 raw MCP counters/category/message; C11 direct typed composite fixture; C12 direct enum matching and common `mutable` field.

**Stress fixture:** Both canonical catalogs plus the one alias and rejected selector neighbors; direct composite reads; a completed root replacement and gated refresh race; all four catalog search/glob calls while root/source delivery gates are armed. Expected: canonical typed identities, one complete root generation or existing `source_unavailable`, immutable SourceResources, and zero root/source I/O for discovery redirects.

**Regression fence:** `crates/resourcefs-core/tests/path_reference_contract.rs::catalog_references_are_typed_with_one_alias`; `crates/resourcefs-sources/tests/compiled_sources_contract.rs::catalog_reads_route_by_typed_address_family`; `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs::workspace_catalog_snapshot_never_mixes_root_generations`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::{namespace_catalog_tracks_client_root_replacement,catalog_search_and_glob_redirect_before_io}` — created in this slice.

**Named mutation:** C1 moves catalog checks after generic workspace parsing; C5 snapshots `launch_view` instead of `active_view`; C8 removes the exact-catalog check from `GlobTarget::new`; C11 removes typed Catalog routing from `CompiledSources`; C12 sets `SourceResource.mutable` true for Catalog identities. Each named fence must turn red independently, then green after restoration.

**Complexity/production scale:** No new data-dependent loop beyond Slice 2's renderers. Typed routing is constant-time enum matching. Exact discovery rejection compares against two constant strings and allocates nothing before returning the error.

**Wall budget/phase:** Always-on search/glob target validation <= 1 ms at the maximum accepted input. Catalog render phases retain Slice 2's measured 250 ms/25 ms budgets.

**Files:** `crates/resourcefs-core/src/reference.rs`; `crates/resourcefs-core/src/resource.rs`; `crates/resourcefs-core/src/read.rs`; `crates/resourcefs-core/src/discovery.rs`; `crates/resourcefs-core/src/lib.rs`; `crates/resourcefs-core/tests/path_reference_contract.rs`; `crates/resourcefs-core/tests/discovery_engine_contract.rs`; `tests/fixtures/workspace_references.json`; `crates/resourcefs-sources/src/compiled.rs`; `crates/resourcefs-sources/src/filesystem.rs`; `crates/resourcefs-sources/tests/compiled_sources_contract.rs`; `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`; `crates/resourcefs-mcp/src/server.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 3–5 hours.

**Diff estimate:** 500 changed lines: 170 implementation, 330 tests/fixtures.

**PR increment:** Namespace discovery.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test path_reference_contract catalog_references_are_typed_with_one_alias` → canonical catalogs and alias match the literal typed/canonical oracle; neighboring references retain categories. C1 and C12 mutations make it red.
- `cargo test -p resourcefs-sources --test compiled_sources_contract catalog_reads_route_by_typed_address_family` → direct typed reads return both immutable catalog Resources without MCP. C11 and C12 mutations are red.
- `cargo test -p resourcefs-sources --test filesystem_adapter_contract workspace_catalog_snapshot_never_mixes_root_generations` → gated refresh yields one generation or existing `source_unavailable`, never mixed. C5's launch-view mutation is red.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract namespace_catalog_tracks_client_root_replacement` → the next read equals replacement roots, excludes removed roots, and changes content/tag exactly once.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract catalog_search_and_glob_redirect_before_io` → all four calls return the approved error/redirect and leave roots/source gates untouched. C8's glob fallthrough mutation is red.

## Slice 4: Fence the common MCP shape and limits

**Claim IDs:** C6, C7

**Expected behavior:** Both routed catalogs use only the unchanged object-rooted `ReadToolOutput`, equivalent complete non-empty text, canonical identity, text content type, content-derived Version Tag, `mutable:false`, and normal boundedness fields. Lower byte/line/column limits compose with server limits and reconstruct losslessly through existing artifact continuation/recovery.

**Oracle:** C6 literal common structured key set plus raw text; C7 independent page reconstruction against complete renderer bytes and the already-PASS core minimum-limit oracle.

**Stress fixture:** Read both catalogs as fitting pages and force source/workspace catalog spills separately with lower byte, line, and column limits. Expected: every page stays under the effective ceiling, continuation progresses, recovery is immutable, reconstructed bytes equal the direct renderer oracle, and no catalog-specific structured key appears.

**Regression fence:** `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::{catalogs_use_common_read_result_shape,catalog_reads_obey_common_limits_and_reconstruct}` plus existing `resourcefs-core/tests/read_engine_contract.rs::server_text_ceiling_and_per_call_ceiling_compose_by_minimum` — created/extended in this slice.

**Named mutation:** C6 adds and populates a catalog-specific `sources` field in `ReadToolOutput`; C7 truncates the catalog `SourceResource` before `ReadEngine`. Each new MCP fence must go red under its mutation and green after restoration.

**Complexity/production scale:** N/A — no new loop; this slice fences the existing `ReadEngine` `O(B)` page/recovery path and Slice 2 renderer loops without changing either.

**Wall budget/phase:** N/A — no new runtime phase. Existing 64-MiB/5-second read and Slice 2 catalog-render budgets remain authoritative.

**Files:** `crates/resourcefs-core/tests/read_engine_contract.rs`; `crates/resourcefs-mcp/src/render.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 2 hours.

**Diff estimate:** 260 changed lines: 0 required production behavior, 260 contract/stress assertions.

**PR increment:** Namespace discovery.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract catalogs_use_common_read_result_shape` → both catalogs expose exactly the common keys and equivalent text with `mutable:false`; C6's added-field mutation is red.
- `cargo test -p resourcefs-core --test read_engine_contract server_text_ceiling_and_per_call_ceiling_compose_by_minimum && cargo test -p resourcefs-mcp --test stdio_mcp_contract catalog_reads_obey_common_limits_and_reconstruct` → minimum-limit oracle remains green and catalog pages reconstruct byte-for-byte; C7's pre-truncation mutation is red.
## Slice 5: Advertise the namespace discovery entry point

**Claim IDs:** C9

**Expected behavior:** Initialization instructions and every currently visible `rfs_*` description begin with `Start with rfs_read of rfs:// to discover mounted sources.` Tool names, input schemas, output schemas, and both protocol revisions remain unchanged.

**Oracle:** The literal cue sentence and exact existing tool-name set `{rfs_glob, rfs_read, rfs_search}` from raw initialization/tools-list responses; schema objects are compared to the established protocol contract rather than regenerated from the new strings.

**Stress fixture:** Initialize once with each supported revision (`2025-11-25`, `2026-07-28`), enumerate tools in server-returned order, and compare by tool name rather than position. Expected: instructions and all three descriptions start with the cue; no missing/extra tool; schemas match prior assertions.

**Regression fence:** `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::initialization_and_all_tool_descriptions_advertise_catalog` plus existing `lists_and_calls_discovery_tools` — created/extended in this slice.

**Named mutation:** From C9: remove the cue from only the `rfs_glob` description in `crates/resourcefs-mcp/src/server.rs`; the new fence must report `rfs_glob`, then return green after restoration.

**Complexity/production scale:** N/A — no new loop or data-dependent runtime computation; static description strings replace existing static strings.

**Wall budget/phase:** N/A — one-off initialization/tool-list metadata; no wall budget. Tool execution paths are unchanged.

**Files:** `crates/resourcefs-mcp/src/server.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 1 hour.

**Diff estimate:** 180 changed lines: 20 production text, 160 protocol assertions.

**PR increment:** Namespace discovery.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract initialization_and_all_tool_descriptions_advertise_catalog` → both negotiated revisions expose the exact cue in instructions and all three descriptions while retaining the literal tool set and established schemas. Removing only the `rfs_glob` cue makes this test red with `rfs_glob` named; restoration is green.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract lists_and_calls_discovery_tools` → existing tool names, strict input schemas, object-rooted output schemas, and live calls remain unchanged.

## Tracker taxonomy

- Permanent non-goals are inherited from approved `design.md`: no catalog search/glob semantics, selectors, extra aliases, backing/grant disclosure, catalog-specific structured fields, per-read probes, MCP renderer, or new error category. Their rationale remains the approved design rationale.
- Intended future work is covered by verified issues: rfs-hwlm (directory-listing projection after the teaching error), rfs-73dz (end-to-end mutation denial), rfs-ww6w (MCP Resources mirror), rfs-60g1 (`local://`), and the source-specific issue IDs listed in `design.md`.
- No untracked deferral is introduced by this plan.

## Self-review

- [x] C1–C14 are assigned exactly once: C13 to Slice 1; C2–C4, C10, C14 to Slice 2; C1, C5, C8, C11, C12 to Slice 3; C6–C7 to Slice 4; C9 to Slice 5.
- [x] Every slice contains all thirteen mandatory fields; conditional fields carry `N/A — reason`.
- [x] Every claim's regression fence and named mutation are created/applied in its owning slice; no fence uses risk acceptance.
- [x] New loops record asymptotic cost, production bounds, explicit accepted runtime, and rationale; always-on phases carry wall budgets.
- [x] Diff arithmetic is exact: 2,060 + 618 = 2,678; one increment is below 4,000 lines and independently verifies without later issues.
- [x] Tracker taxonomy cites verified intended-work IDs and records permanent non-goals with rationale.
- [x] No slice is declared complete; checkpointed-build exclusively judges completion.
