# Plan: the `local://` Session Scratch Source Adapter

## Inputs and partition arithmetic

- Route: Structural (`.rfs-60g1/route.md`). Spec signed; design approved 2026-08-23 ("I approve"), no risk acceptances. Falsification table C1–C12; C2 = PASS, all others `PENDING — checkpointed-build`.
- Slice diff estimates: S1 280 + S2 520 + S3 120 + S4 1,330 + S5 230 = **2,480 changed lines** (S4 grew by 180 under spec Revision 1: C13 family-root listing and C14 literal-wins selectors).
- Churn margin: **25% = 620 lines**. Rationale: adding `ResourceAddress::Local` and `GlobSource::Local` makes ~8 + ~3 exhaustive matches non-exhaustive, and changing `CompiledSources::{load,commit}` from a hardcoded field to source dispatch commonly adds one line in four beyond first decomposition (arm cascades, fixture updates, catalog assertion flips).
- Projected total: **3,100 changed lines** (2,480 + 620) — at or below the 4,000-line review-size gate, so the plan has **one PR increment: "Session Scratch (local://)"**.

### PR increment: Session Scratch (local://)

Slices 1–5. Mergeable definition against the repository default branch (`main`): `local://` parses and validates; scratch is created/read/searched/globbed/replaced/edited/deleted/renamed through the five public tools under Path Session authority with zero Workspace Mutation grants; quota is shared with `artifact://`, atomic, and never evicts; `local://` self-lists in `rfs://`; two sessions are isolated and references die at disconnect. Each slice leaves the repo green with its own observable check; the increment verifies through core grammar/session contracts and direct sources adapter/dispatch/quota/isolation contracts without any later issue. rfs-ww6w (Resources mirror) remains a separate downstream consumer.

## Slice 1: Add the `local://` address family and filename-like grammar

**Claim IDs:** C1

**Expected behavior:** `local://<name>` parses to `ResourceAddress::Local(LocalName)` exactly when the name is filename-like (printable UTF-8; percent-decode then reject a decoded `/` or `\`; reject `.`/`..`/empty/`>255` UTF-8 bytes); every other `local://` spelling returns `invalid_reference`; no existing address family's parse result changes. Every exhaustive `ResourceAddress` match gains a real `Local` arm (no catch-all); until later slices, those arms return a stable `source_unavailable` "local scratch source not mounted" rather than misrouting.

**Oracle:** Hand-authored accept/reject table mapping input string → expected `ResourceAddress` variant or error category, maintained independently of the parser branches.

**Stress fixture:** names `plan.md`, `review notes.md` (space), `设计.md` (Unicode), a 255-byte name, a 256-byte name, `a%2Fb`, `a%5Cb`, `..`, `.`, empty, and a name with a control char; plus every existing scheme (`rfs://workspace/r/p`, `artifact://…`, `rfs://`, `rfs://workspace`, relative, absolute, `file://`). Expected: the first four and the 255-byte name yield `Local`; the 256-byte, both encoded-separator, `.`/`..`/empty/control names yield `invalid_reference`; every existing scheme yields its prior variant unchanged.

**Regression fence:** `crates/resourcefs-core/tests/path_reference_contract.rs::local_name_grammar` (created in this slice) plus the existing golden path-reference corpus (unchanged).

**Named mutation:** From C1 — in `reference.rs`, skip the decoded-separator check in local-name validation; `path_reference_contract::local_name_grammar` turns red because `local://a%2Fb` is accepted. Restore → green.

**Complexity/production scale:** N/A — reason: name validation is a constant-bounded scan of ≤255 bytes inside the existing per-request parse path; no new loop over unbounded input.

**Wall budget/phase:** N/A — reason: constant-time validation folded into the existing `PathReference::parse` path; no new runtime phase.

**Files:** `crates/resourcefs-core/src/reference.rs` (add `ResourceAddress::Local(LocalName)`, `LocalName` newtype + validation, parse before the generic `contains("://")` rejection, canonical render); add `Local` arms to `crates/resourcefs-core/src/{discovery.rs,mutation.rs,read.rs,resource.rs,session.rs}` and `crates/resourcefs-sources/src/{artifact.rs,compiled.rs,filesystem_mutation.rs}` (stable placeholder error); `crates/resourcefs-core/tests/path_reference_contract.rs`.

**Estimate:** 0.75 day. **Diff estimate:** 280 (160 impl, 120 tests). **PR increment:** Session Scratch (local://).

**Commands and expected results:**
- `cargo test -p resourcefs-core --test path_reference_contract local_name_grammar` → accepted/rejected strings agree item-by-item with the literal table; existing families unchanged. Under the named mutation the row for `local://a%2Fb` fails; restored → green.
- `cargo build --workspace --all-targets --all-features` → compiles: every `ResourceAddress` match is exhaustive with a real `Local` arm, no catch-all.

## Slice 2: Scratch mutable named store with shared quota and containment

**Claim IDs:** C6, C12

**Expected behavior:** `PathSession` gains a per-session `HashMap<LocalName, ScratchEntry>` and crate-private `scratch_{load,put,remove,rename,names}` reusing the existing `used_bytes`/object-count accounting and the atomic `SessionStorage::{write_atomic,read,remove}` keyed by an internally allocated object id. Scratch bytes count against the 64 MiB object / 256 MiB / 1,000-object ceilings shared with `artifact://`; exact-limit content is accepted, one byte / one object over returns `limit_exceeded` before commit, and no existing object (scratch or artifact) is evicted. No name is ever turned into a backing filesystem path — the object id is the only key into storage.

**Oracle:** Independent byte/object accounting in the test plus a pre/post inventory hash over all stored objects; a backing-directory listing asserting only id-named objects exist (no name-derived path).

**Stress fixture:** drive the session-byte ceiling through a **lowered `ServerLimits.session_bytes`** (lower-only, validated against `MAX_SESSION_BYTES` in `limits.rs:81-86`) rather than writing a literal 256 MiB — fill to exactly the configured ceiling across mixed scratch + artifact objects, then attempt one more scratch byte; separately drive the object ceiling by creating exactly `MAX_SESSION_ARTIFACTS` (1,000, a hard const — small objects) then attempting the 1,001st; put an object at the configured object ceiling then one byte over; put scratch named `设计.md` and `review notes.md` and confirm the backing store shows only id-named files. Expected: over-ceiling puts return `limit_exceeded` with pre/post inventory byte-identical; no name appears as a filesystem path component. Rationale for the lowered ceiling: it exercises the identical accounting branch at a fraction of the I/O; the real 256 MiB constant is already fenced by rfs-34pz.

**Regression fence:** `crates/resourcefs-core/tests/local_quota_contract.rs::scratch_shares_and_never_evicts` (C6) and `crates/resourcefs-core/tests/local_containment_contract.rs::names_cannot_escape` (C12), both created in this slice. **Located in `resourcefs-core`, not `resourcefs-sources`:** the `scratch_*` API is crate-private to core, so an out-of-crate integration test cannot reach it. Core tests use the in-memory `SessionStorage` fake already established by `path_session_contract.rs`; any accessor these fences need beyond crate visibility follows the existing `#[cfg(feature = "test-support")]` wrapper pattern (`session.rs:360,594,606,672`) rather than widening the public API.

**Named mutation:** C6 — in `session.rs`, do not add scratch bytes to `used_bytes`; `scratch_shares_and_never_evicts` turns red (an over-ceiling scratch write succeeds). C12 — in the store, derive the backing path from the name instead of an allocated id; `names_cannot_escape` turns red (a name-shaped file appears / escapes). Restore each → green.

**Complexity/production scale:** `scratch_names()`/glob enumeration is O(N) over N ≤ 1,000 names; `scratch_put`/`load` is O(object bytes ≤ 64 MiB) for hash + atomic write, O(1) index update. Production scale: ≤1,000 objects, ≤256 MiB/session. Maximum accepted cost: name enumeration < 1 ms at 1,000 names; per-object put reuses the existing artifact write budget (no higher ceiling). Rationale: no new unbounded state; the ceilings are the existing rfs-34pz constants.

**Wall budget/phase:** Always-on (per scratch mutation): `scratch_put`/`load` ≤ the existing artifact atomic-write budget at the 64 MiB object ceiling; rationale: same storage path as `artifact://`, no new cost. Index enumeration is bounded < 1 ms and needs no separate budget.

**Files:** `crates/resourcefs-core/src/session.rs` (scratch map in `SessionState`, `scratch_*` API, `used_bytes`/object-count charge, disconnect clears it with the session, plus any `test-support` accessor the fences need); `crates/resourcefs-sources/src/session_storage.rs` (id-keyed atomic ops reused; no new store — modify only if the id allocation requires it); `crates/resourcefs-core/tests/local_quota_contract.rs`; `crates/resourcefs-core/tests/local_containment_contract.rs`.

**Estimate:** 1.5 days. **Diff estimate:** 520 (260 impl, 260 tests). **PR increment:** Session Scratch (local://).

**Commands and expected results:**
- `cargo test -p resourcefs-core --all-features --test local_quota_contract scratch_shares_and_never_evicts` → over-ceiling puts return `limit_exceeded`; pre/post inventory hash identical. C6 mutation flips it red; restore → green.
- `cargo test -p resourcefs-core --all-features --test local_containment_contract names_cannot_escape` → adversarial names produce only id-named backing objects; no foreign path. C12 mutation flips it red; restore → green.

## Slice 3: Widen the seen-region snapshot key to canonical `local://`

**Claim IDs:** C7

**Expected behavior:** `SnapshotResourceKey::parse` accepts a canonical `local://<name>` in addition to a canonical workspace reference, and still rejects every non-canonical reference and every other family (`artifact://`, catalogs, relative, absolute). Scratch edits can therefore reserve and resolve seen regions keyed by `local://<name>`.

**Oracle:** Literal reference → accept/reject table, independent of `SnapshotResourceKey::parse`'s branches.

**Stress fixture:** canonical `local://plan.md`; canonical `rfs://workspace/r/p`; non-canonical `plan.md`; `artifact://…`; `rfs://`; a `local://` name with a decoded separator (already invalid upstream). Expected: the two canonical references parse to a key; every other reference errors.

**Regression fence:** `crates/resourcefs-core/tests/path_session_contract.rs::snapshot_key_accepts_local_rejects_foreign` (created in this slice).

**Named mutation:** From C7 — leave `SnapshotResourceKey::parse` workspace-only; the new row turns red because a `local://` reference cannot form a key. Restore → green.

**Complexity/production scale:** N/A — reason: key parsing is a constant-bounded string check with no new loop.

**Wall budget/phase:** N/A — reason: folded into the existing snapshot-record path; no new runtime phase.

**Files:** `crates/resourcefs-core/src/session.rs` (`SnapshotResourceKey::parse` at line 211 accepts canonical `local://`); `crates/resourcefs-core/tests/path_session_contract.rs`.

**Estimate:** 0.25 day. **Diff estimate:** 120 (40 impl, 80 tests). **PR increment:** Session Scratch (local://).

**Commands and expected results:**
- `cargo test -p resourcefs-core --test path_session_contract snapshot_key_accepts_local_rejects_foreign` → canonical workspace and `local://` accepted, every other reference rejected, agreeing with the literal table. C7 mutation flips it red; restore → green.

## Slice 4: Local Source Adapter, dispatch-by-source, and catalog mount

**Claim IDs:** C2, C3, C4, C5, C8, C10, C11, C13, C14

**Spec Revision 1 addendum (folded here rather than reworking committed Slice 1):** extend the grammar so bare `local://` parses as the scratch family root (C13) and so `local://<name>:<selector>` yields a selector candidate resolved literal-first (C14) — the grammar extension is inert until this slice's adapter exists, so its fences belong beside the behavior they guard. `LocalSource::read` serves the root listing (catalog-style, reusing the rfs-ewh2 root-listing rendering, NOT rfs-hwlm directory listing) and applies the literal-first fallback already implemented for the workspace family in `filesystem.rs::read_from_view`. Because bare `local://` is now a valid reference, `SearchTarget::resource()` accepts `path: "local://"` directly — **no bare-string accommodation is needed in the search path**, which removes work this slice would otherwise have carried.

**Expected behavior:** a new `LocalSource` (in `sources/local.rs`, mirroring `artifact.rs`) implements `SourceAdapter` (read scratch), `DiscoveryAdapter` (search all/one scratch; glob flat names), `MutationAdapter` (`resolve`/`load`/`commit` for Create/Replace/Delete/Move under Path Session authority with **no** `MutationGrants`), and `SourceCatalogMetadata`. `CompiledSources` gains a `local` field, `Local` arms for read/resolve/search, a `GlobSource::Local` arm, and — the subtractive change — `load`/`commit` dispatch by the target's source instead of hardcoding `self.filesystem`. `MutationEngine` is unchanged (C2, already PASS). Scratch create/replace/edit/REM/same-source-`MV` succeed with zero grants while workspace mutation without a grant still returns `permission_denied` (C3); `artifact://` and catalogs stay immutable (C5); cross-source `MV` returns `unsupported_mutation` with both sides unchanged (C8); concurrent mutation of one name serializes with one `version_conflict` loser (C10); `local://` self-lists in `rfs://` and scratch reads report `mutable: true` (C11).

**Oracle:** per-adapter call counters (a gate on the filesystem adapter's `load`) for dispatch; a literal grant×operation truth table for authority; a literal reference→category table for immutability and cross-source rejection; an atomic commit counter + content hash for serialization; a literal expected catalog line + read-output key set for the catalog/mutable claim; and the existing `FakeAdapter`/`StatefulAdapter` (in-memory) for the C2 engine re-confirmation. Each is a different mechanism than the production Local adapter.

**Stress fixture:** with zero grants, run create → read → search → glob → replace(`ifVersion`) → edit(`PUT`/`CUT`) → `REM` → same-source `MV` on `local://` and assert each succeeds; run a workspace mutation with no grant and assert `permission_denied`; gate the filesystem adapter's `load`, run a `local://` edit and assert the gate is untouched and the scratch store changed, then run a workspace edit and assert `LocalSource` is untouched; attempt `rfs_edit` against `artifact://x` and `rfs://workspace` and assert `permission_denied`; submit `MV local://d` from a workspace source and `MV rfs://workspace/r/p` from `local://s` and assert `unsupported_mutation` with both entries unchanged; two gated concurrent replaces of `local://p` — assert one commits, the loser is `version_conflict`, no torn content; read `rfs://` and assert a `local://` line with grammar + example, and a scratch read reports `mutable:true`.

**Regression fence:** created in this slice — `crates/resourcefs-sources/tests/local_root_contract.rs::{root_lists_all_scratch_sorted, root_is_immutable}` (C13); `crates/resourcefs-sources/tests/local_selector_contract.rs::literal_scratch_name_wins` (C14); `crates/resourcefs-sources/tests/local_mutation_contract.rs::{scratch_needs_no_grant (C3), cross_source_mv_rejected (C8)}`; `crates/resourcefs-sources/tests/local_dispatch_contract.rs::{local_never_touches_filesystem, workspace_never_touches_local (C4), artifact_and_catalog_immutable (C5)}`; `crates/resourcefs-sources/tests/local_concurrency_contract.rs::same_name_serializes (C10)`; the `crates/resourcefs-sources/src/compiled.rs` catalog test (`source_lines.len()` 2→3, ordered) and `crates/resourcefs-sources/src/catalog.rs:375` (local now present) plus a stdio read-shape assertion for `mutable:true` (C11); and the existing `crates/resourcefs-core/tests/mutation_engine_contract.rs` (C2, unchanged). Two C4 fence tests intentionally cover the mutation-dispatch and both routing directions for localized failure attribution while sharing the C4 claim ID.

**Named mutation:** C13 — `local.rs` renders the root listing from unsorted map iteration order; `root_lists_all_scratch_sorted` red on the many-Resource row. C14 — try the selector interpretation before the literal name; `literal_scratch_name_wins` red (the literal-named Resource is shadowed). C3 — `local.rs` `resolve` consults a grant flag and denies; `scratch_needs_no_grant` red. C4 — leave `compiled.rs` `load` hardcoded to `self.filesystem`; `local_never_touches_filesystem` red (a `local://` edit loads workspace `Missing`). C5 — add a Local-style arm resolving `artifact://` for mutation; `artifact_and_catalog_immutable` red. C8 — allow a cross-source move key pair; `cross_source_mv_rejected` red. C10 — give all local targets a constant lock key; `same_name_serializes` red (second/ torn commit). C11 — omit the local entry from `catalog_entries`; the catalog rows and `source_lines.len()==3` turn red. C2 — add `matches!(reference.address(), ResourceAddress::Workspace(_))` guard in `MutationEngine::write`; the fake-adapter rows in `mutation_engine_contract` turn red. Restore each → green.

**Complexity/production scale:** scratch search scans ≤1,000 objects whose total content is bounded by the 256 MiB session ceiling; glob scans ≤1,000 names; both reuse the rfs-vl0u discovery budgets and the bounded-result machinery. Read reuses the common bounded-read path. Maximum accepted cost: search/glob within the existing 5-second discovery wall budget at the session ceiling; rationale: same engine and ceilings as workspace discovery, no new dimension.

**Wall budget/phase:** Always-on (per request): scratch read reuses the existing bounded-read budget; scratch search/glob reuse the existing 5-second discovery budget at the 256 MiB / 1,000-object ceiling; mutation reuses the rfs-73dz per-mutation budget. Rationale: no new engine, only a new adapter behind proven budgets.

**Files:** new `crates/resourcefs-sources/src/local.rs`; `crates/resourcefs-sources/src/compiled.rs` (`local` field, `[&self.filesystem,&self.artifacts,&self.local]` list, `Local` read/resolve/search arms, `load`/`commit` dispatch by source at lines 105/113, `GlobSource::Local` at 153, `catalog_metadata` add local, catalog test len); `crates/resourcefs-core/src/discovery.rs` (`GlobSource::Local` variant + arms); `crates/resourcefs-sources/src/filesystem.rs` (`GlobSource` match arm); `crates/resourcefs-sources/src/catalog.rs` (local entry; flip line 375); `crates/resourcefs-sources/src/lib.rs` (`mod local`); tests: `local_mutation_contract.rs`, `local_dispatch_contract.rs`, `local_concurrency_contract.rs`, and stdio read-shape assertions in `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 3.5 days. **Diff estimate:** 1,330 (640 impl, 690 tests) — 1,150 plus 180 for the Revision 1 grammar extension, the root listing, the literal-first fallback, and the C13/C14 fences. **PR increment:** Session Scratch (local://).

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test local_mutation_contract` → every zero-grant scratch operation succeeds; cross-source `MV` returns `unsupported_mutation` with both sides unchanged. C3 and C8 mutations flip their named rows red; restore → green.
- `cargo test -p resourcefs-sources --test local_dispatch_contract` → a `local://` mutation never touches the gated filesystem adapter and a workspace mutation never touches Local; `artifact://`/catalog mutation is `permission_denied`. C4 and C5 mutations flip their rows red; restore → green.
- `cargo test -p resourcefs-sources --test local_concurrency_contract same_name_serializes` → one commit, one `version_conflict`, no torn content. C10 mutation flips it red; restore → green.
- `cargo test -p resourcefs-sources catalog:: && cargo test -p resourcefs-mcp --test stdio_mcp_contract` → `rfs://` lists a `local://` line with grammar + example; scratch read reports `mutable:true`; `source_lines.len()==3`. C11 mutation flips the catalog rows red; restore → green.
- `cargo test -p resourcefs-core --test mutation_engine_contract` → the in-memory fake-adapter rows stay green (C2). The C2 guard mutation flips them red; restore → green.

## Slice 5: Per-session isolation and disconnect lifecycle

**Claim IDs:** C9

**Expected behavior:** two concurrent Path Sessions holding identically named scratch never observe each other through read, search, glob, or mutation, and on disconnect a session's `local://` references invalidate immediately while its backing state is TTL-cleaned — reusing the rfs-34pz per-`SessionState` scoping and disconnect invalidation, now exercised through the full Local adapter.

**Oracle:** session-scoped expected contents (distinct sentinels per session), independent of the store keying; a post-disconnect resolution that must fail.

**Stress fixture:** sessions A and B each create `local://shared.md` with distinct content; assert A's read/search/glob/mutation only ever sees A's content and never B's, and vice versa; disconnect A and assert every `local://` reference from A fails while B is unaffected; confirm A's backing objects are TTL-eligible.

**Regression fence:** `crates/resourcefs-sources/tests/local_isolation_contract.rs::{sessions_never_cross, disconnect_invalidates}` (created in this slice).

**Named mutation:** From C9 — in `session.rs`, key the scratch map globally instead of per `SessionState`; `sessions_never_cross` turns red (A observes B's scratch). Restore → green.

**Complexity/production scale:** N/A — reason: no new production loop; isolation is the existing per-`SessionState` map scoping and this slice adds the cross-session observable fence.

**Wall budget/phase:** N/A — reason: no new runtime phase; the fence exercises existing per-session and disconnect paths.

**Files:** `crates/resourcefs-core/src/session.rs` (only if disconnect must explicitly drop the scratch map beyond existing `SessionState` invalidation); `crates/resourcefs-sources/tests/local_isolation_contract.rs`.

**Estimate:** 0.75 day. **Diff estimate:** 230 (30 impl, 200 tests). **PR increment:** Session Scratch (local://).

**Commands and expected results:**
- `cargo test -p resourcefs-sources --test local_isolation_contract` → A and B never observe each other's scratch through any tool; A's references fail after disconnect while B is unaffected. C9 mutation (global scratch map) flips `sessions_never_cross` red; restore → green.

## Tracker taxonomy

- Permanent non-goals (rationale in `design.md`; no tracker issue): hierarchical scratch paths, Structural Summary of scratch, binary scratch mutation, cross-session persistence, and FS-promotion.
- Intended future work: **rfs-ww6w** (MCP Resources/templates mirror of `local://`) — verified existing tracker ID, no work added here. Converter-by-extension is classified a permanent non-goal for this change in `design.md` (no commitment, no trigger) — no filing.
- No new deferral phrase was introduced by this plan.

## Self-review

- [x] Every design row is assigned to exactly one slice: S1 C1; S2 C6/C12; S3 C7; S4 C2/C3/C4/C5/C8/C10/C11/C13/C14; S5 C9. (C1's Revision 1 grammar extension is implemented in S4 alongside C13/C14, since the extension is inert without the adapter; S1's committed C1 fence remains green and is re-run there.) Every slice's Claim IDs exist in the design table; every `PENDING` falsifier is discharged by the slice implementing its claim (C2's PASS is re-confirmed, not re-run, in S4).
- [x] Every slice records all thirteen mandatory fields; conditional cells carry `N/A — reason`.
- [x] Every claim's fence is created in its implementing slice; every new fence carries its design-approved named mutation; no fence-less claim exists (design approved zero risk acceptances).
- [x] Every new loop (S2 name enumeration, S4 scratch search/glob) records asymptotic cost, production ceiling, and explicit maximum accepted cost with rationale; every always-on phase (S2 put/load, S4 read/search/glob/mutation) records a wall budget tied to an existing proven ceiling.
- [x] The partition rule was applied: 2,480 + 25% churn (620) = 3,100 ≤ 4,000 → one PR increment ("Session Scratch (local://)"); every slice names it; the increment has a mergeable definition verifying without any later issue.
- [x] The tracker taxonomy is applied; the one intended-future-work item cites verified `rfs-ww6w`.
- [x] The plan declares no slice complete; `checkpointed-build` exclusively judges completion.
