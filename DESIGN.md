# ResourceFS shared understanding

Status: accepted on 2026-08-18 by explicit user sign-off.

## Problem and product boundary

Oh My Pi demonstrates that files, archive members, SQLite rows, remote objects, generated output, and session state can share one path-shaped address space. ResourceFS extracts that product idea—not OMP’s runtime—into a standalone, cross-harness server.

ResourceFS is:

- an independent Rust workspace and dedicated `resourcefs` binary;
- a local stdio MCP server for agent harnesses;
- a path-shaped resource engine behind a small `rfs_*` tool interface;
- independently versioned and released through prebuilt Linux/macOS/Windows binaries and `cargo install`;
- governed by its own SemVer Behavior Contract.

ResourceFS is not an OMP plugin, remote OMP interface, compatibility wrapper, or dependency of the OMP monorepo. It never reads OMP configuration or private session layouts. OMP behavior and fixtures are design input only.

## Public MCP interface

The official Rust `rmcp` SDK is the runtime. The server advertises MCP `2026-07-28`, negotiates supported client revisions, and gates optional features by negotiated client capability. The implementation does not need MCP Tasks.

Model-controlled tools are canonical:

1. `rfs_read`
   - Input: `path`; optional lower-only output `limits` (`bytes`, `lines`, `columns`), `numbered`, and lower-only source `acquisition` controls (currently PR Facts only).
   - Reads a Resource or projection, directory, archive member, SQLite row, document, image, notebook, web URL, or internal Path Reference.
   - Parseable code with no selector returns a Structural Summary; selectors recover exact omitted ranges.
2. `rfs_search`
   - Input: `pattern`, optional `path`, case/gitignore controls, pagination, and lower-only limits.
   - Uses Rust regex by default and PCRE2 for syntax unsupported by Rust regex.
   - Searches a Resource without forcing the model to materialize its complete content.
3. `rfs_glob`
   - Input: path/glob, hidden/gitignore controls, pagination, and lower-only result limits.
   - Performs bounded structure discovery across adapters that can enumerate.
4. `rfs_write`
   - Input: `path`, UTF-8 `content`, optional `ifVersion`, optional `operationId`.
   - Creates text Resources. Replacing an existing Resource requires its current Version Tag in `ifVersion`.
   - Non-idempotent remote Creation Targets require `operationId`.
5. `rfs_edit`
   - Input: one hashline patch document.
   - Implements line-anchored `PUT`, `CUT`, `REM`, and same-source `MV` against a Version Tag from a prior read.

All five tools remain visible. ResourceFS policy—not tool discovery—decides whether a destination may mutate. MCP destructive/read-only annotations are truthful hints; they are never authorization.

Every tool result has:

- a complete, non-empty TextContent representation, including all recovery and error information needed by a text-only client;
- a structured object whose output schema has root `type: "object"`;
- stable semantic fields for contract version, canonical reference, content type, Version Tag, mutability, boundedness, and Recovery Reference where applicable; a bounded read or search whose continuation walks an artifact chain also names the paginated source's next upstream page as a source continuation (ADR-0006).

Operational failures are tool errors with stable categories: invalid reference, invalid pattern, invalid patch, not found, permission denied, version conflict, limit exceeded, source unavailable, unsupported projection, unsupported mutation, ambiguous reference, and cancelled. Protocol/schema failures remain MCP errors.

Operational errors retain `category` and `message` and may add bounded `details`: a machine-readable `reason`, optional `httpStatus`, `accessAmbiguity`, `retryGuidance`, `rateLimitReset`, and `limit` (`kind`, positive `bound`, optional `observed`). Absent details are omitted, not filled with nulls. Retry guidance is an object such as `{"delaySeconds": 5}` or `{"atUnixSeconds": 1800000000}`; rate-limit reset is Unix seconds. Limit kinds are `attempts`, `elapsed_nanoseconds`, `response_body_bytes`, `accepted_body_bytes`, `representation_bytes`, and `collection_records`. Details carry no provider body, arbitrary headers, URLs, or credentials.

PR Facts failures distinguish unauthorized repositories (`permission_denied` / `repository_not_authorized`), denied upstream access (`permission_denied` / `upstream_denied`), and an upstream 404 (`not_found` / `upstream_not_found_or_hidden`, with `accessAmbiguity: "missing_or_access_hidden"`). Rate limiting is `source_unavailable` / `upstream_rate_limited`; malformed data and contradictory identity use `source_unavailable` / `upstream_malformed` or `upstream_identity_mismatch`. Acquisition byte/attempt exhaustion uses `limit_exceeded` with the effective bound where available; deadline expiry uses `source_unavailable` / `deadline_exceeded`, and cancellation remains `cancelled`. Reasons and numeric guidance are actionable observations, not permission to retry indefinitely.

MCP Resources and templates mirror every resolvable Path Reference when negotiated. They are additive application/UI affordances, not a second behavior model and not required for the tool workflow. Resource listing is bounded and paginated. Completion is fast and local; network-backed enumeration never runs on every keystroke. Only local/workspace and Path Session resources publish subscription changes initially.

No MCP Prompts are exposed initially. Essential reference/recovery rules appear early in each tool description as well as server initialization instructions, because clients do not surface those channels consistently.

## Kiro CLI compatibility floor

Kiro CLI is the sole required first-release harness acceptance target.

Current Kiro constraints make the plain-text result contract mandatory:

- Kiro exposes MCP tools, but does not expose autonomous `resources/list`/`resources/read` to the model.
- Current reports show `structuredContent` may be discarded.
- Empty `content` beside structured content may fail.
- A root-level `oneOf` output schema may cause Kiro to reject the entire tool list.
- `resource_link` result items are unreliable.
- Oversized tool results can exhaust the conversation context.
- Current requested MCP protocol revision is undocumented, so real negotiation must be exercised.

ResourceFS therefore emits inline textual Recovery References rather than relying on `resource_link`, keeps output schemas object-rooted, and caps every result.

Release acceptance uses an absolute binary path in `.kiro/settings/mcp.json`, starts `resourcefs serve --root workspace=. --primary-root workspace`, requires MCP startup, and exercises:

1. tool discovery;
2. a local file read that spills omitted content to `artifact://…`;
3. selector recovery through `rfs_read`;
4. `rfs_search` against the same referenced content;
5. `rfs_write` to `local://…` and a verifying read;
6. permission denial for a workspace mutation without grants.

Protected CI runs the headless scenario with `KIRO_API_KEY`; a recorded interactive `/mcp` and approval-flow run is the required fallback where that paid-tier secret is unavailable. Synthetic MCP tests do not substitute for this release gate.

Primary Kiro references: [MCP configuration](https://kiro.dev/docs/mcp/configuration/), [MCP usage](https://kiro.dev/docs/mcp/usage/), [headless CLI](https://kiro.dev/docs/cli/headless/), [MCP security](https://kiro.dev/docs/mcp/security/), and issue evidence for [structured content](https://github.com/kirodotdev/Kiro/issues/10671), [output-schema strictness](https://github.com/kirodotdev/Kiro/issues/10830), [resource links](https://github.com/kirodotdev/Kiro/issues/9439), and [large results](https://github.com/kirodotdev/Kiro/issues/9946).

## Path Reference contract

A Path Reference has a source namespace, Resource identity/hierarchy, and optional projection selector. The operation remains in the tool call.

Examples:

- `src/auth.rs`
- `src/auth.rs:50-100`
- `bundle.zip:src/auth.rs`
- `state.db:users:42`
- `skill://code-review/references/checklist.md`
- `issue://owner/repo/123/body`
- `pr://owner/repo/123/diff/1`
- `pr://owner/repo/123/facts`
- `agent://Reviewer/findings.0.path`
- `artifact://7:100-200`
- `local://review-context.md`
- `mcp://catalog://notes/42`
- `rfs://workspace/docs/docs/architecture.md`

The grammar preserves OMP’s useful syntax: selectors append to filesystem and internal references; literal `:`, `?`, and `#` are percent-encoded where ambiguous; POSIX slash form is canonical in rendered references; Windows drive/UNC paths are parsed as filesystem paths rather than URI schemes; an existing literal path wins over selector interpretation.

All internal schemes are server-relative. `artifact://7`, `agent://Reviewer`, or `mcp://…` is meaningful only when sent back to the ResourceFS process/Path Session that issued or configured it. No generic URI ownership is claimed.

All tools accept `rfs://workspace/<root>/<path>` as the absolute Path Reference for a Resource in any declared Workspace Root; relative spellings resolve only under the Primary Workspace Root. Workspace text displays root-relative paths when unambiguous. MCP Resource identity always uses the canonical private `rfs://` URI. A Server Profile may opt into additional backing `file://` display/metadata, but that never replaces or changes canonical identity.

### Included address families

First-release portable core:

- ordinary files/directories and `file://` within Workspace Roots;
- ZIP and tar-family archives;
- SQLite databases;
- HTTPS documents with reader-mode Markdown/text by default and `:raw` for the bounded original body;
- converted documents, typed images, and Jupyter notebooks;
- server-owned `artifact://` and `local://`;
- configured `memory://`, `skill://`, `rule://`, `vault://`, `issue://`, `pr://`, and `ssh://` sources;
- optional read-only `agent://` and `history://` Agent Export sources;
- optional downstream MCP Resource sources through `mcp://` and otherwise-unclaimed native URI schemes.

Explicitly excluded: OMP-owned `omp://`, `security://`, and `xd://`. They require OMP runtime state or tool devices and are not portable sources.

Downstream MCP is a Resources/templates proxy only. Configured child servers may use direct-argv stdio or allowlisted HTTPS Streamable HTTP. ResourceFS does not proxy their tools or prompts. Concrete native resource URIs must be globally unique across configured downstream servers; configuration checks known collisions, and ambiguous template expansions fail explicitly. Existing OMP-compatible `mcp://<native-resource-uri>` spelling is preserved.

## Resource projections and source behavior

### Workspace files and directories

Canonicalized paths must remain inside an authorized Workspace Root. Symlinks are permitted only when their canonical targets remain contained. Directory and glob output is bounded. Files larger than structural/parser safety limits fall back to selector-driven text rather than being buffered for a summary.

Structural Summary parsers are compiled for these initial language families: Rust, TypeScript/JavaScript, Python, Go, Java, C/C++, and C#. Other code and prose use bounded text fallback. Summaries expose declarations, elide bodies, and name exact recovery selectors.

### Archives

ZIP and tar-family members use `archive:path/member` spelling. Containment rejects absolute/traversing member names. Text members support create, replacement, hashline edit, `REM`, and same-archive `MV` when the containing Workspace Root grants the operations. Mutation rebuilds the archive through an atomic replacement rather than editing it partially.

### SQLite

Rows are versioned JSON Resources. `db.sqlite:<table>:<percent-encoded-key>` addresses a row; scalar primary keys retain OMP’s short spelling, while composite keys encode a canonical JSON object. `rfs_write` inserts or fully replaces a row, `rfs_edit` edits its JSON projection, and `REM` deletes it. Path and JSON key values must agree. Mutations are transactional. Arbitrary mutating SQL is not exposed.

### Documents, images, binary content, and notebooks

Configured converter commands handle document types not natively supported. Commands are argv arrays, never shell snippets.

Image reads return bounded MCP ImageContent plus a complete text metadata block (type, dimensions, size, reference). Unsupported binary Resources return metadata through tools and may be mirrored as typed/blob MCP Resources, but arbitrary binary mutation is out of scope.

`.ipynb` reads use a deterministic editable cell projection such as `# %% [code] cell:0`. Versioned edits round-trip notebook JSON, preserve notebook/cell metadata and existing outputs for retained cells, permit insertion/reorder/deletion, and reject malformed projections rather than corrupting the notebook.

### HTTPS

Only profile-allowlisted HTTPS hosts may be fetched. Reader mode is default; `:raw` returns the bounded original response body. Every redirect and resolved address is revalidated. Loopback, link-local, private, and other special addresses require a separate per-host private-network grant.

### GitHub

GitHub uses native HTTP APIs, configured credentials, and an explicit readable repository allowlist. Mutation grants are a stricter per-repository/per-operation subset. Aggregates are read-only; authoritative fields are separate Resources.

Existing issue/PR field paths include `title`, `body`, and stable conversation `comments/<id>`. `comments/new` accepts Markdown and returns the canonical created comment reference. For PRs, `comments` means issue-style conversation comments; review submissions and inline review comments use explicit read-only collections initially.

`issue://owner/repo/new` accepts strict frontmatter requiring `title`, followed by Markdown body. `pr://owner/repo/new` requires `title`, `head`, and `base`, accepts optional `draft`, and uses the remaining Markdown as body. Post-creation mutation remains limited to title, body, and conversation comments; labels, assignees, status, reviews, and inline review comments are not mutable initially.

Every remote Creation Target requires `operationId`. Within one live Path Session, the journal returns the previous canonical reference when the same operation ID and content repeat, and rejects reuse with different content. Operation IDs never deduplicate across Path Sessions. ResourceFS never automatically retries an upstream outcome that became unknown after transmission; after any unknown outcome or disconnect, the caller must reconcile upstream state before repeating the creation.

Existing remote replacement fetches and compares the authoritative Version Tag immediately before mutation.

#### PR Facts version 1

`pr://owner/repo/<positive-number>/facts` is a read-only JSON Resource, not the PR's human-readable Aggregate and not raw provider JSON. It accepts no projection selector, including `:raw`. Facts are not available for issues, PR collections, reviews, comments, or diffs. Existing PR Aggregate, field, diff and mutation behavior is unchanged.

The JSON representation uses `schemaVersion: {"major": 1, "minor": 0}` and `kind: "github.pull_request"`. Its top-level fields are:

- `resource`: the canonical PR Facts Path Reference.
- `source`: configured `sourceId` and `deployment` with `webOrigin` and `apiOrigin` (the API origin is not the API base path).
- `repository`: requested `owner` and `name`, plus presence-aware `observed` base repository metadata; `request` separately records the repository and PR `number`.
- `observed`: required PR `id` and `number`, optional `nodeId`, and `base`/`head` observations containing required `commitSha`, presence-aware `repository`, and `repositoryAvailability` (`present`, `null`, or `omitted`).
- `acquisition`: `startedAtUnixMs`, `completedAtUnixMs`, `elapsedMs`, requested `restApiVersion`, effective `limits`, and `usage` (`attemptedRequests`, `acceptedBodyBytes`).
- `upstream`: `body` observation, plus `revalidation` only when a cached body was conditionally revalidated. Observations may include `status`, `etag`, `lastModified`, `date`, `selectedApiVersion`, and `observedAtUnixMs`. A 304 observation does not replace the original body's provenance.
- `data`: required `id`, `number`, `base`, `head`, and a `links` object; presence-aware `nodeId`, `title`, `body`, `state`, `author`, `createdAt`, `updatedAt`, `closedAt`, `mergedAt`, `draft`, and `merged`. Branches add presence-aware `refName`. Links use `apiUrl`, `htmlUrl`, `diffUrl`, `patchUrl`, `issueUrl`, and `relations`; relation entries retain native names and presence-aware `href`. Actors expose `id`, `nodeId`, `login`, and `links`; repositories expose `id`, `nodeId`, `name`, `fullName`, `owner`, and `links`, all presence-aware.
- `unavailableFacts`: entries with `field` and `reason` (`null` or `omitted`) for selected optional PR, link, and branch facts, not an exhaustive recursive inventory.

Native numeric IDs and PR numbers are positive unsigned 64-bit values serialized as canonical decimal **strings**, including nested actor/repository IDs; node IDs stay strings. Timestamps and counters retain their schema-specific types. Optional native nulls remain JSON null, missing fields remain omitted, and empty strings or `false` remain real values. Unknown native fields are not copied into the owned schema.

Acceptance requires a matching PR number/API identity and both 40-character lowercase hexadecimal base/head commit SHAs. Available repository identity evidence must agree internally; base repository evidence must agree with the requested repository. A head repository may identify a fork outside the readable allowlist without granting access to it. Missing or null repository metadata remains unavailable, never guessed. Recognizable contradictory object links fail identity validation; opaque links and URI templates remain observations, not trusted destinations.

One native PR-detail acquisition supplies Facts. ResourceFS does not automatically follow links, fetch the head repository, resolve refs, compare commits, or acquire comments, reviews, diffs, or patches to enrich the result. Facts remain read-only even where mutation grants exist. Their Version Tag names the exact JSON representation, including acquisition observations, not a stable PR revision or a base/head comparison result.

Public GitHub derives `webOrigin` as `https://github.com`; `https://api.<tenant>.ghe.com` derives `https://<tenant>.ghe.com`. A configured explicit origin must agree with those documented deployments. A custom `apiBaseUrl` requires explicit HTTPS `webOrigin` to serve Facts; without it, existing reads remain available but Facts return `unsupported_projection` / `deployment_identity_unavailable`. See [operating guidance](docs/operating.md#github-pr-facts) for profile and call examples.

#### Conversation-comment Facts version 1

`pr://owner/repo/<positive-number>/comments/facts` reads the conversation-comment collection and `pr://owner/repo/<positive-number>/comments/<positive-id>/facts` reads one comment. Both are read-only owned JSON using the same envelope, `schemaVersion: {"major": 1, "minor": 0}`, and the same error vocabulary as PR Facts. `kind` is `github.conversation_comment_collection` or `github.conversation_comment`. Each record retains native `kind`, decimal-string `id`, presence-aware `nodeId`, the verified parent PR identity and links, presence-aware `body`/`author`, supplied `createdAt`/`updatedAt`, and original links. The parent PR is read first: a comment whose `issue_url` names another repository or pull request, or a collection addressed under a number that is not a pull request, rejects the whole component.

The collection object is present only for the collection route. It carries `state` (`complete`, `incomplete`, or `unknown`) and `acceptedCount`, plus `localLimit` (`kind`, `bound`, optional `observed`), bounded sanitized `failure` facts, `continuation`, and `inconsistency` only when each is true. `providerCap` and `reportedTotal` are absent for this family because the provider supplies neither; a local ceiling is never presented as a provider cap. `complete` requires a fetched page with no `rel="next"`; a local stop, an ordinary later failure, or observed inconsistent pagination is never reported as complete. `unknown` reports a repeated or stalled target without claiming a snapshot.

Pages are admitted whole and only while the record ceiling (1,000), the accepted-body ceiling, and the resulting final serialized representation — including envelope, coverage, failure and continuation facts — all fit. A page that cannot fit is not admitted: earlier verified pages survive with honest coverage and a continuation when one is valid, while a first unusable page is a typed `limit_exceeded` failure. Cancellation or an authority/requested-identity violation rejects every page of that read.

A truncated collection names an opaque `:cursor:<canonical-base64url>` continuation. The handle is credential-free and binds the provider's native next page to the canonical Resource, the configured API origin, and a one-way digest of the issuing Path Session. It is decoded and confined to the same origin and endpoint family before any request, so a handle from another session, resource, origin, or endpoint family acquires nothing. Continuation is best-effort traversal, not a snapshot, and is named only for a page that was never fetched (a local ceiling or a retryable attempt/deadline exhaustion); a fetched-but-malformed page does not produce one. A caller lowers acquisition ceilings with the same `acquisition` object as PR Facts; the record ceiling is local policy, not a caller control. See [operating guidance](docs/operating.md#github-conversation-comment-facts).

### Jira

An Atlassian Site Mount confines Jira reads to its configured origin and credentialed visibility. Direct issue Aggregates and Fields use `jira://<site>/issues/<issue-id>[/fields[/<field-id>]]`; issue-key aliases resolve to stable issue identity.

`jira://<site>/projects` is a bounded collection. `jira://<site>/projects/<project-id>` reads compact project metadata; `/project-keys/<project-key>` resolves without alias caching and publishes the returned stable project identity. IDs are canonical decimal strings of up to 64 digits, never narrowed to machine integers. Optional metadata preserves missing, null, empty, and false values.

Project collections request at most 100 native rows per fetch and retain at most 1,000 rows per logical page, using at most ten physical attempts and one retry within the original logical deadline. Returned effective maxima and validated advancing next links govern traversal; short or empty pages and changing totals do not imply completion. `:offset:<positive-u64>` selects a subsequent native page. A completed logical page is sorted numerically by stable ID; this does not promise global numeric ordering across pages or scan an entire tenant.

`jira://<site>/issues` and `jira://<site>/projects/<project-id>/issues` enumerate fixed issue collections through enhanced GET search, selecting issue key, summary, status and project identity. Project-scoped reads first validate the stable parent inside the same attempt/retry/deadline budget. Issue pages share the 100-native/1,000-logical row and ten-attempt ceilings and sort the completed logical page numerically, not by native key order.

`jira://<site>/search/<percent-encoded-query>` addresses an Atlassian Query Resource. The query segment decodes exactly once to nonempty UTF-8 JQL, preserved without trimming, normalization or interpretation. Canonical encoding leaves ASCII letters, digits and `-._~` literal and uses uppercase percent escapes for all other UTF-8 bytes; spaces are `%20`, literal plus is `%2B`, and embedded separators are encoded. For example, `jira://work/search/project%20%3D%20DEMO%20ORDER%20BY%20key%20DESC` carries `project = DEMO ORDER BY key DESC`. Jira decides query meaning; ResourceFS does not provide a query language or cross-site fan-out.

Native queries use read-only `POST /rest/api/3/search/jql`, preserving returned order, including caller-authored ordering, rather than fixed-collection numeric sorting. Compact rows navigate to validated stable-ID issue Resources in the owning site. Each logical page is atomic and bounded by 100 rows per native request, 1,000 retained rows and ten physical attempts. An optional valid native `maxResults` can lower subsequent request caps; absent metadata does not imply a clamp. Short or empty native pages do not imply completion. Native token absence terminates, and any supplied terminal marker must agree; no snapshot consistency or token-lifetime guarantee is added.

`:cursor:<canonical-unpadded-base64url>` selects an issue page. The bounded strict envelope preserves an opaque native token and binds the canonical unselected collection, Site ID, endpoint family, parent and configured-origin fingerprint before egress. It is not an authenticated capability. Token absence terminates; a supplied terminal marker must agree. Empty or repeated tokens, foreign parent rows and invalid selected metadata fail atomically. An unrepresentable continuation fails with `limit_exceeded` before it can be forwarded or discarded. Status absence, null and object/name presence remain distinct.

Queries use the same typed `:cursor:<canonical-unpadded-base64url>` selector, with ownership bound to the exact canonical query, Site ID, endpoint family and configured-origin fingerprint before egress. Query cursors cannot select another query or a fixed collection. Duplicate stable IDs, malformed rows or pagination metadata, repeated tokens and failed later fetches reject the whole logical page; unrepresentable continuation fails with `limit_exceeded`.

Query reads are uncached and share the existing HTTP egress, response-size, cancellation and original-deadline controls. Explicitly read-only POSTs may retry once total within the logical page's physical-attempt budget, not once per native page; ordinary mutation POSTs are not replayed. A native HTTP 400 with a valid structured Error Collection becomes `invalid_pattern`; malformed native rejection envelopes remain `source_unavailable`. Local malformed encoding or cursor ownership failures remain `invalid_reference`, with existing limit categories retained. Diagnostics expose bounded structural counts/status, never upstream error prose or arbitrary diagnostic field names.

Canonical GETs revalidate the Path Session cache without stale fallback. Malformed authority, duplicate IDs, failed later pages, or unrepresentable continuation fail atomically. Raw/line projection applies to the rendered first page; source-page and text selectors cannot be stacked. Regex/PCRE2 search retains selected-page identities. Artifact recovery and next-source-page navigation remain independent.

These Resources are read-only and reject glob. Regex/PCRE2 search applies to rendered query content, never interprets the pattern as JQL, and retains selected-page identity. Clipped rendered content uses artifact recovery independently of the next native source page. Fixed issue enumeration does not expose project-key issue collection aliases. Full Atlassian Server Profile mounting/catalog publication remains unimplemented (rfs-ue44); real MCP query contracts use a private test host and do not establish shipped profile support.

### SSH

`ssh://` is read-only initially. It invokes system `ssh` directly, relies on normal host-key verification, permits only profile-allowlisted host aliases, and obeys text/output caps. No remote mutation is exposed.

### Skills and rules

Skills follow the Agent Skills `SKILL.md` format and contained relative files. Native rules use a strict versioned manifest mapping name, description, glob scope, and a contained Markdown path. `skill://` and `rule://` mutations require source grants and containment; create/update/delete operate on their authoritative files.

### Memory and vault

`memory://` is a read-only projection of a configured file or directory root.

`vault://` is an Obsidian-aware configured root with note paths, wikilinks, properties, tasks, and bases. Grant-controlled note create/write/hashline edit/`REM`/same-vault `MV` preserves unrelated frontmatter and remains canonically contained.

### Agent exports

`agent://` and `history://` read a public, strict, versioned Agent Export manifest. The manifest maps stable agent IDs, parent/child relations, status, output files, and transcript files to contained relative paths. ResourceFS never infers a foreign harness’s private directory layout. Imports are read-only and are not watched unless the source contract later versions that behavior.

### Session resources

`local://` Session Scratch is writable without Workspace Root mutation permission; “writable” describes authority, not guaranteed success past quotas. `artifact://` is immutable recovery storage produced by bounded operations. Both belong to one Path Session and expire logically at disconnect.

## Mutation, versioning, and concurrency

Mutation authority is explicit and independent of a harness approval UI:

- each Workspace Root and mutable Portable Source declares separate create, update, and delete grants;
- an adapter being configured never implies mutation permission;
- `MV` is same-source only and requires delete on the source plus create on the destination;
- cross-source moves are rejected;
- binary writes are rejected;
- `rfs_write` replacement requires `ifVersion`;
- `rfs_edit` carries the exact read header and long content-derived Version Tag in its patch;
- source adapters validate current state immediately before commit;
- concurrent mutations serialize per canonical Resource;
- filesystem/archive changes use atomic replacement, SQLite uses transactions, and remote adapters use their strongest available compare-before-write contract.

A Version Tag identifies exact authoritative content, not modification time. Read snapshots also record which source lines/projection regions were actually displayed so an edit cannot target unseen content merely by guessing line numbers.

`REM` is the only deletion form. `MV` is the only rename/move form. Failed validation leaves the Resource unchanged.

## Workspace authority and Path Sessions

One MCP connection owns one Path Session: artifacts, scratch, edit snapshots, caches, mutation operation journal, and source handles. Storage lives in the user cache directory. Disconnect invalidates session references immediately; files remain only for configured TTL cleanup/diagnosis and are garbage-collected on startup/background maintenance.

Workspace roots follow one non-union precedence rule:

1. if a client negotiates and supplies a non-empty MCP Roots set, use it;
2. otherwise use exactly one launch source: either roots from the one explicit Server Profile or roots supplied through CLI flags;
3. reject mixing profile roots with CLI roots;
4. when multiple roots exist, a profile/CLI primary-root selector may name one client-supplied or launch-supplied root; if it does not match uniquely, relative filesystem references remain disabled;
5. on a client root-set change, re-derive authority immediately, invalidate edit snapshots and `rfs://workspace/…` references tied to removed roots, and retain session-owned `local://` and `artifact://` state.

No process working directory becomes authority by accident. Kiro’s launch config uses explicit `--root workspace=.`, where `.` is resolved once at startup.

## Configuration, secrets, and source availability

At most one Server Profile is loaded through `--config`. It is strict, JSON, and carries an integer `schemaVersion`; unknown fields and duplicate/conflicting schemes fail startup. No global/project discovery, layering, or include graph exists.

The profile covers:

- source definitions and per-source `required` state;
- root and source mutation grants;
- repository/host/path allowlists;
- private-network grants;
- lower output/storage limits;
- Session cache/TTL settings;
- backing-path visibility;
- environment-variable or helper-command secret references;
- converter and downstream MCP argv definitions.

Secrets are references, never inline secret values. Credential helpers and converters run without a shell, receive a minimal baseline environment such as `PATH` plus explicitly mapped variables, have output/time caps, and never inherit the full server environment implicitly. Logs redact credentials and go to stderr or a configured file, never protocol stdout. No telemetry is emitted.

Invalid schema, unsafe paths, registration conflicts, or missing required static dependencies fail startup. Runtime-unavailable required sources fail startup. An unavailable optional source enters a visible Degraded Source state; local tools remain active and only references owned by that source fail. `resourcefs check --config …` validates static configuration; an explicit probe mode tests external connectivity without mutation.

CLI surface is intentionally small:

- `resourcefs serve`
- `resourcefs check`
- `resourcefs schema`

There is no interactive profile manager in the first release.

## Limits and recovery

The built-in Kiro-safe hard preset is:

- 48 KiB, 3,000 lines, and 512 columns per text tool result, whichever binds first;
- 5 MiB per embedded image;
- 64 MiB per artifact or scratch object;
- 256 MiB total Path Session storage;
- 1,000 entries per listing page.

A Server Profile and per-call `limits` may lower these values, never raise them above the binary’s hard ceiling.

PR Facts acquisition is independent of output `limits`. A GitHub source profile and `rfs_read` may each supply an `acquisition` object with these positive integer, lower-only dimensions:

| Field | Default and hard ceiling | Meaning |
| --- | ---: | --- |
| `maxAttempts` | 10 | Physical request attempts, including retry |
| `timeoutMs` | 30000 | One logical acquisition deadline, including waits and acceptance |
| `maxResponseBytes` | 8388608 | Bytes in one response body |
| `maxAcceptedBodyBytes` | 16777216 | Cumulative bodies admitted for use, including a reused cached body |
| `maxRepresentationBytes` | 16777216 | Complete serialized Facts JSON bytes |

Effective values are the minimum of the hard default, source policy, per-call policy, and applicable HTTP substrate ceilings. An omitted object or field uses defaults before intersection; `{}` is a valid explicit object, not a request to ignore controls. Nulls, unknown or duplicate fields, arrays, zero, and above-ceiling values are rejected. Explicit acquisition controls on a Resource without support fail with `unsupported_projection` / `acquisition_controls_unsupported` rather than being silently ignored.

Facts allow at most one retry under the original deadline and remaining attempt budget; the attempt ceiling is not a promised request count. Response/admission/representation overflow fails without publishing partial Facts JSON. Cancellation and deadline checks also guard final publication. Output limits apply afterward: a successfully acquired complete representation may spill losslessly to an artifact, but that does not raise acquisition ceilings.

Content omitted from an otherwise successful bounded operation is preserved losslessly as an immutable `artifact://` Recovery Reference and the result names exact selectors/pages. If a lossless spill would exceed object/session quota, the operation fails with a bounded limit error and narrowing instructions. ResourceFS never evicts a reference already returned by a live Path Session and never labels unrecoverable discarded content as a Recovery Reference.

Caller-authored `local://` writes are subject to the same 64 MiB object and 256 MiB Path Session ceilings. Exceeding either fails atomically with a bounded limit-exceeded error; it does not weaken Session Scratch authority or evict an existing Resource.

## Internal architecture

One Cargo workspace contains three principal crates:

1. `resourcefs-core`: Path Reference parser/selector algebra, Resource/result/error types, Behavior Contract version, policy engine, Path Session, snapshots/tags, limits, and the Source Adapter interface.
2. `resourcefs-sources`: complete compiled adapter registry and implementations for filesystem, archives, SQLite, documents/images/notebooks, HTTPS, GitHub, SSH, skills/rules, memory/vault, agent exports, session storage, and downstream MCP Resources.
3. `resourcefs-mcp`: `rmcp` protocol adapter, JSON schemas, capability negotiation, text/structured rendering, cancellation, Resources/templates/completion/subscriptions, and CLI composition.

The shipped binary contains every adapter. Adapters are internal modules behind one capability-bearing Source Adapter seam, not dynamic plugins. Configuration selects instances; it cannot load code.

Source adapters own resolution, projection, mutation, versioning, enumeration, and source-specific policy checks. The core engine owns common selector semantics, bounded results/recovery, session identity, mutation coordination, and errors. MCP-specific content types and client quirks stay in the outer adapter.

The server supports Linux, macOS, and Windows. GitHub uses native HTTP APIs; SSH uses system `ssh`; archives support ZIP/tar families; structural parsing uses a curated tree-sitter registry with text fallback; search uses Rust regex plus PCRE2. All subprocess invocation uses direct argv.

## Verification and release evidence

A 1.0 release requires:

- official MCP server conformance for negotiated supported revisions, including `2025-11-25` and `2026-07-28` where the SDK supports them;
- the real Kiro CLI scenario above, not just a protocol mock;
- Linux/macOS/Windows fixture runs for path parsing, containment, symlinks/reparse points, atomic mutation, archives, SQLite, and notebooks;
- golden Behavior Contract fixtures for every selector/reference form and text/structured result pair;
- regression fixtures for Kiro’s object-root output schemas, non-empty text fallback, inline recovery links, and bounded large results;
- adversarial tests for traversal, symlink escape, archive escape, redirect/DNS changes, private-network policy, secret redaction, stale Version Tags, operation-ID reuse, quota exhaustion, and ambiguous downstream MCP URIs;
- source-specific integration tests using local deterministic fixtures or controlled fake upstreams; live external checks remain explicit probes;
- prebuilt binary smoke tests plus `cargo install` smoke tests.

## Explicit non-goals for 1.0

- OMP runtime integration or continuing OMP conformance.
- Remote/multi-tenant ResourceFS transport.
- `omp://`, `security://`, or `xd://` emulation.
- Dynamic source plugins.
- MCP Prompt or Task surfaces.
- Downstream MCP tool/prompt proxying.
- Arbitrary SQL execution.
- Binary mutation.
- Cross-source moves.
- SSH mutation.
- Mutable memory or imported agent/history data.
- GitHub labels/status/assignees/reviews/inline-review mutation after creation.
- Interactive configuration UI.

## Sign-off

The user confirmed this document matches the intended ResourceFS product and boundaries on 2026-08-18. No design frontier remains open; implementation planning may begin from this accepted contract.
