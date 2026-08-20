# Spec: Safe multi-root Workspace References

## Request (verbatim)
> Claim and implement rfs-cgbq

## What this is
ResourceFS will replace its single launch-root assumption with connection-owned Workspace Root authority that can switch atomically to non-empty client MCP Roots. It will resolve every supported workspace spelling to a private canonical `rfs://workspace/<root>/<path>` identity while rejecting ambiguous, escaping, or stale authority.

## Roles
- **MCP operator**: configures launch Workspace Roots and a Primary Workspace Root selector; sees startup failures for invalid launch authority and no backing paths unless visibility is explicitly enabled.
- **MCP client integrator**: supplies and changes MCP Roots; sees client authority override launch authority and stable canonical identities for unchanged roots.
- **Coding agent**: submits relative, absolute, `file://`, canonical workspace, Windows drive/UNC, and selector-bearing Path References; sees one deterministic canonical workspace identity or a stable error category.
- **ResourceFS maintainer**: verifies the accepted/rejected grammar with a golden corpus, fuzzing, direct adapter checks, and the compiled stdio MCP seam on the supported platform matrix.

## Behavior

### Launch authority
- **Given**: one non-empty launch root source supplies between 1 and 256 existing canonical directories, every launch name is valid and unique, exact canonical duplicates are absent, and a configured Primary Workspace Root selector matches one launch name.
- **When**: `resourcefs serve` starts and the MCP client does not advertise Roots or returns an empty Roots list.
- **Then**: the complete launch set is the connection's Workspace Root authority; CLI and Server Profile roots cannot be represented as one mixed launch source, and invalid launch authority fails before serving protocol traffic.

### Client-root precedence and validation
- **Given**: the MCP client advertises Roots and returns a non-empty list of at most 256 local `file://` root URIs whose canonical directories are valid and distinct.
- **When**: ResourceFS initializes the connection or handles `notifications/roots/list_changed`.
- **Then**: the client set atomically replaces the launch set without unioning; an acquisition taking longer than 5 seconds, any acquisition error, or any invalid member disables all workspace authority until a wholly valid non-empty set or a valid empty set is obtained.

### Primary Workspace Root selection
- **Given**: one active root exists, or multiple active roots exist and the configured selector matches exactly one active launch name or optional client display name.
- **When**: the coding agent submits a relative filesystem Path Reference.
- **Then**: one root resolves it; with multiple or zero selector matches ResourceFS returns `ambiguous_reference` and does not search roots, while canonical workspace references remain usable for every active root.

### Canonical client root identity
- **Given**: a valid client root has any optional display name, including an absent, invalid, or duplicated name.
- **When**: ResourceFS returns that root's canonical workspace identity.
- **Then**: the `<root>` segment is a private deterministic identifier derived from the canonical root URI; the same canonical URI produces the same identifier across root-set changes, and client display-name changes do not rename the root.

### Workspace reference forms
- **Given**: one existing filesystem Resource is canonically contained by exactly one active Workspace Root.
- **When**: the coding agent addresses it by a relative reference under the Primary Workspace Root, native absolute path, accepted local `file://` URI, or `rfs://workspace/<root>/<path>`.
- **Then**: every form resolves to the same private slash-form canonical `rfs://workspace/<root>/<path>` identity; a canonical reference may address a non-primary active root.

### URI, Windows, and delimiter parsing
- **Given**: a Path Reference no longer than 64 KiB contains percent escapes, a native Windows drive or UNC form, or a `file://` URI.
- **When**: ResourceFS parses it on Linux, macOS, or Windows.
- **Then**: input is decoded exactly once as UTF-8; encoded separators, NUL, malformed escapes, and decoded `.` or `..` are rejected; canonical path segments encode literal `%`, `:`, `?`, and `#`; Windows drive/UNC inputs are classified as filesystem paths rather than schemes or selectors, resolve only on Windows, and render with slash separators; POSIX accepts empty-host and `localhost` file URIs, while Windows also accepts UNC file authorities; credentials, ports, query, fragment, and non-local POSIX authorities are rejected.

### Literal-selector precedence
- **Given**: an input can be read either as one literal contained filesystem path or as a filesystem path plus a syntactically valid selector.
- **When**: ResourceFS resolves the input.
- **Then**: an existing literal Resource wins; otherwise the selector is split and preserved, and current `rfs_read` returns `unsupported_projection` rather than executing it.

### Containment and ambiguity
- **Given**: an input contains lexical traversal, resolves through a symlink or reparse point outside its root, races a path replacement toward outside content, names an exact duplicate root set, or an absolute/`file://` input is contained by several distinct nested roots.
- **When**: ResourceFS resolves or reads it.
- **Then**: no bytes outside active authority are returned; traversal and canonical escape return `permission_denied`, exact duplicate root sets are rejected atomically, and multiple-root matches return `ambiguous_reference`.

### Root-change invalidation
- **Given**: a valid client root update removes one root while retaining another.
- **When**: the update becomes active before an old-root read result is returned.
- **Then**: the removed root's canonical references and in-flight result fail as `invalid_reference`, the retained root keeps its canonical identity, the Primary Workspace Root is re-derived, and an invalidation generation/event is available for later snapshot consumers.

### Backing-path visibility
- **Given**: a workspace read runs with Backing-Path Visibility hidden or explicitly visible through the public source policy seam.
- **When**: the adapter and MCP renderer produce the Resource result.
- **Then**: `canonicalReference` and the text header always use private `rfs://` identity; hidden results omit `backingFileUri`, while visible results add that local file URI as structured metadata without replacing canonical identity.

### Golden and fuzz contracts
- **Given**: the checked-in golden corpus and arbitrary parser bytes cover relative, absolute, `file://`, canonical workspace, Windows drive/UNC, traversal, encoding, selector, root ambiguity, and stale-root forms.
- **When**: contract tests and the Path Reference fuzz target run.
- **Then**: every golden row matches its exact accepted canonical value or error category, successful parser output round-trips canonically, no input panics, and no accepted result contains traversal or an undecoded ambiguous delimiter.

## Success criteria

- **Binary / structural / security**: non-empty client Roots replace rather than union with launch roots; an empty client list restores launch roots; invalid client acquisition disables workspace authority, checked by `stdio_mcp_contract` with server-initiated `roots/list` requests and `roots/list_changed` notifications.
- **Binary / structural / security**: one root is implicitly primary, multiple roots require exactly one selector match, and canonical references address every active root, checked by the golden core contract and compiled stdio MCP cases.
- **Binary / structural / security**: relative, native absolute, accepted `file://`, and canonical workspace forms produce one exact canonical identity; malformed, traversing, foreign, ambiguous, over-limit, and removed-root forms produce their specified categories, checked by the checked-in golden corpus.
- **Binary / structural / security**: symlink, Windows reparse-point, and path-replacement fixtures never return outside bytes, checked by direct filesystem adapter tests on native Linux, macOS, and Windows runners.
- **Binary / structural / security**: hidden results contain no absolute backing path or `backingFileUri`; explicitly visible results contain `backingFileUri` while preserving the same `canonicalReference` and text header, checked by direct adapter and MCP renderer contracts.
- **Binary / structural / security**: a valid root-set change rejects removed-root canonical references and any result completing under the removed generation while preserving retained-root identity, checked by the compiled stdio root-change contract.
- **Quantitative**: at most 256 active roots and at most 64 KiB of UTF-8 per Path Reference are accepted; one-over inputs return `limit_exceeded`, measured by golden boundary rows and root-registry contract tests.
- **Quantitative**: every server-initiated `roots/list` acquisition either installs one complete result within 5 seconds or leaves workspace authority disabled, measured by a paused raw JSON-RPC client and the compiled stdio timeout contract.
- **Quantitative**: a 4 KiB reference parses within 5 ms in the existing debug-test timing fence, measured by `path_reference_contract`.
- **Quantitative**: 10,000 libFuzzer executions of the Path Reference target complete with zero crashes, panics, or invariant violations, measured by the cargo-fuzz run used for this change.

## Out of scope

This change does NOT include:

- executing range, multi-range, raw, paging, spill, or Recovery Reference selectors; rfs-34pz is the verified open issue;
- creating Path Session artifacts or `local://` state; rfs-34pz is the verified open issue;
- parsing the strict Server Profile or exposing Backing-Path Visibility through it; rfs-r9m6 is the verified open issue;
- creating or invalidating mutation edit snapshots beyond the root-generation/event seam; rfs-73dz is the verified open issue;
- MCP Resource/template mirroring, subscriptions, or Resource metadata projection; rfs-ww6w is the verified open issue;
- the complete real-Kiro workflow gate; rfs-5os7 is the verified open issue;
- release packaging and publication across platforms; rfs-58r1 is the verified open issue, while this change still supplies native platform contract tests.

## Related issues

- rfs-jk0d: accepted parent Behavior Contract and root authority, private identity, Windows, selector, and invalidation decisions.
- rfs-87cv: closed founding single-root read slice; its approved design explicitly defers this complete grammar and root lifecycle to rfs-cgbq.
- rfs-r9m6: strict Server Profiles and profile-versus-CLI composition build on this issue's launch-authority seam.
- rfs-34pz: selector execution, Path Session artifacts, and session-reference lifecycle build on this issue's reference parser and root transitions.
- rfs-73dz: edit snapshots build later on the root-change invalidation contract established here.
- rfs-ww6w: MCP Resource mirroring and negotiated root/subscription notifications consume canonical identity and root-change events.
- rfs-58r1: release automation supplies the native Windows/macOS/Linux platform smoke gate.
- rfs-5os7: release acceptance runs the complete golden corpus and fuzz targets.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| Which authority wins when non-empty client MCP Roots are available? | Client Roots replace launch Workspace Roots as one atomic authority set. | Accepted parent rfs-jk0d and `DESIGN.md:215-223`. | Client and launch roots are never unioned. |
| Which launch sources may be combined? | Exactly one of Server Profile roots or CLI roots supplies launch authority; mixing them is rejected. | Accepted parent rfs-jk0d; rfs-r9m6 consumes this seam. | Launch composition has one source and no layering. |
| When do relative references resolve? | Only when one Primary Workspace Root is selected uniquely; canonical workspace references remain available for every declared root. | Accepted parent rfs-jk0d and `CONTEXT.md:39-41`. | Relative resolution never searches multiple roots. |
| Is a backing filesystem URI canonical Resource identity? | No. Canonical identity is always private `rfs://`; backing `file://` metadata appears only under explicit Backing-Path Visibility. | Accepted parent rfs-jk0d and `CONTEXT.md:103-105`. | Normal output does not expose absolute backing paths. |
| How are ambiguous delimiters and selectors interpreted? | Ambiguous literal `:`, `?`, and `#` use percent encoding, and an existing literal filesystem path wins over selector interpretation. | Accepted parent rfs-jk0d and `DESIGN.md:87-110`. | Parsing must preserve literal names before projecting selectors. |
| What survives a client root-set change? | Removed-root workspace references and snapshots are invalidated; session-owned `local://` and `artifact://` state remains when those sources exist. | Accepted parent rfs-jk0d; rfs-34pz and rfs-73dz implement the later session/snapshot state. | This issue must expose root invalidation without pre-implementing later session resources or mutation snapshots. |
| How is the canonical ID for a client-supplied MCP Root assigned? | Derive every client-root ID from its canonical URI; use the optional client name only to select the Primary Workspace Root. | Requester selected “URI-derived private ID”. | IDs remain stable across root-set changes and do not reveal backing paths; identity and primary-selection label are separate concepts. |
| How do absolute and `file://` inputs resolve when distinct declared roots overlap? | Keep distinct nested roots valid, but return `ambiguous_reference` when one absolute or `file://` input is contained by more than one root. Reject an authority set containing the same canonical root URI more than once. | Requester selected “Allow roots; reject ambiguous inputs”, then resolved the duplicate-ID conflict with “Reject exact duplicates”. | Canonical workspace references remain unique for every effective root; relative inputs still use only the Primary Workspace Root. |
| Which `file://` authorities are accepted? | On POSIX accept an empty host or `localhost`; on Windows accept local drive URIs and UNC `file://server/share/...`; reject credentials, ports, query, fragment, malformed encoding, and non-local POSIX hosts before containment. | Requester selected “Native local forms”. | File URI authority is explicit and platform-correct rather than delegated to dependency defaults. |
| How is Backing-Path Visibility exposed before Server Profiles exist? | Add a public core/source policy option and optional read metadata now; keep the `resourcefs serve` CLI hidden-by-default until rfs-r9m6 wires the accepted Server Profile field. | Requester selected “Policy seam only”. | Direct adapter and renderer checks prove explicit enablement without creating a second permanent CLI configuration source. |
| What record is required for behavior deferred from this change? | Every intended future behavior must be covered by a verified open tracker issue; otherwise create one before deferring it. | Requester said, “If we defer anything, make sure there's a ticket open for it”. | Out-of-scope entries must cite the covering `rfs-…` issue; permanent non-goals remain rationale-only under the workflow contract. |
| What happens when a roots-capable client cannot supply one wholly valid root set? | Fail closed: disable all Workspace Root authority until the client returns either a valid non-empty set or a valid empty set; do not reactivate launch roots on acquisition or validation error. | Requester selected “Fail closed”. | Root updates are atomic, no invalid member is partially installed, and stale launch or client authority cannot survive an untrusted update. |
| How are percent-encoded workspace path segments handled? | Decode exactly once as UTF-8; reject encoded `/`, `\\`, NUL, malformed escapes, and decoded `.` or `..`; canonical output encodes literal `%`, `:`, `?`, and `#` per segment. | Requester selected “Strict single decode”. | Equivalent alternate separator or traversal spellings cannot bypass containment, while ambiguous literal delimiters round-trip. |
| Which selector behavior is implemented here? | Split and preserve valid selectors only after the full literal filesystem path is found absent; if `rfs_read` receives the resulting non-literal projection, return `unsupported_projection`. | Requester selected “Parse and preserve only”; rfs-34pz is the verified open execution/recovery issue. | This change establishes the precedence seam and corpus without absorbing range, raw, paging, or recovery execution. |
| What hard input envelope applies? | Accept at most 256 active roots and at most 64 KiB of UTF-8 in one Path Reference; reject one-over inputs as `limit_exceeded`. | Requester selected “256 roots; 64 KiB reference”. | Root updates and parser allocation have a falsifiable upper bound while retaining native long-path headroom. |
| What does an empty root set mean? | A valid empty client Roots list activates the valid launch set; zero launch roots fail startup. | Accepted parent rfs-jk0d and the requester-selected fail-closed distinction between an empty success and an acquisition error. | Empty is intentional fallback, not failure. |
| How is a missing client root name handled? | Missing, invalid, or duplicate optional names do not invalidate canonical identity; they cannot create a unique primary-selector match. | Requester-selected URI-derived identity. | A single root remains implicitly primary; multiple unnamed or ambiguously named roots disable relative references. |
| What happens to concurrent reads during a root update? | Root updates serialize as atomic generations; a read may return only if its root remains active in the generation checked before result delivery. | Accepted parent rfs-jk0d's immediate invalidation requirement. | An in-flight removed-root read cannot leak content after authority changes. |
| How are permission failures reported? | Lexical or canonical escape is `permission_denied`; filesystem access denial remains `permission_denied`; error text contains no denied content or outside backing path. | Accepted rfs-jk0d security contract and rfs-87cv error taxonomy. | Callers receive a stable category without data disclosure. |
| Can a root update partially succeed? | No. Validate and canonicalize the complete set, then install it atomically; any member failure disables authority under the fail-closed decision. | Requester-selected fail-closed policy. | Configuration order cannot retain a valid subset accidentally. |
| Are repeated root notifications idempotent? | Yes. Re-fetch and compare canonical authority; an equivalent set preserves identity and generation, while a changed valid set creates one new generation. | Root-change stability required by rfs-jk0d and URI-derived identity. | Retries do not invalidate unchanged references. |
| How do soft deletion, time zones, DST, and replication lag affect this behavior? | N/A — Workspace Roots are live local filesystem authority and this change stores no records or replicated state. | These dimensions have no observable input to this feature. | No hidden temporal or replication semantics are introduced. |
| What is the multi-tenancy boundary? | One stdio MCP connection owns its active root authority and generation; authority is never shared across connections. | Accepted Path Session boundary in rfs-jk0d and `CONTEXT.md`. | Client root changes cannot affect another connection. |
| What cache invalidation is required? | Any valid changed root set atomically replaces root lookup state and invalidates removed-root generations; invalid acquisition leaves no workspace cache active. | Accepted root-change contract plus requester-selected fail-closed policy. | Cached resolution cannot preserve removed authority. |
| How do null or missing root fields behave? | A missing optional client name is allowed; a missing/null root URI or malformed Roots result invalidates the whole client set and invokes fail-closed authority. | MCP Root's URI is required; the requester selected fail-closed validation. | Schema omissions cannot create partial or fallback authority. |
| How do concurrent writes affect this behavior? | N/A — this change exposes no workspace mutation; root updates are authority changes and are serialized under the concurrent-read decision. | Workspace mutation belongs to verified open issue rfs-73dz. | No write ordering or idempotency behavior is added here. |
| How long may `roots/list` acquisition suspend workspace authority? | 5 seconds; timeout is an acquisition error and authority remains fail-closed until a later valid refresh. | Requester selected “5 seconds”. | A lost or stalled server-initiated request cannot suspend workspace reads indefinitely. |

## Approval

Requester approval (verbatim): "Approve"
Date: 2026-08-18

