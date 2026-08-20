# Spec: Recover bounded reads through session artifacts

## Request (verbatim)
> claim and implement rfs-34pz

## What this is
`rfs_read` will execute the Path Reference selector grammar, constrain every successful text result to lower-only call limits and binary hard ceilings, and preserve omitted content behind an immutable Recovery Reference. Each Recovery Reference belongs to the MCP connection's Path Session, remains live until disconnect, and has retained backing state removed after its cleanup TTL.

## Roles
- **Coding agent**: invokes `rfs_read`, consumes bounded text and structured results, follows continuation selectors, and reads Recovery References.
- **ResourceFS operator**: launches the stdio server and needs fixed storage ceilings, session isolation, and bounded retained-state cleanup.
- **ResourceFS maintainer**: verifies selector algebra, quota accounting, adapter behavior, MCP rendering, and lifecycle transitions through deterministic contract suites.

## Behavior

### Selector projection
- **Given**: an authorized UTF-8 workspace Resource and a Path Reference whose selector-shaped suffix does not name an existing literal Resource.
- **When**: a coding agent calls `rfs_read` with a 1-indexed line selector (`:N`, `:N-`, inclusive `:N-M`, `:N+K`, comma ranges in request order with duplicates preserved, or those ranges after `:raw`) or follows a server-generated artifact-only `:page:<zero-based-UTF-8-byte-offset>` continuation.
- **Then**: line selectors return exact source byte spans in request order, preserving each selected line's original `\n` or `\r\n` terminator when present, preserving an unterminated final line, concatenating multi-ranges without inserted separators, and not treating a final terminator as a synthetic empty line; column counts exclude terminators; an end past EOF clips to EOF; a start past EOF fails explicitly; `:raw` changes the source projection but not range math; and each artifact page starts on a UTF-8 scalar boundary and is the largest exact slice satisfying all effective limits.

### Raw projection
- **Given**: a workspace or artifact UTF-8 Resource, whose default projection in this change is already its untransformed text.
- **When**: a coding agent uses `:raw` or `:raw:<ranges>`.
- **Then**: `:raw` returns the same exact authoritative UTF-8 bytes for these source families, and any ranges execute after raw projection; future source-specific transformed projections remain outside this change.

### Empty content and atomic selector failure
- **Given**: an empty Resource, a selector whose start is past EOF, or a multi-range containing any invalid or past-EOF start.
- **When**: a coding agent calls `rfs_read`.
- **Then**: an unselected or `:raw` empty Resource succeeds with empty structured `content`, `bounded: false`, and no recovery/continuation references while TextContent remains non-empty from metadata; any invalid line selection fails the whole call as `invalid_reference` with no partial content or artifact.

### Lower-only result limits
- **Given**: an authorized Resource and optional strict `limits: { bytes?, lines?, columns? }` with positive integer members no greater than their binary ceilings.
- **When**: a coding agent calls `rfs_read`.
- **Then**: each present member is the effective ceiling for the exact Resource `content` page; a missing member uses the binary ceiling; fixed bounded metadata is excluded from these three measurements; and zero, fractional, negative, above-ceiling, or unknown members fail as MCP invalid params before source I/O.

### Lossless spill and recovery
- **Given**: a successful read whose complete selected projection exceeds any effective text-result ceiling and whose full projection fits storage quotas.
- **When**: ResourceFS constructs the tool result.
- **Then**: the returned result is bounded; one immutable same-Path-Session artifact stores the complete pre-bound selected projection; inline content is its first page; the Recovery Reference uses stable artifact line ranges when the boundary is between lines and an artifact-only `:page:<offset>` selector when it is within a line; concatenating page `content` fields reconstructs every selected UTF-8 byte without inserted markers or newlines; and a later byte-identical spill in the same session reuses that artifact without consuming quota again.

### Atomic quota failure
- **Given**: a successful source read whose required lossless spill would exceed the 67,108,864-byte object ceiling or the 268,435,456-byte Path Session ceiling.
- **When**: ResourceFS attempts to admit the spill.
- **Then**: the `rfs_read` call returns a `limit_exceeded` tool error with narrowing guidance, returns no Recovery Reference, stores no partial artifact, and leaves every existing live Resource addressable.

### Session lifecycle
- **Given**: two simultaneous MCP connections or processes with distinct Path Sessions, retained artifact backing state, and independently held live-session leases.
- **When**: a session reads an artifact, disconnects cleanly, terminates abnormally, or startup/explicit maintenance observes a clean disconnect tombstone or abandoned lease whose persisted last-live time is at least 86,400 seconds old.
- **Then**: canonical artifact references use `artifact://<opaque-128-bit-session-token>-<positive-object-id>` with optional selectors; only the owning live session resolves them; syntactically valid unknown, foreign-session, expired, or disconnected references return indistinguishable `not_found` errors; any disconnect invalidates references immediately; cleanup removes only expired state whose live lease is not held; and no lifecycle action can remove or expose another live session's state.

### Compatible MCP result
- **Given**: any successful bounded, unbounded, or artifact-recovery read.
- **When**: `rfs_read` returns through stdio MCP.
- **Then**: structured content has an object root; exact page bytes remain in `content`; `bounded` is true exactly when bytes remain; `recoveryReference` names the immutable artifact root whenever spill-backed recovery applies; `continuationReference` names the exact next selector only when bytes remain; and the non-empty TextContent representation includes the same content, boundedness, Recovery Reference, and continuation information for a text-only MCP client.

## Success criteria

- **Binary / structural / security**: every accepted range, multi-range, raw, and paging corpus row returns byte-for-byte expected UTF-8 content and exact continuation selectors, checked by the core selector golden-corpus test and stdio MCP contract test.
- **Quantitative**: every returned Resource `content` page contains at most 49,152 UTF-8 bytes, 3,000 displayed line segments, and 512 Unicode scalar values in any displayed line segment, measured by adversarial boundary fixtures at each limit and their intersections.
- **Binary / structural / security**: supplied zero, fractional, negative, above-ceiling, or unknown limit members fail as MCP invalid params before source I/O, checked by request-schema and compiled-process MCP contract tests.
- **Quantitative**: one artifact stores at most 67,108,864 bytes and one live Path Session stores at most 268,435,456 bytes, measured by exact-boundary and one-byte-over direct Artifact Source Adapter tests.
- **Binary / structural / security**: every omitted successful UTF-8 byte is recoverable from the returned immutable same-session reference, checked by reconstructing each golden large fixture and comparing its bytes to the source fixture.
- **Binary / structural / security**: quota admission is atomic, never evicts a live reference, and never returns a Recovery Reference on failed admission, checked by direct concurrent session-store contract tests.
- **Quantitative**: clean disconnect invalidates references immediately; disconnected or abandoned state is cleanup-eligible after 86,400 seconds; and an independently held live-session lease is never removed, checked with two stores/processes, controlled time, and clean/abnormal termination fixtures.
- **Binary / structural / security**: every stdio MCP result retains an object-root structured payload and complete non-empty TextContent, checked by the compiled-process MCP golden suite.

## Out of scope

This change does NOT include Structural Summary generation (`rfs-m739`, `rfs-7eqb`, `rfs-n22g`, `rfs-h7iq`), search or glob execution (`rfs-vl0u`), mutable Session Scratch (`rfs-60g1`), MCP Resource mirroring (`rfs-ww6w`), profile-defined limits or TTL configuration, source families other than workspace and artifact Resources, mutation, or a cleanup daemon/process that outlives the stdio server; cleanup runs at store startup and through explicit maintenance.

## Related issues

- `rfs-jk0d`: accepted parent contract supplies the selector, result-limit, Recovery Reference, Path Session, quota, lifecycle, testing, and module-ownership decisions.
- `rfs-cgbq`: completed prerequisite supplies selector-shaped literal precedence, canonical workspace identity, and the parsed selector spellings consumed here.
- `rfs-vl0u`: downstream search/glob work will consume artifact Resources and the common bounded-result contract established here.
- `rfs-m739`: downstream Structural Summary work will emit exact selectors and use the common bounded-result contract established here.
- `rfs-60g1`: downstream Session Scratch work will reuse the Path Session store, quotas, isolation, and lifecycle established here.
- `rfs-ww6w`: downstream Resource Mirror work will project workspace and session Resources after their canonical read behavior exists.
- `rfs-5os7`: release acceptance will exercise bounded read, artifact recovery, complete TextContent, and the golden corpus delivered here.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| Which selector spellings are in scope? | The accepted grammar is `:N`, `:N-M`, `:N-`, `:N+K`, comma-separated ranges, `:raw`, and `:raw:<ranges>`. | `rfs-jk0d` requires OMP-compatible selector spellings; `rfs-cgbq` implemented and tested this syntax while preserving literal-path precedence. | This issue defines execution semantics for the already-parsed grammar rather than adding another selector syntax. |
| How do line selectors execute? | Use the proposed 1-indexed OMP-style contract: `:N` and `:N-` mean N through EOF; `:N-M` is inclusive; `:N+K` returns K lines; comma ranges preserve request order and duplicates; an end past EOF clips; a start past EOF fails explicitly; `:raw` does not change range math. | Requester selected “Adopt this contract” on 2026-08-19. | Selector execution and continuation generation must preserve these exact edge semantics. |
| How are line terminators and multi-range boundaries represented? | Preserve exact selected bytes: retain each selected line's original LF or CRLF when present, retain an unterminated final line, do not create a synthetic line after a final terminator, and concatenate multi-range spans without separators. Exclude terminators from column counts. | Requester selected “Preserve exact bytes” on 2026-08-19. | Selector output, artifacts, and page concatenation round-trip original UTF-8 bytes across line-ending styles. |
| How is content wider than one result page recovered? | Use server-generated artifact-only `:page:<zero-based-UTF-8-byte-offset>` selectors. Every offset is a scalar boundary; each page returns the largest exact slice within all effective limits; no marker or newline is inserted into structured `content`; concatenating page content reconstructs the artifact exactly. | Requester selected “Artifact byte-page selector” on 2026-08-19. | The column and byte ceilings remain hard while even one overlong logical line stays losslessly reachable. |
| What does a spill artifact store? | Store the complete pre-bound selected projection; inline output is page one, and recovery/continuation selectors address stable artifact ranges. Quota accounts for the complete projection. | Requester selected “Complete selected projection” on 2026-08-19. | Artifact line identity remains stable across pages, and downstream reads/search can use one immutable complete Resource. |
| How are the complete artifact and next page exposed? | Use separate fields: `recoveryReference` names the immutable artifact root, while `continuationReference` names the exact next line or byte-page selector and is absent when no bytes remain. Include both in TextContent whenever present. | Requester selected “Separate recovery and continuation” on 2026-08-19. | Callers can retain the complete Resource identity for later search while following an unambiguous paging chain. |
| What does a byte-identical repeated spill do? | Reuse the existing immutable artifact in the same Path Session after content-hash lookup and byte equality confirmation; return the same reference and do not charge quota twice. | Requester selected “Reuse byte-identical artifact” on 2026-08-19. | Read retries are idempotent with respect to artifact identity and quota; changed bytes create a new artifact. |
| What are the text-result ceilings? | 49,152 UTF-8 bytes, 3,000 displayed lines, and 512 Unicode scalar values per displayed line; the first binding dimension bounds the result. | Accepted limits in `rfs-jk0d` and `DESIGN.md`. | Per-call limits may only lower these three numbers. |
| How do per-call limits validate? | Use optional strict `limits: { bytes?, lines?, columns? }`; each value is a positive integer at or below its binary ceiling. Reject zero, fractional, negative, above-ceiling, and unknown values as MCP invalid params. | Requester selected “Reject above-ceiling values” on 2026-08-19; strict rejection exposes caller mistakes instead of silently clamping them. | Every accepted value is equal to or lower than the corresponding binary ceiling, and malformed limits cannot reach source I/O. |
| What do byte, line, and column limits measure? | Measure the exact Resource `content` page only. Fixed headers, Version Tags, and recovery metadata are separately bounded and do not consume the caller's content allowance. | Requester selected “Resource content only” on 2026-08-19; this also preserves the existing `MAX_TEXT_*` contract. | Page size is deterministic from content and limits rather than reference/header length, while TextContent remains finite. |
| What are the storage ceilings? | 67,108,864 bytes per artifact and 268,435,456 bytes per Path Session. | Accepted limits in `rfs-jk0d` and `DESIGN.md`. | Admission must reject one byte over either ceiling without partial state or eviction. |
| Who owns a Recovery Reference? | Exactly one MCP connection's Path Session; it is server-relative and immutable. | Accepted session model in `rfs-jk0d`, ADR 0003, and `CONTEXT.md`. | Cross-session reads cannot resolve a reference, and writes to artifacts are unavailable. |
| What identity and lookup errors do artifacts expose? | Canonical `artifact://<opaque-128-bit-session-token>-<positive-object-id>` references; object IDs increase within the session and selectors append normally. Malformed tokens/non-positive IDs are `invalid_reference`; syntactically valid unknown, foreign-session, expired, or disconnected references are indistinguishable `not_found`. | Reconciled on 2026-08-19 after the requester selected “Opaque session token plus counter”; per-session numeric spellings could collide and violate the earlier isolation/error decision. | A copied foreign reference cannot alias an unrelated local artifact, while references reveal no operator or filesystem identity. |
| What happens at disconnect and cleanup? | Disconnect invalidates Path Session references immediately; retained backing state is eligible for later TTL cleanup. | Accepted session model in `rfs-jk0d` and ADR 0003. | Logical validity and physical retention are separate states. |
| How long is disconnected backing state retained before cleanup eligibility? | 86,400 seconds (24 hours) from the recorded disconnect tombstone; cleanup runs when a session store starts and through an explicit maintenance operation, and never targets a live session. | Requester selected “24 hours” on 2026-08-19. | Profile configurability remains deferred, while startup and deterministic maintenance tests can prove expiry and cross-session isolation. |
| Does TTL cleanup cover abnormal termination and concurrent server processes? | Yes. Persist session age and protect every live session with a cross-process lease/lock. Abandoned state becomes cleanup-eligible after the same 86,400-second TTL, but cleanup must not remove a directory whose live lease is still held by another process. | Requester selected “Cover clean and abnormal exits” on 2026-08-19. | Crash leftovers cannot leak indefinitely, and cleanup remains isolated across concurrently running ResourceFS processes. |
| What does `:raw` mean for source families in this change? | Workspace and artifact Resources already expose exact UTF-8 text, so `:raw` returns the same bytes; `:raw:<ranges>` applies line selection after raw projection. | `rfs-jk0d` defines raw as bypassing source-specific transformation; no transformed source family is introduced here. | The accepted raw selector is executable and golden-tested without inventing a second workspace representation. |
| What happens for empty Resources and partially invalid multi-ranges? | Empty base/raw reads succeed with empty structured content, `bounded: false`, and no recovery/continuation fields; TextContent remains non-empty from metadata. Any invalid or past-EOF range makes the whole selector fail as `invalid_reference` with no partial result or artifact. | The requester adopted explicit failure for starts past EOF; atomic failure avoids representing a partial request as success. | Empty-set and partial-failure edges are deterministic. |
| What happens when required input is null or missing? | Missing/null `path`, non-object/null `limits`, and malformed member types are MCP invalid params; an absent `limits` object or absent members use binary ceilings. | Existing strict `ReadInput` deserialization maps malformed tool schemas to MCP invalid params; the requester adopted a strict limits object. | Protocol/schema failures remain distinct from tool errors and perform no source I/O. |
| How do concurrent spills affect quota and deduplication? | Admission is linearizable per Path Session: byte-identical spills converge on one artifact; distinct spills commit only when the complete object and resulting session usage fit; losers receive `limit_exceeded`; no partial object or live eviction is observable. | `rfs-jk0d` requires atomic quota and no live eviction; the requester adopted byte-identical reuse. | Concurrent calls cannot oversubscribe quota or allocate duplicate identical artifacts. |
| What bytes count toward artifact quotas, and what if backing I/O fails? | Count complete artifact UTF-8 content bytes once; exclude metadata and temporary-file overhead. A backing write/sync/commit failure returns `source_unavailable`, publishes no reference, charges no quota, and leaves existing artifacts unchanged. | The accepted ceilings describe Resource object/session content; atomic failure is required by `rfs-jk0d`. | Quota is stable across filesystems, and storage failure cannot create a false Recovery Reference. |
| How are authority and multi-tenancy edges handled? | Workspace reads retain existing Workspace Root authority. Artifact authority is exactly the live Path Session token; valid foreign/expired references return `not_found`. There is no additional network authentication in a local stdio session. | `rfs-cgbq`, ADR 0003, and the requester's reconciled artifact identity decision. | Permission denial remains source-specific, and one session cannot probe or read another's artifacts. |
| How do canonical references and Version Tags behave under selection/paging? | `requestedPath` preserves caller spelling; `canonicalReference` uses the canonical Resource identity plus the executed selector; selector spelling is preserved where already accepted. Every page carries the Version Tag of the complete authoritative pre-selection Resource, not a hash of the displayed page. | `rfs-jk0d` defines selectors as part of Path References and Version Tags as authoritative-content identities; `rfs-cgbq` preserves selector spelling. | Equivalent pages remain tied to one immutable/source snapshot and later snapshot-based work cannot mistake a page hash for Resource state. |
| Where does retained backing state live? | Under ResourceFS's per-account platform cache directory in an isolated sessions subtree; tests and direct adapter construction supply an explicit temporary root. Backing paths are never rendered in normal Resource output. | `DESIGN.md` requires account-scoped cache storage and private canonical identities. | Cleanup can discover prior crashed sessions without leaking host paths or granting workspace authority. |
| Which artifact page selectors are invalid? | `:page:<offset>` is valid only on an artifact, only for a nonzero offset strictly before EOF, and only at a UTF-8 scalar boundary. Invalid offsets are `invalid_reference`; using page syntax on another source is `unsupported_projection`. | Server-generated continuations must make progress and line selector errors already use explicit failure. | Crafted cursors cannot loop, split UTF-8, or change workspace selector meaning. |
| Are concurrent writes, retries, cache invalidation, soft deletion, time zones, DST, or replication lag otherwise relevant? | Concurrent writes are N/A because artifacts are immutable; retries are covered by byte-identical reuse; cache invalidation is disconnect plus TTL/lease cleanup; soft deletion, civil time, DST, and replication lag are N/A for monotonic-duration local session storage. | Accepted local-stdio, immutable artifact, and Path Session architecture. | Every edge-checklist dimension is either specified above or explicitly inapplicable. |

## Approval

Requester approval (verbatim): "Approve spec"
Date: 2026-08-19
