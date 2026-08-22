# Spec: Strict Server Profile and CLI

## Request (verbatim)
> claim and implement rfs-r9m6

## What this is
ResourceFS will accept one explicit JSON Server Profile and expose `serve`, `check`, and `schema` as its complete operator CLI. The profile boundary will reject unrecognized or unsafe configuration before it can grant filesystem, process, credential, or source authority.

## Roles
- **ResourceFS operator**: authors a Server Profile, launches the stdio server, validates configuration, and inspects diagnostics without exposing credentials.
- **Harness launcher**: invokes `resourcefs serve` with either a profile or CLI Workspace Roots and requires protocol stdout to contain MCP frames only.
- **Source Adapter maintainer**: consumes typed profile entries and separate Mutation Policy grants without inventing another configuration convention.

## Behavior

### Serve from exactly one launch authority
- **Given**: either a valid schema-version-1 profile, one or more valid CLI `--root ID=PATH` arguments, or an intentionally empty scratch-only profile
- **When**: the harness launcher invokes `resourcefs serve`
- **Then**: ResourceFS loads the profile at most once, rejects any `--config`/`--root` or `--config`/`--primary-root` mix, resolves profile paths from the profile directory and CLI paths from the launch directory, starts stdio MCP with no diagnostic bytes on stdout, and uses exit code 0 for clean shutdown

### Validate the complete version-1 catalog
- **Given**: a JSON profile containing `schemaVersion`, optional grouped policy sections, and zero or more tagged Portable Source entries
- **When**: ResourceFS parses the profile
- **Then**: only integer `schemaVersion: 1`, closed objects, the ten selected source kinds, type-appropriate fields, globally non-conflicting IDs/schemes/claims, safe paths and URLs, supported grants, and values within every recorded ceiling are accepted

### Reject invalid configuration before startup
- **Given**: an unreadable or oversized profile, missing/unsupported schema version, unknown or null field value, unsafe path, conflicting source claim, inline secret value, invalid grant, above-ceiling limit, or profile/CLI root mix
- **When**: the ResourceFS operator invokes `serve --config <path>` or `check --config <path>`
- **Then**: the command exits 2 before serving or probing, stdout is empty, and bounded stderr identifies the field/reference without reproducing credential material

### Preserve Workspace Root authority
- **Given**: profile or CLI launch roots, an optional Primary Workspace Root selector, and a client that may later supply non-empty MCP Roots
- **When**: ResourceFS establishes or refreshes root authority
- **Then**: non-empty client Roots replace launch roots, launch sources are never unioned, root IDs/canonical paths remain unique, Backing-Path Visibility defaults hidden, and relative Path References work only when the selector identifies exactly one active root

### Keep mutation grants separate
- **Given**: a Workspace Root or configured Portable Source with absent or explicit grants
- **When**: the profile is checked
- **Then**: absent create/update/delete values each mean false, literal true grants only that operation, per-target grants cannot exceed their source grants, and read-only source kinds reject every true grant

### Resolve referenced secrets
- **Given**: a credential field containing a tagged environment or helper-command Secret Reference
- **When**: probing or serving resolves the credential
- **Then**: ResourceFS rejects inline material, missing/empty/NUL/non-UTF-8/oversized helper output, strips exactly one final LF or CRLF from successful helper stdout, preserves all other characters, and never emits resolved material

### Run configured commands without a shell
- **Given**: a valid non-empty argv command with optional tagged environment mappings and lower-only role limits
- **When**: ResourceFS admits the command under the 32-process global ceiling
- **Then**: argv[0] is executed directly without interpolation, the inherited environment is cleared, only present `PATH`, Windows `SystemRoot`, and explicit literal/inherit/secret mappings are supplied, stdin/stdout/stderr obey the command role, and exact-limit output succeeds

### Cancel and bound configured commands
- **Given**: a configured child exceeds its role timeout/output ceiling or its owning operation is cancelled
- **When**: ResourceFS stops the operation
- **Then**: the owned Unix process group or Windows Job Object receives whole-tree termination, survivors are force-killed after at most 1,000 ms, pipes close, every child is reaped, and a bounded typed error returns with no live descendant

### Apply profile limits, session retention, and logging
- **Given**: omitted or lower-only `limits`, `session`, and `logging` sections
- **When**: ResourceFS starts
- **Then**: omitted values use the recorded binary ceilings, no value can raise them, retained state uses only ResourceFS's namespaced cache child for 0–86,400 seconds, diagnostics use stderr or bounded rotating files, and credential redaction precedes every log write

### Check static configuration
- **Given**: a readable profile
- **When**: the ResourceFS operator invokes `resourcefs check --config <path>`
- **Then**: ResourceFS performs no network access or source mutation, validates schema/paths/grants/claims/environment-variable presence/executable resolution, and emits one deterministic redacted JSON report with `probe: false`, configured source IDs, trailing newline, empty stderr, and exit 0

### Probe configured dependencies
- **Given**: a statically valid profile
- **When**: the ResourceFS operator invokes `resourcefs check --config <path> --probe`
- **Then**: ResourceFS runs every built-in non-mutating source probe within the serving limits, emits one deterministic redacted JSON report containing each `available`, `degraded`, `failed`, or `unsupported` state, exits 0 when only optional sources are degraded, and exits 3 for any required failure or adapter unsupported by the binary

### Emit the profile schema
- **Given**: the installed binary
- **When**: the ResourceFS operator invokes `resourcefs schema`
- **Then**: stdout contains deterministic pretty JSON Schema 2020-12 for schemaVersion 1 with stable metadata, closed objects, descriptions, and one trailing newline; stderr is empty and exit code is 0

### Emit no telemetry
- **Given**: any `serve`, `check`, or `schema` invocation
- **When**: the command runs
- **Then**: ResourceFS initiates no telemetry export and diagnostics never enter protocol stdout

## Success criteria

- **Binary / structural / security**: exactly the grouped root profile and ten tagged source kinds recorded in Decisions deserialize; unknown fields/kinds, nulls, unsupported grants, duplicate kinds/IDs/schemes/claims, inline secrets, and unsafe paths/URLs fail, checked by profile contract fixtures plus emitted-schema/deserializer differential tests.
- **Quantitative**: profile input is at most 1,048,576 bytes; roots/sources are at most 256 each; allowlists at most 4,096 entries; each argv at most 256 elements/65,536 bytes; each command environment at most 256 mappings, checked at exact-limit and one-over boundaries.
- **Binary / structural / security**: profile roots and CLI roots/selectors cannot mix, and client-root replacement, unique Primary Workspace Root selection, hidden-default Backing-Path Visibility, and scratch-only startup remain observable through compiled CLI/MCP contract tests.
- **Binary / structural / security**: create/update/delete booleans remain distinct and default false; source-specific and nested target-grant constraints match Decisions, checked by schema snapshots and positive/negative grant fixtures.
- **Quantitative**: at most 32 configured process trees are live; secret helpers cap at 5,000 ms/65,536 stdout bytes/65,536 stderr bytes; converter/SSH commands cap at 30,000 ms/67,108,864 stdout bytes/1,048,576 stderr bytes; downstream MCP caps at 10,000 ms startup/30,000 ms per call/8,388,608 bytes per frame/65,536 bytes per stderr line, measured by deterministic child fixtures at exact-limit and one-over boundaries.
- **Binary / structural / security**: command fixtures observe only present `PATH`, Windows `SystemRoot`, and explicit tagged mappings; shell metacharacters remain literal argv bytes and helper commands cannot recursively resolve secrets.
- **Quantitative**: cancellation/timeout leaves zero live fixture descendants after the 1,000 ms grace interval on every supported platform, measured by PID/job-handle reconciliation.
- **Binary / structural / security**: zero secret sentinel values appear in stderr, rotating logs, JSON check reports, or MCP stdout across success and every fixture failure, checked by captured-stream/file scanning.
- **Quantitative**: profile text/read/discovery/image/object/session values cannot exceed 49,152 bytes, 3,000 lines, 512 columns, 1,000 search/glob/listing entries, 5,242,880 image bytes, 67,108,864 object bytes, and 268,435,456 session bytes; retention cannot exceed 86,400 seconds, checked at exact-limit and one-over boundaries.
- **Quantitative**: file logging rotates at or before 10,485,760 bytes and retains exactly the configured 1–10 files without touching sibling paths, measured by deterministic log fixtures.
- **Binary / structural / security**: `serve`, `check`, and `schema` are the only top-level commands; success/config/unavailable/internal outcomes use exit codes 0/2/3/1; stdout/stderr match the Behavior triples, checked by compiled-binary CLI tests.
- **Binary / structural / security**: profile parsing is panic-free and schema-consistent for generated input, checked by `cargo fuzz run server_profile -- -max_total_time=60` plus the deterministic profile corpus.
- **Binary / structural / security**: no telemetry dependency, endpoint, background exporter, or protocol-stdout diagnostic exists, checked by architecture dependency assertions and real-binary captured-output/network-denial smoke.

## Out of scope

This change does NOT include an interactive profile manager, profile discovery, profile layering, include files, hot reload, dynamic Source Adapter plugins, telemetry, remote ResourceFS transport, downstream tool or prompt proxying, implementation of Source Adapters owned by dependent issues, workspace mutation itself, or raising any binary hard ceiling.

## Related issues

- rfs-jk0d: accepted parent contract for one strict JSON profile, separate grants, referenced secrets, direct argv, required/degraded sources, the three-command CLI, hard ceilings, and no telemetry.
- rfs-cgbq: completed client-over-launch root precedence, unique primary selection, profile/CLI launch-source distinction, and Backing-Path Visibility behavior that this profile must compose.
- rfs-87cv: completed installable `resourcefs serve`, clean stdio MCP, and explicit CLI Workspace Roots; this change extends that public binary.
- rfs-34pz: completed hard text/object/session ceilings and cancellation fences; its note assigns lower profile limits and configurable session TTL to rfs-r9m6.
- rfs-vl0u: completed prompt typed cancellation and bounded recovery behavior that configured commands must preserve.
- rfs-73dz: consumes the separate Workspace Mutation grants but owns mutation behavior and atomic writes.
- rfs-60g1: establishes that Session Scratch mutation remains independent of Workspace Mutation grants.
- rfs-9o5v: consumes direct-argv converter definitions, explicit environments, secret references, timeout/output caps, and cancellation.
- rfs-vl7z: consumes system-SSH command configuration, host aliases, explicit environments, caps, and cancellation.
- rfs-g2z9: consumes HTTPS source/credential/network policy and owns required-versus-Degraded Source runtime and connectivity-probe behavior.
- rfs-btji: consumes a typed Agent Export source definition and contained manifest path.
- rfs-uo2w: consumes typed file/directory Memory Source definitions.
- rfs-azd3: consumes direct-argv downstream MCP definitions and required/degraded source configuration.
- rfs-45ww: consumes secret references and repository/network policy for GitHub.
- rfs-by2z: defines GitHub create/update scope and excludes delete/status/review mutation.
- rfs-cdlp: consumes skill roots, rule manifests, and full create/update/delete Source Mutation grants.
- rfs-nb4s: consumes named vault roots and per-vault subset grants.
- rfs-5os7: makes the complete public CLI and protocol-stdout behavior part of the Kiro release gate.
- rfs-58r1: requires packaged binaries and `cargo install` to expose the same CLI and compiled source configuration surface.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| Who operates this feature? | A ResourceFS operator authors and checks profiles; a harness launcher starts stdio serving; Source Adapter maintainers consume typed entries. | rfs-jk0d stories 2, 6, 20, 60–61, and 108–115. | Diagnostics, schema output, and authority failures are operator-visible process contracts. |
| How many profiles are loaded and discovered? | Zero or one explicit `--config` profile; no discovery, layering, or includes. | Accepted rfs-jk0d implementation decision and `DESIGN.md:225-251`. | Configuration precedence has one file boundary and no merge semantics. |
| How do client, profile, and CLI roots compose? | Non-empty client Roots replace launch roots; absent client Roots use profile roots or CLI roots, never both. | rfs-cgbq completed contract. | `serve` rejects mixed profile/CLI launch authority and reuses existing root refresh behavior. |
| What does configuring a mutable target authorize? | Nothing by itself; create, update, and delete remain independent for Workspace Roots and Portable Sources. | rfs-jk0d and rfs-73dz. | The schema cannot collapse grants into a `writable` boolean. |
| Where may credentials appear? | In environment-variable or helper-command references, never inline profile values. | rfs-jk0d stories 110–112. | Parsing and logs can identify a reference but cannot serialize resolved secret material. |
| How are configured commands invoked? | Executable plus argv without a shell, with inherited environment cleared, a documented baseline plus explicit mappings, fixed timeout/output ceilings, and cancellation cleanup. | rfs-jk0d plus rfs-9o5v, rfs-vl7z, and rfs-azd3. | One shared bounded command contract must be proven before dependent adapters use it. |
| Can profile limits raise built-in ceilings? | No; they may only lower hard ceilings. | rfs-34pz and its note on rfs-r9m6. | Above-ceiling values fail profile checking rather than being clamped. |
| Does this change implement workspace or Portable Source mutation? | No; it defines and validates policy consumed by rfs-73dz and later source issues. | Tracker dependency boundaries. | Grant tests cover representation and validation, not authoritative writes. |
| Is Session Scratch controlled by profile mutation grants? | No. | rfs-60g1 and parent contract. | Profile grants cannot disable or broaden Session Scratch authority. |
| Is configuration reloaded while serving? | No; the profile is loaded once at process startup. | Parent excludes discovery/layering and defines startup validation; no accepted reload surface exists. | Concurrent profile writes and cache invalidation are N/A for this change. |
| Are soft deletion or replication lag part of profile loading? | N/A — profiles have no record lifecycle or replicated state. | Local startup configuration boundary. | Those edge dimensions add no profile behavior. |
| What happens when a required or optional external source is unavailable? | Required sources fail serving/probing; optional sources report Degraded state and fail only their references. | rfs-jk0d; runtime realization is owned by rfs-g2z9 and later adapters. | This issue must preserve the typed `required` setting without fabricating unavailable adapters. |
| Which source kinds does schema version 1 define before their adapters land? | The complete planned ResourceFS 1.0 source catalog. | Requester selected “Complete v1 catalog.” | `schema` and `check` define and validate every planned source shape now; `serve` rejects a configured source kind until its compiled adapter is implemented. |
| How are Portable Source instances represented? | One `sources` array whose entries form a strict `kind`-tagged union with common `id`, `required`, and grants fields plus kind-specific fields. | Requester selected “Tagged source array.” | Unknown kinds and kind-inappropriate fields fail; source IDs and claimed schemes are checked globally rather than per family. |
| How is the profile root object organized? | Grouped policy sections: `schemaVersion`, `workspace`, `limits`, `session`, `logging`, and `sources`; Workspace Roots carry their grants and source entries carry source-local secret/command references. | Requester selected “Grouped policy sections.” | Unknown fields are rejected within each section; the schema has no top-level secret or command registry and therefore no registry indirection. |
| Which planned families are entries in `sources`? | `https`, `github`, `ssh`, `documents`, `skills`, `rules`, `memory`, `vault`, `agentExport`, and `downstreamMcp`. Archives, SQLite, images, notebooks, Session Scratch, and artifacts are not configured source instances. | Requester selected “Configured families only”; this matches the parent’s distinction between Portable Sources, workspace projections, and session-owned Resources. | The tagged union enumerates ten kinds; automatic workspace projections and internal sources cannot be disabled or granted through `sources`. |
| May a profile omit Workspace Roots and configured sources? | Yes. Optional sections and arrays may be absent or empty; JSON `null` is rejected. | Requester selected “Allow scratch-only.” | `serve` may start without launch Workspace authority, preserving internal/session sources and future source-only deployments. |
| How are secret references encoded? | A strict tagged object: `{ \"kind\": \"environment\", \"name\": \"…\" }` or `{ \"kind\": \"command\", \"command\": { … } }`. | Requester selected “Tagged reference object.” | Inline secret strings are not a schema alternative; unknown reference kinds and extra fields fail validation. |
| How are direct executable commands represented? | A non-empty `argv` array whose first element is the executable and remaining elements are exact arguments, with explicit environment and lower-only limit fields beside it. | Requester selected “Non-empty argv array.” | Empty arrays fail; no command string or shell mode exists; argv elements are passed without interpolation. |
| How are child environment variables configured beyond the baseline? | A map from child variable name to a strict tagged `literal`, `inherit`, or `secret` value object. Secret-helper commands permit only `literal` and `inherit` values. | Requester selected “Tagged value map.” | Variable renaming is explicit, duplicate child names are impossible in valid JSON, and helper-secret resolution cannot recurse through another secret reference. |
| Which parent environment variables are inherited automatically? | `PATH` when present on every platform and `SystemRoot` on Windows; no other variable is automatic. | Requester selected “PATH plus SystemRoot.” | Home, locale, proxy, credential-agent, temporary-directory, and tool-specific variables require explicit tagged mappings; the empirical probe must verify the Windows baseline. |
| How are command limits divided? | Tiered binary defaults/maxima by role: secret helper, one-shot converter/SSH, and downstream MCP startup/per-call/frame; profiles may only lower them. | Requester selected “Tiered hard ceilings.” | Long-lived downstream servers are not forced into one-shot stdout/timeout semantics, and credential helpers cannot inherit document-sized budgets. |
| What are the binary command ceilings when the profile does not lower them? | Secret helpers: 5,000 ms, 65,536 stdout bytes, 65,536 stderr bytes. Converter/SSH one-shot commands: 30,000 ms, 67,108,864 stdout bytes, 1,048,576 stderr bytes. Downstream MCP: 10,000 ms startup, 30,000 ms per call, 8,388,608 bytes per protocol frame, and 65,536 bytes per stderr line. | Requester selected “Balanced ceilings.” | Exact-limit output succeeds; one byte or one millisecond beyond a configured/binary ceiling fails and triggers child-tree cleanup. |
| How does successful secret-helper stdout become a credential? | Require UTF-8, remove one final LF or CRLF, reject an empty result and NUL, and preserve every other character. | Requester selected “Strip one line ending.” | Ordinary line-printing helpers work without whitespace-wide trimming; embedded or additional newlines remain part of the secret. |
| What does `resourcefs check --config <path>` print on success? | One deterministic redacted JSON object on stdout for both static and `--probe` forms; static reports validity and configured source IDs, while probe reports each source state. Successful stderr is empty. | Requester selected “Deterministic JSON report.” | Check output is automation-safe; field ordering and newline termination are stable contract fixtures, and diagnostics never mix into the report. |
| How does `check --probe` handle partial failure? | Probe every configured source within its limits. Optional runtime failures are `degraded` and do not make `ok` false; required failures or an adapter unsupported by the binary make `ok` false and the process unsuccessful. | Requester selected “Probe all sources.” | The JSON report is exhaustive and distinguishes optional degradation from a profile the current binary cannot serve. |
| What parser/cardinality ceilings apply before allocation or execution? | 1,048,576 profile bytes; 256 Workspace Roots; 256 configured sources; 4,096 entries per allowlist; 256 argv elements and 65,536 total argv bytes per command; 256 environment mappings per command. | Requester selected “Balanced profile ceilings.” | Exact ceilings succeed; one-over values fail static checking, and parsing never reads an unbounded profile into memory. |
| What is the base for relative paths inside a profile? | The canonical parent directory of the profile file. Bare command names use the configured child `PATH`; command argv[0] values containing a path separator resolve from the profile directory. | Requester selected “Relative to profile file.” | Launch working directory cannot change profile authority; CLI `--root` retains its existing launch-directory behavior. |
| How are lower-only ResourceFS limits represented? | Nested `limits.text`, `limits.discovery`, `limits.imageBytes`, and `limits.storage` groups. Omitted fields use binary ceilings; present values are positive; `storage.objectBytes` cannot exceed `storage.sessionBytes`. | Requester selected “Nested contract groups.” | The exact maxima remain 49,152 bytes/3,000 lines/512 columns, 1,000 search matches/glob entries/listing entries, 5,242,880 image bytes, 67,108,864 object bytes, and 268,435,456 session bytes. |
| What does the `session` section control? | Optional `cacheDirectory` and `retentionTtlSeconds`; defaults are the OS cache directory and 86,400 seconds. TTL accepts 0 through 86,400; ResourceFS owns and cleans only its namespaced child directory. | Requester selected “Cache base plus zero-to-day TTL.” | Zero requests deletion at disconnect; a profile cannot extend retained state beyond 24 hours or make cleanup traverse an operator-owned directory. |
| What does the `logging` section permit? | A strict `stderr` or rotating `file` destination plus `error`, `warn`, `info`, or `debug` level. File rotation defaults/maxes at 10,485,760 bytes and retains 1–10 files; every message passes credential redaction. | Requester selected “Stderr or rotating file.” | Logs remain bounded and off protocol stdout; relative file paths use the profile directory and rotation never renames or deletes outside the configured log family. |
| Which process exit codes are public? | 0 for success or clean serve shutdown; 2 for CLI usage, unreadable profile, or static validation; 3 for failed required probes or an adapter unsupported by this binary; 1 for internal/runtime failure. Optional degraded probes exit 0. | Requester selected “Small semantic set.” | Automation can distinguish author errors, unavailable configured capabilities, and product failures without platform-specific sysexits values. |
| What does `resourcefs schema` emit? | With no arguments, deterministic pretty-printed JSON Schema 2020-12 for schemaVersion 1, including stable `$schema`, `$id`, title, descriptions, closed objects, and one trailing newline. | Requester selected “Pretty current schema.” | Stdout is diffable and machine-readable; stderr is empty and exit code is 0 on success. |
| How are create, update, and delete grants encoded? | Optional `grants` object with optional `create`, `update`, and `delete` booleans; every absent value means false and only literal `true` grants an operation. | Requester selected “Optional false booleans.” | No `writable` shorthand exists; JSON `null`, non-booleans, and grants unsupported by a source kind fail static validation. |
| What fields define Workspace authority? | Optional `workspace` contains `roots`, `primaryRoot`, and `backingPathVisibility`; each root contains `id`, `path`, and optional grants. Roots default empty, visibility defaults `hidden`, and `primaryRoot` may select a later client-supplied root. | rfs-cgbq and the selected grouped profile shape. | Root IDs and canonical paths are globally unique; every configured root path must resolve to an existing contained directory before serving. |
| Is each Portable Source's required/optional state explicit? | Yes; every source entry must contain `required: true` or `required: false`. | Requester selected “Required boolean.” | Missing or null `required` fails schema validation; optional degradation is never inferred from omission. |
| May a profile repeat a source kind? | No; at most one entry per built-in kind, and that entry contains the kind's hosts, roots, converters, repositories, or downstream servers. | Requester selected “One fixed-scheme entry.” | Duplicate kinds fail static validation; fixed scheme ownership and required/degraded state are per kind rather than per nested item. |
| What fields define the `https` source? | A non-empty `origins` list; each item has an HTTPS `baseUrl` path prefix, explicit `allowPrivateNetwork` boolean, and optional credential header containing `header`, optional `scheme`, and a Secret Reference. | Requester selected “Allowed base URLs.” | Userinfo, query, fragment, wildcard hosts, non-HTTPS URLs, duplicate/overlapping base prefixes, routing/hop-by-hop credential headers, and any true mutation grant fail static validation; redirects and resolved addresses are reauthorized against the same entries. |
| What fields define the `github` source? | HTTPS `apiBaseUrl`, explicit `allowPrivateNetwork`, one Secret Reference credential, and a non-empty repository allowlist. Source-level grants cap per-repository grants, which must be subsets. | Requester selected “Two-level grants.” | Repository names are canonical `owner/repo`, duplicates fail, public GitHub is the default API base, and GitHub Enterprise requires explicit network authority without broadening any repository grant. |
| What fields define the `ssh` source? | One shared direct-argv SSH command/environment definition and a non-empty `hosts` list; each exact SSH config alias has non-empty absolute remote root prefixes. | Requester selected “Shared command plus host roots.” | ResourceFS appends controlled arguments, retains normal SSH config and host-key checks, rejects overlapping/duplicate aliases or roots, and rejects every true mutation grant. |
| What fields define the `documents` source? | A non-empty `converters` list; each converter claims disjoint lowercase extensions, chooses `stdin` or appended canonical `path` input, and provides one direct command whose UTF-8 stdout is the text projection. | Requester selected “Typed stdin or appended path.” | No argv placeholders exist; path mode appends exactly one contained backing path; overlapping extensions, non-UTF-8 output, true grants, or converters without extensions fail. |
| What fields define `skills` and `rules` sources? | `skills` contains non-empty canonical root directories scanned for standard `SKILL.md` packages; `rules` contains non-empty explicit strict manifest paths. Source-level grants apply to every contained item. | Requester selected “Skill roots, rule manifests”; rfs-cdlp supplies the content contract. | Duplicate names across roots/manifests, invalid manifests, missing paths, or escaping links fail; rule discovery adds no filename convention. |
| What fields define the `memory` source? | A non-empty `roots` list of explicit `{ name, path }` entries; each path resolves to one contained file or directory and the name is the stable first reference segment. | Requester selected “Named roots”; rfs-uo2w owns read/search behavior. | Duplicate names or canonical targets, missing paths, escaping links, and every true mutation grant fail; renaming a backing path does not silently rename its Path Reference. |
| What fields define the `vault` source? | A non-empty `vaults` list of stable name, canonical directory path, and per-vault grants constrained by source-level grants. | Requester selected “Named vaults with subset grants”; rfs-nb4s owns projection/mutation behavior. | Duplicate names/targets and broader per-vault grants fail; one profile can keep one vault read-only while granting another selected operations. |
| What fields define the `agentExport` source? | A non-empty explicit `manifests` path list; each manifest is strict/versioned and all agent IDs are globally unique across the list. | Requester selected “Manifest list, global IDs”; rfs-btji owns projections. | Canonical references remain `agent://<id>` and `history://<id>` without profile prefixes; duplicates, missing/unsafe manifests, and every true mutation grant fail. |
| What fields define the `downstreamMcp` source? | A non-empty `servers` list; each stable ID declares unique native-scheme claims and a strict tagged `stdio` direct-command or `http` HTTPS endpoint/credential/private-network transport. | Requester selected “Tagged server transports”; rfs-azd3 owns Resource proxy behavior. | Server IDs and scheme claims are globally unique and cannot shadow built-ins; HTTP userinfo/non-HTTPS URLs, true mutation grants, and any tool/prompt proxy configuration fail. |
| How are source probes defined? | ResourceFS owns one non-mutating probe per kind; profiles cannot supply probe commands. Local kinds open/validate configured paths/manifests, secret references resolve, network kinds perform bounded read-only connectivity, SSH runs controlled remote `true`, downstream MCP initializes and lists Resources, and document probes resolve executables without conversion. | Requester selected “Built-in per kind.” | Probe mode cannot become an arbitrary execution surface; every result is one of `available`, `degraded`, `failed`, or `unsupported` with a redacted diagnostic. |
| What child-process cleanup follows timeout or cancellation? | Put each command in an owned Unix process group or Windows Job Object; request whole-tree termination, wait at most 1,000 ms, force-kill survivors, close pipes, and reap before returning. | Requester selected “Terminate tree, then force.” | No descendant or pipe reader remains live after the bounded error; empirical probes must verify descendant cleanup on supported platforms. |
| Which source kinds may carry true mutation grants in version 1? | `skills`, `rules`, and `vault` may use create/update/delete. `github` may use create/update but not delete. `https`, `ssh`, `documents`, `memory`, `agentExport`, and `downstreamMcp` are read-only. | rfs-cdlp, rfs-nb4s, rfs-by2z, and the accepted parent exclusions. | Static validation rejects unsupported grants before an adapter can interpret them. |
| What grammar and bounds apply to profile-controlled IDs? | 1–128 ASCII bytes using the existing Workspace Root ID grammar: first byte alphanumeric, remaining bytes alphanumeric, `.`, `_`, or `-`, case-sensitive. Native scheme claims use RFC URI-scheme grammar, lowercase-normalized, at most 64 bytes. | Requester selected “Bounded existing grammar.” | The same parser serves root/source/vault/memory/downstream IDs; case-fold collisions are checked where the upstream namespace is case-insensitive. |
| How many configured child process trees may be live? | At most 32 across helpers, converters, SSH, and downstream stdio; profiles may lower the ceiling. Admission waits within the owning timeout and then returns `limit_exceeded`. | Requester selected “Global 32.” | Profile cardinality cannot translate into unbounded process creation, and persistent downstream children consume permits for their lifetime. |
| How are empty and missing values handled? | The profile may omit optional grouped sections or use empty arrays for scratch-only operation; every present source kind requires its non-empty kind-specific collection, `id`, and `required`; JSON null is rejected everywhere. | Selected empty-profile, source-catalog, and strict-schema decisions. | Empty authority is intentional, while a half-specified configured source cannot masquerade as valid. |
| What happens under concurrent configuration/log/process activity? | Serving uses one immutable profile snapshot; logging rotation is serialized; child admission uses the global 32-permit ceiling; `check` invocations are independent readers. | No hot reload plus selected logging/concurrency contracts. | Profile writes are outside the process contract and cannot race live authority updates. |
| How are permission failures reported? | Filesystem, environment, executable, network, and log-destination denial is bounded/redacted; static author/permission failures exit 2 and required runtime/probe unavailability exits 3. | CLI exit and redaction decisions. | Permission denial cannot fall back to broader authority or leak a secret/path payload. |
| Are probes or configured commands retried automatically? | No; each check/operation makes one bounded attempt. | Parent no-automatic-retry principle and non-mutating probe contract. | Repeated `check` is caller-controlled; unknown external outcomes are never hidden by an implicit retry. |
| How do time zones and DST affect retained-session cleanup? | They do not; retention is an elapsed whole-second duration from 0 through 86,400 and persisted timestamps are UTC. | Selected session TTL contract. | Wall-clock display shifts do not extend authority or retention. |
| What is the multi-tenancy boundary? | N/A — one local operator profile configures one process; each MCP connection still owns an isolated Path Session. | ADR 0003 and local stdio product scope. | The profile is shared process policy, while scratch/artifacts/snapshots never cross Path Sessions. |
| What is invalidated when configuration changes on disk? | Nothing in the live process; serve does not reload. A later `check` or serve invocation rereads the file, while client Roots continue using the rfs-cgbq refresh contract. | No-hot-reload decision. | There is no configuration cache invalidation protocol to implement. |
| What are logging defaults? | Omitted logging means `info` to stderr. A file destination defaults to 10,485,760 bytes and three retained files; profiles may lower size and choose 1–10 files. | Selected rotating-file contract, completed with fail-closed defaults. | A profile without logging fields remains bounded and protocol-clean. |
| What is the exact successful `check` report shape? | One object with `ok`, `schemaVersion`, `probe`, and `sources`; each source row has `id`, `kind`, `required`, `state`, and an optional redacted `diagnostic`. Static rows use `notProbed`; completed probe reports keep stderr empty even when `ok` is false. | Selected deterministic JSON and exhaustive probe contracts. | Static/profile parse failures still produce empty stdout and stderr diagnostics; a completed probe is machine-readable as one JSON document. |
| What stable schema metadata is emitted? | JSON Schema draft 2020-12 with `$id` `https://resourcefs.dev/schema/server-profile-v1.json` and title `ResourceFS Server Profile v1`. | Selected pretty current-schema contract. | Schema snapshots have a stable identity independent of the product patch version. |

## Approval

Requester approval (verbatim): "Agree"
Date: 2026-08-20
