# Design: bounded reads and Path Session artifacts

## Route and inputs

- Route: **Empirical**, from `.rfs-34pz/route.md`. Public request/result schemas and all three crate seams change; storage, concurrency, and cleanup carry production-scale risk.
- Behavior source: requester-approved `.rfs-34pz/spec.md`. It defines exact selector bytes/edges, strict lower-only content limits, full-projection spill, separate recovery/continuation references, byte-page cursors, deduplication, quotas, identity/isolation, 24-hour cleanup, and MCP rendering.
- Empirical source: `.rfs-34pz/evidence.md`, `probe_rmcp_disconnect.py`, `probe_session_lease.py`, and `oracle.py`.
  - P1 PASS/learning: `rmcp` 3.1.3 `RunningService::waiting()` can return after its five-second drain timeout while a detached handler still executes. Session invalidation and every resolve/commit path must therefore recheck liveness; handler absence cannot be inferred from `waiting()`.
  - P2 PASS/validated: exclusive lease contention and owner-death release hold on native Linux and Windows/Wine; the selected Unix `flock` call cross-compiles for macOS and published platform contracts agree.
- Behavior set: the six given/when/then sections in `spec.md` under **Behavior**, including raw and empty/error subcases added during interrogation. No behavior is sourced from an absent artifact.
- Delivery increments: N/A — `budgeted-plan` owns increments after this design is approved.

## Input shapes

| ID | Production-reachable shape | Status |
|---|---|---|
| I1 | Workspace references: relative, native absolute, local `file://`, canonical `rfs://workspace`, contained/escaping, literal selector-shaped file present/absent | Covered by C2, C6, C15 |
| I2 | Artifact references: valid own token/id, malformed token/id, unknown id, foreign token, disconnected/expired session | Covered by C2, C9, C12, C15 |
| I3 | Projection: none, `raw`, one line, open/closed/count range, multi-range, out-of-order ranges, duplicates, raw+ranges, valid artifact page, invalid page offset/source | Covered by C2, C3, C5, C12 |
| I4 | Text: empty, ASCII, Unicode scalar boundaries, spaces, LF, CRLF, terminated/unterminated final line, one overlong line, invalid UTF-8 | Covered by C3, C5, C6, C14 |
| I5 | Limits field-presence matrix: object absent; object present with each single member, each pair, all members; members at 1/exact hard ceiling | Covered by C4, C5 |
| I6 | Invalid limits: null/non-object, unknown member, null member, zero, negative, fraction, wrong type, one above each ceiling | Covered by C4, C13, C14 |
| I7 | Boundaries: content/page empty, one below/exact/one above bytes, lines, columns, and intersections where each dimension binds first | Covered by C4, C5, C16 |
| I8 | Selection size: zero, inline-sized, spill-sized through exactly 64 MiB, one byte over object quota; huge source with narrow and open-ended ranges | Covered by C3, C6, C7, C16 |
| I9 | Session state: no artifacts, one, several distinct, byte-identical retry, synthetic hash collision, exact/over session quota, concurrent equal/distinct spills | Covered by C7, C8, C9 |
| I10 | Storage outcomes: atomic success, open/write/sync/persist/read/remove failure, orphan temp, unavailable cache root | Covered by C8, C14 |
| I11 | Lifecycle: live, per-request cancellation, input EOF, handler outliving `waiting()`, clean disconnect, abnormal owner death, before/exact/after TTL, concurrent live lease | Covered by C10, C11 |
| I12 | MCP results: unbounded, bounded first/middle/final page, empty success, operational tool error, malformed-input protocol error | Covered by C13, C14 |
| I13 | Workspace authority races: active, refreshing/disabled, root generation changes during stream, session invalidates during source read/store commit | Covered by C6, C10 |
| I14 | Mutation requests against artifacts | N/A — permanent non-goal: this issue exposes an immutable read/recovery family and no artifact mutation interface |
| I15 | Remote/binary/source-specific transformed projections | N/A — intended work is carried by the source-family tickets under `rfs-jk0d`; current scope is exact UTF-8 workspace/artifact text |

## Removed invariants

The move is subtractive underneath its new behavior: it removes the filesystem adapter's precondition that a readable Resource must already fit the 48 KiB/3,000-line/512-column result limits.

That precondition silently guaranteed: no read retained more than 48 KiB; no read created session state; no concurrent reads contended for quota or identity; selector candidates never reached execution; and a handler completing after disconnect could not publish an artifact. Removing it makes large streaming, duplicate/out-of-order selection, quota races, post-root-refresh delivery, and post-disconnect commit possible. C3, C6, C7, C8, C10, and C16 replace those accidental guarantees with explicit invariants. Existing capability containment and post-read root-generation validation remain required and are not weakened.

## Placement

### Typed Path References, selectors, limits, source/read result types

- **Owner:** `resourcefs-core`. These are Behavior Contract vocabulary shared by adapters and MCP; placing them in either outer crate would reverse dependency direction.
- **New seam:** deepen the existing `PathReference`/`SourceAdapter` seam. `PathReference` stores a typed `ResourceAddress` (`Workspace` or `Artifact`) plus typed `ProjectionSelector` (`Raw`, `Lines`, artifact-only `Page`) while preserving accepted spelling. Split the current complete-source/display-page conflation into `SourceResource` and `ReadResource`.
- **Forbidden:** no `rmcp`, `cap_std`, cache path, or filesystem handle in public core types; no string reparse of selectors in sources or MCP.

### Common selector and page engine

- **Owner:** `resourcefs-core` in `selector.rs` and `read.rs`. Exact line algebra, object-ceiling selection, page boundaries, Recovery Reference construction, and continuation spelling are identical for workspace and artifact UTF-8 text.
- **Chosen interface shape A — core engine:**

```rust
pub struct ReadRequest {
    pub reference: PathReference,
    pub limits: TextLimits,
}

pub struct ReadEngine {
    sources: Arc<dyn SourceAdapter>,
    session: PathSession,
}

impl ReadEngine {
    pub async fn read(
        &self,
        request: ReadRequest,
        operation: &OperationGuard,
    ) -> Result<ReadResource, ResourceError>;
}

#[async_trait]
pub trait SourceAdapter: Send + Sync {
    async fn read(&self, reference: &PathReference) -> Result<SourceResource, ResourceError>;
}
```

Adapters retain source-specific resolution and stream I/O through one public core `select_utf8` implementation, returning the complete selected projection capped at the 64 MiB artifact-object ceiling plus authoritative metadata. `ReadEngine` alone pages that projection, asks `PathSession` to retain it when necessary, and constructs result/recovery/continuation fields.

- **Competing shape B — adapter-carried plan:** change `SourceAdapter::read` to accept `ReadPlan`, `SessionContext`, limits, and spill capability, then make each adapter select, bound, spill, and emit continuations. This makes filesystem streaming natural but duplicates identical line algebra, page legality, continuation spelling, and liveness/quota calls across the two in-scope text adapters; the nominal dispatcher becomes a shallow core engine. Rejected for lower locality and a larger interface.
- **Decision:** shape A. The adapter interface stays small; common behavior remains one deep module. Streaming stays adapter-local through `select_utf8`, so the engine does not learn filesystem handles.
- **Forbidden:** adapters may not construct `ReadResource`, clamp output limits, generate continuation selectors, or write spill files directly; MCP may not slice content.

### Path Session semantics and storage

- **Owner:** `resourcefs-core::session` owns active/cancelled state, opaque token and monotonic object IDs, byte-identical deduplication, quota accounting, artifact records, and linearizable admission. `resourcefs-sources::session_storage` owns platform cache paths, atomic files, live leases, heartbeat/tombstone timestamps, and cleanup I/O.
- **New seam:** a dependency-inverted storage interface because core cannot depend on the sources crate:

```rust
#[async_trait]
pub trait SessionStorage: Send + Sync {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError>;
    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError>;
    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError>;
    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError>;
    async fn mark_disconnected(&self) -> Result<(), ResourceError>;
}
```

`PathSession` holds one async admission mutex across dedupe comparison, quota check, atomic write, final session/operation liveness check, and record publication. Record publication is the only event that charges quota or makes an ID resolvable. A failed post-write liveness check removes the unpublished file and returns no reference.
- **Competing shape:** put quota, token, dedupe, and liveness inside `ArtifactSource`. Rejected: the next Session Scratch adapter (`rfs-60g1`) would duplicate or reach through those semantics, and the core ownership declared in `DESIGN.md` would become fictitious.
- **Forbidden:** storage cannot decide quota, token authority, dedupe identity, or caller-visible errors; core cannot construct backing paths or suppress cleanup failures.

### Source adapters and registry

- **Owner:** `resourcefs-sources`. `FilesystemSource` remains responsible for literal-first resolution, capability containment, streaming/hash I/O, raw/default source projection, and post-stream root-generation validation. New `ArtifactSource` uses `PathSession` for token authority/content and implements the same `SourceAdapter`. `CompiledSources` routes typed address families to these two real adapters.
- **New seam:** no additional interface beyond `SourceAdapter`; this issue makes the existing seam real by adding its second adapter. `CompiledSources` is a composite adapter, not a second behavior layer.
- **Forbidden:** the registry may only match `ResourceAddress`; no selector math, quota logic, error-string matching, or MCP knowledge.

### Cache lease and cleanup

- **Owner:** `resourcefs-sources::session_storage`.
- **New seam:** `SessionLease` is private implementation. One fixed-format session directory contains a held exclusive lease file and object files. A bounded heartbeat keeps persisted last-live time current; clean close records disconnect time. Startup/explicit maintenance attempts the same nonblocking exclusive lock before considering age and deletion. Unix uses pinned `rustix::fs::flock`; Windows uses pinned `windows-sys::LockFileEx` with immediate exclusive locking.
- **Forbidden:** cleanup never follows symlinks, never accepts directory names outside strict token grammar, never deletes a locked directory, and never scans Workspace Roots. No cleanup process outlives the stdio server.

### MCP schema, rendering, and disconnect observation

- **Owner:** `resourcefs-mcp`.
- **New seam:** private `SessionTransport<T>` decorates the rmcp transport and synchronously calls idempotent `PathSession::invalidate()` when input reaches EOF/errors or the transport drops. After `running.waiting()`, `serve` calls async `mark_disconnected`; in-flight handler clones keep the backing lease alive but fail operation/session checks before delivery or artifact publication. `call_tool` maps strict limits-schema failures to MCP invalid params before constructing `ReadRequest`.
- **Forbidden:** do not rely on `RunningService::waiting()` to join handlers (P1); do not return malformed input as a tool-result envelope; do not render backing cache paths; do not maintain a second quota/session map in the server.

## Claims

- **C1:** Core's normal dependency graph remains inward-only and contains neither sources nor MCP crates.
- **C2:** One typed parser accepts exactly the approved workspace/artifact selector grammar while retaining filesystem literal-before-selector precedence.
- **C3:** Line/raw selection preserves exact UTF-8 bytes, LF/CRLF terminators, range order, and duplicates, and fails an invalid multi-range atomically.
- **C4:** `TextLimits` accepts only positive values at or below hard ceilings; MCP rejects every other limits shape before source I/O.
- **C5:** Paging returns the largest exact content slice under every effective limit and reconstructs a bounded projection byte-for-byte through one artifact root plus line/byte continuations.
- **C6:** Filesystem reads stream and hash authoritative content, cap only the selected projection at 64 MiB, and reject delivery after root-generation change.
- **C7:** Path Session admission is linearizable: byte-identical spills reuse one record, distinct content charges once, and exact object/session ceilings cannot be oversubscribed or evict live records.
- **C8:** Atomic backing failures publish no reference, charge no quota, leave existing artifacts unchanged, and expose a typed `source_unavailable` error.
- **C9:** Opaque token-plus-counter identities cannot alias across sessions; malformed references are `invalid_reference` and valid foreign/unknown/inactive references are indistinguishable `not_found`.
- **C10:** Request cancellation or connection invalidation observed during source/storage work prevents result delivery and artifact publication even when an rmcp handler outlives `waiting()`.
- **C11:** Cleanup removes only unlocked state aged at least 86,400 seconds and cannot remove a live session held by this or another process.
- **C12:** Artifact Source Adapter reads immutable complete content and legal selectors directly, with no mutation interface or backing-path disclosure.
- **C13:** Every MCP success/error keeps an object-root structured payload and complete non-empty TextContent; bounded pages expose separate root recovery and next continuation references.
- **C14:** Empty, invalid UTF-8, invalid selector, quota, storage, and malformed-schema cases retain their exact approved success/error categories with no partial artifact.
- **C15:** `CompiledSources` routes only by typed address family; workspace and artifact raw/default reads share core semantics without cross-source authority.
- **C16:** No operation buffers or stores more than the 64 MiB complete selected projection plus one bounded page and fixed bookkeeping; one-byte-over selections fail with narrowing guidance.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Inward core dependencies | Placement; I1-I13 | Inspect Cargo metadata; any normal core dependency named `resourcefs-sources` or `resourcefs-mcp` falsifies. | Direct `crates/resourcefs-core/Cargo.toml` dependency list. | Add `resourcefs-sources` to core dependencies; metadata predicate returns false. | `cargo metadata --no-deps --format-version 1` jq dependency predicate at every checkpoint. | <1 s | PASS |
| C2 | Typed grammar and literal precedence | I1-I3 | Run reference golden corpus plus literal-file fixture; any accepted/rejected drift, workspace page acceptance, or selector beating an existing literal falsifies. | Independent table of grammar productions and actual fixture existence. | In `reference.rs`, treat `:page:` as source-neutral or in `filesystem.rs` split selectors before probing the literal candidate; `path_reference_contract`/`literal_paths_precede_selectors` turns red. | `path_reference_contract`; `literal_paths_precede_selectors`. | <2 s | PENDING — checkpointed-build, parser/source slice |
| C3 | Exact selector bytes | I3-I4,I8 | Compare Rust selector results for LF/CRLF/Unicode/out-of-order/duplicate/EOF cases against an independent Python `splitlines(keepends=True)` oracle; any byte mismatch or partial invalid result falsifies. | Python byte-span evaluator, separate from Rust parser/selector. | In `selector.rs`, normalize CRLF or deduplicate ranges; selector golden corpus turns red. | `selector_golden_contract`. | <2 s | PENDING — checkpointed-build, core selector slice |
| C4 | Strict lower-only limits | I5-I7 | Submit complete presence/invalid matrix; any above-ceiling acceptance, zero acceptance, or source-call counter increment on malformed input falsifies. | Hand arithmetic against 49,152/3,000/512 and serde JSON type rules. | In `server.rs`, clamp with `min` instead of rejecting above ceiling; `stdio_rejects_non_lower_limits` turns red. | `text_limits_contract`; `stdio_rejects_non_lower_limits`. | <2 s | PENDING — checkpointed-build, limits/MCP slice |
| C5 | Lossless bounded paging | I3-I4,I7-I8,I12 | Concatenate structured `content` across continuations for boundary/intersection/overlong-line fixtures; any missing/duplicate/inserted byte, nonprogress cursor, or limit excess falsifies. | `cmp`/SHA-256 against the complete selected fixture produced independently. | In `read.rs`, advance byte cursor by displayed characters rather than UTF-8 bytes; Unicode paging reconstruction turns red. | `bounded_read_reconstructs_every_fixture`. | <3 s | PENDING — checkpointed-build, read-engine slice |
| C6 | Streaming source, authoritative tag, root revalidation | I1,I4,I8,I13 | Read a >64 MiB file through a narrow range while changing root generation at the delivery gate; allocation-independent success with full-file SHA-256 is required, stale delivery must fail. | `sha256sum` for full source plus direct fixture byte slice and recorded root generation. | In `filesystem.rs`, restore `.take(MAX_TEXT_BYTES+1)` or remove final generation check; large-range/stale-delivery tests turn red. | `filesystem_streams_narrow_large_range`; `stale_stream_delivery_is_rejected`. | 5-15 s | PENDING — checkpointed-build, filesystem slice |
| C7 | Linearizable quota/dedupe | I8-I9 | Barrier-start equal/distinct spills at exact and one-over quotas; any duplicate ID for equal bytes, usage overshoot, eviction, or partial success falsifies. | Sequential model using content byte counts and byte equality, independent of lock implementation. | In `session.rs`, release admission mutex before write or charge identical content twice; concurrent quota test turns red. | `path_session_concurrent_admission_contract`. | <5 s | PENDING — checkpointed-build, Path Session slice |
| C8 | Atomic durable backing | I9-I10 | Inject each open/write/sync/persist/remove failure and inspect cache directory, quota, and readable prior records; any new reference/charge/changed prior bytes falsifies. | Directory snapshot plus SHA-256 before/after and fake-storage call log. | In `session_storage.rs`, publish final filename before full write/sync; atomic failure test turns red. | `artifact_storage_failure_is_atomic`. | <5 s | PENDING — checkpointed-build, storage slice |
| C9 | Session identity/isolation | I2,I9,I11 | Create two sessions each with object 1; copy references across them and test malformed/unknown/inactive forms; any alias or distinguishable foreign existence falsifies. | Token/id table constructed before calls, independent of lookup implementation. | In `artifact.rs`, compare object ID but omit session token; cross-session test turns red. | `artifact_session_isolation_contract`. | <2 s | PENDING — checkpointed-build, artifact slice |
| C10 | Cancellation/liveness commit fence | I11,I13 | Gate source read and storage commit, invalidate/cancel before release, then let handler outlive transport waiting; any delivered result, published record, or charged quota falsifies. | Event log order (`invalidate < release < attempted commit`) and post-run directory/quota snapshot. | In `session.rs`, remove final operation/session-active check before record insertion; disconnect race test turns red. | `disconnect_prevents_late_artifact_commit`; `request_cancel_prevents_commit`. | 6-10 s | PENDING — checkpointed-build, lifecycle slice |
| C11 | Lease-safe 24-hour cleanup | I10-I11 | Two-process test at TTL-1/exact/TTL+1 plus killed owner; deleting locked/fresh state or retaining expired unlocked state falsifies. | Published OS lock semantics from `evidence.md` plus controlled timestamp table. | In `session_storage.rs`, test age before acquiring lease or use `>` instead of `>=`; cleanup boundary test turns red. | `session_cleanup_respects_lease_and_ttl`; empirical lease probe at oracle checkpoint. | <5 s | PENDING — checkpointed-build, storage/lifecycle slice |
| C12 | Immutable Artifact Source Adapter | I2-I4,I12,I14 | Direct adapter suite reads root/range/raw/page and inspects public interface; content mismatch, mutation entry point, or cache path output falsifies. | Backing artifact bytes and approved selector oracle. | In `artifact.rs`, return page hash/path or expose write method; direct adapter contract/compile surface turns red. | `artifact_adapter_contract`. | <3 s | PENDING — checkpointed-build, artifact slice |
| C13 | MCP-compatible complete output | I5-I7,I12 | Launch compiled stdio process; parse structured/text output for success/error/empty/pages; non-object schema, empty text, mismatched content, or missing root/continuation falsifies. | Independent JSON parser and field/text reconciliation script. | In `render.rs`, omit `continuationReference` from TextContent; stdio golden turns red. | `stdio_bounded_read_contract`. | <5 s | PENDING — checkpointed-build, MCP slice |
| C14 | Exact edge/error taxonomy | I2-I6,I8-I10,I12 | Execute edge matrix and compare exact category plus absence of partial artifact/source I/O where required; any category/string-based remap or side effect falsifies. | Spec decision table converted to a static expected-category matrix. | In `server.rs`, send malformed limits through tool-error renderer; invalid-params contract turns red. | `read_error_category_contract`; `stdio_mcp_contract`. | <4 s | PENDING — checkpointed-build, integration slice |
| C15 | Address-only registry routing | I1-I3,I15 | Instrument both adapters and run every address/selector family; any double call, selector inspection in registry, or cross-source call falsifies. | Hand route table: workspace forms→filesystem, artifact→artifact. | In `lib.rs` registry, route raw selectors specially; registry routing test turns red. | `compiled_sources_routes_by_address_only`. | <2 s | PENDING — checkpointed-build, source registry slice |
| C16 | Bounded per-operation storage/memory shape | I7-I10 | Counting reader/store fixtures at exact 64 MiB and +1 verify complete selected bytes accepted once, +1 rejected, page ≤ limits, and no store call on impossible spill. | Independent byte counters and fixed ceilings. | In `selector.rs`, collect without object ceiling or in `session.rs` check quota after write; one-byte-over test turns red or records oversized store call. | `selected_projection_object_ceiling_contract`. | 2-8 s | PENDING — checkpointed-build, core/session slice |

## Non-goals and intended future work

- **Intended future work — `rfs-r9m6`:** strict Server Profile fields may lower output/storage ceilings and configure TTL. The ticket carries an rfs-34pz note explicitly covering these fields.
- **Intended future work — `rfs-vl0u`:** search/glob over workspace and artifact Resources reuses this artifact/read contract.
- **Intended future work — `rfs-m739`, `rfs-7eqb`, `rfs-n22g`, `rfs-h7iq`:** Structural Summary producers emit the line selectors defined here.
- **Intended future work — `rfs-60g1`:** mutable Session Scratch reuses Path Session storage/quota/lifecycle semantics without weakening artifact immutability.
- **Intended future work — `rfs-ww6w`:** MCP Resource mirroring projects workspace/artifact/scratch Resources after their canonical interfaces exist.
- **Permanent non-goal:** a global artifact registry or cross-session artifact resolution; server-relative Path Session authority deliberately prevents it.
- **Permanent non-goal:** a cleanup daemon that survives the stdio server; startup/explicit maintenance plus live leases satisfy retained-state cleanup without another process lifecycle.
- **Permanent non-goal:** workspace use of `:page:`; byte-page cursors are server-generated recovery selectors for immutable artifacts only.
- **Permanent non-goal:** arbitrary binary mutation or artifact mutation; this change is exact UTF-8 read/recovery storage.

## Falsifier run log

- 2026-08-19 — C1 cheapest falsifier:
  - Command: `cargo metadata --no-deps --format-version 1 | jq -e '([.packages[] | select(.name == "resourcefs-core") | .dependencies[].name] | all(. != "resourcefs-sources" and . != "resourcefs-mcp"))'`
  - Result: `true` — PASS. Core has no normal dependency on either outer crate.

## Approval

Requester approval (verbatim): Approve design
Date: 2026-08-19
Risk acceptances: None
