## Agent skills

### Issue tracker

Issues are tracked in the repository-local Rivets JSONL store through the `rivets` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Triage uses the five default canonical label strings. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repository with `CONTEXT.md` at the root and ADRs under `docs/adr/`. See `docs/agents/domain.md`.

## Live smoke tests

Every network-backed Source Adapter, and every primary tool path that crosses one, ships with a live smoke: an `#[ignore]` test named `live_*`, gated on an environment variable (`RFS_LIVE=1`; additionally `GITHUB_TOKEN` for GitHub rows) that skips cleanly when the gate is absent and drives the real adapter — or the real `rfs` binary over stdio — against a real public upstream, read-only. `scripts/live-smoke.sh` runs them all and sources the GitHub token from `gh auth token`; `cargo live` does the same once the environment is set. The deterministic fake-upstream contracts remain the permanent suite, and CI never runs the live rows.

A live row targets what a fake cannot be trusted to imitate — pagination cursors, endpoint default page sizes, parent URLs, validators, redirect chains, real markup — and asserts shapes and invariants, never upstream counts, because upstream state moves. Record each run in the owning ticket's `evidence.md` (rfs-45ww's L1–L6 rows are the model). The reason this section exists: the rfs-45ww review found an adapter that could not paginate a real repository while its fake stayed green, because the fixture drifted from the very evidence that had been gathered for it. A live test is sometimes the only proof that something works.

Existing rows: `crates/resourcefs-sources/tests/github_live_smoke.rs` (adapter), `crates/resourcefs-sources/tests/https_live_smoke.rs` (adapter), `crates/resourcefs-mcp/tests/stdio_live_smoke.rs` (profile → `rfs check --probe` → `rfs serve` → tools over stdio).

## Design Principles

Make illegal states unrepresentable — use the type system to prevent bugs at compile time rather than catching them at runtime. ResourceFS already leans hard on this; keep it that way.

**Newtype every domain identifier and validate at construction.** Never pass a raw `String`, `PathBuf`, or `u64` where the domain has a type. Construct through the validating constructor so an invalid value cannot exist downstream:

- `WorkspaceRootId` — validated ASCII, or the `client-<sha256>` form for client-declared roots.
- `WorkspacePath` — rejects `.`, `..`, absolute, and (except via `WorkspacePath::root()`) empty paths, so a contained relative path is the only representable value.
- `VersionTag` — `sha256:` plus 64 lowercase hex; it names exact content, **never** an mtime or counter.
- `ArtifactId`, `SessionToken`, `MutationSourceKey`, `LineNumber` (`NonZeroU64`), `OriginalLineRange` (validated ascending).

**Parse once into a typed sum; match on variants, never re-parse strings.** `PathReference::parse` and `HashlinePatch::parse` are the boundaries. After them, code matches on `ResourceAddress` (`Workspace`/`Artifact`/`Catalog`), `WorkspaceAddress`, `PutTarget`, `MutationState`, `SourceMutation`, `MutationOperation`, and `PatchOperation`. When a new case appears, add a variant rather than a bool or a flag, and keep the sum exhaustive so the compiler finds every site (see the `ResourceAddress` match obligations in `resource.rs::validate_canonical_identity`).

**Use `Option` for absent values, not sentinels.** `WorkspaceRootSet::primary` is `Option`, not a magic root; `PathReference`'s `selector_candidate`/`selector_error` are Options carrying "both interpretations." Never encode "absent" as `""`, `0`, or a catch-all enum arm. Cautionary tale from this repo: `SourceResource::mutable` was hardwired `false` — a constant standing in for real state — until rfs-73dz made it grant-and-link derived. A field that always holds one value is a lie the type carries; compute it or drop it.

**Errors are values with a stable category, never defaults.** There is exactly one operational error type: `ResourceError { category, message }` with private fields exposed through `.category()` / `.message()`. Do not add public fields, and do not invent a second error enum per module (ResourceFS does not use `thiserror`; `ResourceError` is hand-rolled and implements `std::error::Error`).

- `ErrorCategory` is the Behavior Contract's stable vocabulary (`invalid_reference`, `not_found`, `permission_denied`, `version_conflict`, `invalid_patch`, `limit_exceeded`, `source_unavailable`, `unsupported_projection`, `unsupported_mutation`, `ambiguous_reference`, `invalid_pattern`, `cancelled`). Adding or renaming a category is a contract change — update `DESIGN.md`'s category list in the same commit, as rfs-73dz did for `invalid_patch` / `unsupported_mutation`.
- Operational failures are `ResourceError` (rendered as MCP tool-result errors); protocol/schema failures stay MCP errors. Map at the boundary in `resourcefs-mcp` — never leak `rmcp` types into core.
- Never `unwrap_or_default()` / `unwrap_or("")` / `unwrap_or(0)` to paper over a parse or I/O failure. Reject invalid input with the right category: `TextLimits::new` rejects zero or above-ceiling values rather than clamping, and lower-only `limits` never silently raise a hard ceiling. When a bounded operation cannot spill losslessly, fail with `limit_exceeded` plus narrowing guidance — never drop bytes, and never label discarded content as a Recovery Reference.

**Silent-failure discipline.** Log before returning `None` when `None` means something went wrong (not merely "not found"). Return `Err` for malformed input, not an empty `Vec` dressed as `Ok`. Keep "missing" and "corrupt" distinct. Audit `.ok()`, `filter_map(Result::ok)`, and `let _ =` before using them. The `rust-best-practices` skill enumerates these (Rules 35/36/41); load it when editing `.rs` files.

**`unsafe` is confined, not forbidden.** Unlike some sibling crates, ResourceFS needs `unsafe` for capability FFI — PCRE2 (`sources/src/pattern.rs`), post-open path re-resolution and the Windows rename primitive (`sources/src/filesystem*.rs`), and cap-std. Every `unsafe` block carries a `// SAFETY:` justification and lives in the Source Adapter layer. `resourcefs-core` holds no platform FFI: `tests/architecture_contract.rs` fails the build if `cap-std`, `rmcp`, `regex`, `pcre2-sys`, `ignore`, `globset`, `rustix`, or `windows-sys` reach core. Respect the fence; never reach for `unsafe` in core.

**Version Tags and seen-region snapshots are authority, not convenience.** A mutation compares the authoritative `VersionTag` immediately before commit and serializes per canonical Resource; an edit targets only lines a read displayed in the same Path Session, never a guessed coordinate. This is the Behavior Contract (`DESIGN.md`), not a suggestion — mutation adapters revalidate state at commit rather than trusting engine-side snapshots.
