# Spec: Read-only GitHub issue and PR Resources

## Request (verbatim)
> claim and implement rfs-45ww

## What this is
ResourceFS will resolve configured `issue://` and `pr://` Path References through GitHub's native HTTP API. It will expose bounded read-only Aggregate Resources and authoritative Field Resources while preserving the common read, search, selector, listing, recovery, policy, and cancellation contracts.

## Roles
- **Coding agent**: reads and searches issue and pull-request content through stable Path References without learning a GitHub-specific tool schema.
- **Operator**: configures the credential, readable repository allowlist, network policy, and lower-only limits; sees bounded source diagnostics without credential or repository disclosure.
- **Harness integrator**: launches ResourceFS and receives the same non-empty text and structured results used by other Source Adapters.

## Behavior

### Repository authority
- **Given**: a Server Profile with a GitHub source and an explicitly readable `owner/repository` entry.
- **When**: a caller resolves an `issue://owner/repository/...` or `pr://owner/repository/...` Path Reference.
- **Then**: only that repository is eligible for upstream access; an absent repository is rejected before an upstream request and without disclosing whether it exists.

### Aggregate and Field Resources
- **Given**: an allowlisted issue or pull request returned by the deterministic upstream.
- **When**: a caller reads its aggregate or one of its title, body, or stable conversation-comment paths.
- **Then**: the aggregate is a read-only narrative whose fixed header contains kind, number, state, author, created/updated UTC timestamps, and canonical GitHub web URL; PR headers additionally contain draft, merged status/time, head ref, and base ref. Full title/body and conversations follow the header. Each authoritative value is a distinct read-only Field Resource with its own canonical Path Reference and content-derived Version Tag. Labels, assignees, milestone, reactions, requested reviewers, checks, and commit/file statistics are not rendered by this change.

### Pull-request projections
- **Given**: an allowlisted pull request with conversation comments, review submissions, inline review comments, and a diff.
- **When**: a caller addresses those projections.
- **Then**: each projection has a distinct canonical namespace and no projection conflates one GitHub object kind with another.

### Repository collection listing and search
- **Given**: an allowlisted repository containing issues and pull requests in any state.
- **When**: a caller reads or searches `issue://owner/repository`, `pr://owner/repository`, or one explicit object or Field path.
- **Then**: issue collections exclude pull requests; PR collections include open, closed, and merged PRs; collection results sort by `updated_at` descending then numeric ID; bare scheme roots are unsupported; and an empty collection succeeds with an explicit empty rendering.

### Common bounded operations and pagination
- **Given**: any resolvable GitHub Aggregate, Field, or repository collection.
- **When**: a caller lists, reads, searches, or applies a supported selector with limits no higher than the active Server Profile.
- **Then**: ResourceFS uses the common bounded result shapes; requests 100 upstream objects per page; follows at most 10 pages per logical collection in one operation using each returned Link target verbatim; names remaining upstream pages with the approved typed `:page:<next-page>` source continuation; spills already-fetched inline overflow losslessly to an immutable `artifact://` Recovery Reference; and never returns unlabeled partial success after a page failure.

### Conditional session cache
- **Given**: a successful GitHub response cached inside the active Path Session.
- **When**: the same representation is read again.
- **Then**: ResourceFS sends `If-None-Match` when the cached response supplied an ETag, reuses cached content only after `304 Not Modified`, replaces it after a new success response, performs a normal fetch when no ETag exists, never serves stale content after failure, and discards the cache when the Path Session ends.

### Bounded failures and retry
- **Given**: missing or rejected credentials, repository denial, cancellation, network failure, an upstream HTTP error, malformed upstream data, exhausted GitHub rate limit, or a valid `Retry-After`.
- **When**: the adapter cannot complete an operation.
- **Then**: `401` returns `permission_denied`; every upstream `404` returns `not_found`, intentionally collapsing absent and access-hidden objects; `429`, or `403` with `X-RateLimit-Remaining: 0` or `Retry-After`, returns `source_unavailable`; every other `403` returns `permission_denied`; `5xx`, network failure, and malformed upstream data return `source_unavailable`; ceilings return `limit_exceeded`; cancellation returns `cancelled`; and no human error-message matching or partial successful Resource is used. A transient transport failure retries once; `429` or `503` retries once only after valid `Retry-After`; all attempt and wait time shares the configured at-most-30-second logical deadline and remains cancellation-aware.

### Valid absent fields
- **Given**: a valid GitHub object whose body or author is null, or whose diff file has no textual patch.
- **When**: a caller reads the Aggregate or corresponding Field.
- **Then**: a null body remains an empty authoritative Field and renders as “no body” in the Aggregate; a null author renders `[deleted]`; an unavailable patch remains a readable metadata Field stating that the patch is unavailable; a review without `submitted_at` renders as pending; nullable review `commit_id` or inline `pull_request_review_id` remains absent rather than invalid; and missing required identity fields fail as malformed upstream data.

### Required and degraded lifecycle
- **Given**: GitHub credential resolution or the explicit source connectivity probe is unavailable.
- **When**: ResourceFS starts or runs the existing profile check path.
- **Then**: a required source failure prevents startup; an optional source remains visibly degraded and only its own references fail; normal startup performs no additional network probe beyond the established source lifecycle.

### Deterministic verification
- **Given**: a deterministic fake GitHub upstream and no live GitHub mutation authority.
- **When**: the direct Source Adapter contract suite exercises success, policy, pagination, limits, caching, cancellation, and failure rows.
- **Then**: every behavior above is observable from requests received by the fake upstream and Resources/errors returned by the adapter.

## Success criteria
- **Binary / security**: zero upstream requests for a repository absent from the readable allowlist, checked by the fake upstream request log.
- **Binary / structural**: Aggregate Resources, title/body Field Resources, conversation comments, review submissions, inline review comments, and diffs have distinct canonical references, checked by direct adapter contract tests.
- **Quantitative**: at most 48 KiB, 3,000 lines, and 512 columns per inline text result, measured by the common read/search result assertions under default limits; lower profile/call limits remain authoritative.
- **Binary / recovery**: every omitted successful byte remains reachable through an immutable Recovery Reference or the operation fails `limit_exceeded`, checked by bounded large-response fixtures.
- **Binary / security**: zero credential bytes appear in references, results, errors, catalog output, or logs, checked by sentinel scanning with a positive-control authenticated request.
- **Binary / behavior**: cancellation stops an in-flight paginated/body read before the fake upstream finishes the uncancelled control transfer, checked by direct adapter cancellation fixtures.
- **Binary / classification**: every credential, repository, network, upstream-status, rate-limit, limit, and cancellation fixture asserts one exact existing `ErrorCategory`, checked by the direct adapter error matrix.
- **Quantitative**: each logical collection fetches 100 objects per upstream page and at most 10 pages/1,000 objects per operation, measured by fake-upstream fixtures at 0, 1, 100, 101, 1,000, and 1,001 objects; object 1,001 is reachable through the returned `:page:11` continuation.
- **Binary / atomicity**: a failure on any followed page returns one classified error and no partial successful collection/Aggregate, checked by failure injection on pages 2 and 10.
- **Binary / cache**: a repeated ETag-bearing read sends `If-None-Match`, reuses bytes only on `304`, replaces them on a new success response, and returns rather than masking revalidation failure, checked by the fake-upstream request/header log.
- **Quantitative**: the first attempt, any `Retry-After` wait, and the sole retry complete or fail within the configured logical deadline of at most 30 seconds, measured with paused deterministic time and request counts.
- **Binary / lifecycle**: a required unavailable GitHub source fails startup while an optional unavailable source is visibly degraded and leaves non-GitHub adapters operational, checked by profile-launch and check fixtures.

## Out of scope
This change does NOT include GitHub mutation or Creation Targets; label, assignee, status, review, or inline-review mutation; automatic retry of unknown mutation outcomes; cross-session operation deduplication; live GitHub mutations in tests; network-backed keystroke completion; a second HTTP client or policy layer; telemetry; or interactive profile management. `rfs-by2z` owns GitHub mutation and creation behavior.

## Related issues
- rfs-jk0d: accepted ResourceFS 1.0 roles, common limits, GitHub Aggregate/Field split, allowlist authority, and direct fake-upstream testing.
- rfs-g2z9: supplies the audited bounded HTTP substrate, credential-safe transport, address policy, cancellation, timeout, and required/degraded source lifecycle used here.
- rfs-by2z: owns all GitHub mutation and Creation Targets and therefore defines this change's read-only boundary.
- rfs-q12y: the HTTP dependency tree is already governed by cargo-deny and CI; this change must reuse that dependency path.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| Who consumes this behavior? | Coding agents read; operators configure authority; harness integrators launch and transport results. | rfs-jk0d accepted roles and behaviors 1–8, 72–82, and 108–116. | Behaviors name those roles and expose no GitHub-specific MCP tool family. |
| Is any GitHub mutation included? | No. | rfs-by2z owns title/body/comment mutation and Creation Targets. | Every Resource in this change is read-only; writes return `unsupported_mutation`. |
| May GitHub use another HTTP client or network policy path? | No. | rfs-g2z9 established `HttpSubstrate` as the workspace's only HTTP client. | GitHub requests reuse the existing resolver, redirect, private-network, timeout, cancellation, and body ceilings. |
| Are unallowlisted repositories probed upstream? | No. | rfs-45ww acceptance and rfs-jk0d readable-repository authority. | Repository policy is checked before request construction; denial leaks no existence signal. |
| Are conversation comments, reviews, and inline comments interchangeable? | No. | rfs-45ww acceptance and rfs-jk0d story 80. | They occupy distinct projections and typed response variants. |
| What happens for concurrent writes? | N/A — this change exposes no GitHub write operation. | rfs-by2z owns mutation concurrency. | No write serialization or mutation journal is added here. |
| Are network-backed references used for keystroke completion? | No. | rfs-jk0d implementation decision excludes network-backed completion. | Listing/search are explicit operations only. |
| Are live GitHub mutations used as test evidence? | No. | rfs-45ww acceptance requires a deterministic fake upstream. | Direct adapter tests own the behavioral oracle. |
| Which canonical hierarchy names GitHub Resources? | Flat typed collections: `issue://owner/repository/<number>` and `pr://owner/repository/<number>` aggregates; `/title`, `/body`, and `/comments/<id>` fields; PR-only `/reviews/<id>`, `/review-comments/<id>`, `/diff`, and `/diff/<file-index>`. | Requester selected “Flat typed collections.” It matches the accepted `pr://…/diff/1` example and keeps GitHub object kinds distinct. | Parsers, renderers, listings, and recovery references use these spellings; inline review comments are not nested beneath review submissions. |
| How are repeated GitHub reads cached? | Cache bounded responses within one Path Session, conditionally revalidate every repeat with `If-None-Match`, reuse content only after `304 Not Modified`, replace it after a new `200`, and never serve stale content after a failed revalidation. | Requester selected “Revalidate every read.” | Cache entries expire at disconnect, have no freshness TTL, count against session authority/limits, and cannot hide network, upstream, credential, or cancellation failures. |
| How are GitHub failures categorized? | Machine-signal conservative: `401` and ordinary `403` are `permission_denied`; every upstream `404` is `not_found`, deliberately collapsing absent and access-hidden objects; `429`, or `403` carrying remaining `0` or `Retry-After`, is `source_unavailable`; `5xx`, network failure, and malformed upstream data are `source_unavailable`; ceilings are `limit_exceeded`; cancellation is `cancelled`. Human error prose is never matched. | Requester selected “Machine-signal conservative” after empirical evidence P5 disproved perfect machine-only disambiguation. | No new Behavior Contract category, preflight request, existence leak, or error-string match is added; an unsignaled secondary-limit `403` intentionally appears as `permission_denied`. |
| What does an issue or PR Aggregate render? | A complete bounded narrative: stable metadata, full title and body, conversation comments, and for PRs review submissions, inline review comments, and diff-file navigation. Every authoritative value names its child Field reference; comments/reviews use chronological order with stable ID tie-breaking, and diff files retain upstream order. | Requester selected “Complete bounded narrative.” | One aggregate read is useful by itself, while common inline limits and lossless Recovery References bound large conversations and diffs. |
| What do collection listing and search cover? | Repository-scoped collections only. `issue://owner/repository` lists issues in all states and excludes pull requests; `pr://owner/repository` lists open, closed, and merged PRs. Both sort by `updated_at` descending with numeric ID tie-breaking. Search accepts one explicit object, Field, or repository collection. Bare scheme roots do not enumerate or search repositories. An empty collection is a successful explicit empty rendering. | Requester selected “Repository-scoped, all states.” | No cross-repository network fan-out or allowlist enumeration is introduced; closed and merged work remains addressable without a new state selector. |
| Are failed GitHub GET requests retried? | At most once: transient transport failures retry once, and `429` or `503` retries once after a valid bounded `Retry-After`; every other failure returns immediately. | Requester selected “Honor Retry-After once.” | The adapter needs one retry budget shared by ordinary fetches and conditional revalidation; retry never serves stale cache content and remains cancellation-aware. |
| What bounds a retry? | The configured HTTP timeout, never above 30 seconds, is one total logical-operation deadline shared by the first attempt, `Retry-After` wait, and retry. Waiting or retrying occurs only while time remains; cancellation interrupts either. | Requester selected “One shared profile timeout.” | Automatic retry does not increase the existing worst-case operation latency. |
| How does GitHub source lifecycle behave? | Credential resolution and the explicit connectivity probe use the established required/degraded contract: unavailable required sources fail startup; unavailable optional sources remain visibly degraded and only their own references fail; normal startup performs no network probe unless the existing profile lifecycle requires one. | rfs-g2z9 and rfs-jk0d already fix this Source Adapter behavior. | GitHub introduces no second lifecycle or probe model. |
| Can multiple GitHub sources own the same schemes? | No. One mounted GitHub source owns both `issue://` and `pr://`; a second claimant fails profile validation. Repository identities compare case-insensitively in canonical `owner/repository` form. | rfs-jk0d requires registration conflicts to fail startup; existing `GithubConfig` already rejects case-insensitive duplicate repositories. | Dispatch never depends on configuration order and one repository has one authority/cache namespace. |
| How are timestamps and replication lag represented? | Render upstream timestamps as UTC RFC 3339 values without local-time conversion. No freshness claim exceeds the response GitHub returned; every repeat conditionally revalidates and may observe later upstream state. | Deterministic output and the selected revalidation policy; local time-zone and DST must not affect a Resource. | Time-zone/DST is N/A, and replication lag is exposed rather than hidden or estimated. |
| Which generic edge dimensions are inapplicable? | Concurrent writes are N/A because this change is read-only; cross-tenant reads are denied by the repository allowlist and Path Session cache isolation; mutation idempotency is N/A; empty collections succeed explicitly. | rfs-by2z owns writes; rfs-jk0d owns repository authority and Path Session isolation; the selected collection policy owns empty sets. | No write journal, cross-session cache, or sentinel empty result is introduced. |
| How are valid absent upstream values represented? | Preserve absence explicitly: a null body is an empty authoritative Field and “no body” in its Aggregate; a null author is `[deleted]`; an upstream-missing object is `not_found`; a diff file without a patch remains a readable metadata Field stating that the patch is unavailable. Missing required identity such as ID, number, or title is malformed upstream data and returns `source_unavailable`. | Requester selected “Preserve absence explicitly.” | Valid GitHub states remain readable without sentinels in the typed model, while corrupt response shapes never masquerade as empty success. |
| How are large collections paginated? | Fetch 100 upstream objects per page and follow each returned Link target verbatim for at most 10 pages/1,000 objects per logical collection in one operation. If more exist, return the approved `:page:<next-page>` on that source collection; already-fetched inline overflow uses immutable `artifact://` recovery. A page failure fails the operation without partial success. | Requester selected “Bounded source continuations”; evidence P4/P6 established opaque cursor-bearing Links and numeric page equivalence for the fixed query. | Network work and retained object count are bounded; fixtures cover 0, 1, 100, 101, 1,000, and 1,001 objects, exact Link following, and intermediate-page failure. |
| How are soft-deleted or minimized records treated? | This feature exposes only objects the configured GitHub REST API returns. An absent/deleted object is `not_found`; a returned object is rendered from the fields supplied. No synthetic soft-deleted or minimized state is inferred. | The selected explicit-absence policy and native-API boundary. | Listing and Aggregate output never invent hidden content or silently substitute an empty object. |
| Which metadata appears in every Aggregate? | Both families render kind, number, state, author, created/updated UTC timestamps, and canonical GitHub web URL. PRs additionally render draft, merged status/time, head ref, and base ref. Labels, assignees, milestone, reactions, requested reviewers, checks, and commit/file statistics are excluded. | Requester selected “Decision-relevant metadata.” | The Aggregate carries review context without adding unnamed API projections or calls. |




## Approval
Previous approval (superseded by empirical evidence P5): "Agree"
Requester approval (verbatim): "Agree"
Date: 2026-08-25
