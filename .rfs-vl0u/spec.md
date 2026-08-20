# Spec: Bounded workspace and artifact discovery

## Request (verbatim)
> claim and implement rfs-vl0u

## What this is
ResourceFS will expose `rfs_search` for content matches and `rfs_glob` for structure discovery over authorized Workspace and immutable Artifact Resources. Both tools will return deterministic bounded results without requiring a preceding full read, and every omitted successful result will remain recoverable within the live Path Session.

## Roles
- **Coding agent**: invokes `rfs_search` and `rfs_glob` through an MCP harness and receives compact matches, canonical Path References, pagination state, and Recovery References.
- **MCP harness**: discovers object-rooted tool schemas, supplies cancellation, and receives complete non-empty text plus equivalent structured content.
- **ResourceFS operator**: declares Workspace Roots and expects discovery to preserve containment, hard ceilings, and Path Session quotas.

## Behavior

### Search one text Resource
- **Given**: a non-empty pattern of at most 65,536 UTF-8 bytes and one authorized UTF-8 Workspace file or immutable Artifact Resource, with optional `caseSensitive`, `skip`, and lower-only `limits`.
- **When**: the Coding agent calls `rfs_search`.
- **Then**: ResourceFS compiles the pattern with Rust regex first and retries with PCRE2 only when Rust rejects the syntax; returns `engine` as `rust_regex` or `pcre2`; and returns one group for the canonical Resource containing each matching line once as a 1-based `line` plus line text without its terminator, ordered by line number.

### Search a Workspace directory
- **Given**: a Workspace directory target containing searchable text, hidden or ignored descendants, unreadable or non-UTF-8 descendants, and contained or escaping links.
- **When**: the Coding agent calls `rfs_search` with optional `caseSensitive`, `gitignore`, `hidden`, `skip`, and lower-only `limits`.
- **Then**: ResourceFS recursively searches included text Resources without a preceding `rfs_read`; returns canonical Resource groups in bytewise reference order; follows contained links while deduplicating final identities and cycles; never returns outside-root content; and reports each skipped unreadable, unsupported, disappearing, or escaping Resource in an ordered `diagnostics` entry with canonical reference when one can be resolved, stable category, and message.

### Glob Workspace structure
- **Given**: one non-empty relative or canonical Workspace glob using `/`, `*`, `?`, character classes, recursive `**`, or `{a,b}` alternation.
- **When**: the Coding agent calls `rfs_glob` with optional `caseSensitive`, `gitignore`, `hidden`, `skip`, and a lower-only result limit.
- **Then**: ResourceFS resolves the fixed path prefix under exactly one Workspace Root; matches paths using globset-compatible component semantics; returns each final canonical identity once with `kind` equal to `file` or `directory`; renders directories with a trailing `/` in text; sorts by canonical reference; follows only contained links; and reports non-fatal traversal failures through ordered diagnostics.

### Glob live Artifact Resources
- **Given**: one `artifact://` glob and a Path Session containing zero or more immutable artifacts.
- **When**: the Coding agent calls `rfs_glob`.
- **Then**: ResourceFS snapshots and matches only that Path Session's pre-existing canonical Artifact References, returns each as `kind: artifact`, returns no synthetic directories, and does not include a Recovery Artifact created for this same result.

### Filtering and explicit targets
- **Given**: omitted discovery controls or a directly named Resource that would be hidden or ignored during recursive traversal.
- **When**: either discovery tool runs.
- **Then**: matching defaults to case-sensitive, `gitignore` defaults to true, and `hidden` defaults to false; `caseSensitive: false` performs Unicode-aware case folding for content search and ASCII-only case folding for path globbing; contained `.gitignore` files from the Workspace Root through nested target directories control descendant traversal; `.ignore`, `.git/info/exclude`, and global Git excludes have no effect; a directly named exact search target is attempted regardless of descendant filters; and hidden means any descendant path with a component beginning `.`.

### Empty and no-match results
- **Given**: a valid target and pattern or glob that matches no Resource or line.
- **When**: discovery completes without diagnostics.
- **Then**: the tool returns `ok: true`, zero groups or entries, zero returned and total records, complete non-empty text stating that no matches were found, and no Recovery or continuation reference.

### Pagination, bounds, and recovery
- **Given**: a deterministic complete result, `skip` defaulting to zero, and omitted or supplied lower-only limits.
- **When**: the result is rendered.
- **Then**: search applies `limits.maxResults`, `maxBytes`, `maxLines`, and `maxColumns` against hard ceilings of 1,000 records, 48 KiB, 3,000 lines, and 512 columns; glob applies `limits.maxResults` against 1,000 entries; `skip` counts matching-line records globally for search and entries for glob; a skip at or beyond the total returns an empty inline page; and any omitted successful result causes one complete deterministic text rendering to be retained as an immutable Artifact Resource with a Recovery Reference and, when later records remain, a continuation reference readable through `rfs_read`.

### Validation and operational failure
- **Given**: missing required fields, null or wrong-typed fields, unknown fields, an empty pattern/glob, a search pattern longer than 65,536 UTF-8 bytes, a pattern rejected by both regex engines, invalid glob syntax, a failed exact target, a result too large to retain, or MCP request cancellation.
- **When**: either discovery tool validates or executes the request.
- **Then**: schema-shape failures are MCP `invalid_params`; empty, doubly rejected, or syntactically invalid pattern/glob failures are `invalid_pattern` tool errors; a search pattern longer than 65,536 UTF-8 bytes is `limit_exceeded` before source I/O or either regex compiler runs; existing Path Reference, containment, source, projection, and quota categories remain precise; an exact-target failure is a tool error rather than a partial success; an unretainable omitted result is `limit_exceeded` with no false Recovery Reference; and cancellation returns `cancelled` within 250 ms in the controlled cancellation fixture without publishing a late artifact.

### Concurrent Workspace changes
- **Given**: Workspace entries are created, replaced, or removed while discovery is running.
- **When**: ResourceFS reaches each entry.
- **Then**: ResourceFS revalidates authority and final-handle containment, observes each successfully opened entry once, reports entries that disappear or fail as diagnostics, and makes no whole-tree snapshot claim; repeated calls are byte-for-byte deterministic when source state and input are unchanged.

## Success criteria

- **Binary / structural / security**: plain text, Rust-regex metacharacters, and a PCRE2-only lookbehind fixture return the exact expected matching lines and report the selected engine, checked by core engine contracts and the compiled stdio MCP suite.
- **Binary / structural / security**: Unicode-insensitive search, ASCII-insensitive glob, contained `.gitignore`, hidden, explicit-target, skip, and every lower-only limit boundary produce the specified records and identical ordering in five repeated runs, checked by direct Filesystem Source Adapter and MCP contracts.
- **Binary / structural / security**: relative and canonical Workspace globs plus `artifact://*` return the exact canonical file/directory/artifact set and kinds, checked by direct Filesystem and Artifact Source Adapter contracts.
- **Binary / structural / security**: lexical traversal, escaping links or reparse points, link cycles, aliases, and concurrent retargeting never return an outside byte or duplicate final identity, checked by native Linux, macOS, and Windows adapter fixtures.
- **Quantitative**: inline search output never exceeds 49,152 bytes, 3,000 lines, 512 columns, or 1,000 matching-line records, and inline glob output never exceeds 1,000 entries, measured by exact-boundary and one-over contract fixtures.
- **Quantitative**: a synthetic 100,000-Resource traversal with 10,000 matching lines and an initial `maxResults` of 100 returns exactly 100 inline records and a Recovery Artifact whose complete rendering contains all 10,000 records in canonical order, measured by the core stress fixture.
- **Binary / structural / security**: a complete rendered result at the 64 MiB artifact ceiling is recoverable, while one byte over returns `limit_exceeded`, publishes no reference, and does not evict prior live artifacts, checked by Path Session quota contracts.
- **Quantitative**: cancellation during deliberately held traversal or matching returns the `cancelled` tool error within 250 ms and publishes no late artifact, measured from cancellation-token notification through response receipt in the compiled stdio fixture.
- **Binary / structural / security**: empty or doubly rejected regex patterns and malformed globs return `invalid_pattern`; search patterns at 65,536 bytes compile normally while 65,537 bytes return `limit_exceeded` before source I/O or regex compilation; malformed tool objects return MCP `invalid_params`; exact-target and partial-directory failures follow their specified surfaces, checked by core, adapter, and stdio error contracts.
- **Binary / structural / security**: both Source Adapters are exercised directly and both public tools are exercised through the compiled `resourcefs` process with complete non-empty text and equivalent object-rooted structured content, checked by the focused source and stdio contract suites.

## Out of scope

This change does NOT include: discovery for Session Scratch, archives, notebooks, converted documents, databases, network sources, Vaults, Memory, Skills/Rules, GitHub, SSH, Agent Exports, or downstream MCP; indexes, caches, watchers, or background scans; occurrence columns, match context, replacements, or Structural Summary search; multi-target or semicolon input; shell extglobs; `.ignore`, repository-local Git excludes, or global Git excludes; Workspace Mutation; MCP Resource listing; or cross-Path-Session Artifact discovery.

## Related issues

- rfs-jk0d: accepted parent product contract; fixes the five-tool surface, source-neutral Source Adapter seam, hard ceilings, Rust-regex-plus-PCRE2 direction, and direct adapter/public MCP tests.
- rfs-34pz: closed prerequisite; supplies immutable Artifact Resources, lower-only text limits, Path Session quotas, selector recovery, cancellation fences, and lossless spill behavior adopted here.
- rfs-cgbq: closed prerequisite to rfs-34pz; supplies relative/absolute/`file://`/canonical workspace resolution, Primary Workspace Root behavior, root containment, and root-change authority behavior adopted here.
- rfs-87cv: closed MCP/read slice; supplies object-rooted schemas, complete text fallbacks, operational error rendering, request cancellation wiring, and the compiled stdio contract-test seam adopted here.
- rfs-5os7: future release gate; requires a real Kiro session to prove search and bounded recovery through the public seam.
- rfs-ww6w: future Resource Mirror work; its independently paginated resource listing must remain distinct from canonical `rfs_glob` tool discovery.
- rfs-r9m6: future Server Profile work; it may lower binary result limits but may not raise the hard ceilings implemented here.
- rfs-60g1: future Session Scratch adapter will consume the search contract; `local://` discovery is not implemented in this issue.
- rfs-trdn: future archive adapter will consume both search and glob contracts; archive-member discovery is not implemented in this issue.
- rfs-xiwg and rfs-9o5v: future notebook and converted-document adapters will expose searchable projections through the common search contract, not through this issue.
- rfs-btji, rfs-nb4s, rfs-uo2w, rfs-cdlp, rfs-vl7z, and rfs-g2z9: future Agent Export, Vault, Memory, Skill/Rule, SSH, and HTTPS adapters depend on the common discovery contract; their source-specific behavior is not implemented here.
- rfs-45ww and rfs-c85i: future GitHub and SQLite Source Adapters require common bounded listing/search conventions but retain source-specific projection semantics outside this issue.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| Which public tools expose this feature? | Exactly `rfs_search` and `rfs_glob`; no additional discovery tool. | rfs-jk0d accepted product surface. | MCP schemas, text rendering, and structured results are added under those two names. |
| Which Source Adapters are implemented now? | `FilesystemSource` and `ArtifactSource` only. | rfs-vl0u acceptance criteria name both; downstream tracker issues own later sources. | The source-neutral contract supports later adapters, but this build contains no speculative implementations for them. |
| What identity and containment rules apply to workspace input? | Reuse the complete Path Reference and Workspace Root rules from rfs-cgbq; canonical targets may not escape authorized roots. | Closed prerequisite rfs-cgbq. | Relative input resolves only under the Primary Workspace Root, and canonical output uses `rfs://workspace/<root>/...`. |
| What client result compatibility applies? | Every result has complete non-empty text and equivalent object-rooted structured content; operational failures use stable semantic categories. | rfs-jk0d and rfs-87cv. | Kiro-compatible text cannot omit recovery or error information carried by structured content. |
| What hard result ceilings apply? | Search output is bounded by 48 KiB, 3,000 lines, 512 columns, and 1,000 matching-line records per page; glob pages contain at most 1,000 entries. Per-call fields may lower but never raise these values. | rfs-jk0d and DESIGN.md limits contract. | Invalid higher limits fail schema/validation rather than silently increasing resource use; whichever byte, line, column, or result ceiling binds first ends the inline page. |
| What happens to omitted successful output? | Spill the complete deterministic result losslessly to a new immutable Artifact Resource, subject to the 64 MiB object and 256 MiB Path Session ceilings; never claim discarded output is recoverable. | rfs-34pz and rfs-jk0d. | Bounded responses name valid Recovery References, and spill quota failure is `limit_exceeded`. |
| Does this issue add future source behavior? | No; future adapters consume the common contract in their verified tracker issues. | Tracker taxonomy and the downstream issues listed above. | Archive, notebook, document, local scratch, network, database, and other source-specific semantics remain out of scope. |
| What hard input ceiling applies to `rfs_search.pattern`? | Accept at most 65,536 UTF-8 bytes and return `limit_exceeded` before source I/O or regex compilation for a larger pattern. | Requester selected “64 KiB”; this reuses the Path Reference ceiling and bounds untrusted compiler input. | Exact-boundary and one-over fences cover the cap; glob paths remain subject to the existing 64 KiB Path Reference ceiling. |
| How does `rfs_search` select literal, Rust-regex, and PCRE2 matching? | Automatic fallback: compile every pattern with Rust regex first; retry with PCRE2 only when Rust rejects the syntax; plain text with no metacharacters is the literal case. There is no syntax mode field. | Requester selected “Automatic fallback.” | A pattern rejected by both engines is an invalid-pattern failure; callers escape regex metacharacters when they require literal punctuation. |
| What is one semantic `rfs_search` match? | One matching line. Results are grouped by canonical Resource; each matching line appears once with its 1-based line number and bounded line text even if the pattern matches more than once on that line. | Requester selected “One matching line.” | Structured output has Resource groups containing line records; it carries no occurrence columns or duplicate rows for repeated matches on one line. |
| How does pagination remain deterministic across later Workspace changes? | Canonically order the complete result; `skip` and a lower-only result limit select the initial page. If any result is omitted, retain the complete rendered result once and return its immutable Recovery Reference plus a continuation reference readable through `rfs_read`. Do not add an opaque cursor. | Requester selected “Artifact continuation”; rfs-34pz already defines immutable artifact paging. | Repeating search/glob is not required to recover later pages; pagination after the first result observes the retained result snapshot. |
| What visibility and case defaults apply when controls are omitted? | Use repository defaults: matching is case-sensitive, gitignore files are respected, and hidden entries are excluded. Callers explicitly request case-insensitive matching, ignored entries, or hidden entries. Artifact Resources are unaffected by gitignore and hidden controls. | Requester selected “Repository defaults.” | Identical inputs and source state produce identical results without smart-case inference; default Workspace discovery avoids repository noise. |
| What does `rfs_glob` enumerate for Artifact Resources? | Treat each live Path Session artifact as one file-like Resource and match patterns against its canonical `artifact://<id>` reference. Emit no artifact directories and never enumerate another Path Session. | Requester selected “Session artifact catalog.” | `artifact://*` discovers current-session artifacts; artifact text is never interpreted as virtual paths. |
| What happens when directory discovery reaches an unreadable or non-UTF-8 Resource? | Return matches from readable Resources plus a deterministically ordered `diagnostics` list containing each skipped canonical reference and its stable error category/message. Failure of a concrete single-Resource target remains a tool error. | Requester selected “Matches plus diagnostics.” | Partial directory discovery is explicit rather than fail-fragile or silently incomplete; diagnostics participate in bounded recovery. |
| How are invalid patterns and cancellation categorized? | Add dedicated operational categories `invalid_pattern` and `cancelled`. Schema-shape failures remain MCP `invalid_params`; a well-shaped pattern rejected by both engines and an interrupted operation return ordinary tool errors. | Requester selected “Dedicated categories.” | Error rendering and the public `ErrorCategory` contract gain precise variants rather than misusing `invalid_reference` or `source_unavailable`. |
| How do callers target Resources? | `rfs_search.path` is one optional exact Workspace file/directory or Artifact reference and defaults to the Primary Workspace Root. `rfs_glob.path` is one required relative, canonical workspace, or artifact glob. Multiple targets require multiple calls. | Requester selected “Single path target.” | No semicolon list or separate root/pattern grammar is added; canonical and relative workspace rules remain those of rfs-cgbq. |
| Which ignore files does `gitignore: true` honor? | Honor contained `.gitignore` files from the Workspace Root through nested target directories. Do not read `.ignore`, `.git/info/exclude`, or global Git excludes. | Requester selected “Contained .gitignore files.” | Identical Workspace bytes and tool input produce identical filtering across operator machines while preserving nested repository rules. |
| What consistency applies during concurrent Workspace changes? | Per-entry consistency: do not lock or snapshot the whole tree; revalidate containment and read each entry once. Entries that disappear or fail during traversal become explicit diagnostics. Deterministic ordering is guaranteed for a stable source state; Artifact Resources remain immutable. | Requester selected “Per-entry consistency.” | Discovery remains bounded under active builds and editor saves without claiming a false point-in-time Workspace snapshot. |
| What cancellation-response budget applies? | Return a `cancelled` tool error within 250 ms after MCP cancellation is observed in a deterministic long-running discovery fixture. | Requester selected “250 milliseconds.” | The compiled stdio contract test measures cancellation from request-token cancellation through receipt of the tool error while traversal or matching is deliberately held open. |
| How are successful records and diagnostics ordered? | Sort Resource groups and glob entries by canonical Path Reference bytewise ascending; sort matching lines by 1-based line number; sort diagnostics by canonical reference, category, then message. | Deterministic-results acceptance criterion and canonical identity from rfs-cgbq. | Traversal scheduling, filesystem enumeration order, and regex engine choice cannot change observable ordering. |
| Is an empty search pattern valid? | No. Return an `invalid_pattern` tool error; callers use an explicit regex such as `.*` when matching every line is intentional. | Requester selected “Reject empty pattern.” | Blank interpolation cannot accidentally generate a whole-tree result, and empty input is not redefined as a successful no-match query. |
| Which glob syntax is observable? | Globset-compatible path syntax using `/` as the canonical separator: `*`, `?`, and character classes stay within one component; `**` crosses directories; `{a,b}` provides alternation. Invalid or empty glob syntax returns `invalid_pattern`. | Requester selected “Globset-compatible paths.” | Windows input and output still use canonical slash form; shell extglobs are not part of the Behavior Contract. |
| How does recursive Workspace discovery treat symlinks or reparse points? | Follow them only when final targets remain inside active authority; match and emit the final canonical Resource identity, deduplicate aliases, and stop directory cycles. Escaping targets produce permission diagnostics. | Requester selected “Follow and deduplicate”; rfs-cgbq already requires final-handle containment. | A contained target appears at most once regardless of aliases, and discovery never exposes outside bytes or loops through link cycles. |
| What happens when valid PCRE2 matching exhausts bounded engine work? | Enforce explicit PCRE2 match and depth limits. A concrete target returns `limit_exceeded`; a directory search adds that Resource to diagnostics and continues. Cancellation remains independently responsive. | Requester selected “Limit-exceeded failure.” | PCRE2-only syntax remains available without allowing one Resource to consume unbounded backtracking work. |
| What does `caseSensitive: false` mean for non-ASCII text? | Content search uses Unicode-aware case folding; path globbing uses ASCII-only case folding. | Empirical evidence found Rust regex is Unicode-aware while globset's non-Unicode mode folds ASCII only; requester selected “Unicode search, ASCII glob.” | The tool-specific distinction is observable and must be stated in both tool descriptions. |
| How is backslash interpreted in glob patterns? | Backslash is an escape on every platform, never a path separator; callers use canonical `/`, and a dangling escape is `invalid_pattern`. | Globset's default is platform-dependent, so the Behavior Contract must set one cross-platform meaning. | Glob construction enables backslash escaping explicitly on Linux, macOS, and Windows. |
| What is the empty-set behavior? | A valid no-match operation is successful with zero records, complete non-empty no-match text, and no recovery metadata when nothing else was omitted. | Explicit no-match behavior above. | Empty discovery cannot be confused with invalid input or failure. |
| How are null, missing, wrong-typed, and unknown fields handled? | Required `pattern` or glob `path` fields must be present, non-null strings; optional fields may be omitted but not set to null; wrong types and unknown fields are MCP `invalid_params`. | Existing deny-unknown-fields MCP contract from rfs-87cv. | Tool-result errors remain reserved for well-shaped operational input. |
| Do hidden/ignore controls suppress an explicitly named search target? | No. Filters govern recursive descendants; a concrete search file or Artifact Resource is attempted exactly as named. | A direct Path Reference is an explicit authority-bounded request. | Callers can search one ignored file without disabling repository defaults for a whole tree. |
| Can a Recovery Artifact appear in the Artifact glob result that created it? | No. Artifact catalog enumeration snapshots pre-existing live artifacts before spill; a later call may observe the newly retained artifact. | Deterministic finite result requirement. | `artifact://*` cannot recursively include its own output. |
| How are permission denial and missing authentication handled? | Workspace containment or capability denial uses `permission_denied`; no separate authentication behavior exists for local Workspace and Path Session sources. | Current source scope has no remote authentication surface. | Permission failures follow exact-target or partial-diagnostic policy; unauthenticated is N/A for this issue. |
| What retry or idempotency behavior applies? | Discovery is read-only. Repeating a call starts a fresh per-entry observation; following the returned Artifact references reads the immutable retained result. | Per-entry consistency and Path Session immutability. | Retry observes current Workspace state, while continuation observes the original result snapshot. |
| How do soft-deleted records affect discovery? | N/A — Workspace and Artifact Resources have no soft-delete state. | Current source models expose present or absent Resources only. | No hidden lifecycle state enters filtering. |
| What multi-tenancy boundary applies? | One MCP connection owns one Path Session; Artifact enumeration never crosses it, and Workspace discovery never crosses active Workspace Roots. | rfs-34pz and rfs-cgbq authority contracts. | Simultaneous connections cannot discover one another's artifacts or undeclared roots. |
| How do time zone or daylight-saving transitions affect discovery? | N/A — matching, ordering, limits, and identity do not depend on civil time. | No time-derived field participates. | Results do not change at clock transitions. |
| How does replication lag affect discovery? | N/A — current Workspace and Artifact adapters are local and non-replicated. | No replicated source is in scope. | No stale-replica behavior is claimed. |
| What cache invalidation behavior applies? | N/A — this issue adds no discovery index or cache; each call observes current per-entry state, and root changes immediately replace active authority per rfs-cgbq. | Indexes, caches, and watchers are out of scope. | There is no stale discovery cache to invalidate. |

## Approval

Requester approval (verbatim): "Approve amendment"
Date: 2026-08-20
