# Plan: Versioned workspace text mutation

## Inputs and partition arithmetic

- Route: Empirical; `route.md`, approved/re-signed `spec.md`, `evidence.md`, probes/oracle, and approved amended `design.md` are present.
- Design gate: C20 is PASS; C1-C19 are PENDING with checkpointed-build owners; no FAIL and no approved-risk fence exists.
- Estimated slice diffs: 750 + 1,000 + 650 + 1,200 + 1,050 + 350 + 800 = **5,800 changed lines**.
- Churn margin: **20% = 1,160 lines**. Rationale: the change adds two public tools, a core seam, platform code, schema fixtures, and adversarial concurrency/limit tests; exact generated schema and cross-platform error handling commonly add one line in five beyond first decomposition.
- Projected total: **6,960 changed lines**. This exceeds the 4,000-line review-size gate, so the approved three independently mergeable increments are mandatory.

### PR increment A — Writable source-neutral foundation

Slices 1-4. Mergeable definition: reads expose truthful editable coordinates/mutability, profile/client grant propagation is complete, the source-neutral parser/engine/adapter seam is stable, and `rfs_write` create/replace works through MCP with direct filesystem contracts. No edit/delete/move syntax commits state yet; accepted `rfs_edit` parsing may exist only behind core tests. Verification does not depend on increments B or C.

### PR increment B — Snapshot-bound PUT/CUT

Slice 5. Mergeable definition: `rfs_edit` applies only approved `PUT`/`CUT` range/gap forms against the same-session seen union, atomically replaces content, and records receipt-derived coverage. It builds on increment A and does not depend on `REM` or `MV`.

### PR increment C — Explicit REM/MV and platform completion

Slices 6-7. Mergeable definition: `REM` and no-clobber same-source `MV` complete the grammar, with native atomicity/containment/cancellation fences and final public-tool integration. It builds on A+B and leaves no issue acceptance criterion pending. Its gate includes a native Windows execution of `filesystem_mutation_contract` plus the macOS cross-check, mirroring the rfs-r9m6 final Windows gate, because the spec's atomicity criterion requires platform-native direct adapter tests and not only the probe.

## Slice 1: Carry exact editable coordinates through reads and Path Sessions

**Claim IDs:** C1, C3

**Expected behavior:** Direct workspace reads preserve byte-exact structured content, optionally render numbered text, report only fully displayed original line ranges/EOF, and union quota-bounded coverage only within one canonical reference/full tag/live Path Session; unique prefixes of at least 12 hex characters resolve deterministically.

**Oracle:** Hand-counted byte/line tables for selectors/pages plus a test-local `BTreeSet<u64>` union and enumerated prefix table, independent of selector/session implementation.

**Stress fixture:** A 64 MiB UTF-8 file containing short lines, one over-column line split by the inline page, disjoint multi-ranges, Unicode, CRLF, a no-final-newline tail, and an empty file. Expected: only complete source lines enter `displayedRanges`; `displayedEof` is false until every tail byte is shown; default structured/text content remains exact; numbered text prefixes only the presentation channel.

**Regression fence:** `crates/resourcefs-core/tests/read_engine_contract.rs` coordinate cases; `crates/resourcefs-core/tests/path_session_contract.rs` snapshot cases; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs` read rendering cases.

**Named mutation:** C1 — in `read.rs`, mark the page's final partial line seen; `read_engine_contract::displayed_ranges_exclude_partial_lines` turns red. C3 — remove the Version Tag from the snapshot key in `session.rs`; `path_session_contract::seen_regions_never_cross_tags` turns red.

**Complexity/production scale:** Selector scan remains $O(n)$ for $n \le 64$ MiB; projection mapping is $O(r)$ for selected ranges; sorted interval union is $O(k)$ for stored disjoint intervals and every key/range byte is charged to the existing 256 MiB Path Session ceiling. Maximum accepted cost: one 64 MiB scan plus metadata bounded by the session ceiling; rationale: no new uncharged state and no higher source ceiling.

**Wall budget/phase:** Always-on read phase: ≤5 seconds for a 64 MiB local fixture, matching the repository's existing maximum-size read/session test budget.

**Files:** `crates/resourcefs-core/src/selector.rs`, `crates/resourcefs-core/src/resource.rs`, `crates/resourcefs-core/src/read.rs`, `crates/resourcefs-core/src/session.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/tests/selector_golden_contract.rs`, `crates/resourcefs-core/tests/read_engine_contract.rs`, `crates/resourcefs-core/tests/path_session_contract.rs`, `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`. (`schema_contract.rs` and `server-profile-v1.schema.json` cover the Server Profile schema only, which this issue does not change; the new optional `numbered` read field is a tool input schema asserted through `tools/list` fixtures in `stdio_mcp_contract.rs`.)

**Estimate:** 1.5 engineering days.

**Diff estimate:** 750 changed lines.

**PR increment:** A — Writable source-neutral foundation.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test selector_golden_contract --test read_engine_contract --test path_session_contract` → hand-counted ranges, EOF, exact content, session unions, prefixes, and quota rows agree item-by-item; each named mutation makes its named fence fail and restoration returns green.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract --test schema_contract` → default/numbered read text and the `tools/list` input schema (new optional `numbered` field) match fixtures; structured content is never numbered; the Server Profile schema hash is unchanged.

## Slice 2: Add the source-neutral engine seam, strict parser, locks, and commit-aware cancellation

**Claim IDs:** C2, C5, C9, C14, C17

**Expected behavior:** `resourcefs-core` exposes a deep `MutationEngine::{write, edit}` and separate object-safe `MutationAdapter::{resolve, load, commit}`; parsing accepts exactly the approved one-Resource grammar with teaching errors; canonical lock coordination serializes same keys and orders paired keys; cancellation wins only before `begin_commit`; parser/source/result bounds are exact and deterministic.

**Oracle:** Cargo dependency allowlist; hand-authored grammar table; test-only immutable line grammar parser; atomic counter/timeout schedule; repeated parse category comparison and fixture length arithmetic.

**Stress fixture:** Empty and 64 MiB patch documents; Unicode headers/bodies; every reserved `N*`/`>N*`/`@name` spelling; duplicate/overlapping headers; 1,000 snapshot tags sharing prefixes; two reverse key pairs released simultaneously. Expected: deterministic AST/category, bounded allocation, unique ≥12 prefix only, max one same-key critical section, and no reverse-pair deadlock.

**Regression fence:** New `crates/resourcefs-core/tests/hashline_patch_contract.rs` and `mutation_engine_contract.rs`; `crates/resourcefs-mcp/tests/architecture_contract.rs`; new `fuzz/fuzz_targets/hashline_patch.rs` registered in `fuzz/Cargo.toml`.

**Named mutation:** C2 — import `cap_std` in `resourcefs-core::mutation`; `architecture_contract::mutation_engine_dependency_direction` turns red. C5 — accept `PUT 1*:`; `hashline_patch_contract::reserved_block_forms_teach` turns red. C9 — acquire paired keys in request order; `mutation_engine_contract::reverse_moves_do_not_deadlock` turns red. C14 — keep drop-on-cancel behavior after commit start; `mutation_engine_contract::commit_masks_cancellation` turns red. C17 — remove the pre-AST input bound; `hashline_patch_contract::one_over_limit_fails_before_ast_growth` turns red.

**Complexity/production scale:** Parse/apply preparation is $O(p)$ for patch bytes $p \le 64$ MiB; overlap sorting is $O(h \log h)$ for h hunks whose storage is bounded by patch size; lock lookup is expected $O(1)$ and prefix resolution is $O(s)$ for snapshot records charged to the 256 MiB session ceiling. Maximum accepted cost: ≤5 seconds and no allocation beyond input/AST/output buffers plus 16 MiB measurement headroom; rationale: matches existing 64 MiB allocation fences.

**Wall budget/phase:** Always-on parse/lock phase: ≤5 seconds at the 64 MiB patch ceiling; fuzzing is one-off and has no wall budget beyond its specified 60-second run.

**Files:** New `crates/resourcefs-core/src/mutation.rs`, `crates/resourcefs-core/src/lib.rs`, `crates/resourcefs-core/src/error.rs`, `crates/resourcefs-core/src/session.rs`, new `crates/resourcefs-core/tests/hashline_patch_contract.rs`, new `crates/resourcefs-core/tests/mutation_engine_contract.rs`, `crates/resourcefs-mcp/tests/architecture_contract.rs`, `fuzz/Cargo.toml`, new `fuzz/fuzz_targets/hashline_patch.rs`, `fuzz/fuzz_targets/path_reference.rs` (repair per the approved design's housekeeping note: it calls the renamed `literal()` accessor and does not compile; switch it to `address()` so every fuzz target builds).

**Estimate:** 2 engineering days.

**Diff estimate:** 1,000 changed lines.

**PR increment:** A — Writable source-neutral foundation.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test hashline_patch_contract --test mutation_engine_contract` → every accepted/rejected grammar row, bound, lock schedule, and cancellation event matches its independent table/log; named mutations turn the named fences red then restore green.
- `cargo test -p resourcefs-mcp --test architecture_contract` → core has no MCP/filesystem/platform dependency and the separate adapter seam remains source-neutral.
- `cargo +nightly fuzz run --fuzz-dir fuzz hashline_patch -- -max_total_time=60` → no crash, panic, nondeterministic parse category, or unbounded allocation for 60 seconds.
- `cargo +nightly fuzz build --fuzz-dir fuzz` → all three targets (`path_reference`, `server_profile`, `hashline_patch`) compile; the pre-existing `path_reference` breakage is repaired in this slice.

## Slice 3: Propagate exact grants and truthful mutability through root replacement

**Claim IDs:** C15, C18, C19

**Expected behavior:** Profile grants reach filesystem roots; CLI roots are read-only; client-declared roots inherit grants only from a profile root with the identical canonical path; subdirectory/parent/unrelated clients stay read-only; `mutable` is true only for update-granted unlinked workspace UTF-8 text; an authority refresh cannot interleave a stale-generation commit.

**Oracle:** Parsed profile values and canonical-path equality table; explicit mutability truth table; root-generation event log controlled by gates.

**Stress fixture:** 256 profile roots and 256 client roots including identical path aliases, canonical-equal spellings, one parent, one subdirectory, one symlink alias, Unicode IDs, and concurrent equivalent/removing refreshes. Expected: only exact final canonical path equality inherits the matching grants; all other client/CLI roots have three false grants; removed generation never commits.

**Regression fence:** `crates/resourcefs-mcp/tests/profile_contract.rs`; `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`; `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Named mutation:** C15 — leave `SourceResource::text_projection` hardcoded false; `filesystem_adapter_contract::mutable_matches_update_authority` turns red. C18 — omit `grants` from `LaunchRoot`; `profile_contract::workspace_grants_reach_launch_roots` turns red; and, second C18 mutation from the approved design, match client roots to profile roots by canonical-path prefix instead of exact equality; `filesystem_adapter_contract::client_roots_inherit_grants_only_by_exact_path` turns red because a subdirectory client root gains grants. C19 — release the authority read guard before commit; `filesystem_mutation_contract::refresh_cannot_race_commit` turns red.

**Complexity/production scale:** Exact client/profile grant matching is $O(r_p r_c)$ with each set capped at 256, at most 65,536 canonical path comparisons per root refresh; path/link mutability inspection is $O(d)$ in path components under the 64 KiB reference ceiling. Maximum accepted root refresh cost: ≤5 seconds, the existing acquisition deadline; read mutability must stay within the existing 5-second maximum-file read budget.

**Wall budget/phase:** Root refresh is one-off per MCP roots event: N/A — reason: one-off phase; the existing five-second acquisition deadline is the gate. Mutability inspection is always-on read work and shares the ≤5-second maximum-file read budget.

**Files:** `crates/resourcefs-sources/src/filesystem.rs`, `crates/resourcefs-sources/src/catalog.rs`, `crates/resourcefs-sources/src/compiled.rs`, `crates/resourcefs-sources/src/lib.rs`, `crates/resourcefs-mcp/src/cli.rs`, `crates/resourcefs-mcp/src/profile/model.rs`, `crates/resourcefs-mcp/src/launch.rs`, `crates/resourcefs-mcp/tests/profile_contract.rs`, `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`, new `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 1 engineering day.

**Diff estimate:** 650 changed lines.

**PR increment:** A — Writable source-neutral foundation.

**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test profile_contract --test stdio_mcp_contract` → parsed/profile/client/CLI grant and `mutable` outputs match the canonical-path and truth tables.
- `cargo test -p resourcefs-sources --test filesystem_adapter_contract --test filesystem_mutation_contract` → root refresh gates prevent stale commits and each C15/C18/C19 named mutation makes its named fence fail before restoration.

## Slice 4: Implement atomic versioned rfs_write and generic public receipts

**Claim IDs:** C4, C8, C11, C16, C20

**Expected behavior:** Both new tools remain visible with discovery-first descriptions and object-rooted outputs; `rfs_write` creates only missing contained text with create grant and existing parent, replaces only current UTF-8 text with update grant/current full `ifVersion`, commits complete same-directory content atomically, preserves ordinary permissions, records authored coverage, and returns compact created/replaced receipts; policy denials precede state disclosure. Same-directory temporaries carry the `.rfs-tmp-` prefix, are removed on every in-process failure path, and are never swept automatically inside a Workspace Root.

**Oracle:** Eight-row grants × state truth table; pre/post content hashes and directory inventory; OS contract oracle/probe; checked-in expected schema/receipt JSON and literal text.

**Stress fixture:** Empty, Unicode, mixed-newline, exact 64 MiB and +1 content; missing parent; existing/missing/stale states; catalogs/artifacts; directories/binary/special files; Unix mode and Windows security/readonly fixtures; gated same-target writes and injected write/permission/rename failures. Expected: exact accepted bytes/receipt coverage, one stale loser, unchanged state and no leftover `.rfs-tmp-` file after every in-process failure, no transient partial/missing successful replacement.

**Regression fence:** `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs` (created in Slice 3, extended here); `crates/resourcefs-core/tests/mutation_engine_contract.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`, `schema_contract.rs`, and `architecture_contract.rs`; retained `.rfs-73dz` platform evidence.

**Named mutation:** C4 — treat missing `ifVersion` as unconditional replace; `mutation_engine_contract::write_state_matrix` turns red. C8 — check existence before grants; `mutation_engine_contract::policy_precedes_state` turns red. C11 — use `ReplaceFileW`; `filesystem_mutation_contract::replacement_has_no_missing_window` turns red. C16 — hide tools when grants are false; `stdio_mcp_contract::all_five_tools_remain_visible` turns red. C20 — switch the evidence probe to `ReplaceFileW`; P1 exposes missing/sharing windows.

**Complexity/production scale:** Current-state scan/hash is $O(n)$ and temp write/permission preparation is $O(n)$ for $n \le 64$ MiB; lock-held commit uses at most two complete 64 MiB buffers plus bounded metadata, with peak allocation ≤144 MiB. Maximum accepted cost: ≤5 seconds for exact-limit local create/replace and ≤144 MiB measured peak; rationale: two contents plus 16 MiB allocator/platform headroom.

**Wall budget/phase:** Always-on mutation request: ≤5 seconds for an exact-64-MiB local create/replace on the platform fixture; rationale matches existing maximum artifact/read budgets and prevents lock starvation.

**Files:** `crates/resourcefs-core/src/mutation.rs`, `crates/resourcefs-core/src/session.rs`, `crates/resourcefs-core/tests/mutation_engine_contract.rs`, `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/src/filesystem.rs`, new `crates/resourcefs-sources/src/filesystem_mutation.rs`, `crates/resourcefs-sources/src/compiled.rs`, `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`, `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`, `crates/resourcefs-mcp/tests/architecture_contract.rs`. The Server Profile schema (`server-profile-v1.schema.json`, guarded by `schema_contract.rs`) is not modified: grants already exist per root since rfs-r9m6, and the new tools' input/output schemas are asserted through `tools/list` fixtures in `stdio_mcp_contract.rs`.

**Estimate:** 2.5 engineering days.

**Diff estimate:** 1,200 changed lines.

**PR increment:** A — Writable source-neutral foundation.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test mutation_engine_contract` → write state/grant/precedence/receipt/concurrency rows agree with the independent matrix; C4/C8 mutations turn red then restore green.
- `cargo test -p resourcefs-sources --test filesystem_mutation_contract` → bytes, permissions, failure state, and exact-limit costs match precomputed fixtures on the current platform; C11 mutation turns red then restores green.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract --test schema_contract --test architecture_contract` → five visible tools, discovery-first descriptions, `tools/list` schemas, errors, and compact receipts match fixtures, and the Server Profile schema hash is unchanged; C16 mutation turns red then restores green.
- `./.rfs-73dz/probe_atomic_replace.py && ./.rfs-73dz/oracle_atomic_contracts.py` → Linux/Windows runtime and published Linux/Apple/Windows contracts agree item-by-item; C20 remains PASS.

## Slice 5: Apply snapshot-bound PUT/CUT and propagate safe receipt coverage

**Claim IDs:** C6, C7

**Expected behavior:** `rfs_edit` applies approved `PUT`/`CUT` hunks only to fully seen lines/gaps under the resolved same-session tag, uses original coordinates, rejects every overlap/unseen/partial target before commit, preserves untouched LF/CRLF/mixed bytes, atomically replaces content, and records only remapped seen plus authored coverage in the compact edited receipt.

**Oracle:** Naive test-only immutable line/gap interpreter plus per-byte provenance labels (`seen`, `authored`, `unseen`) propagated independently from production application.

**Stress fixture:** Multi-hunk reversed order; duplicate and adjacent gaps; overlapping ranges; disjoint same-tag reads; partial 513-column line; EOF/not-EOF `>$`; LF/CRLF/mixed/no-final-newline/empty files; Unicode; output exactly 64 MiB and +1. Expected: independent interpreter byte equality for accepted rows, `invalid_patch` and unchanged state for rejected rows, and zero unseen provenance promoted in receipts.

**Regression fence:** `crates/resourcefs-core/tests/mutation_engine_contract.rs`; `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Named mutation:** C6 — apply hunks sequentially to prior output; `mutation_engine_contract::hunks_use_original_coordinates` turns red. C7 — mark the entire edited result seen; `mutation_engine_contract::receipt_never_promotes_unseen_content` turns red.

**Complexity/production scale:** Application is $O(n + p + h \log h)$ for source $n$, patch $p$, each ≤64 MiB, and h hunks bounded by patch size; interval remap is $O(k+h)$ with k session-quota-bounded ranges. Peak allocation ≤208 MiB (source, patch, result, plus 16 MiB headroom). Maximum accepted cost: ≤5 seconds and ≤208 MiB for exact-limit deterministic fixtures.

**Wall budget/phase:** Always-on edit request: ≤5 seconds at the 64 MiB source/patch/result ceilings; rationale bounds the canonical lock interval and matches maximum-resource tests.

**Files:** `crates/resourcefs-core/src/mutation.rs`, `crates/resourcefs-core/src/session.rs`, `crates/resourcefs-core/tests/hashline_patch_contract.rs`, `crates/resourcefs-core/tests/mutation_engine_contract.rs`, `crates/resourcefs-sources/src/filesystem_mutation.rs`, `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`, `crates/resourcefs-mcp/src/server.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 2 engineering days.

**Diff estimate:** 1,050 changed lines.

**PR increment:** B — Snapshot-bound PUT/CUT.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test hashline_patch_contract --test mutation_engine_contract` → every accepted result matches the naive interpreter and provenance oracle; overlaps/unseen/partial rows stay unchanged; C6/C7 mutations turn red then restore green.
- `cargo test -p resourcefs-sources --test filesystem_mutation_contract` → accepted edit bytes/permissions and rejected state match fixture hashes.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract` → edited receipts, same-call errors, and consecutive edit coverage match object/text fixtures without content echo.

## Slice 6: Make REM the only deletion form

**Claim IDs:** C12

**Expected behavior:** A sole `REM` document deletes only an existing current snapshot-seen regular workspace text Resource under delete grant; empty writes/PUTs never delete; stale, missing, linked, unauthorized, cancelled, or extra-hunk forms preserve the entry and return the approved category; the receipt is `deleted` with no Version Tag.

**Oracle:** Pre/post directory-entry inventory and SHA-256 content hash over the explicit grant/state/syntax matrix.

**Stress fixture:** Empty file versus absent file; read-only/delete-only/update-only grants; stale/current prefixes; `REM` plus a second hunk; cancellation immediately before/after commit gate; injected remove failure. Expected: exactly one authorized current sole-REM row deletes; every other row has identical pre/post inventory/hash.

**Regression fence:** `crates/resourcefs-core/tests/mutation_engine_contract.rs`, `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Named mutation:** Treat empty `PUT` as delete; `mutation_engine_contract::rem_is_only_delete_form` turns red.

**Complexity/production scale:** Version validation hashes $O(n)$ bytes for $n \le 64$ MiB; directory removal is constant metadata work. Maximum accepted cost: ≤5 seconds and ≤80 MiB peak (content plus 16 MiB headroom).

**Wall budget/phase:** Always-on delete request: ≤5 seconds for a 64 MiB source validation and commit.

**Files:** `crates/resourcefs-core/src/mutation.rs`, `crates/resourcefs-core/tests/mutation_engine_contract.rs`, `crates/resourcefs-sources/src/filesystem_mutation.rs`, `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 0.75 engineering day.

**Diff estimate:** 350 changed lines.

**PR increment:** C — Explicit REM/MV and platform completion.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test mutation_engine_contract rem_` → sole-REM truth table matches precomputed outcomes and the named mutation turns the fence red before restoration.
- `cargo test -p resourcefs-sources --test filesystem_mutation_contract rem_` → every failure preserves entry/hash; current authorized row removes exactly one entry.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract rem_` → deleted receipt has no Version Tag and non-empty equivalent text.

## Slice 7: Complete contained atomic no-clobber MV on every supported platform

**Claim IDs:** C10, C13

**Expected behavior:** A sole `MV` locks sorted source/destination canonical keys, holds stable capability parent handles and authority generation, rejects every link/reparse/missing-parent/cross-source/cross-filesystem/existing-destination shape, and atomically renames only current snapshot-seen text under source-delete/destination-create grants; failure changes neither entry and success returns source/final destination plus unchanged Version Tag.

**Oracle:** Outside sentinel and final-handle path comparison; pre/post source/destination file IDs, inventory, and hashes; published/probed no-clobber primitive semantics.

**Stress fixture:** Relative/canonical/absolute aliases; Unicode/spaces; same path; existing destination; parent/subdirectory/cross-root/cross-mount destinations; symlink/reparse at every parent/final component retargeted during resolution; reverse paired moves; root refresh and cancellation gates. Expected: only missing same-source/same-filesystem/unlinked destination moves; no outside sentinel or copy; conflicts preserve both entries; reverse races terminate without deadlock.

**Regression fence:** `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`; `crates/resourcefs-core/tests/mutation_engine_contract.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`; platform probe/oracle.

**Named mutation:** C10 — replace stable parent handle with ambient canonicalize/reopen; `filesystem_mutation_contract::retargeted_links_never_mutate_outside` turns red. C13 — enable copy fallback/cross-filesystem move; `filesystem_mutation_contract::mv_never_copies_across_filesystems` turns red.

**Complexity/production scale:** Source validation hashes $O(n)$ for $n \le 64$ MiB; link/final-path checks are $O(d)$ in path components under the 64 KiB reference ceiling; sorted two-key acquisition is $O(1)$; rename is metadata-constant within one filesystem. Maximum accepted cost: ≤5 seconds and ≤80 MiB peak.

**Wall budget/phase:** Always-on move request: ≤5 seconds for maximum source validation plus atomic metadata commit; reverse-pair stress must finish within 2 seconds after gate release to detect deadlock.

**Files:** `crates/resourcefs-core/src/mutation.rs`, `crates/resourcefs-core/tests/mutation_engine_contract.rs`, `crates/resourcefs-sources/src/filesystem.rs`, `crates/resourcefs-sources/src/filesystem_mutation.rs`, `crates/resourcefs-sources/tests/filesystem_mutation_contract.rs`, `crates/resourcefs-mcp/src/render.rs`, `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`, `.rfs-73dz/probe_atomic_replace.py`, `.rfs-73dz/oracle_atomic_contracts.py`.

**Estimate:** 1.5 engineering days.

**Diff estimate:** 800 changed lines.

**PR increment:** C — Explicit REM/MV and platform completion.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test mutation_engine_contract mv_` → lock/state/grant rows match the inventory oracle.
- `cargo test -p resourcefs-core --test mutation_engine_contract reverse_moves_do_not_deadlock -- --exact` → reversed pairs terminate; named lock-related mutations are already fenced by Slice 2.
- `cargo test -p resourcefs-sources --test filesystem_mutation_contract mv_` → only valid no-clobber rows move and conflicts preserve both IDs/hashes.
- `cargo test -p resourcefs-sources --test filesystem_mutation_contract retargeted_links_never_mutate_outside -- --exact` → the outside sentinel remains untouched and C10/C13 mutations turn red then restore green.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract mv_` → moved receipt and stable errors match structured/text fixtures.
- `./.rfs-73dz/probe_atomic_replace.py && ./.rfs-73dz/oracle_atomic_contracts.py` → Linux/Windows runtime plus Apple cross-check and published contracts retain P1/P2 PASS.
- Native Windows VM run of `cargo test -p resourcefs-sources --test filesystem_mutation_contract` (statically linked, per the rfs-r9m6 Windows gate pattern) and `cargo zigbuild --target x86_64-apple-darwin -p resourcefs-sources --tests` → platform-native atomic visibility, permission, containment, and no-clobber fences pass on Windows and compile for macOS; this is increment C's release evidence for the spec's platform-native criterion.

## Tracker taxonomy

- Permanent non-goals: automatic parent creation, registers in one-Resource patches, cross-source/copy-delete moves, power-loss durability, richer metadata, arbitrary binary mutation, and mutating SQL retain the rationales recorded in `design.md`.
- Intended extensions: rfs-60g1, rfs-trdn, rfs-c85i, rfs-by2z, rfs-nb4s, rfs-cdlp, rfs-xiwg, and rfs-6le8 are verified tracker coverage; rfs-m739, rfs-7eqb, rfs-n22g, and rfs-h7iq cover structural parser work required before block-star patch forms.
- No new deferral was introduced by this plan.

## Self-review

- [x] C1-C20 are assigned exactly once: S1 C1/C3; S2 C2/C5/C9/C14/C17; S3 C15/C18/C19; S4 C4/C8/C11/C16/C20; S5 C6/C7; S6 C12; S7 C10/C13.
- [x] Every slice records all thirteen mandatory fields; every conditional field has a reason.
- [x] Every claim's regression fence and named mutation land in its owning slice; no fence-less approved risk exists.
- [x] Every new loop records asymptotic cost, production ceiling, measured maximum, and rationale; every always-on phase has a wall budget.
- [x] The 5,800 + 1,160 = 6,960 line projection requires and uses the approved A/B/C independently mergeable increments.
- [x] Every non-goal and intended extension is classified with rationale or verified tracker IDs.
- [x] No slice is declared complete; checkpointed-build exclusively judges completion.
