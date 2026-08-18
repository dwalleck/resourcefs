# Plan: rfs-87cv

## Inputs

- Route: Empirical, `.rfs-87cv/route.md`.
- Approved design: `.rfs-87cv/design.md`, requester words “approved as written” on 2026-08-18, no approved risk acceptances.
- Evidence: `.rfs-87cv/evidence.md`; both empirical premises PASS.
- Review re-entry: N/A — this is the initial implementation plan and no review decision log exists.

## Partition arithmetic

| Slice | Implementation/tests/fixtures | Cargo.lock estimate | Diff estimate |
|---|---:|---:|---:|
| 1. Core contract | 700 | 250 | 950 |
| 2. Filesystem adapter | 650 | 200 | 850 |
| 3. Stdio MCP binary | 1,050 | 1,700 | 2,750 |
| **Sum** | **2,400** | **2,150** | **4,550** |

Churn margin: 15% = 683 lines. Rationale: this is a greenfield Rust workspace, so generated lockfile growth and raw JSON-RPC fixture ergonomics are the two uncertain line-count sources; 15% covers either without hiding a material design expansion.

Projected cumulative total: 4,550 + 683 = **5,233 changed lines**. This exceeds the 4,000-line review-size gate, so the plan has two independently mergeable PR increments.

### PR increment A: `core-filesystem-read`

- Slices: 1 and 2.
- Mergeable definition: a green Cargo workspace containing the source-neutral Behavior Contract and a directly tested filesystem Source Adapter. It has no binary or MCP surface and exposes no placeholder behavior.
- Independent verification: public core contract tests and direct adapter tests run without `rmcp`, `clap`, or any later increment.

### PR increment B: `stdio-mcp-read`

- Slices: 3.
- Mergeable definition: adds the installable `resourcefs` binary, explicit revision negotiation, CLI launch authority, tool schemas/rendering, and real-process MCP verification on top of increment A.
- Independent verification: raw JSON-RPC compiled-process tests plus the architecture dependency check; it does not depend on any issue outside `rfs-87cv`.

## Slice 1: Establish the source-neutral read contract

**Claim IDs:** C3, C5

**Expected behavior:** Public core interfaces parse the approved relative/canonical workspace-reference subset, reject malformed/foreign/absolute/traversing references, expose stable error categories and read values, and derive exact `sha256:<64 lowercase hex>` Version Tags solely from content.

**Oracle:** Hand-authored grammar tables from the approved design determine parser outcomes; fixed published SHA-256 vectors independently determine Version Tags.

**Stress fixture:** A 4,096-byte Unicode/space-containing nested reference remains linear and valid when relative; a sibling reference containing `..` is rejected as `permission_denied`; a 48 KiB byte buffer hashes to the same tag regardless of caller-supplied path metadata.

**Regression fence:** `crates/resourcefs-core/tests/path_reference_contract.rs` and `crates/resourcefs-core/tests/version_tag_contract.rs`, created in this slice.

**Named mutation:** C3 — in `crates/resourcefs-core/src/reference.rs`, remove the `ParentDir` rejection; `path_reference_contract::rejects_parent_traversal` must turn red. C5 — in `crates/resourcefs-core/src/version.rs`, include path or metadata in the digest; `version_tag_contract::ignores_path_and_metadata` must turn red.

**Complexity/production scale:** Reference parsing is O(n) in UTF-8 bytes/components; at a production-shaped 4 KiB path it must complete in at most 5 ms in a debug test process, a deliberately loose bound over a single linear scan. Version hashing is O(n) over at most the 48 KiB hard read ceiling and must complete in at most 10 ms, bounding one digest pass without avoidable copies.

**Wall budget/phase:** Always-on phases: reference parsing ≤5 ms per call at 4 KiB; Version Tag hashing ≤10 ms per successful 48 KiB read. Rationale: both are CPU-only linear passes and together remain a small fraction of the 250 ms end-to-end local-read budget in Slice 3.

**Files:** `Cargo.toml`; `Cargo.lock`; `crates/resourcefs-core/Cargo.toml`; `crates/resourcefs-core/src/lib.rs`; `crates/resourcefs-core/src/error.rs`; `crates/resourcefs-core/src/reference.rs`; `crates/resourcefs-core/src/resource.rs`; `crates/resourcefs-core/src/source.rs`; `crates/resourcefs-core/src/version.rs`; `crates/resourcefs-core/tests/path_reference_contract.rs`; `crates/resourcefs-core/tests/version_tag_contract.rs`.

**Estimate:** 2–3 hours.

**Diff estimate:** 950 changed lines including the initial lockfile.

**PR increment:** `core-filesystem-read`

**Commands and expected results:**

- `cargo test -p resourcefs-core --test path_reference_contract --test version_tag_contract` → every approved path-table row agrees with the hand-authored oracle; fixed SHA-256 vectors match; equal content has equal tags and changed content differs.
- Apply the C3 mutation, then run `cargo test -p resourcefs-core --test path_reference_contract rejects_parent_traversal` → red because `../secret` is accepted; restore, rerun → green with `permission_denied`.
- Apply the C5 mutation, then run `cargo test -p resourcefs-core --test version_tag_contract ignores_path_and_metadata` → red because equal bytes no longer yield equal tags; restore, rerun → green.
- Run the stress tests in the two contract binaries → the 4 KiB parse is ≤5 ms and 48 KiB hash is ≤10 ms, while outcomes still match their independent oracles.

## Slice 2: Implement the bounded filesystem Source Adapter

**Claim IDs:** C4, C9

**Expected behavior:** A `FilesystemSource` constructed from one valid canonical root reads contained UTF-8 through core's Source Adapter interface, permits contained symlinks, rejects lexical/canonical escapes, maps missing/directory/binary cases to their approved categories, and fails rather than returning partial data beyond any hard ceiling.

**Oracle:** Temporary fixtures declare exact bytes and inside/outside topology before adapter construction; expected categories and boundary counts are fixed in the test rather than computed through production resolution/limit helpers.

**Stress fixture:** One temporary tree contains a Unicode filename with spaces, an internal symlink, an escaping symlink, an empty file, invalid UTF-8, a directory, a missing name, exact-limit files, and byte/line/column fixtures exceeding each ceiling by one unit. Windows skips only symlink creation when the host denies that OS capability; all non-symlink containment rows remain mandatory.

**Regression fence:** `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`, created in this slice.

**Named mutation:** C4 — in `crates/resourcefs-sources/src/filesystem.rs`, read the joined lexical path instead of the validated canonical target; `rejects_symlink_escape` must turn red by exposing the outside fixture. C9 — replace bounded `take(MAX_BYTES + 1)` reading and line/column checks with `tokio::fs::read`; `enforces_hard_read_limits` must turn red by accepting a one-over fixture.

**Complexity/production scale:** Canonical resolution is O(path components) plus OS lookups. Reading is O(min(file bytes, 48 KiB + 1)); UTF-8, line, column, and SHA-256 passes are each O(n) over at most 48 KiB. Total resident content buffers must remain ≤2 × 48 KiB plus fixed metadata; maximum accepted direct-adapter wall cost is 100 ms for a 48 KiB local temporary file, chosen to tolerate debug/CI filesystem variance while rejecting accidental unbounded work.

**Wall budget/phase:** Always-on read phase ≤100 ms per 48 KiB local file, including canonicalization, bounded read, UTF-8 validation, dimensions, and hashing. Root canonicalization is a one-off constructor phase; no wall budget — it runs once per configured root at process startup.

**Files:** `Cargo.toml`; `Cargo.lock`; `crates/resourcefs-sources/Cargo.toml`; `crates/resourcefs-sources/src/lib.rs`; `crates/resourcefs-sources/src/filesystem.rs`; `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs`.

**Estimate:** 3–4 hours.

**Diff estimate:** 850 changed lines including dependency lockfile growth.

**PR increment:** `core-filesystem-read`

**Commands and expected results:**

- `cargo test -p resourcefs-sources --test filesystem_adapter_contract` → direct adapter success content/tags equal fixture literals; missing=`not_found`; directory/binary=`unsupported_projection`; lexical/canonical escape=`permission_denied`; exact ceilings pass and each one-over fixture=`limit_exceeded` without partial content.
- Apply the C4 mutation, then run `cargo test -p resourcefs-sources --test filesystem_adapter_contract rejects_symlink_escape` → red because outside bytes are returned; restore, rerun → green with `permission_denied` and no outside bytes.
- Apply the C9 mutation, then run `cargo test -p resourcefs-sources --test filesystem_adapter_contract enforces_hard_read_limits` → red because at least one one-over fixture succeeds; restore, rerun → green with `limit_exceeded`.
- Run `cargo test -p resourcefs-sources --test filesystem_adapter_contract reads_maximum_sized_file_within_budget` → exact 48 KiB content and tag agree with the fixture oracle in ≤100 ms and bounded allocation remains at the planned maximum.

## Slice 3: Ship the clean stdio MCP read path

**Claim IDs:** C1, C2, C6, C7, C8, C10

**Expected behavior:** The Cargo-installable `resourcefs` binary accepts exactly one valid named launch root, serves clean stdio MCP, explicitly advertises/defaults to `2026-07-28`, negotiates `2025-11-25`, advertises tools only, lists only `rfs_read` with object-root strict schemas, and renders every success/tool error as non-empty text plus equivalent object structured content.

**Oracle:** A raw newline-delimited JSON-RPC process driver hand-authors initialize/list/call requests and expected wire objects without using `rmcp`; fixture bytes and the approved error table determine results. Published `rmcp` source oracle remains the independent comparison for C1. The accepted `DESIGN.md:269-279` dependency graph independently determines C6.

**Stress fixture:** Run the compiled process separately at both required revisions against a root containing non-empty, empty, Unicode/space, directory, missing, invalid UTF-8, escaping symlink, and over-limit resources; also send malformed tool arguments and launch every invalid CLI-root matrix row. Capture stdout and stderr independently and require every stdout line to parse as JSON-RPC.

**Regression fence:** `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs` and `crates/resourcefs-mcp/tests/architecture_contract.rs`, created in this slice.

**Named mutation:** C1 — replace explicit `V_2026_07_28` server info with `ProtocolVersion::LATEST`; `negotiates_required_revisions` turns red. C2 — accept zero roots or skip primary-name equality; `rejects_invalid_root_configuration` turns red. C6 — add `rmcp` to core; `enforces_dependency_direction` turns red. C7 — enable resources or omit output schema; `lists_object_root_read_tool_only` turns red. C8 — omit structured error content or emit empty TextContent for an empty file; `renders_complete_success_and_errors` turns red. C10 — register no-op `rfs_search`; `lists_object_root_read_tool_only` turns red on the extra tool.

**Complexity/production scale:** CLI/root setup has O(number of arguments + path components) work with exactly one root. Tool rendering is O(n) over at most 48 KiB and performs one structured serialization plus one TextContent construction; maximum transient MCP-render allocations are ≤3 × 48 KiB plus JSON overhead. The raw process fence accepts ≤250 ms per 48 KiB local `rfs_read` after initialization, a generous bound over Slice 2's 100 ms adapter maximum plus debug JSON/protocol overhead.

**Wall budget/phase:** One-off phases: CLI parse, root construction, MCP initialization; no wall budget — each runs once per process/connection. Always-on phase: initialized `rfs_read` request through response ≤250 ms for a 48 KiB local fixture. No background phase is introduced.

**Files:** `Cargo.toml`; `Cargo.lock`; `crates/resourcefs-mcp/Cargo.toml`; `crates/resourcefs-mcp/src/lib.rs`; `crates/resourcefs-mcp/src/cli.rs`; `crates/resourcefs-mcp/src/render.rs`; `crates/resourcefs-mcp/src/server.rs`; `crates/resourcefs-mcp/src/main.rs`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`; `crates/resourcefs-mcp/tests/architecture_contract.rs`.

**Estimate:** 5–7 hours.

**Diff estimate:** 2,750 changed lines including `rmcp`/CLI dependency lockfile growth and raw JSON-RPC fixtures.

**PR increment:** `stdio-mcp-read`

**Commands and expected results:**

- `python3 .rfs-87cv/probe.py && python3 .rfs-87cv/oracle.py` → C1 remains PASS: both revisions supported/negotiated, `LATEST=2025-11-25` learning unchanged, object schemas and split content representation available.
- `cargo test -p resourcefs-mcp --test architecture_contract` → metadata has exactly the accepted forward dependency edges and no `rmcp`/`clap` dependency from core or sources.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract` → both revisions initialize; only tools capability and `rfs_read` appear; schemas root at object; success headers/text/structured fields agree with fixture bytes; every approved error category and invalid CLI row matches; stdout contains only JSON-RPC.
- Apply each C1/C2/C6/C7/C8/C10 named mutation separately and run its named focused test → each mutation turns that fence red for the exact expected mismatch; restore after each and rerun → green.
- `cargo test --workspace` → both PR increments remain independently integrated and every permanent fence is green.
- `cargo run -p resourcefs-mcp --bin resourcefs -- serve --root workspace=. --primary-root workspace` under the raw MCP smoke driver → the actual binary initializes, lists `rfs_read`, reads a fixture, and exits cleanly; no non-protocol stdout.
- Process stress fixture at 48 KiB → exact content and structured fields return within 250 ms after initialization; one-over content returns bounded `limit_exceeded` with no partial payload.

## Tracker taxonomy

The plan implements only the approved `rfs-87cv` behavior. Intended future work remains owned by verified issues: complete roots/references/containment `rfs-cgbq`; recovery/per-call limits/Path Sessions `rfs-34pz`; search/glob `rfs-vl0u`; mutation `rfs-73dz`; Structural Summaries `rfs-m739`; Server Profiles and remaining CLI composition `rfs-r9m6`; Resource mirroring `rfs-ww6w`; full Kiro acceptance `rfs-5os7`. Permanent non-goals remain OMP coupling, shell interpretation, telemetry, protocol logging on stdout, and placeholder surfaces, for the rationales in approved `design.md`.

No new deferral or tracker issue is introduced by this plan.

## Self-review

- [x] Every design claim C1–C10 is assigned exactly once; every PENDING falsifier is assigned to its implementing slice.
- [x] Every slice contains all thirteen mandatory fields and every conditional field has an explicit N/A rationale where applicable.
- [x] Every claim's permanent fence and mechanical named mutation are created in the same slice.
- [x] Every new loop records asymptotic cost, production-shaped size, memory/work bound, and an explicit maximum accepted cost; every always-on phase has a wall budget.
- [x] The 5,233-line projected total exceeds the review-size gate and is partitioned into two independently mergeable increments.
- [x] Every future-work phrase cites a verified Rivets ID; permanent non-goals carry settled rationales.
- [x] No slice is declared complete; `checkpointed-build` exclusively judges completion.
