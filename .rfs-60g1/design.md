# Design: the `local://` Session Scratch Source Adapter

## Route and inputs

- **Route:** Structural, from `.rfs-60g1/route.md` — new public `local://` address family, a new source-adapter module, and the first non-filesystem `MutationAdapter`, with no unverified empirical premise.
- **Behavior source:** `.rfs-60g1/spec.md`, signed 2026-08-23. Its Decisions table is authoritative: flat names; filename-like grammar; scratch writable under Path Session authority with zero Workspace Mutation grants; reuse the rfs-73dz mutation contract; reuse the rfs-vl0u discovery engine; `artifact://`/catalogs stay immutable; cross-source MV → `unsupported_mutation`; numbered projection yes / Structural Summary no; shared 64 MiB / 256 MiB / 1,000-object session budget.
- **Empirical premises:** N/A — Structural route, `route.md` T1 = no. All reused infrastructure (rfs-73dz mutation engine, rfs-34pz session storage/quotas, rfs-vl0u discovery) is proven in-repo.
- **Evidence/probe input:** N/A — no probe on a Structural route.
- **Generalization premise checked now:** the cheapest falsifier (below) confirms the `MutationEngine` already drives write/replace/edit/REM/MV through non-filesystem `MutationAdapter` impls, so scratch is a new adapter plus dispatch/snapshot-key widening — not engine surgery.

## Input shapes

| Input | Production-reachable shapes | Status |
|---|---|---|
| `local://` reference | valid filename-like name (ASCII, spaces, Unicode, with extension); bare `local://` (the scratch family root — a valid reference, spec Revision 1); `local://<name>:<selector>` (literal-wins, spec Revision 1); `local://<glob>` pattern; invalid: `.`, `..`, `>255` UTF-8 bytes, decoded `/` or `\`, control char | Covered by C1, C12, C13, C14 |
| Existing address families | `rfs://workspace/...`, `artifact://...`, `rfs://`, `rfs://workspace`, relative/absolute/file-URI workspace — must retain exact behavior | Covered by C1 (no existing family changes) |
| Scratch content | empty, ASCII, Unicode, LF/CRLF/mixed newlines, exact 64 MiB, one byte over | Covered by C6; text conventions inherited from rfs-73dz |
| `rfs_write.ifVersion` | absent (create), full Version Tag, unique ≥12-hex prefix, stale, tag against missing name | Covered by C3, C7 (semantics reused from rfs-73dz) |
| Mutation operation | create, replace, hashline `PUT`/`CUT`, sole `REM`, same-source `MV` (missing dest / existing dest), cross-source `MV` (`local://`↔workspace) | Covered by C3, C4, C8, C10 |
| Session scratch state | zero names; one; many; name collision within a session; identical names across two sessions; at byte ceiling; at object ceiling; disconnected session | Covered by C6, C9 |
| Read view | full; bounded; numbered projection; content over the text ceiling spilling to `artifact://` recovery | Covered by C2-read, C11; Structural Summary excluded (spec) |
| Search / glob target | `local://` (all scratch); `local://<name>` (one); `local://<glob>`; unchanged workspace/artifact/catalog routing | Covered by C4 |
| Immutable targets | `rfs://`, `rfs://workspace`, any `artifact://` under mutation | Covered by C5 |
| Concurrency | two writers, same `local://` name; two writers, distinct names; `MV` races | Covered by C10 |
| Numeric | name length 1 / 255 / 256 bytes; content 0 / 64 MiB / 64 MiB + 1; object count 1000 / 1001 | Covered by C1, C6 |

## Removed invariants

The change is additive at the tool surface (a new adapter) but **subtractive** in two support layers whose current form assumes workspace + artifact are the only families. Each removed invariant becomes a claim.

- **Invariant removed:** "the seen-region snapshot store identifies only canonical *workspace* references" — `SnapshotResourceKey::parse` (`session.rs`) rejects anything but `Workspace(Canonical)`. Scratch edits reserve/resolve seen regions keyed by `local://<name>`, so this key must accept canonical local references too. Now-possible violation: a scratch edit cannot reserve its seen region, or the key is widened too far and accepts a foreign reference. → **C7**.
- **Invariant removed:** "all mutation `load`/`commit` route to the filesystem adapter" — `CompiledSources::{load,commit}` hardcode `self.filesystem` (`compiled.rs`), and `MutationAdapter::resolve` maps only Workspace→filesystem with Artifact/Catalog→denied. Removing it means dispatching `load`/`commit` by the target's source. Now-possible violation: a `local://` mutation loads or commits against the filesystem adapter (or vice versa). → **C4**.
- **Still safe:** `MutationTarget::new` (`mutation.rs`) rejects only *non-canonical Workspace* references, so a canonical `local://` target already flows through unchanged — no widening needed. The `MutationEngine` write/edit/REM/MV bodies contain no workspace-specific branch (confirmed by the cheapest falsifier). Per-canonical-Resource lock identity is `source_key + reference`, already source-distinct.
- **Additive (not subtractive):** the new `ResourceAddress::Local` and `GlobSource::Local` variants make existing exhaustive matches non-exhaustive at compile time — a compiler-enforced completeness signal, not a removed guarantee. No catch-all `_ =>` arm may absorb them.

## Placement

### `local://` address family
- **Owner:** `resourcefs-core/src/reference.rs`. Add `ResourceAddress::Local(LocalName)` and parse `local://<name>` before the generic `contains("://")` rejection, validating the filename-like grammar (printable UTF-8; percent-decode then reject a decoded `/`/`\`; reject `.`/`..`/empty/`>255` bytes). Canonical rendering is `local://<name>`.
- **New seam:** none — extends the existing closed `ResourceAddress` sum, exactly as `Catalog` did (rfs-ewh2).
- **Forbidden:** accepting a decoded separator, `.`/`..`, empty, or over-length as valid; MCP string-prefix interception of `local://`; any exhaustive `ResourceAddress` match using a catch-all arm instead of a real `Local` arm (`compiled.rs`, `resource.rs::validate_canonical_identity`, `session.rs`, `read.rs`).

### Scratch mutable named store
- **Owner:** `PathSession` (`resourcefs-core/src/session.rs`) owns the name→(object-id, Version Tag) index plus quota accounting; `DiskSessionStorage` (`resourcefs-sources/src/session_storage.rs`) owns the durable bytes.
- **New seam — chosen shape:** crate-private `PathSession` scratch operations — `scratch_load(name) -> MutationState`, `scratch_put(name, content) -> VersionTag`, `scratch_remove(name)`, `scratch_rename(src, dest)`, `scratch_names()` — that reuse the existing `used_bytes` / object-count accounting and the atomic `SessionStorage::{write_atomic, read, remove}` keyed by an internally allocated object id, with a `HashMap<LocalName, ScratchEntry>` in `SessionState`. Mutability lives in the name→id indirection (replace overwrites atomically; the id is private).
  - **Alternative A — rejected:** a new `ScratchStorage` trait beside `SessionStorage`. Duplicates the atomic-write, disconnect, and TTL-GC machinery rfs-34pz already owns, and splits the shared session budget across two stores.
  - **Alternative B — rejected:** hold scratch content inline in `SessionState` memory. Unbounded process memory, no disconnect/TTL parity with `artifact://`, and it breaks the "shared 256 MiB session budget" contract the spec pins.
- **Forbidden:** a scratch store outside the Path Session; scratch bytes not charged to `used_bytes`; evicting any existing object on quota failure; scratch surviving disconnect; a global (non-per-session) name map.

### Seen-region snapshot key widening
- **Owner:** `SnapshotResourceKey::parse` (`resourcefs-core/src/session.rs`). Accept a canonical `local://<name>` in addition to a canonical workspace reference.
- **New seam:** none.
- **Forbidden:** accepting any non-canonical reference or any family other than workspace/local; a scratch edit resolving against a workspace snapshot.

### Local Source Adapter
- **Owner:** new `crates/resourcefs-sources/src/local.rs`, mirroring `artifact.rs`. Implements `SourceAdapter` (read scratch), `DiscoveryAdapter` (search/glob over `PathSession::scratch_names()` + content), `MutationAdapter` (`resolve`/`load`/`commit` for `Create`/`Replace`/`Delete`/`Move`), and `SourceCatalogMetadata` (the `local://` entry).
- **New seam:** none — fills the existing `SourceAdapter`/`DiscoveryAdapter`/`MutationAdapter`/`SourceCatalogMetadata` seams.
- **Forbidden:** importing `cap-std`/`rustix`/`windows-sys`/workspace types; consulting a `MutationGrants` (scratch authority is session ownership); binary mutation; a `commit` that is not atomic-or-unchanged.

### Composite dispatch and authority routing
- **Owner:** `CompiledSources` (`resourcefs-sources/src/compiled.rs`). Add a `local: LocalSource` field; add `Local` arms to `read`, `resolve`, and `search`; change `load`/`commit` to dispatch by the target's source (reference address / `source_key`) rather than hardcoding `self.filesystem`; add a `GlobSource::Local` glob arm.
- **New seam:** `GlobSource::Local` variant in `resourcefs-core/src/discovery.rs`, and `GlobTarget::new` maps a `local://` glob to `GlobSource::Local`.
- **Forbidden:** `load`/`commit` reaching the wrong adapter; a catch-all match arm hiding a missing `Local` case; consulting `MutationGrants` for scratch; marking `artifact://` mutable.

### Catalog mount and mutable classification
- **Owner:** `LocalSource::catalog_entries` + `CompiledSources::catalog_metadata` (add the local source); the read path reports `mutable: true` for `local://`.
- **Forbidden:** advertising `local://` while unmounted. Update the two existing assertions: `compiled.rs` catalog test (`source_lines.len()` 2 → 3, ordered) and `catalog.rs:375` (`local://` now present, not absent).

## Claims

- **C1:** `local://<name>` parses to `ResourceAddress::Local` exactly when the name is filename-like (printable UTF-8, no decoded `/`/`\`, not `.`/`..`, ≤255 UTF-8 bytes); bare `local://` parses as the scratch family root (C13); every other `local://` spelling is `invalid_reference`, and no existing address family's parse result changes.
- **C2:** the `MutationEngine` drives create/replace/edit/REM/MV for any `MutationAdapter` with no workspace-specific branch, so a `local://` target needs no engine change (source-neutral).
- **C3:** scratch create/replace/edit/REM/same-source-`MV` succeed under Path Session authority with zero `MutationGrants`, while the same operation on a workspace reference without a grant returns `permission_denied`.
- **C4:** `CompiledSources` routes read/resolve/load/commit/search/glob for a `local://` target to the Local adapter and never to the filesystem adapter, and routes a workspace target to the filesystem adapter and never to Local.
- **C5:** `artifact://` and catalog references remain immutable (`permission_denied`) after scratch mutation lands; only `local://` is writable.
- **C6:** scratch bytes are charged to `used_bytes` and objects to the 1,000-object / 256 MiB / 64 MiB ceilings shared with `artifact://`; exact-limit content is accepted, one byte / one object over returns `limit_exceeded` before commit, and no existing object is evicted.
- **C7:** the seen-region snapshot store accepts canonical `local://` references (and still canonical workspace), so scratch edits reserve/resolve seen regions; every non-canonical or non-{workspace,local} reference is still rejected.
- **C8:** cross-source `MV` (`local://`↔workspace, either direction) returns `unsupported_mutation` with both sides unchanged.
- **C9:** two concurrent Path Sessions never observe each other's scratch through read/search/glob/mutation, and disconnect invalidates `local://` references immediately while backing state is TTL-cleaned.
- **C10:** concurrent mutations of one `local://` Resource serialize per canonical Resource; exactly one stale contender may commit and every loser returns `version_conflict` with no torn content.
- **C11:** `local://` self-lists in the `rfs://` source catalog with a grammar line and a valid example, the previously-absent catalog assertions are updated, and scratch reads report `mutable: true`.
- **C13:** bare `local://` parses as the scratch family root and reads as a bounded, deterministic, name-sorted listing of the session's scratch Resources (each entry a resolvable canonical reference), reports `mutable: false`, emits an explicit non-empty status line when the session holds no scratch, and returns `permission_denied` for every mutation.
- **C14:** `local://<name>:<selector>` resolves literal-first — an existing scratch Resource whose literal name contains the trailing text wins, and the trailing text is interpreted as a projection selector only when no such literal Resource exists.
- **C12:** no valid `local://` name resolves to a backing path outside the Path Session store — a decoded separator is rejected at parse, so traversal and symlink attempts against the backing store cannot escape.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Filename-like `local://` grammar; no existing family changes | valid/invalid names; every existing scheme | Parse each string and compare the address variant/error category to a literal table; any invalid name accepted, valid name rejected, or changed existing-family result falsifies | Hand-authored accept/reject table (`.rfs-60g1` or inline), independent of the parser branches | `reference.rs`: skip the decoded-separator check in local-name validation; `path_reference_contract::local_name_grammar` turns red on `local://a%2Fb` accepted | `crates/resourcefs-core/tests/path_reference_contract.rs::local_name_grammar` (created this change) + existing golden corpus | <1 min | PENDING — checkpointed-build, slice in plan.md |
| C2 | `MutationEngine` is source-neutral | non-filesystem `MutationAdapter` (fake) driving all ops | Drive write/replace/edit/REM/MV through a fake adapter; any workspace-specific branch in the engine falsifies | `FakeAdapter`/`StatefulAdapter` in `mutation_engine_contract.rs` — a different (in-memory) mechanism than the production filesystem adapter | `mutation.rs`: add `matches!(reference.address(), ResourceAddress::Workspace(_))` guard in `MutationEngine::write`; the fake-adapter rows turn red | `crates/resourcefs-core/tests/mutation_engine_contract.rs` (existing) | <1 min | **PASS** |
| C3 | Scratch authorized by session, not grants | create/replace/edit/REM/MV with zero grants; workspace op without grant | Run every scratch op in a zero-grant session and assert success; run a workspace mutation without a grant and assert `permission_denied` | Literal grant×operation truth table, independent of the adapter | `local.rs`: have `LocalSource::resolve` consult a grant flag and deny; `local_mutation_contract::scratch_needs_no_grant` turns red | `crates/resourcefs-sources/tests/local_mutation_contract.rs::scratch_needs_no_grant` (created) | <2 min | PENDING — checkpointed-build, slice in plan.md |
| C4 | Dispatch by source, both directions | `local://` and workspace mutation/read/discovery targets | Gate the filesystem adapter's `load`, run a `local://` edit, assert the gate is untouched and the local store changed; run a workspace edit and assert Local is untouched | Per-adapter call counters, independent of the dispatch match | `compiled.rs`: leave `load` hardcoded to `self.filesystem`; `local_dispatch_contract::local_never_touches_filesystem` turns red (local edit loads workspace `Missing`) | `crates/resourcefs-sources/tests/local_dispatch_contract.rs::{local_never_touches_filesystem,workspace_never_touches_local}` (created) | <2 min | PENDING — checkpointed-build, slice in plan.md |
| C5 | `artifact://`/catalog stay immutable | write/edit against `artifact://`, `rfs://`, `rfs://workspace` | Attempt each mutation and assert `permission_denied` with no state change | Literal reference→category table, independent of routing | `compiled.rs`: add a Local-style arm that resolves `artifact://` for mutation; `local_dispatch_contract::artifact_and_catalog_immutable` turns red | `crates/resourcefs-sources/tests/local_dispatch_contract.rs::artifact_and_catalog_immutable` (created) | <1 min | PENDING — checkpointed-build, slice in plan.md |
| C6 | Shared quota, atomic, no eviction | 64 MiB object; 256 MiB / 1000-object session at ceiling; +1 | Fill the session to each ceiling, attempt one more scratch write, assert `limit_exceeded` before commit and pre/post inventory byte-identical (scratch + artifacts) | Independent byte/object accounting in the test plus pre/post inventory hash | `session.rs`: don't add scratch bytes to `used_bytes`; `local_quota_contract::scratch_shares_and_never_evicts` turns red (over-ceiling write succeeds) | `crates/resourcefs-sources/tests/local_quota_contract.rs::scratch_shares_and_never_evicts` (created) | <2 min | PENDING — checkpointed-build, slice in plan.md |
| C7 | Snapshot key accepts local, still rejects foreign | canonical local, canonical workspace, non-canonical, artifact | Reserve/resolve a seen region for a `local://` edit and assert success; assert a non-canonical/foreign reference still errors | Literal reference→accept/reject table, independent of `SnapshotResourceKey::parse` | `session.rs`: leave `SnapshotResourceKey::parse` workspace-only; `local_edit_contract::scratch_edit_reserves_seen` turns red (edit cannot reserve) | `crates/resourcefs-sources/tests/local_edit_contract.rs::scratch_edit_reserves_seen` + a core `path_session_contract` row | <2 min | PENDING — checkpointed-build, slice in plan.md |
| C8 | Cross-source MV rejected | `MV local://<dst>` from workspace; `MV <workspace>` from `local://` | Submit each cross-source move and assert `unsupported_mutation`, both entries unchanged | Pre/post two-entry inventory, independent of the move path | `local.rs`/`compiled.rs`: allow a cross-source move key pair; `local_mutation_contract::cross_source_mv_rejected` turns red | `crates/resourcefs-sources/tests/local_mutation_contract.rs::cross_source_mv_rejected` (created) | <1 min | PENDING — checkpointed-build, slice in plan.md |
| C9 | Per-session isolation + disconnect | two sessions with colliding scratch names; post-disconnect read | Create identically named scratch in sessions A and B; assert A's read/search/glob/mutation never sees B's; disconnect A and assert its references fail | Session-scoped expected contents, independent of the store keying | `session.rs`: key the scratch map globally instead of per `SessionState`; `local_isolation_contract::sessions_never_cross` turns red | `crates/resourcefs-sources/tests/local_isolation_contract.rs::{sessions_never_cross,disconnect_invalidates}` (created) | <2 min | PENDING — checkpointed-build, slice in plan.md |
| C10 | Per-Resource serialization | two concurrent writers of one `local://` name | Gated concurrent replace of one name; assert exactly one commits and the loser is `version_conflict`, no torn content | Atomic commit counter + content hash, independent of the lock registry | `mutation.rs`: give all local targets a constant lock key; `local_concurrency_contract::same_name_serializes` turns red (torn/second commit) | `crates/resourcefs-sources/tests/local_concurrency_contract.rs::same_name_serializes` (created) | <2 min | PENDING — checkpointed-build, slice in plan.md |
| C11 | Catalog lists local; scratch `mutable:true` | `rfs://` catalog read; `local://` read | Read `rfs://` and assert a `local://` line with grammar + example; read a scratch Resource and assert `mutable: true` | Literal expected catalog line + a literal read-output key set, independent of the renderer | `catalog.rs`: omit the local entry from `catalog_entries`; `catalog` test row for local turns red; `compiled.rs` `source_lines.len()==3` turns red | `crates/resourcefs-sources/src/catalog.rs` tests + `crates/resourcefs-sources/src/compiled.rs::tests` (updated) + stdio read shape | <1 min | PENDING — checkpointed-build, slice in plan.md |
| C13 | Bare `local://` is the scratch family root | zero / one / many scratch Resources; mutation of bare `local://` | Read bare `local://` at each population and compare the rendered listing to an independently built sorted name list; assert every entry re-parses and resolves; assert `mutable:false`; attempt `rfs_write`/`rfs_edit` on it. Any missing/extra/misordered entry, an empty body at zero scratch, `mutable:true`, or a successful mutation falsifies | A `BTreeSet<String>` of names built by the test from what it created, independent of the adapter's listing code | `local.rs`: render the listing from the unsorted map iteration order; `local_root_contract::root_lists_all_scratch_sorted` turns red on the many-Resource row | `crates/resourcefs-sources/tests/local_root_contract.rs::{root_lists_all_scratch_sorted, root_is_immutable}` (created) | <1 min | PENDING — checkpointed-build, slice in plan.md |
| C14 | Scratch selectors resolve literal-first | a scratch Resource literally named `plan.md:1-5`; a Resource `plan.md` plus the same trailing text as a selector | Create both shapes and read `local://plan.md:1-5`; with the literal Resource present it must return that Resource's content; with only `plan.md` present it must return lines 1-5 of it. Either result swapped falsifies | Test-held expected contents keyed to which Resource was created, independent of the resolution order in the adapter | `local.rs`/`reference.rs`: try the selector interpretation before the literal name; `local_selector_contract::literal_scratch_name_wins` turns red (the literal-named Resource is shadowed) | `crates/resourcefs-sources/tests/local_selector_contract.rs::literal_scratch_name_wins` (created) | <1 min | PENDING — checkpointed-build, slice in plan.md |
| C12 | Name containment | traversal/symlink names against the backing store | Attempt `local://` names that would target outside the store; assert every one is `invalid_reference` at parse and the backing store shows no foreign path | The C1 accept/reject table plus a backing-directory inventory, independent of the store | `reference.rs`: accept a decoded `..`/separator; `local_containment_contract::names_cannot_escape` turns red | `crates/resourcefs-sources/tests/local_containment_contract.rs::names_cannot_escape` (created) | <2 min/platform | PENDING — checkpointed-build, slice in plan.md |

## Non-goals and future work

### Permanent non-goals (rationale recorded; no tracker issue)
- Hierarchical scratch paths / directories — flat names only (spec Decision); containment is structural.
- Structural Summary of scratch text — scratch is not registry-language files under a Workspace Root (spec Decision).
- Binary scratch Resources — `unsupported_mutation`, consistent with rfs-73dz.
- Persistence across disconnect or cross-session sharing — scratch is Path-Session-scoped by contract (rfs-34pz).
- Promoting scratch to a real filesystem path (OMP's `local://` FS-promotion) — out of the consolidation product per ADR-0005.

### Intended future work (verified tracker IDs)
- **rfs-ww6w** — MCP Resources/templates mirror consumes `local://` once it exists; no work added here.
- Converter/type behavior keyed on a scratch name extension — no covering issue exists and it is deliberately deferred, not a 1.0 goal; not filed (permanent non-goal for this change; revisit belongs with the documents source **rfs-9o5v** if ever wanted).

## Falsifier run log

- 2026-08-23 — C2 cheapest falsifier: `cargo test -p resourcefs-core --all-features --test mutation_engine_contract` — **PASS** (`14 passed; 0 failed`). The suite drives create/replace/edit/REM/MV through `FakeAdapter` and `StatefulAdapter` (in-memory, non-filesystem `MutationAdapter` impls at `mutation_engine_contract.rs:64,100`), proving the `MutationEngine` carries no workspace-specific branch. Scratch is therefore a new adapter plus dispatch/snapshot-key widening (C4, C7), not an engine change — the design's central premise holds.

## Approval

Requester approval (verbatim): "I approve"
Date: 2026-08-23
Approved risk acceptances: None (no `N/A — approved risk` rows in the Falsification table).

Revision 1 (2026-08-23) — follows `spec.md` Revision 1, which pinned two addressing decisions Slice 1 surfaced. C1 extended (bare `local://` is the family root, not `invalid_reference`); C13 (family-root listing) and C14 (literal-wins selectors) added with full falsification rows. No risk acceptances added; no existing claim weakened.

Requester approval (verbatim): "lets go with your recommendations and also reopen the spec queue and add the new decisions"
Date: 2026-08-23
