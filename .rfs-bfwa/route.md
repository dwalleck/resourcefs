# Route: rfs-bfwa

Change: Deliver PR conversation-comment collection and individual `/facts` reads with atomic page admission, typed partial outcomes, opaque session continuation and lossless MCP recovery.
Date: 2026-09-08

## Request and isolation

Requester: "claim and implement rfs-bfwa in its own isolated branch or worktree. Base this work on the feat/rfs-0n97 branch".

Claimed in the authoritative primary-checkout Rivets store as `in_progress`, assignee `omp`. Active worktree: `/home/dwalleck/repos/resourcefs-wt-rfs-bfwa-impl`; branch `feat/rfs-bfwa`; source base `feat/rfs-0n97` = `084d136e681179eb33a31f6b84a0fb17abf47daa`. Git reports this checkout's own top level and the primary repository's shared Git common directory (`/home/dwalleck/repos/resourcefs/.git`). Use this checkout's `target` directory exclusively; never share build outputs with another revision. No other checkout or branch is modified by this work.

No `.rfs-bfwa/` artifact directory existed, so this stage runs the change workflow before any downstream artifact.

Scope is the ticket's own text plus the approved contract specification in the sibling documentation worktree (`resourcefs-wt-github-resource-contract/docs/github-resource-contract-spec.md`, user stories 23–24 and 28–38, implementation decisions 2/3/5/6). Ticket dependency `rfs-0n97` is delivered on the base branch (envelope, bounded HTTP acquisition, MCP recovery, gated live smokes); this slice is its named successor `rfs-bfwa conversation collections/individual comments/partial retention/cursors` (`.rfs-0n97/design.md:262`). Exact field and grammar spellings the specification marks PROPOSED remain design-approval work; issue creation did not waive that gate.

## Route tests

| # | Test | Evidence | Verdict |
|---|---|---|---|
| 1 | Empirical premise | Design premises depend on provider behavior not covered by valid retained evidence: (a) a conversation-comment record's native `node_id` and `url`/`html_url`/`issue_url` links and nullable body/author, and (b) the real pagination/coverage behavior of `GET /repos/{owner}/{repo}/issues/{number}/comments` (Link `rel="next"`, `per_page` ceiling, end-of-collection shape). Retained evidence does not cover them: `.rfs-0n97/evidence.md` P1 observed only the PR payload of `rust-lang/rust` PR 159232; `.rfs-45ww/evidence.md` P3–P5 covered native diff, conditional ETag/304 and status/header signals. The existing wire decoder retains only `id/body/user/created_at/updated_at/issue_url` (`crates/resourcefs-sources/src/github/wire.rs:71-78`), and the live row `live_github_reads_hold_up` L2 asserts only an `Author: ` prefix on the human projection (`crates/resourcefs-sources/tests/github_live_smoke.rs:171-176`). Provider-shaped evidence for the new fact family is therefore missing, exactly as the rfs-0n97 route found for PR facts. Numeric enforcement, atomic admission and MCP recovery remain design/implementation obligations, not empirical premises. | yes |
| 2 | Structural module shape | Public reference grammar, owned machine JSON schema and read acquisition inputs change. `pr://<owner>/<repository>/<number>/comments/facts` is not parseable today: the five-segment arm reads the trailing segment as a native child ID (`crates/resourcefs-core/src/reference.rs:1350-1375`), so `facts` fails `github_number`. A collection envelope (`collection.state`/counts/failure/continuation/inconsistency) and an opaque session-scoped continuation are new public surface. `crates/resourcefs-sources/src/github/facts.rs` is 741 physical lines against a 750 cap in `.rfs-0n97/oracles/module-ledger.json`, so collection orchestration cannot live there without an approved ownership change. Current owners: core `reference.rs` (validated identities/grammar), core `acquisition.rs` (source-neutral controls), GitHub `facts.rs` + private `facts/identity.rs` (private decoding/projection), GitHub `fetch.rs` (private fetch/cache/pagination), MCP `server/read.rs` + `render/error.rs` (boundary mapping). Candidate owners: one new private GitHub-owned collection module under the approved GitHub facts owner, and — only if source-neutral continuation state is required — one private core-owned continuation value. Protected parents: `crates/resourcefs-sources/src/github/mod.rs` (protected limit 1498) and `crates/resourcefs-mcp/src/server.rs` (protected limit 1431); the staged placement fence `scripts/module_shape.py` (wired at `scripts/ci-gates.py:135`) currently ends at stage `facts` and rejects unlisted production files, so a successor ledger/stage is part of the change. No new provider trait, second HTTP client, generic continuation list, or Cyril capture coordinator. | yes |
| 3 | Production-scale risk | One logical collection read must bound 1,000 records, 16 MiB cumulative accepted response bodies, 16 MiB serialized JSON including outcome/provenance/continuation overhead, 10 physical attempts and 30 seconds shared across send/body/wait, with atomic page admission before an unreturnable result is built. Existing `collection()` follows up to `MAX_PAGES_PER_OPERATION = 10` pages of `per_page=100` and accumulates before any ceiling check (`crates/resourcefs-sources/src/github/fetch.rs:20` and `:459-483`), so admission, allocation and retention behavior need budget/stress machinery and implementation measurement. | yes |
| 4 | Explicit behavior | The ticket plus the user-approved contract specification (`docs/github-resource-contract-spec.md`, stories 23–24/28–38 and decisions 2/3/5/6) pin observable behavior as the triples below. What remains open — exact field/route spellings the specification marks PROPOSED, the continuation envelope encoding, and module placement — is explicitly deferred to the specification's design-approval gate, so it is design work, not unresolved behavior. | yes |

Unknown tests: none.

### T4 observable behavior contract

1. **Given** one permitted configured GitHub deployment and repository with a verified pull request, **when** a library consumer or real MCP client reads `pr://<owner>/<repo>/<number>/comments/facts`, **then** it receives owned machine JSON with the shared envelope and a record collection preserving each conversation comment's native kind/id/nodeId, verified parent PR identity/links, nullable body/author, supplied times and original links; the existing human `/comments` projection keeps its meaning.
2. **Given** a comment whose parent is not the requested pull request, or an absent/contradictory parent identity, **when** the collection or individual facts read runs, **then** the entire component is rejected through the existing `ResourceError` with no records and no continuation, before any record is published.
3. **Given** whole provider pages, **when** they are acquired, **then** a page is admitted only if its records and the resulting final serialized representation — including envelope, coverage, failure and continuation facts — fit the 1,000-record, 16 MiB accepted-body and 16 MiB representation ceilings and the shared attempt/deadline budget; otherwise the read stops before admitting that page and reports verified earlier pages with honest coverage and a continuation only when a valid next acquisition is named. 600+400 records are accepted; 600 followed by 401 retains only the first page; a 1,001-record first page is a typed `limit_exceeded` failure.
4. **Given** an ordinary later transport/malformed/deadline/limit failure, **when** at least one verified page was admitted, **then** the result retains those pages with incomplete coverage and bounded failure facts; when no usable data was admitted, **then** it is a typed `ResourceError`, never empty success. Explicit cancellation or an authority/requested-identity violation rejects every page of that read.
5. **Given** a truncated collection, **when** the caller resumes through the named continuation, **then** the continuation is opaque, credential-free, bound to the original resource/operands/authority and issuing Path Session, preserves exact native cursor/query meaning, and acquires the next upstream page; reuse from another session or principal, or against another resource/authority, acquires no data and fails explicitly. Repeated/stalled or observed changed pagination is reported as explicit inconsistency without snapshot claims; provider caps stay separate from local limits, with no invented recovery or exhaustive fallback hiding them.
6. **Given** an individual `pr://<owner>/<repo>/<number>/comments/<id>/facts` read, **when** it succeeds, **then** it returns the same envelope with singular comment facts and no collection object; every selector, including `:raw` and any cursor, is refused.
7. **Given** a facts document larger than the inline display limit, **when** a real MCP client reads it, **then** output recovery reconstructs the exact acquired JSON before parsing independently of whether a source continuation exists; the source continuation separately acquires new upstream data and is never a condition for retaining recovered bytes (ADR-0006 meanings preserved).
8. **Given** configured deployment/repository authority and embedder-supplied credentials, **when** collection or item facts are acquired, **then** every request is read-only, authorized for the requested repository/fork/origin, charged to one shared per-read attempt/deadline/byte budget, and produces zero writes and zero egress to unauthorized origins; denied/hidden/not-found/rate/malformed outcomes map to the existing stable categories with bounded sanitized structured details, with no message parsing and no hidden-404-to-empty or denial-to-expired-auth inference.
9. **Given** a PR with zero conversation comments, **when** the collection read succeeds, **then** it returns a complete, empty collection with count zero and no records — never a typed failure — and an inaccessible or unknown collection is never reported as empty.
10. **Given** native ids/numbers above 2^53, supplied null versus absent versus empty values, unknown native enum values and Unicode/escaped bodies, **when** facts are returned, **then** those distinctions survive exactly, and an unsupported schema major is rejected by consumers.
11. **Given** applicable authorized public or controlled upstreams, **when** reusable acceptance runs, **then** public-library, independent-fixture, actual-MCP-recovery and credential-gated live read-only smoke evidence name their exact source and proof limits; skipped live rows are not passes, and corporate ghe.com/native Windows acceptance remains the separate Cyril gate.

## Selected route

Empirical — the new fact family's provider record shape and collection pagination/coverage behavior are unverified premises, and the route precedence places Empirical above Structural.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior fully explicit in the T4 contract above, inherited from the approved ticket and contract specification |
| evidence.md, probe.* | prove-it-prototype | required — Empirical route (T1 verdict); provider-shape and pagination premises with an independent oracle |
| design.md | falsifiable-design | required — schema/grammar/continuation/placement design and explicit approval |
| plan.md | budgeted-plan | required after approved design |

Oracle checkpoint in `checkpointed-build`: required — Empirical route.

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

No production edits before design approval. No commits, pushes, PR creation or tracker closure are implied by entering this route.

## Terminal criterion

Empirical — every downstream artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no `FAIL`.
