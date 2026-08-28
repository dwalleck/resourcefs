# Spec: Mutate and create GitHub work items

## Request (verbatim)
> claim and implement rfs-by2z

## What this is
ResourceFS will mutate grant-authorized GitHub title, body, and conversation-comment Field Resources with authoritative Version Tag checks. It will also create GitHub issues, pull requests, and conversation comments through text Creation Targets, with Path-Session-local operation deduplication and explicit handling of outcomes that become unknown after transmission.

## Roles
- **Coding agent**: reads GitHub Resources, submits versioned replacements or creation documents through `rfs_write`, and receives canonical references or stable operational errors.
- **Operator**: configures readable repositories, credentials, and narrower per-repository create/update grants; sees denied mutations fail without an upstream write.
- **Harness integrator**: transports `operationId`, `ifVersion`, text and structured receipts, and stable error categories without adding GitHub-specific tools.
- **Maintainer**: verifies direct adapter behavior against a deterministic fake upstream and preserves the shared core/source/MCP boundaries.

## Behavior

### Version-checked Field replacement
- **Given**: an allowlisted GitHub issue or pull request title, body, or stable issue-style conversation comment Field Resource, its current content-derived Version Tag, repository update authority, no `operationId`, and content valid for that field.
- **When**: the coding agent calls `rfs_write` for that canonical Field Resource with replacement UTF-8 content and the current tag in `ifVersion`.
- **Then**: ResourceFS performs an uncached authoritative read immediately before one PATCH attempt, changes only that field when the tag still matches, derives the receipt's new Version Tag from the authoritative success response, invalidates affected Path-Session GitHub cache entries, and names the same canonical Field Resource.

### Rejected Field replacement
- **Given**: an existing GitHub Field Resource with a missing/stale `ifVersion`, absent repository update authority, an `operationId`, invalid empty content, or a projection outside title/body/stable conversation comments.
- **When**: the coding agent requests replacement through `rfs_write` or any GitHub mutation through `rfs_edit`.
- **Then**: ResourceFS returns the applicable `version_conflict`, `permission_denied`, `invalid_reference`, or `unsupported_mutation` error without a mutating upstream request.

### Operation ID outside a Creation Target
- **Given**: any workspace, local, existing GitHub Field, or other non-Creation Target `rfs_write` carrying `operationId`.
- **When**: the coding agent submits the write.
- **Then**: ResourceFS returns `invalid_reference` before adapter or upstream access and does not mutate the Resource.

### Comment creation
- **Given**: an allowlisted issue or pull request, repository create authority, non-blank Markdown, no `ifVersion`, and a valid `operationId`.
- **When**: the coding agent writes the Markdown to canonical `comments/new`.
- **Then**: ResourceFS makes one POST attempt for an issue-style conversation comment and returns a creation receipt naming canonical `comments/<created-id>` with `versionTag: null`.

### Issue creation
- **Given**: an allowlisted repository, repository create authority, no `ifVersion`, a valid `operationId`, and a document beginning with an opening `---` line and ending its frontmatter with a closing `---` line, containing exactly one non-blank one-line string `title`, no unknown or duplicate field, and a possibly empty Markdown body.
- **When**: the coding agent writes that document to `issue://<owner>/<repo>/new`.
- **Then**: ResourceFS makes one POST attempt with the parsed title and exact remaining Markdown body and returns a creation receipt naming canonical `issue://<owner>/<repo>/<created-number>` with `versionTag: null`.

### Pull-request creation
- **Given**: an allowlisted repository, repository create authority, no `ifVersion`, a valid `operationId`, and the same strict frontmatter form containing exactly one non-blank one-line string each for `title`, `head`, and `base`, optionally one boolean `draft`, no unknown or duplicate field, and a possibly empty Markdown body.
- **When**: the coding agent writes that document to `pr://<owner>/<repo>/new`.
- **Then**: ResourceFS makes one POST attempt with those fields and exact remaining Markdown body and returns a creation receipt naming canonical `pr://<owner>/<repo>/<created-number>` with `versionTag: null`.

### Invalid creation request
- **Given**: a Creation Target write with missing/invalid `operationId`, any `ifVersion`, blank required content, malformed/unsupported frontmatter syntax, missing/duplicate/unknown/wrongly typed fields, a Path Session already holding 10,000 distinct operation IDs, or exact content retention that would exceed the shared 256 MiB Path-Session storage ceiling.
- **When**: the coding agent submits `rfs_write`.
- **Then**: ResourceFS returns `invalid_reference` for invalid input or `limit_exceeded` for a new journal entry beyond either ceiling, without an upstream request and without discarding an existing journal entry.

### Completed-operation repeat
- **Given**: one live Path Session recorded a successful Creation Target result for an operation ID bound session-wide to one canonical target and exact content bytes.
- **When**: that session repeats the same operation ID, target, and content, including while the first call is still in flight.
- **Then**: at most one upstream attempt runs; concurrent calls wait, and every identical caller receives the same canonical created reference with `versionTag: null`.

### Operation-ID conflict
- **Given**: one live Path Session already bound an operation ID to one canonical target and exact content bytes.
- **When**: that session reuses the ID with a different target or different content.
- **Then**: ResourceFS returns `version_conflict` without another upstream request.

### Conclusive creation failure
- **Given**: a Creation Target attempt received a `3xx` or `4xx` response, which conclusively reports no creation.
- **When**: the call fails, or the same operation ID/target/content is later submitted again after conditions change.
- **Then**: ResourceFS returns the mapped stable category for the failed call, retains the ID binding, permits the later caller-initiated identical attempt, and continues to reject a changed meaning.

### Unknown upstream outcome
- **Given**: a Creation Target attempt received `5xx`; or transport, timeout, or cancellation failed after attempt start; or a response dropped, exceeded bounds, used the wrong `2xx`, contained malformed JSON, or failed success identity/field validation.
- **When**: the call fails or that operation ID is presented again in the same Path Session.
- **Then**: ResourceFS returns `source_unavailable` with reconciliation guidance, never automatically retransmits the request, and blocks that ID until the session ends; after inspecting upstream and establishing absence, the coding agent may use a new operation ID.

### Unknown Field-replacement outcome
- **Given**: a Field replacement PATCH may have reached GitHub but ResourceFS did not receive and validate a conclusive response.
- **When**: the call fails.
- **Then**: ResourceFS returns `source_unavailable`, performs no automatic retry, and directs the coding agent to read the authoritative Field Resource before deciding whether another version-checked replacement is necessary.

### Path-Session isolation
- **Given**: a new Path Session with no journal entry for an operation ID used by a previous or disconnected session.
- **When**: the coding agent submits a Creation Target with that ID.
- **Then**: ResourceFS makes no cross-session deduplication promise and evaluates it as a new operation.

### Creation Target reads
- **Given**: `issue://…/new`, `pr://…/new`, or issue/PR `comments/new`.
- **When**: the coding agent calls `rfs_read`.
- **Then**: ResourceFS returns `unsupported_projection` without an upstream request.

### Non-mutable GitHub projections
- **Given**: a GitHub Aggregate Resource, label, assignee, status, milestone, reaction, review, inline review comment, review submission, or any other non-title/body/stable-conversation-comment projection.
- **When**: the coding agent attempts mutation.
- **Then**: ResourceFS returns `unsupported_mutation` without a mutating upstream request.

### Upstream response classification
- **Given**: one GitHub mutation attempt returns an HTTP response.
- **When**: ResourceFS classifies it.
- **Then**: `400` and `422` map to `invalid_reference`; `401` and non-rate-limit `403` to `permission_denied`; hidden/missing `404` and `410` to `not_found`; `409` to `version_conflict`; every `3xx`, rate-limit `403`, `429`, and `5xx` to `source_unavailable`; mutation redirects are never followed; bounded errors never include upstream body text, and a Creation Target journal separately records whether the outcome is conclusive or unknown.

## Success criteria

- **Binary / structural / security**: Every accepted Field replacement requires repository update authority and a matching uncached authoritative Version Tag immediately before exactly one PATCH attempt, checked by direct fake-upstream state and ordered request observations.
- **Binary / structural / security**: The success response's authoritative field bytes determine the replacement receipt Version Tag, and the next read cannot revive stale cached bytes, checked by transformed-response and ETag-cache fake-upstream rows.
- **Binary / structural / security**: Every Creation Target requires repository create authority, a valid `operationId`, and no `ifVersion`; every success receipt names the response-derived canonical Resource and has `versionTag: null`, checked by direct fake-upstream state and receipt assertions.
- **Binary / structural / security**: Issue and pull-request documents accept only the approved one-line strict YAML subset and preserve the post-delimiter Markdown body byte-for-byte, checked by parser contract matrices plus fake-upstream JSON request bodies.
- **Binary / structural / security**: A completed or concurrently in-flight operation ID with identical target/content produces at most one upstream attempt and every caller receives the same result; changed target/content returns `version_conflict`, checked by per-session concurrent request counts.
- **Quantitative**: 10,000 distinct operation IDs remain journaled in one live Path Session and the 10,001st new ID returns `limit_exceeded` while existing IDs still repeat correctly, measured by a core journal contract test.
- **Quantitative**: Operation-journal entries compare exact content bytes, retain them without a second copy, and count them against the shared 256 MiB Path-Session ceiling; exact-limit retention succeeds and one byte over returns `limit_exceeded`, measured by a forced-fingerprint-collision and quota core contract test.
- **Quantitative**: Every GitHub PATCH/POST operation performs at most 1 transmitted request and follows 0 redirects, measured by fake-upstream request counts across connection failure, redirect, `Retry-After`, rate-limit, and `5xx` rows.
- **Binary / structural / security**: A transmitted Creation Target outcome that cannot be validated becomes blocked for that ID and requires reconciliation; a conclusive non-creation may repeat only with the same bound target/content, checked by response-drop, malformed-success, conclusive-error, and repeat rows.
- **Binary / structural / security**: Separate Path Sessions do not share operation journals, checked by two bound adapters whose identical operation IDs each reach the fake upstream.
- **Binary / structural / security**: `rfs_read` of Creation Targets returns `unsupported_projection`; `rfs_edit` and writes to Aggregate/auxiliary projections return `unsupported_mutation`; the denial matrix observes zero mutating upstream requests.
- **Binary / structural / security**: The public `rfs_write` input schema exposes optional `operationId`, plain-text and structured receipts agree on response-derived canonical references/null creation tags, and the HTTP status matrix returns the named categories without upstream body text, checked through the real stdio MCP process and direct fake adapter.

## Out of scope

This change does NOT include label, assignee, status, milestone, reaction, review, inline-review-comment, review-submission, or Aggregate Resource mutation; GitHub object deletion; automatic retry after an unknown remote creation outcome; cross-session operation-ID deduplication; bulk mutation; GraphQL; a second HTTP client/policy layer; live writes to public upstreams; new GitHub-specific MCP tools; or mutation through `rfs_edit`.

## Related issues

- rfs-jk0d: accepted product behavior 51–65 and 72–82 defines the five-tool surface, GitHub Field/Creation Target scope, Path-Session journal, grants, Version Tags, deduplication, and unknown-outcome boundary.
- rfs-73dz: supplies the shared versioned `rfs_write` mutation engine, receipts, authoritative compare-before-commit, per-Resource serialization, and stable mutation errors; explicitly defers remote operation deduplication to rfs-by2z.
- rfs-45ww: supplies typed GitHub addresses, allowlisted read-only Aggregate/Field Resources, native REST transport, fake/live read contracts, and distinct conversation/review/inline-comment projections; explicitly defers every GitHub mutation to rfs-by2z.
- rfs-g2z9: supplies the only bounded HTTP substrate, transmission/cancellation/timeout policy, credential-safe transport, deterministic fake-server support, and source lifecycle reused here.
- rfs-r9m6: supplies strict Server Profile source/repository create/update grants and rejects unsupported delete authority for GitHub.
- rfs-34pz: supplies Path Session lifecycle and isolation that bounds mutation-journal lifetime.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| Who consumes and authorizes this behavior? | Coding agents invoke it; operators grant repository authority; harness integrators carry the common MCP contract; maintainers verify adapter behavior. | rfs-jk0d roles and accepted stories 51–82. | Every role is explicit, and no GitHub-specific tool family is introduced. |
| Which GitHub values are mutable? | Existing title, body, and stable issue-style conversation comments only; creation adds issues, pull requests, and conversation comments. | rfs-jk0d stories 72–82 and rfs-45ww's explicit read-only hand-off. | Aggregates, reviews, inline comments, labels, assignees, and status remain non-mutable. |
| Which authority applies? | Per-repository update grants authorize existing Field replacement; per-repository create grants authorize all three Creation Targets; readable allowlisting remains independently required. | rfs-jk0d stories 60–61 and 81–82; rfs-r9m6's existing nested GitHub grants. | Configuring credentials or read authority never implies mutation. |
| Which public mutation tool changes existing GitHub fields? | Full replacement through `rfs_write` with `ifVersion` only; every GitHub hashline `rfs_edit` remains unsupported. | Requester selected “Write replacement only.” | GitHub reads do not enter seen-region snapshots, and the core engine must return a clean unsupported-mutation error for GitHub edits. |
| Which strict creation-document grammar applies? | YAML frontmatter delimited by opening and closing `---`; only scalar `title` for issues; scalar `title`, `head`, `base`, and optional boolean `draft` for PRs; unknown, duplicate, missing, or wrongly typed fields fail; remaining bytes are Markdown body. | Requester selected “Strict YAML frontmatter.” | Creation parsing is deterministic and text-first; malformed documents never reach GitHub. |
| Which YAML scalar syntax is accepted? | Plain or quoted one-line string values plus literal `true`/`false` for `draft`; block/folded scalars, sequences, mappings, anchors, aliases, tags, directives, trailing value comments, multi-document YAML, and other forms are rejected. | Requester selected “Simple one-line scalars.” | The strict parser stays deterministic and small while quoted titles can contain delimiters such as `:`. |
| Are Creation Targets readable Resources? | No; `rfs_read` of `issue://…/new`, `pr://…/new`, or `comments/new` returns `unsupported_projection` without upstream access. | Requester selected “unsupported_projection.” | `/new` remains a write-only Creation Target rather than a synthetic versioned Resource. |
| Which empty values are accepted? | `title`, `head`, `base`, and new or replacement comment Markdown must be non-blank; issue/PR creation bodies and existing issue/PR body replacements may be empty. Invalid empty values fail before upstream access. | Requester selected “GitHub-valid matrix.” | Agents may clear an issue/PR body but cannot create blank-titled work items, blank refs, or blank comments. |
| What does one `operationId` identify? | One creation globally within one Path Session: the ID binds to one canonical Creation Target and exact content bytes. | Requester selected “Session-wide identity.” | Reuse on a different target or with different content is rejected; identical target/content returns the recorded result. |
| How is conflicting operation-ID reuse categorized? | `version_conflict`. | Requester selected “version_conflict.” | The journal's bound target/content is logical-operation state; a changed meaning conflicts with that state without adding a public error category. |
| Which `operationId` values are valid? | A case-sensitive 1–128 character ASCII token using only `[A-Za-z0-9._:-]`. | Requester selected “Opaque ASCII token.” | Empty, overlength, whitespace, control, non-ASCII, and other punctuation fail before journal or upstream access. |
| How do concurrent identical operation-ID calls behave? | One upstream creation attempt runs; later identical calls wait for its journal transition and receive the same canonical success, conclusive failure, or unknown-outcome error. | Requester selected “Wait and share result.” | Safe repeat behavior is timing-independent and never duplicates an in-flight creation. |
| How are invalid operation IDs and creation documents categorized? | `invalid_reference`, with a bounded message naming the exact invalid field or document rule. | Requester selected “invalid_reference.” | Malformed `rfs_write` creation input fails locally without reusing `invalid_patch` or adding a contract category. |
| What is the operation-journal scale ceiling? | 10,000 distinct operation IDs per live Path Session. The next new ID returns `limit_exceeded`; already-recorded IDs remain queryable for repeat/conflict/unknown handling. | Requester selected “10,000 entries.” | Journal memory remains bounded without discarding deduplication state already returned to the caller. |
| How are exact operation bytes retained without unbounded memory? | `WriteRequest` and the journal share one immutable content buffer; journal bytes count against the existing 256 MiB Path-Session ceiling in addition to the 10,000-entry ceiling, and live entries are never evicted. | Exact-byte operation identity is approved, while the existing Path Session contract forbids unbounded or evictable live state. | Digest collisions cannot merge distinct operations; quota exhaustion returns `limit_exceeded` before upstream access. |
| What Version Tag does a Creation Target receipt return? | None; the receipt names the canonical created Resource with `versionTag: null`, and a later read supplies its authoritative current tag. | Requester selected “Omit the tag.” | ResourceFS does not mislabel submitted creation-document bytes as the created Resource's authoritative representation and performs no receipt-only follow-up GET. |
| May a non-Creation Target write carry `operationId`? | No; workspace, local, existing GitHub Field, and every other non-Creation Target write reject it before adapter/network access. | Requester selected “Reject it” for existing Fields and “Reject outside Creation Targets” for authored text. | Operation-ID semantics are exclusive to remote Creation Targets and never imply journaling where none exists. |
| How does a caller recover from an unknown transmitted creation outcome? | The operation ID is blocked for the remainder of that Path Session; repeats return `source_unavailable` with reconciliation guidance. The coding agent inspects upstream and, only after establishing absence, may make a new attempt with a new operation ID. | Requester selected “Block ID for session.” | ResourceFS never retransmits an unknown operation and exposes no reconciliation-assertion flag. |
| May a conclusively rejected creation be tried again? | Yes, but only with the same operation ID, canonical target, and exact content. The ID remains bound; a later caller-initiated attempt may run after conditions change, while a changed meaning returns `version_conflict`. | Requester selected “Allow same request retry.” | Conclusive non-creation differs from an unknown outcome and does not force a new logical operation. |
| Which failures are conclusive versus unknown for a Creation Target? | Received `3xx`/`4xx` responses are conclusive non-creation. Every `5xx`; transport, timeout, or cancellation after attempt start; response drop; wrong `2xx`; truncated/oversize body; malformed JSON; and invalid success identity/field are unknown and block the ID. | Requester selected “5xx and unvalidated transport/success.” | A bad success or server failure cannot authorize retransmission when GitHub may already have committed. |
| Are aggregate and auxiliary GitHub mutations included? | No. | The issue and accepted parent contract permanently limit post-creation mutation. | Those references fail before any mutating upstream request. |
| How are GitHub mutation HTTP statuses and redirects categorized? | `400`/`422` → `invalid_reference`; `401` and non-rate-limit `403` → `permission_denied`; hidden/missing `404` and `410` → `not_found`; `409` → `version_conflict`; every `3xx`, rate-limit `403`, `429`, and `5xx` → `source_unavailable`. Mutation redirects are never followed, and messages never include upstream body text. | Requester selected the input-aware matrix, then selected “400 invalid; 410 missing; reject redirects” after P2 found the omitted documented statuses. | Caller-correctable input/removal stays distinct from outages, and a redirect cannot retransmit PATCH/POST. |
| May GitHub mutations retry automatically before transmission? | No; every PATCH or POST receives exactly one transport attempt regardless of failure phase. | Requester selected “Never retry mutations.” | Mutation retry policy is simpler than read retry policy and cannot accidentally broaden into retransmission after an unknown outcome. |
| Are automatic retry and cross-session deduplication included? | No. | rfs-jk0d stories 77–79 and the rfs-by2z acceptance criteria. | The journal is Path-Session-local and unknown transmitted outcomes require reconciliation. |
| Are live upstream mutation tests required? | No; deterministic fake-upstream direct adapter tests are the permanent proof. | rfs-by2z acceptance explicitly names fake-upstream tests, while repository live-smoke policy requires read-only upstream use. | Empirical evidence must use official contracts and non-mutating probes/oracles rather than writing public GitHub state. |
| How are missing/null mutation fields handled? | Existing replacements require `ifVersion` and reject `operationId`; Creation Targets require `operationId` and reject `ifVersion`; omitted PR `draft` means `false`; issue/PR Markdown bodies may be empty; explicit null and missing required frontmatter fields fail `invalid_reference`. | Existing `rfs_write` contract plus the approved creation grammar. | The create/update state matrix is unambiguous before adapter dispatch. |
| What is the maximum mutation input size? | The existing 64 MiB UTF-8 `rfs_write` content ceiling remains authoritative; `operationId` is additionally bounded to 128 characters and the journal to 10,000 entries. | rfs-73dz and the approved journal decisions. | Oversize content or journal growth returns `limit_exceeded` before upstream access. |
| How do concurrent writes behave? | Existing Field replacements serialize per canonical Field Resource; identical concurrent Creation Target IDs coalesce; different operation IDs remain distinct logical creations. | rfs-73dz serialization plus the approved operation-journal decision. | In-process races cannot bypass tag checks or duplicate one logical ID, but distinct IDs may intentionally create distinct objects. |
| Can one operation partially succeed? | N/A — each tool call performs one upstream mutation. A transmitted request with an unvalidated response is represented as unknown rather than success or rollback. | The ticket excludes bulk mutation, and GitHub supplies no rollback for a committed remote create/update. | No multi-item partial-success result shape is introduced. |
| How are soft-deleted or access-hidden objects handled? | N/A — deletion is out of scope; a missing/deleted/access-hidden target follows GitHub's `404` and returns `not_found`. | Existing rfs-45ww hidden-resource contract and this change's no-delete boundary. | ResourceFS never attempts to resurrect or distinguish hidden from absent GitHub objects. |
| What is the multi-tenancy boundary? | N/A — ResourceFS 1.0 is local stdio, not a remote multi-tenant service. Path Sessions isolate journals/caches; configured repository grants and the upstream GitHub repository remain process-shared authority/state. | rfs-jk0d accepted transport and Path Session model. | Cross-session deduplication is absent, but no session can widen configured repository grants. |
| Do time zone or DST affect mutation? | N/A — no mutation decision, journal TTL, receipt, or version check in this change uses civil time. | All identities and transitions are content/request/session based. | Tests need no clock or time-zone fixture. |
| What replication-lag guarantee applies? | ResourceFS performs the strongest available compare-before-write: an uncached authoritative GET immediately precedes PATCH, but GitHub exposes no promised atomic conditional write against ResourceFS's content hash. | `DESIGN.md` accepts source-native strongest-available comparison rather than claiming unavailable upstream atomicity. | In-process writes serialize; an external GitHub writer may still race in the GET/PATCH gap, and ResourceFS does not claim otherwise. |
| How are caches invalidated after mutation? | A successful mutation removes the entire GitHub HTTP-cache namespace for that Path Session before returning its receipt; authoritative mutation preflight never trusts an ETag cache entry. | Version Tags name authoritative content, and one changed issue/PR/comment can affect aggregates, fields, comments, repository collections, and search results. | Subsequent GitHub reads in that session cannot revive known stale bytes through `304`; non-GitHub cache entries remain intact. |
| Which line endings delimit frontmatter and body? | Opening/closing delimiter lines accept LF or CRLF; the frontmatter delimiters are consumed and every byte after the closing line ending is the Markdown body unchanged. A closing delimiter at EOF yields an empty body. | Portable text input plus the approved byte-exact body contract. | Journal identity hashes the original complete input bytes; JSON payloads carry the exact extracted body bytes. |
| How is authoritative replacement output tagged? | The validated PATCH success response's title/body/comment value, not the submitted bytes, determines the receipt Version Tag. | The repository's Version Tag invariant names authoritative content. | Upstream normalization cannot produce a receipt tag that identifies content ResourceFS never observed. |

## Approval

Requester approval (verbatim): "Yes, approved"
Date: 2026-08-27
