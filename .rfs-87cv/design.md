# Design: rfs-87cv

## Route and inputs

- Route: **Empirical**, from `.rfs-87cv/route.md`.
- Behavior source: `spec.md` is N/A because `route.md` T4 records the complete explicit behavior set. That set requires one named launch root, clean stdio negotiation, tools-only capability advertisement, object-root `rfs_read` schemas, complete contained UTF-8 content through text and structured representations, stable tool-error categories, direct filesystem-adapter tests, and a compiled-process MCP test.
- Empirical source: `.rfs-87cv/evidence.md`, `.rfs-87cv/probe.py`, and `.rfs-87cv/oracle.py`.
- Empirical premises: current `rmcp` must support `2026-07-28` and `2025-11-25`; it must support object-root tool schemas and independent TextContent plus structured content. Both are PASS.
- Empirical learning: `rmcp` 3.1.3 supports both revisions, but `ProtocolVersion::LATEST` is still `2025-11-25`. ResourceFS must explicitly advertise/default to `V_2026_07_28` and explicitly retain `V_2025_11_25` in its supported set.
- Spec edge-case table: N/A — no `spec.md`; the production-reachable shapes below derive from route T4, the accepted `DESIGN.md`, and the empirical comparison.
- Delivery increments from spec: N/A — no `spec.md`; `plan.md` will derive independently green slices from the approved claims.

## Input shapes

### CLI and Workspace Root

| Shape | Status |
|---|---|
| Exactly one `--root <name>=<path>` and matching `--primary-root <name>`; root path is an existing directory | Covered by C2 |
| Relative root backing path | Covered by C2 — resolve and canonicalize once at startup |
| Absolute root backing path | Covered by C2 |
| Root path missing or backed by a file | Covered by C2 — startup configuration error on stderr with non-zero exit |
| No root, repeated roots, missing primary, or primary name mismatch | Covered by C2 — startup configuration error |
| Empty root name or a name containing `/`, `:`, `?`, `#`, or `=` | Covered by C2 — startup configuration error |
| Multiple launch roots, client-supplied Roots, or root-set changes | N/A — intended future work `rfs-cgbq` owns complete root precedence and transitions |
| Server Profile roots | N/A — intended future work `rfs-r9m6` owns the strict Server Profile |

### MCP session and protocol

| Shape | Status |
|---|---|
| Client requests `2026-07-28` | Covered by C1 and C7 |
| Client requests `2025-11-25` | Covered by C1 and C7 |
| Client requests an unsupported revision | Covered by C1 — SDK negotiation returns the explicit server fallback `2026-07-28`; no unsupported capability is advertised |
| Client capabilities empty or unrelated optional capabilities present | Covered by C7 — server advertises tools only |
| Missing/malformed JSON-RPC method or tool arguments | Covered by C7 — remains an MCP protocol/schema error, not a ResourceFS tool error |
| More than one simultaneous MCP connection | N/A — this slice carries no shared mutable/session state; connection-owned Path Session storage starts in `rfs-34pz` |

### `rfs_read` input object

| Shape | Status |
|---|---|
| Object with one string `path` | Covered by C3, C4, C7, and C8 |
| Missing `path`, null, non-string, non-object arguments, or unknown fields | Covered by C7 — schema/protocol rejection; unknown fields are rejected so misspellings cannot be ignored |
| Optional per-call `limits` | N/A — intended future work `rfs-34pz` adds lower-only limits together with lossless Recovery References; adding an optional field is schema-compatible |

### Path Reference string

| Shape | Status |
|---|---|
| Non-empty relative ASCII path, including nested components | Covered by C3 and C4 |
| Relative Unicode path or path with embedded spaces | Covered by C3 and C4 — strings are not trimmed |
| Canonical `rfs://workspace/<configured-root>/<relative-path>` | Covered by C3 and C4 |
| Empty string, `.` resolving to the root directory, malformed `rfs://` reference, unknown scheme, or wrong root name | Covered by C3 and C8 |
| `..` component or relative spelling that escapes the root lexically | Covered by C3 and C8 |
| Symlink whose canonical target stays inside the root | Covered by C4 |
| Symlink whose canonical target is outside the root | Covered by C4 and C8 |
| Absolute filesystem paths, `file://`, Windows drive/UNC forms, percent-encoded ambiguous delimiters, selectors, and existing-literal/selector precedence | N/A — intended future work `rfs-cgbq` owns the complete Path Reference grammar and multi-platform forms |

### Backing Resource

| Shape | Status |
|---|---|
| Existing non-empty UTF-8 text below all hard ceilings | Covered by C4, C5, and C8 |
| Existing empty UTF-8 file | Covered by C4, C5, and C8 — structured `content` is exactly empty; TextContent remains non-empty through the read header |
| Missing target | Covered by C4 and C8 — `not_found` |
| Directory target | Covered by C4 and C8 — `unsupported_projection` |
| Invalid UTF-8/binary target | Covered by C4 and C8 — `unsupported_projection` |
| Text at exactly 48 KiB/3,000 lines/512 columns | Covered by C9 |
| Text exceeding any hard ceiling by one unit | Covered by C9 — bounded `limit_exceeded`, never partial success |
| File changes while read is in flight | N/A — this read-only slice returns the bytes actually read and their content hash; mutation serialization and snapshots are intended future work `rfs-73dz` |
| Adversarial path replacement during canonicalize/open | N/A — intended future work `rfs-cgbq` owns complete race-resistant cross-platform containment; this slice deterministically rejects lexical and canonical/symlink escapes |

### Tool output

| Shape | Status |
|---|---|
| Success with non-empty content | Covered by C8 |
| Success with empty content | Covered by C8 |
| Operational error | Covered by C8 — non-empty TextContent, object structured content, `isError=true` |
| Protocol/schema error | Covered by C7 — MCP error, not a tool-result envelope |
| Omitted/recovered successful content | N/A — intended future work `rfs-34pz` owns immutable `artifact://` recovery and lower-only read limits |

## Removed invariants

Purely additive change: the repository has no production code or prior constraints to remove.

## Placement

### Behavior Contract types and reference parsing

- **Owner:** `resourcefs-core`. It owns `PathReference`, `RootName`, `VersionTag`, `ResourceError`/`ErrorCategory`, the hard read ceilings, and source-neutral `ReadRequest`/`ReadResource` values because every later adapter and MCP surface must share those semantics.
- **New seam:** Shape A places source-neutral parsed references and results in core and exposes a minimal async `SourceAdapter::read` interface; Shape B lets each adapter parse raw strings and return adapter-specific values. Choose A: one caller-visible reference/result interface gives leverage across future adapters and keeps grammar/error drift local. Although only one adapter exists in this slice, the accepted three-crate architecture and already-filed adapter work make this an intentional first implementation of a committed seam, not a hypothetical extension point.
- **Forbidden:** core may not depend on `rmcp`, `clap`, Tokio filesystem types, or filesystem implementation details; it may not know CLI arguments or TextContent wire rendering.

### Filesystem resolution and read

- **Owner:** `resourcefs-sources`, in a `FilesystemSource` adapter implementing core's `SourceAdapter` interface. Resolution, root canonicalization, deterministic containment, bounded open/read, UTF-8 validation, and filesystem error mapping stay together.
- **New seam:** Shape A has MCP call `FilesystemSource` directly; Shape B has MCP depend only on `Arc<dyn SourceAdapter>`. Choose B because the accepted adapter registry requires one source-neutral call site and direct adapter tests can exercise the same interface. The interface contains only the behavior this slice implements; no placeholder enumeration or mutation methods are added.
- **Forbidden:** the MCP crate may not join/canonicalize paths or map `std::io::Error`; the filesystem adapter may not construct MCP `CallToolResult`, inspect client capabilities, or write protocol stdout.

### MCP/CLI adaptation

- **Owner:** `resourcefs-mcp`. The `resourcefs` binary, clap `serve` command, `rmcp` handler, explicit supported-version set, schemas, capabilities, result envelope, TextContent header, error rendering, and stdio lifecycle live here.
- **New seam:** Shape A makes core/source results implement MCP conversion traits and depend on `rmcp`; Shape B keeps a private renderer in the MCP crate that maps core values into `CallToolResult`. Choose B: protocol churn and Kiro-specific rendering remain in the outer adapter, while core/source contract tests stay transport-free.
- **Forbidden:** no protocol diagnostics on stdout; no resources/prompts/tasks capability; no stub registrations for unimplemented tools; no OMP runtime/configuration dependency; no shell invocation.

### Workspace structure

- **Owner:** one Cargo workspace with principal crates `resourcefs-core`, `resourcefs-sources`, and `resourcefs-mcp`; the last package ships `[[bin]] name = "resourcefs"` and is installable with Cargo.
- **New seam:** N/A — the accepted architecture already fixes the three principal crate modules; this change instantiates it.
- **Forbidden:** dependency direction is `resourcefs-mcp -> resourcefs-sources -> resourcefs-core`, with `resourcefs-mcp -> resourcefs-core` allowed for rendering; core/sources must not depend on MCP. No fourth pass-through crate.

## Claims

- **C1.** `rmcp` 3.1.3 can explicitly advertise/default ResourceFS to MCP `2026-07-28`, negotiate `2025-11-25`, and limit advertised capabilities to tools.
- **C2.** `resourcefs serve` starts only with exactly one valid named Primary Workspace Root whose backing directory is canonicalized once at startup.
- **C3.** core parsing accepts only this slice's relative and matching canonical workspace references and rejects malformed, foreign-root, absolute, and lexically traversing inputs without consulting the process working directory.
- **C4.** `FilesystemSource` returns only UTF-8 bytes read from a canonical target contained under its canonical root and maps missing, directory, binary, and escape cases to stable core categories.
- **C5.** every successful read carries `sha256:<64 lowercase hex>` derived only from the exact authoritative content bytes.
- **C6.** source-neutral contracts remain in core, filesystem behavior remains in sources, and MCP/CLI/rendering behavior remains in MCP under the accepted dependency direction.
- **C7.** the compiled binary's stdio stream is valid MCP at both required revisions; `tools/list` exposes only `rfs_read` with object-root input/output schemas and tools-only capabilities.
- **C8.** every `rfs_read` success and operational error returns non-empty TextContent plus object structured content; success structured `content` exactly equals the file, and error mappings are `invalid_reference`, `not_found`, `permission_denied`, `unsupported_projection`, or `limit_exceeded` as applicable.
- **C9.** filesystem reads allocate and return no more than the 48 KiB/3,000-line/512-column hard ceilings; an over-limit file fails without partial content because Recovery References are not yet present.
- **C10.** the founding server exposes no unimplemented tool, Resource, Prompt, Task, mutation, or recovery behavior.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Explicit 2026 default, 2025 negotiation, tools-only capability are supported. | Both required revisions; unsupported revision fallback; empty optional client capabilities. | Run the pinned runtime probe; falsified if either supported revision does not negotiate exactly or if schemas/content/capabilities differ. | `.rfs-87cv/oracle.py` statically computes the answer from published crate source, independent of runtime probe and production. | In `crates/resourcefs-mcp/src/server.rs`, replace explicit `V_2026_07_28` server info with `ProtocolVersion::LATEST`; `stdio_mcp_contract::negotiates_required_revisions` must report 2025 instead of expected 2026 advertisement/fallback. | `stdio_mcp_contract::negotiates_required_revisions` using a raw JSON-RPC client, not an `rmcp` client. | ~15 seconds on warm Cargo cache | PASS |
| C2 | Exactly one valid launch root becomes authority. | Valid relative/absolute directory; missing/file root; zero/repeated root; missing/mismatched primary; invalid name. | Launch the compiled binary for every CLI matrix row; falsified if an invalid row serves or a valid row fails. | Fixture topology and exit status/stderr are declared by the test before process launch; no production parser computes the expected row. | In `crates/resourcefs-mcp/src/cli.rs`, accept zero roots or skip the primary-name equality check; `stdio_mcp_contract::rejects_invalid_root_configuration` must become green-to-red. | `stdio_mcp_contract::rejects_invalid_root_configuration`. | ~3 seconds | PENDING — checkpointed-build, slice implementing CLI/root startup |
| C3 | Minimal Path Reference parsing is deterministic and contained. | Empty, ASCII, Unicode, spaces, nested, canonical matching/wrong root, malformed scheme, absolute, `..`. | Table-test `PathReference::parse`; falsified by any accepted/rejected row differing from the design table. | The table is hand-authored from the accepted grammar subset and uses platform path components only to classify absolute/traversal, not production parser code. | In `crates/resourcefs-core/src/reference.rs`, remove the `ParentDir` rejection; `path_reference_contract::rejects_parent_traversal` must accept `../secret` and fail. | `path_reference_contract` public-interface test suite. | <1 second | PENDING — checkpointed-build, slice implementing core contract |
| C4 | Filesystem reads stay under the canonical root and map source failures. | Existing text, missing, directory, invalid UTF-8, internal symlink, escaping symlink. | Call the adapter directly against a temporary root/outside fixture; falsified if outside bytes appear, inside text changes, or a category differs. | The fixture records known bytes and known inside/outside topology before adapter construction; expected categories are fixed literals rather than another resolver. | In `crates/resourcefs-sources/src/filesystem.rs`, read the joined lexical path instead of the validated canonical target; `filesystem_adapter_contract::rejects_symlink_escape` must expose outside bytes and fail. | `filesystem_adapter_contract` direct Source Adapter suite. | ~1 second | PENDING — checkpointed-build, slice implementing filesystem adapter |
| C5 | Version Tags are exact content-derived SHA-256. | Empty and non-empty content; same bytes under different paths/mtimes; changed byte. | Compare tags with fixed published SHA-256 vectors and equality/difference cases; falsified by any mismatch. | Hard-coded SHA-256 test vectors provide an independent expected digest; no production hashing helper computes it. | In `crates/resourcefs-core/src/version.rs`, hash the path or metadata with the content; `version_tag_contract::ignores_path_and_metadata` must produce unequal tags for equal bytes and fail. | `version_tag_contract` public-interface test suite. | <1 second | PENDING — checkpointed-build, slice implementing core contract |
| C6 | The three crate modules own the accepted behaviors without reverse dependencies. | Core types, one async adapter seam, filesystem implementation, MCP renderer/CLI. | Inspect `cargo metadata` dependency edges and compile direct core/source tests without MCP; falsified if core or sources depends on `rmcp`/`clap`, or MCP performs filesystem resolution. | The accepted dependency graph in `DESIGN.md:269-279` is independent of production manifests and the runtime probe. | Add `rmcp` to `resourcefs-core/Cargo.toml`; `architecture_contract::enforces_dependency_direction` must report the forbidden edge and fail. | `architecture_contract::enforces_dependency_direction`, backed by `cargo metadata --no-deps`. | ~1 second | PENDING — checkpointed-build, first workspace slice |
| C7 | Real stdio MCP is clean and exposes the exact founding tool contract at both revisions. | 2026/2025 initialize; list; valid call; malformed protocol/tool args. | Spawn the compiled binary and drive newline-delimited raw JSON-RPC; falsified by non-JSON stdout, wrong negotiation/capabilities, absent/extra tool, non-object schema, or protocol error misclassification. | The test's raw JSON-RPC driver and hand-authored expected wire objects do not use `rmcp`, unlike production and the empirical probe. | In `crates/resourcefs-mcp/src/server.rs`, enable resources or omit the explicit output schema; `stdio_mcp_contract::lists_object_root_read_tool_only` must observe an extra capability or missing object root and fail. | `stdio_mcp_contract` compiled-process suite. | ~4 seconds | PENDING — checkpointed-build, MCP process slice |
| C8 | Success/error tool results remain complete for text-only and structured clients. | Non-empty/empty text; malformed, missing, directory, escape, binary, and limit errors. | Through raw MCP, compare TextContent/header, structured envelope, exact `content`, `isError`, and category for every case; falsified by empty text, lost file content, absent structured object, or wrong category. | Fixture literals and the design's explicit category table determine expected wire values without calling the adapter directly. | In `crates/resourcefs-mcp/src/render.rs`, omit `structured_content` on error or emit empty text for an empty file; `stdio_mcp_contract::renders_complete_success_and_errors` must fail. | `stdio_mcp_contract::renders_complete_success_and_errors`. | ~4 seconds | PENDING — checkpointed-build, MCP process slice |
| C9 | Reads remain within hard ceilings without partial success. | Exact and one-over byte/line/column boundaries. | Directly read generated boundary fixtures; falsified if exact limits fail, one-over succeeds, allocation reads beyond max+1 sentinel, or partial content is returned. | Test-generated counts and lengths are computed before writing fixtures; expected boundary outcomes do not use production limit code. | In `crates/resourcefs-sources/src/filesystem.rs`, replace bounded `take(MAX_BYTES + 1)` reading and dimension checks with `tokio::fs::read`; `filesystem_adapter_contract::enforces_hard_read_limits` must accept an over-limit fixture and fail. | `filesystem_adapter_contract::enforces_hard_read_limits`. | ~1 second | PENDING — checkpointed-build, filesystem adapter slice |
| C10 | No stub surface ships. | Tool/resource/prompt/task enumeration with one implemented read tool. | Inspect initialization and `tools/list`; falsified if any unimplemented tool or optional surface is advertised. | The issue's acceptance list and verified future issue IDs define the exact current surface independently of production registration. | In `crates/resourcefs-mcp/src/server.rs`, register a no-op `rfs_search`; `stdio_mcp_contract::lists_object_root_read_tool_only` must observe the extra tool and fail. | `stdio_mcp_contract::lists_object_root_read_tool_only`. | ~2 seconds | PENDING — checkpointed-build, MCP process slice |

## Non-goals and future work

### Permanent non-goals

- OMP runtime/configuration discovery or private OMP session-layout access: ResourceFS is an independent product; adding this would violate ADR 0001.
- Shell interpretation, protocol logging on stdout, telemetry, or mutation authority: none is required for a local read path, and each would add authority or corrupt the transport.
- Placeholder implementations for incomplete tools/capabilities: incomplete behavior is less safe than an absent surface and is prohibited by C10.

### Intended future work

- Complete multi-root/client-root authority, absolute/file/Windows references, delimiter encoding, selectors, root transitions, and race-resistant cross-platform containment: `rfs-cgbq`.
- Bounded successful omission, lower-only per-call limits, Path Sessions, and immutable Recovery References: `rfs-34pz`.
- Workspace search/glob and their tool registrations: `rfs-vl0u`.
- Workspace write/hashline edit and mutation snapshots: `rfs-73dz`.
- Rust Structural Summary behavior: `rfs-m739` (other language tickets depend on it).
- Strict Server Profiles plus `check`/`schema` composition: `rfs-r9m6`.
- MCP Resource/template mirroring: `rfs-ww6w`.
- Full real-Kiro workflow acceptance: `rfs-5os7`.

Every ID above was verified in the repository-local Rivets store on 2026-08-18.

## Falsifier run log

- 2026-08-18 — `python3 .rfs-87cv/probe.py` — PASS. `rmcp` 3.1.3 negotiated both required revisions; schemas were object-rooted; TextContent and structured content agreed; only tools capability was present.
- 2026-08-18 — `python3 .rfs-87cv/oracle.py` — PASS. Published source independently confirmed both supported revisions, default `LATEST=2025-11-25`, supported-version echo/fallback negotiation, stdio transport, schema validation, and split text/structured result representation.
- Cheapest falsifier: C1, PASS.

## Approval

- Status: APPROVED
- Requester words: “approved as written”
- Date: 2026-08-18
- Approved risk acceptances: None proposed; every claim has a deterministic regression fence.
