# Design: Versioned workspace text mutation

## Route and inputs

- Route: **Empirical**, from `.rfs-73dz/route.md` — cross-platform atomic replacement was unverified; the change also alters public schemas and module seams and carries concurrency/scale risk.
- Behavior source: `.rfs-73dz/spec.md`, approved 2026-08-22 with requester words "Approve revised spec". Its complete behavior set is: create workspace text; replace workspace text; expose editable coordinates; apply one-Resource range/gap hashline edits; `REM`; no-clobber same-source `MV`; stable rejection; per-canonical-Resource serialization; immutable catalogs/artifacts; compact mutation receipts.
- Edge and delivery source: `.rfs-73dz/spec.md` Decisions table. The approved increments are A (coordinates, mutability, numbered view, engine seam, parser, write), B (`PUT`/`CUT` and receipt coverage), C (`REM`/`MV` and platform proof).
- Empirical source: `.rfs-73dz/evidence.md`, `probe_atomic_replace.py`, and `oracle_atomic_contracts.py`. P1 PASS selects Unix `rename` and Windows `FileRenameInfoEx(REPLACE_IF_EXISTS | POSIX_SEMANTICS)` for atomic replacement; P2 PASS selects `rustix::RenameFlags::NOREPLACE` and Windows `FileRenameInfoEx(POSIX_SEMANTICS)` without replace for no-clobber moves. The Windows probe rejects `ReplaceFileW` for visibility because it exposed missing/sharing windows.
- Spec input: required and present. Route T4 fallback: N/A — `spec.md` is the complete behavior source.
- Approved risk acceptances: none.

## Input shapes

| Input family | Production-reachable shapes | Status |
|---|---|---|
| `rfs_write.path` | Relative; canonical workspace; contained native absolute; contained `file://`; ASCII, Unicode, spaces, percent-encoded delimiters; catalog; artifact; unsupported source | Covered by C1, C4, C8, C10, C19 |
| `rfs_write.ifVersion` | Missing; full exact tag; malformed tag; current tag; stale tag; tag against missing target | Covered by C4, C8 |
| `rfs_write.content` | Empty; ASCII; Unicode; LF; CRLF; mixed newlines; exact 64 MiB; one byte over | Covered by C4, C6, C17 |
| Workspace target state | Missing with existing parent; missing parent; regular UTF-8; invalid UTF-8; directory; special file; contained symlink/reparse traversal; removed root generation | Covered by C4, C8, C10, C15, C20 |
| Grants | All eight create/update/delete combinations; profile root; CLI root; client-declared root whose canonical path equals a profile root; subdirectory, parent, or unrelated client-declared root | Covered by C4, C15, C18, C19 |
| Read view | Default exact text; opt-in numbered text; full; bounded at newline; bounded mid-line; multi-range; zero-line; EOF displayed/not displayed; recovery artifact | Covered by C1, C3, C7 |
| Snapshot state | No snapshot; one read; disjoint same-tag reads; duplicate/overlapping reads; other tag; unique 12+ hex prefix; short/ambiguous/unknown prefix; other/invalidated Path Session; receipt-derived remap | Covered by C3, C7, C8 |
| Patch document | Empty; one header; extra header; full/prefix tag; `PUT N.=M:`, `<N`, `>N`, `>$`; `CUT N.=M`; sole `REM`; sole `MV`; multiple distinct hunks; duplicates; overlaps; original-coordinate reorder | Covered by C5, C6, C8, C12, C13 |
| Patch body | `+TEXT`; `+`; `+-`; `++`; missing `+`; Unicode; LF input rows applied to LF/CRLF/mixed target; exact/over-limit result | Covered by C5, C6, C17 |
| Reserved/malformed patch | `N*`; `>N*`; `CUT N*`; `@name`; bodyless register paste; zero/descending/past-EOF range; mid-line/partial-line target; unseen gap | Covered by C5, C7, C8 |
| Move destination | Same source/root or another root in the compiled workspace source; missing/existing; same path; missing parent; linked parent; cross-filesystem; cross-source | Covered by C8, C10, C13 |
| Concurrency | One mutation; two same-target mutations; distinct targets; two moves with reversed source/destination pairs; root refresh; external byte change; cancellation before/during/after commit | Covered by C9, C11, C13, C14, C20 |
| Receipt | Created; replaced; edited; deleted; moved; full authored coverage; line-delta remapped coverage; no content echo | Covered by C7, C16 |
| Non-workspace source-native semantics | Session Scratch, archives, SQLite, GitHub, Vault, skills/rules, notebooks | N/A — intended work is already tracked by rfs-60g1, rfs-trdn, rfs-c85i, rfs-by2z, rfs-nb4s, rfs-cdlp, and rfs-xiwg |
| Binary image/blob mutation | Image and unsupported binary write/edit | N/A — intended behavior is tracked by rfs-6le8; this change returns `unsupported_mutation` |
| Parent-directory tree creation | Zero or many missing parents | N/A — permanent non-goal: one write creates exactly one Resource and never synthesizes directories |
| Block-star grammar | Tree-sitter blocks across supported languages | N/A — intended work is tracked by rfs-m739, rfs-7eqb, rfs-n22g, and rfs-h7iq |
| Register grammar | Named/anonymous cut-and-paste registers | N/A — permanent non-goal: one-Resource documents make cross-section registers weightless |

## Removed invariants

The change is subtractive underneath its additive tools: it removes the repository-wide invariant that every compiled Source Adapter and every returned Resource is read-only.

- The old read-only invariant made grant propagation irrelevant. Removing it makes dropped profile grants a policy bug; C15 and C19 preserve explicit authority.
- It made `SourceAdapter::read` the only source seam. Removing it could push parser, locking, and Version Tag logic into MCP/filesystem callers; C2 preserves a source-neutral deep module.
- It made `SourceResource.mutable == false` truthful. Removing it requires grant/link-aware mutability; C15 preserves metadata truth.
- It made Path Session state artifact-only. Removing it could leak snapshots across tags/sessions or grow without bound; C3 and C7 preserve isolation and quota accounting.
- It made cancellation safe by dropping a pending worker. Removing it allows cancellation after a commit begins; C14 preserves known outcomes.
- It made concurrent source writes impossible. Removing it requires canonical lock identity and ordered two-key acquisition; C9 preserves serialization and deadlock freedom.
- Still safe: existing read/search/glob authority-generation checks remain the source of truth; mutation adds a commit-time generation check rather than changing read/discovery semantics.
- Known bound, stated so it is never assumed away: per-canonical-Resource serialization is in-process. Mutations by another process on the same files are detected only by compare-before-commit, which narrows but does not close the window between `load` and rename; no cross-process lock is claimed.

## Placement

### Source-neutral mutation engine

- **Owner:** new `resourcefs-core::mutation` module. `MutationEngine` owns `write`/`edit`, parsing/application, Version Tag rules, lock coordination, error precedence, receipts, and snapshot updates. Core wins because seven tracked Source families consume the same behavior and neither MCP nor filesystem types belong in the interface.
- **New seam alternatives:**
  1. Extend `SourceAdapter` with mutation methods. Smaller trait count, but every immutable adapter must expose meaningless mutation methods and read-only routing becomes coupled to write dependencies.
  2. Add an object-safe `MutationAdapter` beside `SourceAdapter`, with `resolve`, `load`, and `commit` operations over source-neutral `MutationTarget`, `MutationState`, and `SourceMutation` values. Slightly more interface, but immutable routing stays explicit and core tests use a real fake adapter. **Chosen.**
  3. Put a filesystem-only engine in `resourcefs-sources`. No new core seam, but each additional source adapter would reimplement parser, snapshots, locks, prefix resolution, receipts, and precedence. Rejected for shallow duplication.
- **Interface:** `MutationEngine::{write, edit}` is the caller interface. `MutationAdapter::{resolve, load, commit}` is the internal source seam: `resolve` canonicalizes and enforces operation policy before state disclosure; `load` supplies current source-neutral bytes/state after lock acquisition; `commit` rechecks authority, policy, type/link state, and expected Version Tag immediately before one source-native commit.
- **Forbidden:** MCP parsing hashline syntax; filesystem code unioning seen regions; core importing `cap_std`, `rmcp`, `rustix`, or `windows-sys`; adapters trusting engine-side state without commit-time revalidation.

### Path Session snapshots and cancellation

- **Owner:** `resourcefs-core::session`, using a private `SeenRegionSet` keyed by canonical reference plus full `VersionTag`. Sorted non-overlapping inclusive intervals and EOF flags union same-tag reads; key/range storage is charged to the existing Path Session byte quota and removed at invalidation.
- **New seam:** no external seam. Add narrow `PathSession::{record_seen, resolve_snapshot, record_receipt}` methods consumed only by read/mutation engines.
- **Cancellation interface:** extend `OperationGuard` with an atomic `Active -> Committing -> Complete` transition and `try_cancel`. Mutation calls `begin_commit` only after every fallible validation. MCP cancellation returns `cancelled` only when `try_cancel` wins; if commit already began, dispatch awaits the actual result.
- **Forbidden:** global snapshot maps; storing authoritative file content in snapshots; eviction that silently invalidates a live snapshot; returning cancellation after state may have changed.

### Read coordinate metadata

- **Owner:** `selector.rs` computes selected source-line/output-byte spans; `read.rs` intersects those spans with page/column limits and records only fully displayed lines/EOF; `resource.rs` carries `displayed_ranges`, `displayed_eof`, and numbered-view intent; MCP `render.rs` alone changes text presentation.
- **New seam:** no new adapter seam. Extend `SelectedText`/`SourceResource` projection metadata so source line identity survives selection without making MCP reconstruct it.
- **Forbidden:** prefixing structured `content`; treating a partial line as seen; artifact reads recording workspace coverage; MCP counting lines from rendered text.

### Filesystem mutation adapter

- **Owner:** a private child module of `resourcefs-sources::filesystem` (path-backed as `filesystem_mutation.rs`) implementing `MutationAdapter for FilesystemSource`; `CompiledSources` delegates workspace targets and denies catalogs/artifacts before state. Keeping it a child permits stable access to capability/root internals without widening crate visibility.
- **New seam:** it fills the core `MutationAdapter` seam; no second filesystem-specific interface.
- **Implementation:** extend `LaunchRoot`/`FilesystemRoot` with `MutationGrants`; profile conversion preserves configured grants; CLI roots use `MutationGrants::default()`; client MCP Roots inherit the grants of the profile root whose canonical path exactly equals theirs (after the existing canonical resolution) and otherwise use `MutationGrants::default()`, with inheritance computed inside the existing client-root refresh so a superseded refresh cannot carry stale grants. Resolve a stable parent `cap_std::fs::Dir`, compare its final handle path to the lexical parent to reject every link/reparse traversal, and operate on the basename relative to that handle. Temporary files are same-directory, random, `create_new`, fully written, and permission-prepared before commit; they carry a recognizable `.rfs-tmp-` prefix, are removed on every in-process failure path, and after abnormal termination are left in place for the operator — ResourceFS never sweeps or deletes files inside a Workspace Root on its own.
- **Platform commits:** Unix uses directory-relative `renameat` for replacement and `renameat_with(NOREPLACE)` for create/move; Windows uses directory-handle-relative `SetFileInformationByHandle(FileRenameInfoEx)` with replace/POSIX flags for replacement and POSIX without replace for create/move. Unix mode bits and Windows ordinary security/readonly data are copied before rename; richer metadata remains outside the approved guarantee.
- **Forbidden:** ambient canonicalize-then-reopen; `ReplaceFileW`; copy/delete move fallback; following a link for mutation; creating parents; writing target bytes in place; suppressing I/O results.

### MCP tools and receipts

- **Owner:** `resourcefs-mcp::server` defines strict camelCase `WriteInput { path, content, if_version }`, `EditInput { patch }`, and optional `ReadInput.numbered`; dispatch owns deserialization/root refresh/cancellation only. `render.rs` owns object-rooted receipts/errors and equivalent non-empty text.
- **New seam:** no new seam; both tools call `MutationEngine` and reuse the existing tool router/error rendering pattern.
- **Forbidden:** hiding tools based on grants; returning Resource content in receipts; protocol errors as operational tool errors; mutation authorization in tool descriptions.

### Parser fuzzing

- **Owner:** public deterministic `HashlinePatch::parse` in `resourcefs-core::mutation`, with a new `fuzz/fuzz_targets/hashline_patch.rs` target.
- **New seam:** no extra seam; fuzz and engine call the same parser interface.
- **Housekeeping:** the existing `fuzz/fuzz_targets/path_reference.rs` target does not compile (it calls the renamed `literal()` accessor); repair it in the same fuzz-crate change so all three targets build.
- **Forbidden:** regex-only parsing that accepts trailing garbage; panics/unchecked arithmetic; parser allocation above bounded input/result storage; treating reserved forms as generic syntax.

## Claims

- **C1:** Every direct workspace read reports exact fully displayed source ranges/EOF while preserving byte-exact structured content and optional numbered text.
- **C2:** One source-neutral core engine owns mutation semantics behind a separate object-safe adapter seam.
- **C3:** Seen regions union only for one canonical reference/full tag/live Path Session, resolve only unique prefixes of at least 12 hex characters, and remain quota-bounded.
- **C4:** `rfs_write` implements create-if-missing versus replace-if-current with exact grants, parent/type checks, 64 MiB bounds, and no implicit overwrite.
- **C5:** The parser accepts exactly approved range/gap/body/REM/MV forms and emits teaching `invalid_patch` errors for reserved or malformed forms.
- **C6:** `PUT`/`CUT` apply in original-coordinate space, reject every overlap/unseen/partial target, preserve untouched bytes/newline policy, and never exceed 64 MiB.
- **C7:** Successful authored receipts record only justified remapped/authored seen coverage, enabling safe consecutive edits without content echo.
- **C8:** Policy-before-state precedence and typed catalog/artifact/unsupported/link denials return the approved stable categories without state change.
- **C9:** Canonical per-Resource locks serialize validation through commit; two-target moves acquire sorted keys and cannot deadlock.
- **C10:** Capability-held parent directories and no-link checks prevent traversal, symlink/reparse mutation, missing-parent creation, and stale-root commits.
- **C11:** Create/replace/edit commit only complete same-directory temporary content, preserve ordinary permissions, and expose old-or-new bytes but never partial content.
- **C12:** `REM` alone deletes after delete grant, snapshot, type, generation, and Version Tag revalidation; every failed deletion preserves the entry.
- **C13:** `MV` is same-source, same-filesystem, atomic no-clobber under source-delete/destination-create grants; every conflict/failure preserves both entries.
- **C14:** Cancellation before commit changes nothing; cancellation after commit begins is masked until the actual result is returned.
- **C15:** `mutable` is true exactly for directly mutable, update-granted, unlinked workspace text and false for all immutable/ungranted/unsupported Resources.
- **C16:** Five truthful tools remain visible and return object-rooted structured output plus equivalent non-empty text; mutation receipts never repeat content.
- **C17:** Patch/source/result inputs obey exact 64 MiB bounds, checked arithmetic, deterministic parsing, and a permanent fuzz target.
- **C18:** Profile grants reach filesystem roots; CLI roots remain read-only; client-declared roots inherit grants only from a profile root with the identical canonical path and are otherwise read-only; no alternate grant convention exists.
- **C19:** Authority generation is rechecked under a read guard spanning commit, so refresh cannot authorize a stale root mutation.
- **C20:** The selected Linux, Windows, and macOS replacement/no-clobber primitives have the required published and observed semantics.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Exact coordinate metadata/content | Full, bounded, partial-line, multi-range, numbered/default, EOF/empty | Read fixtures and compare raw content plus computed complete-line ranges; any prefix in structured content or partial line marked seen falsifies | Hand-counted byte/line table independent of selector implementation | In `read.rs`, mark the page's final partial line seen; `read_engine_contract::displayed_ranges_exclude_partial_lines` turns red | Core read/selector and stdio read-coordinate contracts | <1 min | PENDING — checkpointed-build increment A |
| C2 | Source-neutral engine seam | Core engine, compiled/filesystem adapters, immutable adapters | Compile fake adapter core contracts and dependency-direction checks; any core filesystem/MCP dependency or MCP parser falsifies | Cargo manifest graph plus architecture allowlist | Import `cap_std` in `resourcefs-core::mutation`; `architecture_contract::mutation_engine_dependency_direction` turns red | Architecture contract plus fake-adapter engine tests | <1 min | PENDING — checkpointed-build increment A |
| C3 | Session-local bounded snapshot union/prefix | Same/disjoint/duplicate tags, prefixes, other session, quota edge | Compare unions/prefix resolution/quota accounting to explicit interval sets; cross-tag/session match or uncharged growth falsifies | Test-local `BTreeSet<u64>` line model and enumerated prefix table | Remove tag from snapshot key in `session.rs`; `path_session_contract::seen_regions_never_cross_tags` turns red | Path Session snapshot contracts | <1 min | PENDING — checkpointed-build increment A |
| C4 | Conditional write and grants | Missing/existing, all grant combinations, ifVersion branches, empty/64 MiB/over | Fake adapter and temp filesystem matrix; any overwrite without current tag, parent creation, or wrong grant succeeds falsifies | Explicit eight-row grants × four-state truth table | Treat missing `ifVersion` as unconditional replace in `mutation.rs`; `mutation_engine_contract::write_state_matrix` turns red | Core engine and filesystem write contracts | <2 min | PENDING — checkpointed-build increment A |
| C5 | Exact parser grammar/teaching errors | Every accepted, reserved, malformed, multi-header, Unicode body form | Golden corpus parses twice and compares AST/errors; any extra form accepted or reserved form lacking accepted grammar falsifies | Hand-authored grammar table from approved spec, not parser code | Accept `PUT 1*:` in parser; `hashline_patch_contract::reserved_block_forms_teach` turns red | Parser golden contract and fuzz invariants | <1 min | PENDING — checkpointed-build increment A |
| C6 | Original-coordinate safe application | Distinct/duplicate/overlap/unseen ranges/gaps, newline forms, boundaries | Apply corpus to LF/CRLF/mixed byte fixtures; any overlap commits, unseen target passes, or untouched byte changes falsifies | Naive test-only line/gap interpreter over immutable original rows | Apply hunks sequentially to prior output; `mutation_engine_contract::hunks_use_original_coordinates` turns red | Core patch application contracts | <2 min | PENDING — checkpointed-build increment B |
| C7 | Receipt-derived safe coverage | Create/replace/edit/delete/move and line-delta remaps | Compare receipt coverage to explicit old-seen/remap/authored set; any unobserved preserved byte becomes editable falsifies | Test-local provenance labels (`seen`, `authored`, `unseen`) propagated independently | Mark the whole edited result seen; `mutation_engine_contract::receipt_never_promotes_unseen_content` turns red | Receipt coverage and consecutive-edit contracts | <1 min | PENDING — checkpointed-build increment B |
| C8 | Stable policy/error precedence | Catalog/artifact/link/ungranted plus malformed/stale/unsupported states | Cross-product calls assert first approved category and unchanged state; state leak under denied policy falsifies | Approved precedence table copied as test data | Check existence before grants in filesystem adapter; `mutation_engine_contract::policy_precedes_state` turns red | Core/MCP error matrix | <2 min | PENDING — checkpointed-build increments A-C by operation |
| C9 | Serialization/deadlock freedom | Same/different targets, reverse paired moves, stale contenders | Gated multi-thread runs assert max one same-key commit, parallel distinct keys, one stale loser, and bounded reverse moves | Atomic counters and timeout-controlled schedule independent of lock registry | Acquire move keys in request order; `mutation_engine_contract::reverse_moves_do_not_deadlock` times out/red | Deterministic lock coordinator contracts | <2 min | PENDING — checkpointed-build increments A-C |
| C10 | Contained unlinked stable parent | Traversal, link/reparse each component, retarget race, missing parent, root refresh | Native fixtures race link/root changes and assert no outside sentinel/state change; outside mutation falsifies | Outside sentinel known only to test plus final-handle path comparison | Replace stable parent handle with ambient canonicalize/reopen; `filesystem_mutation_contract::retargeted_links_never_mutate_outside` turns red | Native filesystem containment contracts | <5 min/platform | PENDING — checkpointed-build increment C |
| C11 | Atomic permission-preserving commit | Empty/1 MiB/64 MiB, LF/CRLF, Unix modes, Windows access/readonly, injected failures | Reader stress and failure gates observe only complete versions, unchanged failure state, and equivalent ordinary permissions | OS docs/evidence probe plus independently sampled mode/security descriptor | Use `ReplaceFileW` on Windows; `filesystem_mutation_contract::replacement_has_no_missing_window` turns red | Native atomic replacement contracts | <5 min/platform | PENDING — checkpointed-build increments A/B; primitive premise C20 already PASS |
| C12 | Explicit safe REM | Sole REM, extra hunk, grants, stale/unseen/linked/missing, cancel | State matrix verifies only exact authorized current REM deletes and failed rows preserve bytes/entry | Pre/post directory-entry inventory and content hash | Treat empty PUT as delete; `mutation_engine_contract::rem_is_only_delete_form` turns red | Core and filesystem REM contracts | <2 min | PENDING — checkpointed-build increment C |
| C13 | Atomic no-clobber MV | Missing/existing/same/cross-source/cross-filesystem destinations, reversed races | Native matrix verifies conflict preservation and one successful no-copy rename; copy/delete or clobber falsifies | File IDs plus pre/post two-entry inventory; evidence oracle for primitive | On a cross-filesystem rename failure, fall back to `MoveFileExW(MOVEFILE_COPY_ALLOWED)` instead of returning `unsupported_mutation`; `filesystem_mutation_contract::mv_never_copies_across_filesystems` turns red | Native MV contracts | <5 min/platform | PENDING — checkpointed-build increment C; primitive premise C20 already PASS |
| C14 | Known cancellation outcome | Before/during/after commit gates | Cancel at each gate; `cancelled` with changed state or success before commit falsifies | Gate event log ordered independently of operation state enum | Keep generic drop-on-cancel dispatch for mutation; `stdio_mcp_contract::mutation_masks_cancellation_after_commit` turns red | Core operation-state and stdio cancellation contracts | <2 min | PENDING — checkpointed-build increments A-C |
| C15 | Truthful mutable metadata | Every grant/type/link/catalog/artifact shape | Direct reads compare `mutable` to approved truth table; any false writable or true denied Resource falsifies | Explicit table independent of renderer/adapter | Leave `SourceResource::text_projection` hardcoded false; `filesystem_adapter_contract::mutable_matches_update_authority` turns red | Source and stdio mutability contracts | <1 min | PENDING — checkpointed-build increment A |
| C16 | Five visible truthful tools/receipts | list_tools, every operation/error receipt, numbered/default reads | Schema/stdio fixtures assert five tools, every listed description beginning with the `rfs_read`/`rfs://` discovery cue (extending rfs-ewh2's `initialization_and_all_tool_descriptions_advertise_catalog`), object roots, non-empty equivalent text, no content echo | Checked-in expected JSON schemas and literal text receipts | Hide write/edit when grants false; `stdio_mcp_contract::all_five_tools_remain_visible` turns red | MCP schema, tool-list, and receipt contracts | <2 min | PENDING — checkpointed-build increments A-C |
| C17 | Bounded deterministic parser/mutation | Exact 64 MiB and +1 patch/source/result; arbitrary bytes | Exact/one-over allocation tests plus 60-second fuzz run; panic, nondeterminism, unchecked overflow, or over-limit commit falsifies | Length arithmetic in fixture builder and repeated parse category comparison | Remove preallocation bound before body parse; `hashline_patch_contract::one_over_limit_fails_before_ast_growth` turns red | Boundary contracts plus `hashline_patch` fuzz target | 1-2 min | PENDING — checkpointed-build increments A/B |
| C18 | Grant propagation has one convention | Profile/CLI roots; client roots matching a profile path, or a subdirectory, parent, or unrelated path; eight grant combinations | Profile conversion, launch construction, and client-root refresh assertions; dropped/non-default grants, a matching client root that loses grants, or a non-matching client root that gains any grant falsifies | Parsed profile fixture values and canonical paths compared directly to constructed/refreshed root grants | Omit `grants` in `LaunchRoot` (`profile_contract::workspace_grants_reach_launch_roots` turns red); match client roots by path prefix instead of equality (`filesystem_adapter_contract::client_roots_inherit_grants_only_by_exact_path` turns red) | Profile and filesystem construction contracts | <1 min | PENDING — checkpointed-build increment A |
| C19 | Commit uses current root generation | Retained/removed/equivalent roots, refresh before/during commit | Gate commit across refresh; removed generation mutation or refresh interleaving commit falsifies | Root IDs/generation event log controlled by test | Drop authority read guard before rename; `filesystem_mutation_contract::refresh_cannot_race_commit` turns red | Filesystem authority/mutation contract | <2 min | PENDING — checkpointed-build increments A-C |
| C20 | Platform primitives satisfy premises | Linux/Windows runtime and macOS contract/cross-check | Run published-contract oracle and runtime probe; any false contract check, partial/missing final primitive read, clobber, or compile failure falsifies | Published Linux/Apple/Microsoft contracts through `oracle_atomic_contracts.py` | Switch Windows probe primitive to `ReplaceFileW`; P1 records missing/sharing windows | `.rfs-73dz` probe/oracle evidence plus native permanent fences C11/C13 | ~25 sec probe; ~1 sec oracle | PASS |

## Non-goals and future work

### Permanent non-goals

- Automatic parent creation: excluded because one write mutates one Resource and avoids directory-tree rollback authority.
- Register grammar: permanently excluded because named/anonymous registers add no leverage to a one-Resource document; reserved spellings remain teaching errors.
- Cross-source or copy/delete cross-filesystem move: excluded because it cannot satisfy one atomic no-clobber commit.
- Power-loss durability and metadata beyond approved ordinary permissions: excluded because the contract is atomic visibility, not filesystem journaling/metadata cloning.
- Arbitrary binary mutation and mutating SQL: excluded to preserve the text/versioned Resource interface.

### Intended work with verified tracker IDs

- rfs-60g1: Session Scratch mutation consumes increment A's engine seam.
- rfs-trdn: archive mutation consumes the engine and adds archive rebuilds.
- rfs-c85i: SQLite row mutation consumes the engine and adds transactions.
- rfs-by2z: GitHub mutation consumes Version checks and adds remote operation reconciliation.
- rfs-nb4s: Vault mutation consumes the engine and adds frontmatter preservation.
- rfs-cdlp: skill/rule mutation consumes the engine and adds manifest semantics.
- rfs-xiwg: notebook editing consumes parser/version behavior and adds projection round-tripping.
- rfs-6le8: image/binary Resources retain explicit unsupported mutation behavior.
- rfs-m739, rfs-7eqb, rfs-n22g, and rfs-h7iq: language structural-summary work owns tree-sitter block boundaries before block-star patch forms can be added.

## Falsifier run log

- 2026-08-22 — C20 cheapest falsifier: `./.rfs-73dz/oracle_atomic_contracts.py` — PASS in 1.17 s. Linux replacement/no-clobber, Apple replacement/no-clobber, and Windows new-open/replace/ACL/explicit-copy contract checks all returned `true`.
- Upstream empirical runtime: `./.rfs-73dz/probe_atomic_replace.py` — PASS for native Linux, native Windows VM, and `x86_64-apple-darwin` cross-check; exact comparison is recorded in `evidence.md`.

## Approval

Requester approval (verbatim): "Approve. Re-read spec and design. C16 and C18 are ammended. proceed to budgetted plan with the amended design"
Date: 2026-08-22
Approved risk acceptances: None
