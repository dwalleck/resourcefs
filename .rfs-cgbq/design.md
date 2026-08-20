# Design: Safe multi-root Workspace References

## Status and approved contract

This design implements the requester-approved `.rfs-cgbq/spec.md` without expanding selector execution, session resources, mutation, profiles, or MCP Resource mirroring. It replaces the founding single-root assumption with one connection-owned workspace authority that can move atomically between launch roots, client roots, refresh suspension, and fail-closed disabled state.

Accepted decisions carried unchanged:

- A non-empty valid client Roots result replaces launch authority; a valid empty result deliberately restores launch authority. Roots are never unioned.
- Each client-root ID is `client-` plus the full lowercase SHA-256 digest of its normalized canonical local `file://` URI. Display names select a primary but never determine identity.
- Distinct nested roots are valid. An absolute or `file://` input contained by more than one root returns `ambiguous_reference`; canonical `rfs://workspace/<root>/<path>` inputs remain unambiguous.
- Exact duplicate canonical roots are rejected.
- Invalid, failed, or more-than-5-second client-root acquisition disables all workspace authority until a later wholly valid acquisition.
- Percent decoding is strict and single-pass. Existing literal paths win over selector splitting. This change parses and preserves supported selector spellings but does not execute them.
- Workspace references are capped at 64 KiB and active roots at 256.
- Backing `file://` metadata is policy-only in this change. The CLI has no new visibility flag before rfs-r9m6 supplies the accepted Server Profile field.

## Selected shape

`resourcefs-core` owns syntax and source-neutral authority values. `resourcefs-sources` owns all ambient filesystem facts, capability handles, canonicalization, resolution, and the root-generation state machine. `resourcefs-mcp` owns rmcp conversion, connection lifecycle, CLI composition, and result rendering.

The deep public filesystem interface remains `FilesystemSource`: callers provide one parsed `PathReference`, call `read`, and receive a bounded `ReadResource` or a stable `ResourceError`. Root acquisition is a second, narrow control plane on the same source. There is no second resolver, no MCP-side path normalization, and no public adapter API for filesystem internals.

This shape preserves the accepted dependency direction:

```text
resourcefs-mcp
  └── resourcefs-sources
        └── resourcefs-core
```

Rejected alternatives:

1. **Put root lifecycle and path containment in MCP.** Rejected because direct adapter callers could bypass final generation checks, and future non-MCP callers would need a second implementation.
2. **Put `cap_std::fs::Dir`, canonical paths, or rmcp Root values in core.** Rejected because core would cease to be source-neutral and would acquire ambient-I/O or protocol dependencies.
3. **Add a generic registry/plugin framework now.** Rejected because the change has one compiled source and one verified lifecycle seam. A registry would expose policy and invalidation ordering before a second source needs it.
4. **Canonicalize an ambient path and then prefix-check it.** Rejected because it creates a check/use race. The selected design opens through a capability directory and revalidates authority after I/O.

## Core interface

### Reference syntax

`crates/resourcefs-core/src/reference.rs` becomes a pure parser. The existing `RootName` is cleanly renamed to `WorkspaceRootId`, and every caller migrates in the same change.

```rust
pub const MAX_PATH_REFERENCE_BYTES: usize = 64 * 1024;

pub struct WorkspaceRootId(String);

pub struct PathReference {
    requested: String,
    literal: WorkspaceAddress,
    selector_candidate: Option<SelectedWorkspaceAddress>,
}

pub enum WorkspaceAddress {
    Relative(WorkspacePath),
    Absolute(PathBuf),
    FileUri(url::Url),
    Canonical {
        root: WorkspaceRootId,
        path: WorkspacePath,
    },
}

pub struct SelectedWorkspaceAddress {
    base: WorkspaceAddress,
    selector: ProjectionSelector,
}

pub struct WorkspacePath(PathBuf);
pub struct ProjectionSelector(String);
```

Public construction and inspection are deliberately small:

```rust
impl PathReference {
    pub fn parse(input: impl Into<String>) -> Result<Self, ResourceError>;
    pub fn canonical(root: WorkspaceRootId, path: WorkspacePath) -> Self;
    pub fn requested(&self) -> &str;
    pub fn literal(&self) -> &WorkspaceAddress;
    pub fn selector_candidate(&self) -> Option<&SelectedWorkspaceAddress>;
}
```

`PathReference::parse` performs syntax work only. It never accesses the filesystem and never chooses a root. Resolution occurs in `FilesystemSource`, where literal existence and the current root view are available.

Classification order is fixed:

1. Reject input longer than 64 KiB, empty input, NUL, malformed percent escapes, invalid UTF-8 after percent decoding, encoded `/` or `\\`, and decoded `.` or `..` components.
2. Recognize fully-qualified Windows drive, UNC, and verbatim paths before URI parsing. Drive-relative (`C:foo`) and backslash-root-relative (`\foo`) forms are rejected everywhere. Slash-rooted `/foo` is an absolute path on POSIX and a rejected root-relative form on Windows.
3. Recognize `rfs://workspace/<root>/<path>`.
4. Recognize local `file://` URIs. POSIX permits empty host or `localhost`; Windows additionally permits local drive URIs and UNC `file://server/share/...`. Credentials, ports, query, fragment, malformed encoding, and non-local POSIX hosts are rejected.
5. Recognize a native POSIX `/`-rooted path as `WorkspaceAddress::Absolute`.
6. Reject other URI schemes for this source seam.
7. Otherwise parse a relative workspace path.
8. Preserve a selector candidate only when an unencoded trailing colon begins a syntactically valid line projection: `:N`, `:N-`, `:N-M`, `:N+COUNT`, comma-separated combinations, `:raw`, or `:raw:<ranges>`. Integers are one-based and non-zero; closed ranges cannot descend. The full input remains the literal candidate.

Strict single-pass decoding retains delimiter intent: `notes%3A2` is one literal path, while `notes:2` is a literal candidate plus selector candidate. `%253A` decodes to the literal characters `%3A`, not a second delimiter. Raw `?` and `#` are rejected for workspace inputs; literal filename characters use `%3F` and `%23`.

`WorkspacePath` stores validated platform components. Rendering percent-encodes literal `%`, `:`, `?`, `#`, `/`-inside-a-component, and non-UTF-8 is not representable. Canonical references therefore round-trip through the same parser without ambiguity.

### Root values

`reference.rs` also defines source-neutral immutable root values:

```rust
pub struct WorkspaceRoot {
    id: WorkspaceRootId,
    canonical_uri: String,
    selector_name: Option<String>,
}

pub struct WorkspaceRootSet {
    roots: Vec<WorkspaceRoot>,
    primary: Option<WorkspaceRootId>,
}
```

Constructors accept already-canonical normalized URIs, reject empty or over-256 sets, duplicate IDs, duplicate canonical URIs, and client-ID collisions. `WorkspaceRootSet::equivalent_to` compares the sorted `(id, canonical_uri)` set and effective primary ID; list order and display-only changes are ignored. A display-name change that changes the effective primary is not equivalent.

Primary derivation is deterministic:

- one active root: that root is primary;
- multiple roots and exactly one selector match: that root is primary;
- otherwise: no primary. Relative inputs then return `ambiguous_reference`, while canonical inputs remain usable.

Launch selectors match explicit launch IDs. Client selectors match optional client display names exactly. Missing names and names that are empty or contain NUL are ignored for selection but do not invalidate identity; duplicate usable names simply cannot produce a unique match. Other Unicode names are valid selector labels. The two concepts never rename one another.

`WorkspaceRootId::from_client_uri` hashes the normalized canonical URI with the existing `sha2` dependency and emits `client-<64 lowercase hex digits>`. The canonical URI is retained privately for equality and collision checks but is never rendered as workspace identity.

### Source-neutral result metadata

`crates/resourcefs-core/src/resource.rs` adds an optional `backing_file_uri: Option<String>` to `ReadResource`. `ReadResource::text` keeps it absent. A source may set it only after policy authorization. This is metadata; `canonical_reference` remains private `rfs://workspace/...` identity.

No new error category is needed. Stable mappings are:

| Condition | Category |
|---|---|
| malformed form/escape, encoded separator, decoded `.`, or ambiguous delimiter spelling | `invalid_reference` |
| reference/root count over accepted cap | `limit_exceeded` |
| unknown or removed canonical root, or an in-flight result whose root was removed | `invalid_reference` |
| contained missing literal | `not_found` |
| lexical `..` traversal, outside-root resolution, symlink/reparse escape, or denied capability open | `permission_denied` |
| no unique primary for a relative input, or one absolute/`file://` input resolves under multiple distinct roots | `ambiguous_reference` |
| root acquisition currently refreshing or disabled | `source_unavailable` |
| valid preserved selector reaches current `rfs_read` | `unsupported_projection` |

Outside-path and capability errors use fixed operation text and never include the rejected absolute path.

## Filesystem source and authority state

### Launch and client inputs

`crates/resourcefs-sources/src/filesystem.rs` exposes configuration values that contain paths but no rmcp types:

```rust
pub struct LaunchRoot {
    pub id: WorkspaceRootId,
    pub path: PathBuf,
}

pub enum LaunchRootSource {
    Cli(Vec<LaunchRoot>),
    Profile(Vec<LaunchRoot>),
}

pub struct ClientRoot {
    pub uri: String,
    pub name: Option<String>,
}

pub enum BackingPathVisibility {
    Hidden,
    Visible,
}
```

`FilesystemSource::new` accepts exactly one `LaunchRootSource`, an optional primary selector, and visibility policy. It validates every launch member and builds the launch view before protocol traffic. The enum makes a mixed CLI/profile authority set unrepresentable. This change constructs only `LaunchRootSource::Cli`; `Profile` is the policy seam reserved for rfs-r9m6.
For every valid root, construction opens one `cap_std::fs::Dir` with `ambient_authority`, retrieves the final path from that directory handle, verifies it is an existing directory, and derives the normalized private canonical URI from that handle-final path. It does not canonicalize one ambient pathname and then open another. Ambient authority ends at this root-handle acquisition. All later lookup and read operations are relative to capability handles and run in `tokio::task::spawn_blocking`.

### Connection-owned state machine

`FilesystemSource` is cloneable around one `Arc<Inner>` and therefore one connection-owned authority. `Inner` stores a short-held `parking_lot::RwLock<AuthorityControl>`. Reads clone an immutable view under a read lock and release it before I/O. Refresh transitions take the write lock only to advance an epoch or publish a completed state; no lock is held across filesystem or protocol I/O.

```rust
struct AuthorityControl {
    epoch: u64,
    state: AuthorityState,
    identity_history: HashMap<WorkspaceRootId, String>,
}

enum AuthorityState {
    Active(Arc<WorkspaceView>),
    Refreshing {
        epoch: u64,
        baseline: Option<Arc<WorkspaceView>>,
    },
    Disabled {
        epoch: u64,
        generation: u64,
    },
}

struct WorkspaceView {
    generation: u64,
    roots: WorkspaceRootSet,
    filesystems: HashMap<WorkspaceRootId, Arc<FilesystemRoot>>,
}
```

Keeping epoch, published state, and identity history under one lock makes the commit indivisible: a stale completion cannot update collision history and cannot install after a newer refresh begins. A later different canonical URI with an ID already observed during the connection disables the whole candidate set instead of aliasing a stale canonical reference.

The lifecycle API enforces suspend-before-fetch:

```rust
impl FilesystemSource {
    pub fn begin_client_root_refresh(&self) -> ClientRootRefresh;
}

impl ClientRootRefresh {
    pub fn start_acquisition(self) -> ClientRootAcquisition;
}

impl ClientRootAcquisition {
    pub fn remaining(&self) -> std::time::Duration;
    pub async fn complete(
        self,
        result: Result<Vec<ClientRoot>, ResourceError>,
    ) -> RootRefreshOutcome;
}

pub enum RootRefreshOutcome {
    Installed(RootChange),
    Unchanged { generation: u64 },
    Disabled(RootChange),
    Superseded,
}

pub struct RootChange {
    pub previous_generation: u64,
    pub generation: u64,
    pub removed: Vec<WorkspaceRootId>,
    pub active: Vec<WorkspaceRootId>,
    pub primary: Option<WorkspaceRootId>,
}
```

`begin_client_root_refresh` synchronously publishes `Refreshing` before returning a token. Every workspace read started or completed while refreshing fails `source_unavailable`. Dropping a token leaves authority suspended, which is fail-closed.

When `begin_client_root_refresh` re-enters an existing `Refreshing` state, it carries forward that state's original `baseline` rather than replacing it with `None`. A token begun from `Active` therefore keeps the last active view across any number of overlapping notifications; a token begun from `Disabled` has no reusable baseline. This is the entire generation-reuse rule—no separate boolean state is needed.

`complete` validates and canonicalizes the entire candidate set off-thread before one atomic installation. Empty client Roots selects the already-validated launch set. Non-empty valid client Roots replace it. Retained canonical URIs reuse their existing capability handles; only genuinely new roots acquire ambient handles. Any invalid member, timeout/error supplied by MCP, duplicate, collision, or open failure atomically stores `Disabled`; no valid subset, old handle, or launch authority remains active.

`ClientRootRefresh::start_acquisition` stamps an internal deadline at `now + 5 seconds` and arms a source-owned Tokio watchdog for that epoch; callers cannot widen it. If acquisition is dropped, request enqueue stalls, response wait stalls, or validation blocks, the watchdog publishes `Disabled` at the deadline only when its epoch is still current. `ClientRootAcquisition::remaining` supplies only the current remainder to MCP. Candidate validation runs in a `spawn_blocking` task that can produce data but cannot publish state. `complete` cancels the watchdog only after an early install/disable decision, waits no later than the same deadline, ignores any later blocking result, and checks epoch/deadline immediately before commit. Thus one deadline bounds protocol wait plus validation across as many as 256 roots without allowing late installation.

Concurrent refreshes are ordered by epoch. Only the newest token may install or disable; a late older result returns `Superseded` without changing state or identity history. Two overlapping equivalent refreshes both inherit the same active baseline; the newest valid completion restores that exact `Arc<WorkspaceView>` and returns `Unchanged` with the original generation. Any effective membership, identity, or primary change installs a fresh monotonically increasing generation and returns a `RootChange` naming every removed and active ID. Failure returns a disabling `RootChange` that removes all prior roots. After a failed acquisition, a later valid set always gets a fresh generation even when its members equal the last valid set, because the failure invalidated prior authority. This outcome is the event seam consumed later by rfs-34pz/rfs-73dz; no snapshot state is implemented here.

### Resolution and containment

A read captures one `Arc<WorkspaceView>`, resolves and opens through its capability directories in one `spawn_blocking` closure, then re-reads `AuthorityState` immediately before returning. Delivery succeeds only when the current state is `Active` and contains the same root ID mapped to the same canonical URI. Therefore:

- a result whose root is absent from a newly active valid set fails `invalid_reference`;
- a result completing while authority is refreshing or disabled fails `source_unavailable`;
- a retained root can finish after a valid change if ID and canonical URI remain active;
- a hash collision cannot make an old reference select new authority;
- session scratch/artifact state is untouched because it is not stored here.

Relative inputs resolve only against the effective primary. Canonical inputs resolve only against the named root. For an absolute or `file://` input, ambient canonicalization may be used only to map the caller's existing spelling to candidate root-relative paths; content is still opened through the selected capability, never through the ambient result. Zero root matches is `permission_denied`, and multiple distinct matches is `ambiguous_reference`.

Each candidate is opened directly through `cap_std::fs::Dir`; contained symlinks/reparse points are followed, while resolution escaping the capability returns permission denial. The opened `cap_std::fs::File` is the only handle used for metadata and content—there is no ambient reopen.

After the read, the source obtains the final path from that same open handle: `/proc/self/fd/<fd>` on Linux, `rustix::fs::getpath` (`F_GETPATH`) on macOS, and `GetFinalPathNameByHandleW` on Windows. Failure to obtain a final handle path fails closed; it never falls back to lexical containment. The final path is normalized only for comparison, checked component-wise beneath the selected root, compared with every active root for absolute/`file://` ambiguity, and then relativized to construct private `rfs://workspace/<id>/<path>` identity. This avoids a `canonicalize`-then-ambient-open identity race while retaining cap-std's containment during the actual open.

An alias below a parent root that opens a target inside a nested declared root is therefore ambiguous for absolute/`file://` input. Linux rejects a handle path carrying the kernel's deleted-file marker; macOS consumes the NUL-terminated `F_GETPATH` result; Windows comparison removes the handle-final verbatim prefix and normalizes separators and drive-letter case for comparison only. Rendered private paths preserve canonical component spelling. The implementation must not delegate classification to `PathIsRelative`, which the recorded probe disproved for drive-relative and root-relative forms.

Literal precedence occurs inside this resolver:

1. Resolve/check the complete literal candidate under current authority.
2. If it exists, use it even when its name ends like a selector.
3. If it is definitely absent and a preserved selector candidate exists, return `unsupported_projection` for this issue.
4. Propagate permission, ambiguity, malformed, and source errors; never convert them into selector fallback.
5. Without a selector candidate, return `not_found` for a contained missing literal.

Reads retain the existing UTF-8-only and 48 KiB/3,000-line/512-column all-or-error contract. The blocking reader consumes at most `MAX_TEXT_BYTES + 1` bytes from the already-open file handle before UTF-8/line/column validation, so a concurrent grow cannot turn a metadata check into an unbounded allocation. Over-limit content returns only `limit_exceeded`; no partial read or spill behavior is added.

### Backing-path visibility

`BackingPathVisibility::Hidden` is the default supplied by every current CLI path. In this state neither `ReadResource` nor renderer contains a backing URI. `Visible` constructs a normalized local `file://` URI only from the registered canonical root plus capability-resolved relative path. Direct source and renderer tests exercise both states. No environment variable, ad hoc CLI flag, or implicit debug mode can enable it.

## MCP lifecycle and CLI composition

### CLI

`crates/resourcefs-mcp/src/cli.rs` changes `--root NAME=PATH` from exactly one occurrence to one-or-more occurrences. It keeps one root source: all arguments in the invocation become `LaunchRootSource::Cli`. `--primary-root <selector>` becomes optional; one root selects itself, while multiple roots without exactly one match leave relative references disabled rather than failing startup. Invalid launch sets fail before `server::serve`, write diagnostics only to stderr, and leave stdout clean.

### Server

`crates/resourcefs-mcp/src/server.rs` holds `Arc<FilesystemSource>` rather than a path-aware field split between source and primary root. `rfs_read` parses once in core and delegates once to the source. The existing manual malformed-argument pre-validation remains intact so schema errors stay MCP `invalid_params` rather than tool-result envelopes.

`ServerHandler::on_initialized` examines the negotiated client capability through `NotificationContext.peer`. If Roots is absent, launch authority remains active. If Roots is present, it synchronously calls `begin_client_root_refresh` and parks the newest token in an MCP-owned refresh coordinator. `ServerHandler::on_roots_list_changed` performs the same suspend-and-replace step for every received notification. Neither handler sends `roots/list`.

This deferral is required by MCP `2026-07-28` SEP-2260. rmcp 3.1.3 rejects `ListRootsRequest` from notification scope or a spawned task; its `ORIGINATING_REQUEST` association exists only while an incoming request handler is executing. The server therefore calls an inline `ensure_workspace_authority` helper at the start of both `list_tools` and `call_tool`. A Tokio mutex serializes helpers across concurrent client requests. Under that mutex, the helper takes the newest parked token, gives that token one absolute five-second acquisition deadline, sends `ListRootsRequest`, completes validation within the remaining time, and repeats with a fresh per-token deadline only if a newer notification superseded it. It never spawns the request. `list_tools` continues to return the stable tool catalog if acquisition disables authority; `call_tool` continues and lets the source return the stable workspace error.

The MCP-only helper converts the parked token with `start_acquisition`, supplies `PeerRequestOptions::with_timeout(acquisition.remaining())` to `Peer::send_cancellable_request` for bounded response wait and protocol cancellation, then passes the result to that same acquisition's `complete`; the source watchdog independently bounds authority even if request enqueue itself stalls. The helper converts rmcp `Root { uri, name }` values into source-owned `ClientRoot` values, passes any timeout/protocol/deserialization error as a failed completion, and never exposes rmcp Roots types below this crate. The explicit option is required because rmcp's generated `list_roots()` uses `PeerRequestOptions::no_options()`. Since rmcp marks Roots deprecated for SEP-2577 while MCP `2026-07-28` still exposes them, `#[allow(deprecated)]` is scoped to this helper and the two notification methods only; no crate-wide suppression is permitted.

If a notification arrives during acquisition, its newer source epoch suspends authority and replaces the parked token. The older completion returns `Superseded`; the serialized helper consumes the newer token before serving from workspace authority. If a notification arrives just after the helper observes no pending token, any concurrent read sees `Refreshing` and fails closed, and the next relevant request performs acquisition. Idle connections may remain suspended without sending an unassociated request; the five-second clock starts when request-associated acquisition starts.

Disconnect drops the server-owned source and refresh coordinator. A blocking validation that outlives timeout holds no commit token and cannot install into this or another connection. Path Session-wide scratch/artifact teardown remains deferred to rfs-34pz; this issue creates no such state.

### Rendering

`crates/resourcefs-mcp/src/render.rs` adds optional `backingFileUri` to `ReadToolOutput` and populates it only from `ReadResource`. `canonicalReference` remains `rfs://workspace/...`. TextContent always contains requested and canonical references plus bounded content as before; visibility never changes the canonical reference.

## File and dependency map

| File | Change |
|---|---|
| `Cargo.toml` | Add workspace dependencies `cap-std`, `parking_lot`, `rustix = { features = [\"fs\"] }`, `url`, and `windows-sys = { features = [\"Win32_Foundation\", \"Win32_Storage_FileSystem\"] }`; retain exactly three workspace members. |
| `crates/resourcefs-core/Cargo.toml` | Add `url`; keep parser and value logic synchronous. |
| `crates/resourcefs-core/src/reference.rs` | Replace primary-bound parser with syntax-only forms, strict decoding, Windows/file/canonical classification, selector preservation, root IDs/sets, limits, and canonical rendering. |
| `crates/resourcefs-core/src/resource.rs` | Add optional backing-file URI metadata and construction methods. |
| `crates/resourcefs-core/src/lib.rs` | Re-export the clean-cutover reference/root types. |
| `crates/resourcefs-sources/Cargo.toml` | Add `cap-std`, `parking_lot`, `url`, macOS-targeted `rustix` with `fs`, and Windows-targeted `windows-sys` with `Win32_Foundation`/`Win32_Storage_FileSystem`; add non-default `test-support` for the deterministic compiled-process delivery gate. |
| `crates/resourcefs-sources/src/filesystem.rs` | Replace one-root adapter with launch/client root validation, capability handles, refresh state, resolution, overlap checks, literal precedence, final generation validation, and policy metadata; compile the gate only under `test-support`. |
| `crates/resourcefs-sources/src/lib.rs` | Re-export filesystem configuration and refresh types required by MCP/tests. |
| `crates/resourcefs-mcp/Cargo.toml` | Add `parking_lot` for the parked-token coordinator and forward non-default `test-support` only for the stdio generation test. |
| `crates/resourcefs-mcp/src/cli.rs` | Compose repeated roots, optional primary selector, and launch-source validation. |
| `crates/resourcefs-mcp/src/server.rs` | Wire concrete source, parked client-root refreshes, request-associated SEP-2260 acquisition, one total deadline, and unchanged invalid-params behavior. |
| `crates/resourcefs-mcp/src/render.rs` | Render optional backing metadata without changing canonical identity or text completeness. |
| `crates/resourcefs-core/tests/path_reference_contract.rs` | Replace founding parser rows with the shared reference corpus and property-oriented boundary assertions while retaining the existing 4 KiB/5 ms debug timing fence. |
| `crates/resourcefs-sources/tests/filesystem_adapter_contract.rs` | Add direct multi-root, containment, overlap, refresh race, generation, selector precedence, visibility, and limit contracts. |
| `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs` | Extend compiled-process tests for repeated launch roots, exact primary selection, client initial/change Roots, removed-root references/results, timeout, and both protocol revisions while retaining malformed-schema fences. |
| `tests/fixtures/workspace_references.json` | One shared golden corpus of accepted/rejected POSIX, Windows, file URI, canonical, percent, selector, traversal, primary-selection, nested-root ambiguity, stale-root, and 64 KiB boundary rows. |
| `fuzz/Cargo.toml`, `fuzz/fuzz_targets/path_reference.rs` | Standalone cargo-fuzz package, excluded from the product workspace, fuzzing parse/canonical round-trip/no-traversal properties. |

No files under `.rfs-87cv` are modified. They are historical evidence for the founding slice, not production inputs.

## Falsification table

| ID | Claim | Cheapest falsifier | Independent oracle | Named mutation | Regression fence | Status before build |
|---|---|---|---|---|---|---|
| C1 | Client Roots replace rather than union launch roots; empty intentionally restores launch roots. | Compiled stdio client advertises Roots, reads client-only/launch-only references, changes to empty, and repeats. | Test driver owns expected canonical IDs and visibility independently of server code. | Union candidates with launch roots; treat empty as acquisition failure. | `stdio_mcp_contract::client_roots_replace_and_empty_restores_launch_authority` | PENDING — checkpointed-build Slice 3 |
| C2 | Client canonical IDs are stable, path-private, and collision-safe. | Reorder/rename roots and exercise private root-set/history validators with one ID mapped to two canonical URIs. | Test computes SHA-256 over fixed normalized URI fixtures directly; unit tests construct the impossible-on-demand collision at the validation boundary. | Hash display name; truncate digest; omit cross-generation URI history. | `filesystem_adapter_contract::client_ids_ignore_order_and_names`; `reference::tests::duplicate_root_id_is_rejected`; `filesystem::tests::seen_id_cannot_alias_another_uri` | PENDING — checkpointed-build Slice 3 |
| C3 | Relative resolution requires one deterministic primary; zero or multiple selector matches return `ambiguous_reference`; canonical references remain usable without a primary. | Build one/two-root cases with implicit, zero, one, and two selector matches and read relative plus every canonical form. | Golden corpus states configured roots/selector and exact category/identity independently. | Pick first root; search every root for relatives; return `invalid_reference` instead of `ambiguous_reference`. | `path_reference_contract::golden_workspace_references`; `filesystem_adapter_contract::relative_reads_require_unique_primary`; `stdio_mcp_contract::launch_primary_selection_is_exact` | PENDING — checkpointed-build Slice 2 |
| C4 | Strict one-pass decoding and literal-wins selector precedence round-trip ambiguous filename characters. | Create `notes:2`, `notes`, and percent-named fixtures and read encoded/raw selector-like inputs. | Shared JSON corpus records literal and selector candidates, not implementation structs. | Decode twice; split encoded colon; split before literal existence check. | `path_reference_contract::golden_workspace_references`; `filesystem_adapter_contract::literal_paths_precede_selectors` | PENDING — checkpointed-build Slice 2 |
| C5 | Capability-relative open prevents symlink/reparse and path-replacement escape without leaking outside paths. | Concurrently retarget a parent/child link while reads run; no outside sentinel may appear. | Outside sentinel is generated by the test and never supplied to source code; cap-std platform behavior is source-verified. | Ambient canonicalize then reopen; lexical prefix check; suppress permission errors into not-found. | `filesystem_adapter_contract::retargeted_links_never_escape`; native `windows_reparse_points_never_escape` | PENDING — checkpointed-build Slice 2 |
| C6 | Absolute/file inputs matching nested roots fail ambiguous, including aliases resolving into the nested root. | Declare parent+nested roots and read direct and alias absolute/file forms. | Fixture root topology supplies the expected candidate count. | Choose longest prefix; choose primary; compare only lexical input. | `filesystem_adapter_contract::overlapping_absolute_inputs_are_ambiguous` | PENDING — checkpointed-build Slice 2 |
| C7 | Refresh suspends immediately; a removed-root in-flight result fails `invalid_reference`; retained identities can finish; overlapping equivalent notifications preserve generation; every effective transition reports its generation and removed/active IDs. | Gate adapter and compiled-stdio reads, apply removing/retaining/equivalent/concurrent refreshes including two back-to-back equivalent tokens, release gates, and inspect outcomes. | Tests control the gate, expected membership, and prior generation outside the source. | Validate only before I/O; discard the original baseline on re-entry; let an old refresh finish last; increment generation on reordered equivalent set; omit removed IDs from outcome. | `filesystem_adapter_contract::root_refresh_invalidates_removed_inflight_reads`; `root_change_outcome_names_authority`; `latest_refresh_wins`; `overlapping_equivalent_refreshes_preserve_generation`; `stdio_mcp_contract::removed_generation_result_is_rejected` | PENDING — checkpointed-build Slice 3 |
| C8 | Backing `file://` metadata is absent by default and visible only through the policy seam. | Direct adapter and stdio reads under Hidden/Visible construction. | Expected URI built by `url::Url::from_file_path` in test. | Populate metadata unconditionally; use backing URI as canonical identity. | `filesystem_adapter_contract::backing_uri_requires_visibility_policy`; `stdio_mcp_contract::canonical_identity_stays_private` | PENDING — checkpointed-build Slice 2 |
| C9 | MCP `2026-07-28` Roots acquisition is request-associated and each protocol-plus-validation acquisition is bounded by one five-second deadline. | Probe notification/request scopes; in compiled stdio delay `roots/list` beyond five seconds; in direct source tests drop an acquisition and block validation past its deadline; then require disabled authority and rejection of late completion until a later valid notification. | Recorded P1 independently checks rmcp's association scopes; the raw client owns request timing; source tests own the watchdog clock/deadline. | Send from a notification/spawned task; call generated unbounded `list_roots`; omit/cancel the watchdog; reset the timeout before validation; let late blocking validation commit; retain launch authority on failure. | `probe_rmcp.py`; `stdio_mcp_contract::roots_timeout_fails_closed_and_later_refresh_recovers`; `filesystem_adapter_contract::dropped_acquisition_deadline_disables_authority`; `late_root_validation_cannot_commit` | PENDING — checkpointed-build Slice 3 |
| C10 | Every supported filesystem spelling resolves to one canonical private identity or its specified stable error. | Run the golden corpus plus native platform filesystem cases. | JSON expected rows and OS-created fixtures are independent data. | Delegate Windows classification to `PathIsRelative`; accept non-local POSIX hosts; accept query/fragment. | `path_reference_contract::golden_workspace_references`; native `windows_path_forms_follow_contract` | PENDING — checkpointed-build Slice 2 |
| C11 | Limits are exact: 256 roots and 64 KiB references pass where otherwise valid; one-over fails `limit_exceeded`. | Boundary tests at N and N+1. | Tests construct byte/root counts independently. | Off-by-one comparison; count Unicode scalars instead of UTF-8 bytes. | `path_reference_contract::reference_byte_limit_is_exact`; `filesystem_adapter_contract::root_count_limit_is_exact` | PENDING — checkpointed-build Slice 2 |
| C12 | Existing bounded UTF-8 read semantics and MCP schema/protocol behavior do not regress. | Run existing direct adapter and compiled stdio suites under both revisions. | Existing tests predate this design and black-box the installed binary. | Partial content at a limit; remove manual argument pre-validation; default protocol to SDK `LATEST`. | Existing `filesystem_adapter_contract`; existing `stdio_mcp_contract` | PENDING — checkpointed-build Slice 3 |
| C13 | Core remains source-neutral and workspace stays exactly three packages. | Architecture contract plus dependency metadata inspection. | Existing architecture test reads manifests, not Rust type behavior. | Add cap-std/tokio/rmcp to core; add fuzz as workspace member. | `architecture_contract::workspace_enforces_dependency_direction`; `cargo metadata --no-deps` | PENDING — checkpointed-build Slice 3 |
| C14 | The parser retains its existing 4 KiB/5 ms debug bound. | Run the existing timing fence against the rewritten parser. | `Instant` measurement and fixed 4 KiB input live in the pre-existing contract test. | Add one full-input scan inside every input-byte iteration before classification. | `path_reference_contract::parses_production_shaped_reference_within_budget` | PENDING — checkpointed-build Slice 1 |
| C15 | 10,000 fuzzer executions produce no panic, nondeterminism, failed canonical round-trip, traversal, or encoded-separator bypass. | Run libFuzzer for exactly 10,000 executions. | Target assertions construct/check invariants independently of parser internals. | Decode percent escapes twice; accept a decoded separator; render canonical components without encoding literal `:`. | `cargo +nightly fuzz run path_reference -- -runs=10000` | PENDING — checkpointed-build Slice 1 |

P1-P3 in `.rfs-cgbq/evidence.md` are PASS. P4 (Windows case rules) and P5 (cap-std internals) remain covered implementation facts: production code defers to OS/capability primitives and permanent tests exercise the observable contract. No design claim depends on an unverified external service.

## Test strategy

### Shared golden corpus

`tests/fixtures/workspace_references.json` is table-driven. Every row declares `kind`, `input`, `platform`, and an independent expected parse or resolution result. Resolution rows additionally declare symbolic `launchRoots`, optional `clientRoots`, `primarySelector`, fixture files, `priorActiveRoots`, and `activeRoots`; the source test substitutes temporary absolute paths for symbols. This makes root selection, nested ambiguity, and stale canonical authority explicit test inputs rather than expectations inferred from implementation state. Rows cover:

- POSIX relative/absolute, canonical `rfs://`, local/non-local `file://`;
- Windows drive absolute, drive relative, UNC, root relative, and verbatim spellings;
- one implicit primary, multiple roots with zero/one/two selector matches, and canonical access to every active non-primary root;
- nested roots where one absolute and one `file://` input expect `ambiguous_reference`;
- a canonical root present in `priorActiveRoots` but absent from `activeRoots`, expecting `invalid_reference`;
- exact `%3A/%3F/%23/%25`, lowercase escapes, malformed escapes, `%253A`, encoded separators, UTF-8, NUL, dot and dot-dot;
- existing/non-existing selector-like names separated into parser expectations and source fixture expectations;
- empty, exactly 64 KiB, and 64 KiB plus one byte.

Core consumes syntax and source-neutral root-selection rows. Sources consume resolution/state rows against temporary roots. Platform-specific rows are filtered only by the declared platform field, never by expected outcome inferred from the implementation. The core contract also retains the existing fixed 4 KiB parse input and 5 ms debug timing assertion.

### Direct source tests

The adapter suite remains mandatory and is the primary security fence. It creates temporary contained/outside sentinels, nested roots, duplicate roots, gated reads, concurrent refreshes, and symlink/reparse fixtures. Race tests run many retarget/read iterations but assert only the invariant: content is the inside value or a stable denial/error, never outside content and never a panic. Unix uses symlinks and directory replacement; Windows requires a non-privileged junction/reparse case and adds symlink cases when runner privileges permit.

### Compiled stdio tests

The existing raw JSON-RPC harness gains a client request responder for `roots/list` and emits `notifications/roots/list_changed`. It verifies that initialization/change notifications suspend authority without emitting an unassociated request, and that the next `tools/list` or `tools/call` request receives the nested `roots/list` before workspace use. It also checks single-root implicit primary, multi-root zero/one/two selector matches, canonical access to every root, initial client override, empty fallback, valid and invalid all-or-nothing changes, removed-root canonical rejection, retained identity, timeout/recovery, and clean EOF under both MCP `2026-07-28` and `2025-11-25`. It retains existing output-schema, TextContent/structured-content equality, malformed input, unknown tool/method, and source error rows.

The removed-generation process case is deterministic, not a scheduling race. The non-default `test-support` feature compiles a filesystem delivery gate that is activated only by an explicit harness environment variable naming a temporary synchronization directory. After reading from the real capability file handle but before final authority validation, the child writes an `entered` marker and waits with a hard test timeout for `release`. The parent starts `rfs_read`, waits for `entered`, completes a client root change removing that root, writes `release`, and asserts the original call returns `invalid_reference`. Default builds contain no gate code, and the gate never substitutes content, authority, or protocol behavior.

### Fuzzing

The standalone `fuzz` package depends only on `resourcefs-core` and `libfuzzer-sys`. `path_reference` feeds arbitrary byte strings and bounded constructed UTF-8 into `PathReference::parse`, asserting no panic, deterministic category/result, accepted canonical round-trip, and absence of decoded `.`/`..` or encoded-separator bypass. It is not a workspace member and cannot alter the three-package architecture contract. The checkpoint gate is mandatory and runs exactly 10,000 executions.

### Verification commands

Checkpointed build runs the smallest gates first:

1. `python3 .rfs-cgbq/probe_rmcp.py` and `python3 .rfs-cgbq/oracle.py` after any probe change.
2. `cargo test -p resourcefs-core --test path_reference_contract`.
3. `cargo test -p resourcefs-sources --test filesystem_adapter_contract`.
4. `cargo test -p resourcefs-mcp --features test-support --test stdio_mcp_contract`.
5. `cargo test --workspace`.
6. `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
7. Run the source and stdio contract suites on native Linux, macOS, and Windows runners; every required symlink/reparse fence must execute rather than silently skip.
8. `cargo check --workspace --all-targets --target x86_64-pc-windows-gnu` as an additional compile fence when the target is installed; it does not substitute for native behavior.
9. `cargo +nightly fuzz run path_reference -- -runs=10000`; missing `cargo-fuzz` or a nightly Rust toolchain with `rustfmt` is a blocking prerequisite, not a skipped gate.
10. Install to a fresh temporary root with `cargo install --quiet --path crates/resourcefs-mcp --root <temp>`, then launch the installed `resourcefs` over stdio for one relative and one canonical read.

## Placement and forbidden seams

| Concern | Owner | Forbidden placement |
|---|---|---|
| Syntax classification, percent grammar, root value equality, ID derivation | `resourcefs-core::reference` | MCP handlers; filesystem OS code |
| Canonicalization, capability handles, containment, root lookup, authority epochs | `resourcefs-sources::filesystem` | core; renderer; CLI |
| rmcp Roots capability/request/notification conversion and timeout | `resourcefs-mcp::server` | core or sources public types |
| CLI/profile launch-source choice | `resourcefs-mcp::cli` now; rfs-r9m6 profile loader later | environment-variable fallback inside source |
| Backing visibility decision | source construction policy | renderer inference, debug mode, canonical identity |
| Selector execution and recovery | rfs-34pz | this parser/resolver beyond preserve-and-reject |
| Mutation snapshots and root-change mutation invalidation | rfs-73dz | this read-only source |
| Resource list/templates/subscriptions | rfs-ww6w | tool handler or filesystem adapter |

## Input map

| Input artifact/source | Exact use in this design |
|---|---|
| `.rfs-cgbq/route.md` | Empirical route, required probe/oracle artifacts, T1-T4 acceptance gates, and terminal PASS/no-FAIL rule. |
| `.rfs-cgbq/spec.md` | Entire approved Behavior section; Decisions table lines 112-138; 5-second timeout amendment; requester approval dated 2026-08-18. |
| `.rfs-cgbq/evidence.md` | P1 explicit-2026 SEP-2260 notification rejection/request-scope success and no-default-timeout learning for C9; P2 Windows classifier mismatch for C10; P3 handle-final link behavior for C5. |
| `.rfs-cgbq/probe_rmcp.py` and `.rfs-cgbq/oracle.py` | Runtime and independent source-oracle evidence that notification-scoped `roots/list` is rejected, request-scoped initial/change acquisition succeeds under `2026-07-28`, capabilities are visible, and the SDK supplies no generated timeout. |
| `.rfs-cgbq/probe_windows.py` | Wine/Win32 evidence for drive/UNC classification and handle-final link behavior used by C5/C10. |
| `CONTEXT.md` | Domain terms and constraints: Workspace Root, private `rfs://` identity, Backing-Path Visibility, Source Adapter, strict path-shaped product boundary. |
| `DESIGN.md` sections “MCP operator”, “Reference and Resource contract”, “Sessions, roots, configuration, and security”, and “Testing Decisions” | Accepted product-level precedence, canonical privacy, client-root invalidation, limits, adapter testing, and platform matrix. |
| `docs/adr/0003-session-and-authority-model.md` | One-connection authority boundary; client roots override fallback roots; filesystem containment and explicit grants. |
| `crates/resourcefs-core/src/reference.rs`, `resource.rs`, `source.rs`, `error.rs`, `lib.rs` | Existing public parser/result/trait/error surface being cleanly migrated; no second convention retained. |
| `crates/resourcefs-sources/src/filesystem.rs` and its contract test | Existing bounded filesystem read behavior, error mapping, and direct adapter seam retained/deepened. |
| `crates/resourcefs-mcp/src/cli.rs`, `server.rs`, `render.rs`, and `tests/stdio_mcp_contract.rs` | Existing single-root CLI composition, manual invalid-params validation, protocol negotiation, output rendering, and compiled-process harness to extend. |
| rmcp 3.1.3 `handler/server.rs` (`on_initialized`, `on_roots_list_changed`), `handler/client.rs` (`list_roots`), `service/server.rs` (SEP-2260 restriction), `service.rs` (`ORIGINATING_REQUEST`, `PeerRequestOptions`, `send_cancellable_request`), and `model.rs` (`Root`, capability types) | Exact protocol lifecycle, legal request association, and explicit bounded request API; rmcp types remain isolated in MCP. |
| cap-std 4.0.2 `src/fs/dir.rs`, `src/fs/open.rs`, and cap-primitives Windows/Linux open implementations | Capability-relative construction/open, public raw-handle access, and escape-denial premise; drives the selected no-ambient-reopen seam and C5 fences. |
| rustix 1.1 `fs::getpath`, Linux `/proc/self/fd` semantics, and Win32 `GetFinalPathNameByHandleW` | Final-path retrieval from the exact opened root/file handle on macOS, Linux, and Windows; prevents canonical identity and overlap checks from racing an ambient reopen. |
| Rust standard-library Windows path implementation (`std/sys/path/windows.rs`) | Prefix+root definition for fully-qualified Windows input; prevents use of `PathIsRelative` as contract oracle. |
| Verified open issues rfs-r9m6, rfs-34pz, rfs-73dz, rfs-ww6w, rfs-58r1, and rfs-5os7 | Owner map for profile enablement, selector/session behavior, mutation state, Resource mirroring, release platform execution, and real-Kiro evidence. No new issue is needed. |

## Future work already owned

- rfs-r9m6 supplies the strict Server Profile field that can construct `BackingPathVisibility::Visible` and profile launch roots.
- rfs-34pz executes selectors, snapshots displayed regions, spills recoverable output, and owns scratch/artifact lifecycle.
- rfs-73dz consumes root-change invalidation for mutation snapshots and adds serialized writes.
- rfs-ww6w mirrors workspace identities through MCP Resources and emits list/subscription notifications.
- rfs-58r1 runs native Linux/macOS/Windows packaging and release smoke tests.
- rfs-5os7 proves the complete public seam in real Kiro CLI.

Those behaviors are not scaffolded here. The present interfaces expose only the minimum seams each owner needs: immutable root identity/generation, policy-only backing metadata, and one source refresh control plane.

## Approval

Requester approval (verbatim): "Approve"

Date: 2026-08-18

Approved risk acceptances: none.