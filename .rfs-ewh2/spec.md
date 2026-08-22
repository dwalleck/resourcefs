# Spec: Discover the mounted ResourceFS namespace

## Request (verbatim)
> Claim and implement rfs-ewh2

## What this is
`rfs_read` will make the mounted ResourceFS address space self-describing from the canonical catalog references `rfs://` and `rfs://workspace`. The source catalog is generated from compiled Source Adapter self-descriptions; the workspace catalog is generated from the calling Path Session's current declared Workspace Roots.

## Roles
- **Agent caller**: invokes the visible `rfs_*` tools through an MCP harness and needs to discover valid Path Reference families without external configuration instructions.
- **ResourceFS operator**: configures compiled sources and Workspace Roots and needs the catalog to report only mounted schemes, including a safe degraded marker when a mounted optional source is unavailable.

## Behavior

### Read the mounted-source catalog
- **Given**: a live Path Session and the process's compiled Source Adapter registry, whose entries each carry a non-empty scheme label, one-line grammar, grammar-valid literal example, and active or degraded mount state
- **When**: the Agent caller invokes `rfs_read` with `path: "rfs://"` and valid optional lower-only text limits
- **Then**: the call succeeds through the common `rfs_read` result contract; its complete projection has a header that names `rfs_read` of `rfs://workspace` as the next discovery step and states the common selector grammar once, followed by exactly one line per mounted registry entry sorted by scheme label, each line contains the scheme, grammar, and example, an entry in degraded state additionally contains `[degraded: <public reason>]`, and no scheme absent from the registry appears

### Read declared Workspace Roots
- **Given**: a live Path Session whose current `WorkspaceRootSet` contains zero through 256 declared roots and may or may not identify a Primary Workspace Root
- **When**: the Agent caller invokes `rfs_read` with `path: "rfs://workspace"` (or the accepted alias `rfs://workspace/`) and valid optional lower-only text limits
- **Then**: the call succeeds through the common `rfs_read` result contract with canonical reference `rfs://workspace`; its complete projection contains exactly one line per declared root sorted by root ID using canonical `rfs://workspace/<root>/` spelling, each of which is a valid Path Reference addressing that root's directory (today `rfs_read` of it returns `unsupported_projection` with a directory teaching message naming rfs-hwlm, because directory reads are unimplemented for every directory; rfs-hwlm's bounded directory-listing reads will return the listing for the same reference), marks only the Primary Workspace Root with ` (primary)`, exposes no backing path or grant summary, emits `No Workspace Roots declared.` for zero roots, and emits `No Primary Workspace Root selected.` when roots exist without a primary

### Reflect client root-set changes
- **Given**: an MCP client that declares Roots capability, a completed `roots/list_changed` notification, and a successful replacement root acquisition
- **When**: the Agent caller next invokes `rfs_read` with `path: "rfs://workspace"`
- **Then**: the returned complete projection contains the replacement root set and primary selection, contains no removed root, and carries the Version Tag derived from that projection; a root notification racing an already-started read is reflected by the next read after acquisition completes

### Preserve common bounded-read behavior
- **Given**: either catalog's complete UTF-8 projection and valid lower-only `maxBytes`, `maxLines`, or `maxColumns` limits below the projection size
- **When**: the Agent caller invokes `rfs_read` for that catalog
- **Then**: the common `ReadEngine` returns non-empty inline text within every supplied and server ceiling, sets `bounded: true`, supplies progressing `continuationReference` and lossless `recoveryReference` values, and preserves the complete immutable projection in the Path Session artifact; a projection fitting the effective limits returns `bounded: false` without continuation or recovery

### Keep catalog Resources immutable
- **Given**: either catalog reference and any Workspace Root or Portable Source mutation grants
- **When**: shared mutation eligibility classifies the Resource
- **Then**: it reports the catalog as unconditionally immutable; when `rfs_write` and `rfs_edit` are implemented by rfs-73dz, mutation attempts must return a `permission_denied` tool error and leave authoritative state unchanged

### Redirect unsupported search and glob operations
- **Given**: a valid catalog reference, `rfs://` or `rfs://workspace`
- **When**: the Agent caller supplies it as the concrete target of `rfs_search` or `rfs_glob`
- **Then**: the operation returns an `unsupported_projection` tool error whose complete text directs the caller to `rfs_read` of the same catalog reference; synthetic discovery Resources are the explicit exception to any readable-implies-searchable convention

### Advertise discovery at initialization
- **Given**: ResourceFS initialization or tool discovery
- **When**: the MCP client reads server initialization instructions or any currently visible `rfs_*` tool description
- **Then**: the text names `rfs_read` of `rfs://` as the discovery entry point before operation-specific detail

## Success criteria

- **Binary / structural / security**: the `rfs://` projection's scheme set equals the compiled registry's mounted scheme set, every entry has one non-empty grammar and one grammar-valid literal example, entries are sorted, degraded entries include only their bounded public reason, and unmounted schemes are absent; checked by direct registry/catalog tests with active, degraded, and unmounted fixture entries.
- **Quantitative**: `rfs://workspace` enumerates exactly 0–256 active `WorkspaceRootSet` entries and marks zero or one primary, measured by direct tests at empty, single-root, unselected-multi-root, selected-multi-root, and 256-root boundaries.
- **Binary / structural / security**: every reference rendered by either catalog parses through `PathReference::parse`, `rfs_read` of each workspace-root line returns `unsupported_projection` with a directory teaching message naming `rfs_glob` and rfs-hwlm (never `invalid_reference`), and `rfs://workspace/` resolves to the same catalog as `rfs://workspace` with canonical reference `rfs://workspace`; checked by direct parser tests over rendered catalog output and MCP read assertions.
- **Binary / structural / security**: the `rfs://` header contains the chained `rfs://workspace` reference and each common selector form (`:N`, `:N-M`, `:N-`, comma-separated ranges, `:raw`, `:page:N`); checked by direct catalog-rendering assertions.
- **Binary / structural / security**: the first `rfs://workspace` read after a completed MCP root-set refresh equals the replacement root IDs and primary selection and excludes removed IDs, checked by an MCP server integration test that sends `roots/list_changed`, supplies replacement roots, and reads the catalog.
- **Quantitative**: every inline catalog page obeys the effective ceilings of at most 49,152 UTF-8 bytes, 3,000 lines, and 512 columns plus any lower caller limits, measured by focused `ReadEngine` tests that force byte, line, and column pagination and recover the complete projection.
- **Binary / structural / security**: both catalog Resources serialize through the existing object-rooted `ReadToolOutput` without new catalog-specific fields, with canonical reference, `text/plain; charset=utf-8`, content-derived Version Tag, `mutable: false`, boundedness, and complete non-empty `content`; checked by direct renderer/integration assertions over text and `structuredContent`.
- **Binary / structural / security**: both catalog addresses are unconditionally immutable and future mutation routing is pinned to `permission_denied`; checked now by direct shared address/resource classification tests and recorded on rfs-73dz for end-to-end tool enforcement.
- **Binary / structural / security**: `rfs_search` and `rfs_glob` reject each catalog target with `unsupported_projection` and a text redirect naming `rfs_read` plus the same target, checked by direct MCP tool-result tests.
- **Binary / structural / security**: initialization instructions and every currently visible `rfs_*` description contain the `rfs_read`/`rfs://` discovery cue, checked by MCP initialization and `tools/list` assertions.

## Out of scope

This change does NOT include: implementing `rfs_write` or `rfs_edit`; end-to-end mutation-tool rejection before rfs-73dz; searching or globbing catalog contents; catalog-specific structured-content arrays; MCP Resources/templates mirroring; new Source Adapters; advertising roadmap schemes that are not compiled and mounted; `local://` Session Scratch before rfs-60g1; backing filesystem paths or mutation-grant summaries in workspace-root output; network probing initiated by a catalog read; aliases other than `rfs://workspace/` and the slash-less root-directory form; selectors on catalog references; bounded directory-listing reads for any directory (rfs-hwlm); or a new operational error category.

## Related issues

- rfs-jk0d: parent epic; establishes the fixed five-tool Path Resource Server and self-describing namespace goal.
- rfs-5os7: downstream Kiro acceptance; consumes this discovery entry point through visible tools.
- rfs-cgbq: prior Workspace Root identity and refresh semantics adopted here, including the 256-root limit and optional Primary Workspace Root.
- rfs-34pz: prior bounded read and `artifact://` recovery contract adopted unchanged; `artifact://` is a compiled catalog entry.
- rfs-r9m6: prior strict Server Profile and source probe/degraded-state vocabulary adopted; catalog reads must not perform new probes.
- rfs-73dz: future mutation tools; must enforce this spec's unconditional catalog immutability as `permission_denied`.
- rfs-ww6w: future MCP Resources/templates mirror; catalog mirroring is not included here.
- rfs-60g1: future `local://` Source Adapter; it remains absent until compiled and mounted.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| Who consumes discovery? | Agent caller through visible tools; ResourceFS operator configures what is mounted. | ADR-0005 names an unconfigured harness and the issue names configured/degraded sources. | The catalog is self-sufficient for a text-only tool caller and truthful to operator configuration. |
| Which schemes may appear? | Compiled Source Adapter registry only. | Requester selected “Compiled registry only.” | Filesystem (`rfs://workspace`) and artifact (`artifact://`) appear today; future schemes appear only when their adapters compile and mount. |
| How does degraded state reach the catalog? | Each adapter self-description carries active or degraded-with-public-reason state. | Requester selected “State in adapter self-description.” | Reads do not run probes; degraded fixtures can be tested before optional production adapters land. |
| What is the source catalog text layout? | Header plus one line per scheme, sorted by scheme label; degraded reason is appended as `[degraded: reason]`. | Requester selected “One line per scheme, sorted.” | Output is deterministic, compact, and model-scannable. |
| What does each workspace-root line show? | Canonical `rfs://workspace/<root>/` spelling and ` (primary)` only. | Requester selected “Canonical spelling + primary marker.” | Backing paths and grant summaries are absent. |
| How are zero roots and absent primary represented? | Explicit status lines; structured `content` contains the same text. | Requester selected “Explicit status lines.” | Text remains non-empty for both valid edge states. |
| Does structured output gain catalog-specific fields? | No; reuse the existing common `ReadToolOutput` unchanged. | Requester selected “Reuse common output unchanged.” | The catalog is carried in `content`; no output-schema branch is added. |
| What does “valid example spelling” mean? | Grammar-valid literal syntax, not guaranteed current resolvability. | Requester selected “Grammar-valid syntax”; empty artifact/root states make guaranteed resolvability impossible. | Every example must parse, but may name a Resource absent from the current Path Session. |
| Which tool descriptions carry discovery guidance? | Every currently visible `rfs_*` description and initialization instructions. | Requester selected “Every visible rfs_* tool.” | `rfs_read`, `rfs_search`, and `rfs_glob` all direct the caller to `rfs_read` of `rfs://`; later tool descriptions inherit the rule. |
| Are catalogs searchable or globbable? | No; they are small synthetic discovery Resources and an explicit readable-but-not-searchable exception. | Requester explained that searching saves nothing and globbing invents a hierarchy. | Search/glob reject catalog targets and teach the one-call correction. |
| Which search/glob error category applies? | `unsupported_projection`. | Requester selected the existing category instead of adding `unsupported_operation`. | The error message must redirect to `rfs_read` of the same target. |
| How is mutation rejection delivered before mutation tools exist? | Classify catalog Resources unconditionally immutable now; rfs-73dz enforces tool calls later. | Requester selected “Classify now; enforce in rfs-73dz.” | Runtime acceptance is explicitly narrowed for this issue and the dependency is recorded. |
| Which future mutation error category applies? | `permission_denied`. | Requester selected `permission_denied`. | A valid readable catalog reference remains valid; the denied action is mutation. |
| Which catalog spellings are valid? | Canonical `rfs://` and `rfs://workspace`; `rfs://workspace/` is accepted as an alias that resolves to the same catalog and reports canonical reference `rfs://workspace`. Selectors on catalog references remain unsupported. | Requester reversed the exact-only decision: the parser's canonical prefix is literally `rfs://workspace/` (`reference.rs:12`) and every workspace-catalog line ends in `/`, so callers will produce the slash form; inside the discovery feature, discoverability outranks grammar purity. | The alias parses to the same address, never reaches the invalid-reference path, and is not separately advertised; the canonical spelling is what results and descriptions render. |
| Are rendered workspace-root references valid? | Yes. `rfs://workspace/<root>/` becomes a valid Path Reference addressing that root's directory (the slash-less `rfs://workspace/<root>` form is an alias of it); every reference either catalog renders must parse. Reading a root reference today returns `unsupported_projection` with a directory teaching message naming rfs-hwlm, because directory reads are unimplemented for every directory; rfs-hwlm returns the listing for the same reference. | Requester pushback: today `WorkspacePath::new` rejects the empty path (`reference.rs:95-98`), so the catalog would advertise references the server itself rejects on the caller's very first follow-up read. Review then found `read_address` rejects all non-files (`filesystem.rs:2327-2331`), so a directory listing is a separate projection (rfs-hwlm), not a catalog concern. | A small grammar extension in `reference.rs` (`WorkspacePath::root()`), a canonical reference for the filesystem adapter's existing empty-path directory entry, and a teaching error on directory reads; a success criterion checks that every rendered reference parses and each root read teaches rather than failing as invalid. |
| What does the `rfs://` catalog header contain? | The header names `rfs_read` of `rfs://workspace` as the next discovery step and states the common selector grammar once (`:N`, `:N-M`, `:N-`, comma-separated ranges, `:raw`, `:page:N`), before the per-scheme lines. | Requester pushback: the selector algebra is the highest-value thing a caller learns from the catalog and is cross-cutting rather than per-scheme, so a pure one-line-per-scheme layout would omit it; discovery must chain from the root to the workspace catalog. | Header content is bounded and deterministic; tests assert the chained `rfs://workspace` reference and the selector forms are present; per-scheme lines stay one line each. |
| How are lower text limits handled? | Existing `ReadEngine` pagination and lossless artifact recovery, unchanged. | The issue requires the common limits/result/error contract; rfs-34pz already owns it. | Catalog adapters produce complete projections; the shared engine bounds inline pages. |
| What happens when a root refresh races a read? | A read uses the root snapshot acquired before its source read; a concurrent later refresh appears on the next read. | Existing `RootSync` serializes acquisition before dispatch. | No mixed root-set projection is observable. |
| Empty source set | N/A — production registry contains mandatory filesystem and artifact adapters. | CompiledSources construction provides both today. | The source catalog always has at least two entries in this build. |
| Maximum scale | 256 Workspace Roots; source output additionally obeys common read ceilings. | `MAX_WORKSPACE_ROOTS` and existing text ceilings are public contract constants. | Boundary fixtures cover 256 roots and lower-limit pagination. |
| Null or missing metadata | Adapter self-description rejects empty scheme, grammar, example, or degraded reason; absent primary is valid and explicit. | Every catalog line requires these fields; `WorkspaceRootSet::primary` is optional. | Malformed compiled metadata cannot silently produce an incomplete entry. |
| Concurrent writes | N/A — catalog Resources are immutable; root-set replacement is the only concurrent state change and uses snapshot semantics above. | Read-only classification and existing root refresh machinery. | No catalog write serialization is introduced. |
| Permission denied or unauthenticated | Catalog reads do not depend on mutation grants; stdio MCP connection establishment is outside this Resource behavior. | Discovery must work in an unconfigured harness; policy must not hide mounted schemes. | Grants cannot hide or mutate catalogs. |
| Partial failure | A mounted degraded source remains listed with its public reason; unmounted sources remain absent. | Issue acceptance explicitly distinguishes degraded from unmounted. | One unavailable optional source does not fail the whole catalog read. |
| Retries and idempotency | Repeated reads against the same registry/root snapshot produce identical content and Version Tags. | Content-derived Version Tags and deterministic sorting. | Callers can compare catalog versions without side effects. |
| Soft-deleted records | N/A — catalogs contain registry descriptors and active root identities, not records with deletion lifecycle. | No applicable domain object. | No tombstones appear. |
| Multi-tenancy boundaries | Compiled source entries are process-wide; Workspace Roots and recovery artifacts are scoped to the calling Path Session. | Existing session/authority model from rfs-cgbq and rfs-34pz. | One connection cannot observe another connection's root set or recovery artifacts. |
| Time-zone or DST | N/A — no timestamps are rendered or compared. | Catalog state is structural. | Clock changes cannot alter output. |
| Replication lag | N/A — no replicated store is consulted. | Catalog reads use local registry and root snapshots. | No eventual-consistency claim exists. |
| Cache invalidation | No catalog cache; source text derives from the current registry snapshot and workspace text from the current root snapshot on each read. | Root-set changes must be visible on the next read. | Version Tags change exactly when rendered content changes.

## Approval

Requester approval (verbatim): "Yes, approved"
Date: 2026-08-22
Approved revision: canonical workspace-catalog alias, valid root-directory references with teaching reads, and the chained selector-grammar header.
