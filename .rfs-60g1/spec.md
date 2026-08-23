# Spec: Session Scratch as independently writable state

## Request (verbatim)
> start with rfs-60g1

Issue rfs-60g1 — "Make local Session Scratch a complete independently authorized Resource family: agents can create, read, search, replace, hashline-edit, delete, and move local Resources without Workspace Mutation grants while retaining quotas, Version Tags, isolation, and lifecycle guarantees."

## What this is

ResourceFS adds the writable `local://` Session Scratch Resource family: a new Local Source Adapter that serves create/read/search/glob/replace/hashline-edit/delete/same-source-move over caller-authored Resources through the five public tools. Scratch is authorized by Path Session authority — no Workspace Mutation grant — while enforcing the existing object/session quotas, content-derived Version Tags, per-session isolation, and disconnect lifecycle. It is the first non-filesystem implementation of the rfs-73dz `MutationAdapter` seam.

## Roles

- **Coding agent**: invokes the five `rfs_*` tools over one MCP connection; creates and edits `local://` scratch to hold plans, review context, and intermediate work without needing (or being granted) workspace write authority; observes structured receipts and stable operational errors.
- **ResourceFS operator**: configures Workspace Roots and their grants in the Server Profile; grants the coding agent *no* new authority for scratch — scratch is writable by session ownership alone, bounded only by quotas.

## Behavior

### Create scratch Resource
- **Given**: a valid `local://<name>` (name grammar below) that does not exist in the live Path Session, UTF-8 content ≤ 64 MiB, and session budget available.
- **When**: `rfs_write` with that reference and content and no `ifVersion`.
- **Then**: ResourceFS creates exactly one scratch text Resource under Path Session authority (no Workspace Mutation grant consulted), and returns a compact `created` receipt with a content-derived Version Tag and full-coverage `displayedRanges`/`displayedEof` recorded as seen. A read returns exactly the submitted bytes. An existing name returns `version_conflict`.

### Read scratch Resource
- **Given**: an existing `local://<name>` in the live Path Session.
- **When**: `rfs_read` of it, optionally with lower-only `limits`, the numbered projection, or a trailing projection selector (`local://<name>:<selector>`).
- **Then**: returns bounded text through the common read contract with the content-derived Version Tag, `mutable: true`, and — when the content exceeds the effective ceilings — a lossless `artifact://` recovery reference and continuation in the same session. No Structural Summary is applied; scratch is plain bounded text. A trailing selector resolves by the established literal-wins rule: an existing scratch Resource whose literal name contains the trailing text wins, and only when no such Resource exists is the trailing text interpreted as a projection selector against the preceding name.

### Read the scratch family root
- **Given**: a live Path Session holding zero or more scratch Resources.
- **When**: `rfs_read` of the bare reference `local://`.
- **Then**: returns a bounded, deterministic listing of the session's scratch Resources as canonical `local://<name>` references sorted by name, through the common read contract, with `mutable: false` (the listing itself is a synthetic read-only projection; only named scratch Resources are writable). A session holding no scratch emits an explicit non-empty status line rather than empty content. Mutating the bare `local://` reference returns `permission_denied`.

### Search scratch Resources
- **Given**: scratch Resources exist in the live Path Session.
- **When**: `rfs_search` with `path: "local://"` (all scratch — the bare family root parses as a valid reference, so it needs no special-case string handling) or `path: "local://<name>"` (one), pattern and controls as usual.
- **Then**: returns matching lines through the shared `DiscoveryAdapter` engine under the common bounded/paginated result contract; scratch with no match is omitted.

### Glob scratch names
- **Given**: scratch Resources exist.
- **When**: `rfs_glob` with a `local://<glob>` pattern; `*`, `?`, character classes, and `{a,b}` match within the single flat name segment (`**` has no cross-directory effect since names are flat).
- **Then**: returns matching names as canonical `local://<name>` references under the common bounded result contract.

### Replace scratch Resource
- **Given**: an existing `local://<name>` and its current Version Tag.
- **When**: `rfs_write` with replacement content and that tag in `ifVersion`.
- **Then**: ResourceFS compares authoritative content immediately before commit, atomically replaces only if the tag still matches, re-checks session budget, and returns a compact `replaced` receipt with the new Version Tag and full-coverage seen ranges. A stale/mismatched tag returns `version_conflict`.

### Edit scratch Resource (hashline)
- **Given**: a prior read in the same live Path Session displayed regions of an existing `local://<name>` under its Version Tag, and update is implied by session authority.
- **When**: `rfs_edit` with a `[local://<name>#<Version-Tag-or-≥12-hex-prefix>]` document of `PUT`/`CUT` hunks targeting only displayed lines/gaps.
- **Then**: applied through the shared `MutationEngine` against the original-coordinate space, atomic replace, `edited` receipt with new tag and remapped seen coverage. Seen-region, prefix-tag, and overlap rules are identical to workspace edits (rfs-73dz).

### Delete scratch Resource (REM)
- **Given**: an existing scratch Resource and its Version Tag/snapshot.
- **When**: `rfs_edit` with a sole `REM` on `[local://<name>#<tag>]`.
- **Then**: ResourceFS deletes it, returns a compact `deleted` receipt with no Version Tag, and frees its bytes from the session budget without evicting any other Resource.

### Move/rename scratch Resource (MV)
- **Given**: an existing `local://<src>`, its Version Tag, and a non-existent `local://<dest>` in the same session.
- **When**: `rfs_edit` with a sole `MV local://<dest>`.
- **Then**: ResourceFS performs an atomic no-clobber rename within scratch and returns a `moved` receipt with the unchanged content-derived Version Tag. An existing destination returns `version_conflict`; a destination that is not a `local://` reference (workspace or any other source) returns `unsupported_mutation` (cross-source), leaving both sides unchanged.

### Authorize scratch without Workspace Mutation grant
- **Given**: a Path Session with no Workspace Mutation grants — including a scratch-only launch with zero Workspace Roots.
- **When**: any scratch create/replace/edit/REM/MV.
- **Then**: it succeeds under Path Session authority, while the same operation against a Workspace reference still returns `permission_denied` for lack of a grant. Scratch authority is independent of workspace authority.

### Enforce quota atomically without eviction
- **Given**: a scratch write whose object would exceed 64 MiB, or whose addition would exceed the 256 MiB or 1,000-object Path Session ceiling (shared with `artifact://`).
- **When**: the create/replace/edit is attempted.
- **Then**: it fails atomically with `limit_exceeded` and narrowing guidance, commits no bytes, evicts no live scratch or artifact Resource, and leaves Session Scratch authority unchanged.

### Isolate per session and invalidate at disconnect
- **Given**: two simultaneous Path Sessions that each hold scratch.
- **When**: session A reads, searches, or globs `local://`.
- **Then**: A observes only its own scratch; identical names across sessions never interfere. On disconnect, a session's `local://` references invalidate immediately and its backing store is TTL-cleaned and garbage-collected; a post-disconnect reference fails rather than resolving.

### Reject immutable, non-text, and out-of-grammar targets
- **Given**: a mutation aimed at `artifact://` or a catalog, a `local://` name that decodes to contain a separator or is `.`/`..`/empty/over-255-bytes, or non-UTF-8/binary content.
- **When**: `rfs_write` or `rfs_edit`.
- **Then**: returns the stable category — `permission_denied` for immutable `artifact://`/catalog, `invalid_reference` for an out-of-grammar name, `unsupported_mutation` for binary — with no state change.

## Success criteria

- **Binary / structural / security**: create, read, search, glob, replace, hashline-edit, delete, and same-source move all succeed on `local://` through the five public tools with **zero** Workspace Mutation grants configured, checked by direct Local Source Adapter contract tests exercising each operation.
- **Quantitative**: scratch enforces the 64 MiB object and 256 MiB / 1,000-object session ceilings (shared with `artifact://`); exact-limit content is accepted and one byte / one object over returns `limit_exceeded` before commit, measured by boundary tests at each ceiling.
- **Binary / structural / security**: every `local://` reference that decodes to contain `/` or `\`, or is `.`/`..`/empty/over-255-UTF-8-bytes, returns `invalid_reference`; no scratch name resolves to a backing path outside the Path Session store, checked by name-grammar golden tests plus containment tests including symlink/traversal attempts against the backing store.
- **Binary / structural / security**: two concurrent Path Sessions never observe each other's scratch through read, search, glob, or mutation, and disconnect invalidates references immediately while backing state is TTL-cleaned, checked by dual-session isolation and lifecycle tests.
- **Binary / structural / security**: a quota-exhausting scratch write fails atomically and leaves the pre-write scratch and artifact inventory byte-for-byte unchanged, checked by quota-exhaustion fixtures comparing pre/post inventory.
- **Binary / structural / security**: concurrent mutations of one `local://` Resource serialize per canonical Resource; one stale contender returns `version_conflict` with no torn content, checked by a gated race test.
- **Binary / structural / security**: cross-source `MV` (`local://`↔workspace) returns `unsupported_mutation` with both sides unchanged, checked by MV contract tests.
- **Binary / structural / security**: `local://` self-lists in the `rfs://` source catalog with a grammar line and a valid example once mounted, and the existing "`local://` absent" catalog assertion (`catalog.rs` test) is updated accordingly, checked by the catalog contract test.
- **Binary / structural / security**: `rfs_read` and mutation receipts for scratch use the common object-rooted output contract with non-empty equivalent text and `mutable: true`, checked by stdio MCP contract tests.
- **Binary / structural / security**: `rfs_read` of bare `local://` returns a bounded listing whose entries are exactly the session's scratch names, sorted, each a valid `local://<name>` reference that resolves; a scratch-free session returns a non-empty explicit status line; the listing reports `mutable: false` and every mutation of bare `local://` returns `permission_denied`; checked by direct Local adapter contract tests at zero, one, and many scratch Resources.
- **Binary / structural / security**: `local://<name>:<selector>` resolves literal-first — a scratch Resource whose literal name contains the trailing text is returned in preference to any selector interpretation, and the selector applies only when no such literal Resource exists; checked by a direct test pairing a Resource literally named with a colon against a same-prefixed Resource plus a range selector.
- **Quantitative**: the Local Source Adapter has direct contract tests for every operation and lifecycle transition (create/read/search/glob/replace/edit/delete/move/quota/isolation/disconnect), measured by test presence and pass on Linux, macOS, and Windows.

## Out of scope

This change does NOT include: hierarchical scratch paths or directories (flat names only); Structural Summary of scratch text; binary scratch Resources (`unsupported_mutation`, consistent with rfs-73dz); persistence of scratch across disconnect or sharing between Path Sessions; promoting scratch to a real filesystem path (OMP's `local://` FS-promotion); MCP Resources/templates mirroring of `local://` (rfs-ww6w); converter/type behavior keyed on the name extension; and any change to `artifact://` immutability.

## Related issues

- **rfs-34pz** (closed): Path Session, `artifact://` immutable recovery, `DiskSessionStorage`, the 64 MiB/256 MiB/1,000-object quotas, disconnect invalidation, and TTL cleanup — scratch shares this storage and quota accounting.
- **rfs-73dz** (closed): source-neutral `MutationEngine`/`MutationAdapter{resolve,load,commit}`, hashline parser, seen-region snapshots, Version Tags, per-canonical-Resource serialization, receipts, and prefix-tag rules — reused wholesale; scratch is the first non-filesystem adapter.
- **rfs-vl0u** (closed): `DiscoveryAdapter` search/glob engine — scratch search/glob route through it.
- **rfs-ewh2** (closed): `SourceCatalogMetadata`; a catalog test currently asserts `local://` is absent and must be updated when scratch mounts.
- **rfs-ww6w** (open, dependent): MCP Resources/templates mirror consumes `local://` once it exists.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| What is the `local://` namespace shape? | Flat names — `local://<name>`, no hierarchy. | Requester selected "Flat names"; simplest, lowest-risk generalization test, and containment becomes structural. | Glob matches flat names; MV is rename; a name cannot escape the session; no dependency on directory listing (rfs-hwlm). |
| What is the scratch name grammar? | Filename-like segment: printable UTF-8, no `/` or `\`, not `.`/`..`/empty, ≤255 UTF-8 bytes; percent-encoding decodes but a decoded separator is rejected. | Requester selected "Filename-like segment"; scratch holds agent-authored documents that benefit from readable names, spaces, unicode, and extensions. | Names like `plan.md`, `review notes.md`, `设计.md` are valid; `a%2Fb`, `..`, empty, and over-length are `invalid_reference`. |
| Does scratch mutation require a Workspace Mutation grant? | No — Path Session authority alone authorizes it, subject to quotas. | DESIGN.md + issue: "writable without Workspace Root mutation permission". | Scratch succeeds with zero workspace grants; workspace mutation still requires its grant. |
| Which mutation semantics does scratch use? | The rfs-73dz contract verbatim: create-if-missing / replace-if-current (`ifVersion`), hashline `PUT`/`CUT` against seen regions, sole `REM`, same-source `MV`, content-derived Version Tags, ≥12-hex prefix tags, per-canonical-Resource serialization. | Prior art rfs-73dz; the mutation engine is source-neutral by design. | No new mutation grammar; scratch is the first non-filesystem `MutationAdapter`. |
| How do search and glob target scratch? | Reuse the rfs-vl0u `DiscoveryAdapter`: `local://` searches all scratch, `local://<name>` one; `local://<glob>` globs flat names. | Prior art rfs-vl0u; consistency with workspace discovery. | Same bounded/paginated result contract. |
| Where does scratch live and how is it budgeted? | In `DiskSessionStorage` alongside `artifact://`, sharing the 64 MiB object / 256 MiB / 1,000-object Path Session ceilings. | Prior art rfs-34pz; DESIGN.md shared session budget. | A scratch write competes with artifacts for the same session budget; quota failure evicts neither. |
| Is `artifact://` still immutable when scratch is writable? | Yes — `artifact://` and catalogs remain immutable (`permission_denied`); only `local://` is the writable session family. | rfs-73dz / rfs-ewh2 immutability decisions. | Mutation routing distinguishes writable `local://` from immutable `artifact://`. |
| Is bare `local://` a valid reference? | Yes — it is the scratch family root and reads as a bounded, sorted listing of the session's scratch names (`mutable: false`; mutation returns `permission_denied`). | Requester adopted the recommendation on 2026-08-23 (queue reopened after Slice 1). ADR-0005 pins that the namespace is self-describing from its root — `rfs://` lists sources and `rfs://workspace` lists roots, so a scheme root that errors on read is a hole in that principle. It also removes work: a valid bare reference makes `SearchTarget::resource()` accept `path: "local://"` with no special-case string handling. | The grammar accepts bare `local://`; the Local adapter serves a catalog-style listing (reusing the rfs-ewh2 root-listing machinery, not rfs-hwlm directory listing); search needs no accommodation. |
| Does scratch support projection selectors? | Yes — `local://<name>:<selector>` resolves by the established literal-wins rule: an existing literal name wins, and the selector is interpreted only when no such Resource exists. | Requester adopted the recommendation on 2026-08-23. One addressing grammar across families is the product thesis; `:N-M` meaning "lines" in the workspace but "part of a name" in scratch is a silent inconsistency, and `not_found` for a reasonable range request is a confusing failure. The machinery (`PathReference::selector_candidate`/`selector_error` plus the literal-first read fallback) already exists and is golden-tested. | Scratch reads accept selectors; a scratch name legitimately containing a colon still wins because literal is tried first. |
| Does the numbered read projection / Structural Summary apply? | Numbered projection: yes (source-neutral read option). Structural Summary: no. | Numbered read is a rendering option; scratch is arbitrary text, not registry-language files under a Workspace Root. | Scratch reads are plain bounded text; edits can request numbered coordinates. |
| Empty set | Zero scratch: search/glob return an empty bounded page with the common status line; `rfs_read` of a nonexistent `local://` returns `not_found`. | Discovery + read contracts. | No special-case; reuse existing empty-page rendering. |
| Max scale | 1,000 objects and 256 MiB per session; 64 MiB per object. | Inherited rfs-34pz ceilings. | Enforced atomically; boundary-tested. |
| Null / missing field | Empty UTF-8 content is a valid zero-line scratch Resource; a missing/empty name is `invalid_reference`. | rfs-73dz empty-content rule; name grammar. | Empty create/replace succeeds within budget. |
| Concurrent writes | Per-canonical-Resource serialization from the mutation engine; one stale contender → `version_conflict`. | rfs-73dz. | Lock identity is the canonical `local://` Resource. |
| Permission denied / unauthenticated | Scratch never needs a grant; mutation of `artifact://`/catalog → `permission_denied`; local stdio has no principal. | rfs-73dz / ADR-0003. | Authority is session ownership + quotas, not a grant. |
| Partial failure | Single-Resource atomic commit; quota or validation failure leaves state unchanged. | rfs-73dz atomic-visibility contract. | No partial scratch write. |
| Retries / idempotency | No `operationId` (local, known outcome); repeated create → `version_conflict`; identical replace with the current tag may succeed without changing the tag. | rfs-73dz filesystem retry semantics. | Callers reconcile by reading current state. |
| Soft-deleted records | N/A — scratch is present or absent; `REM` removes. | No deletion lifecycle in session storage. | No tombstones. |
| Multi-tenancy boundaries | The Path Session is the tenancy boundary; cross-session scratch is never observable. | rfs-34pz session isolation. | Names may collide across sessions with no interference. |
| Time-zone / DST | N/A — Version Tags are content-derived; TTL cleanup is the existing rfs-34pz mechanism and not part of the observable scratch contract. | Content-derived state. | No clock dependency in behavior. |
| Replication lag | N/A — local session storage only. | Explicit scope. | No eventual consistency. |
| Cache invalidation | Disconnect invalidates references immediately; commit validation consults authoritative storage, no content cache. | rfs-34pz / rfs-73dz. | Post-disconnect references fail. |

## Approval

Requester approval (verbatim): "i agree with these decisions"
Date: 2026-08-23

Revision 1 — queue reopened after Slice 1 surfaced two unpinned addressing decisions (bare `local://`; scratch selectors). Both added to the Decisions table above with new Behavior entries and success criteria.

Requester approval (verbatim): "lets go with your recommendations and also reopen the spec queue and add the new decisions"
Date: 2026-08-23
