# PR #10 code review (rfs-bfwa → main, head `acd02a9`)

Automated review run on 2026-09-08 at effort `max`, with the instruction not to cap
functional or structural findings at fifteen. Ten agents ran (six angle passes plus four
verification and gap-sweep passes); forty-eight findings survived deduplication and
verification. Findings are numbered in severity order (F1 is most severe) and grouped by
theme below; Q1–Q16 are the quality/reuse/efficiency tail with no functional impact.

Many of the top findings were confirmed by running targeted probes against the PR head in
a scratch worktree. **The PR's own 42-test suite passes** — these are coverage gaps, not
test failures.

**Scope.** 42 changed files, 5,631 additions / 311 deletions, rebased onto `origin/main`
= `340d08b`.

## The through-line

Two structural patterns account for most of what follows.

1. **Guards were not carried across the family seam.** This PR forks an established
   family (`pr://.../facts`) into two new ones without re-establishing that family's
   guards. Comment-id identity (`github/mod.rs:268`), link origin confinement
   (`identity.rs`), duplicate-id validation (`mod.rs:1107`) and the post-serialization
   generation re-check (`pull.rs:259`) all exist a few files away and were simply not
   brought along. F1, F5, F6, F10 and F22 are one defect wearing five hats.

2. **The admission loop reasons about a document it never builds.** It estimates bytes in
   a different encoding than the one it emits (F3), miscounts the array punctuation it
   does estimate (F18), under-reserves for upstream-controlled header retention by 16x
   (F17), and consumes its own resume state at the loop head so that the two ceiling paths
   that *most* need a continuation are the two that cannot issue one (F4).

## Index by theme

| # | Theme | Location | One line |
|---|---|---|---|
| F1 | Dropped sibling guards | `sources/src/github/facts/comment.rs:182` | `read_item` never compares the observed comment id to the addressed one |
| F5 | Dropped sibling guards | `sources/src/github/facts/comment.rs:152` | Comment links published verbatim with no origin confinement |
| F6 | Dropped sibling guards | `sources/src/github/facts/collection.rs:351` | `validate_comment_ids` dropped; duplicates published as `complete` |
| F10 | Dropped sibling guards | `sources/src/github/facts/comment.rs:267` | Parent fetch's `cache_generation` discarded; no post-serialize re-check |
| F22 | Dropped sibling guards | `sources/src/github/facts/comment.rs:104` | Parent `Presence` fields vanish without an `unavailableFacts` entry |
| F2 | Continuation handle | `sources/src/github/facts/continuation.rs:79` | `next` is unsigned; a tampered handle drives off-allowlist egress |
| F20 | Continuation handle | `sources/src/github/fetch.rs:490` | `confine_next` ignores userinfo; reqwest appends `Authorization: Basic` |
| F23 | Continuation handle | `sources/src/github/facts/continuation.rs:60` | Unsalted SHA-256 of the session token is published in the cursor |
| F3 | Admission arithmetic | `sources/src/github/facts/collection.rs:130` | Admission measures compact JSON; emission is pretty |
| F17 | Admission arithmetic | `sources/src/github/facts/collection.rs:386` | 512-byte per-page reserve against a 16 KiB retained-header ceiling |
| F18 | Admission arithmetic | `sources/src/github/facts/collection.rs:385` | `array_bytes` double-counts commas and re-adds brackets per page |
| F4 | Continuation issuance | `sources/src/github/facts/collection.rs:382` | Local-ceiling truncation names no continuation |
| F9 | Continuation issuance | `sources/src/github/facts/collection.rs:334` | Two retryable break paths strand the page URL |
| F13 | Continuation issuance | `sources/src/github/facts/collection.rs:137` | Deterministic body-size breach yields a poison cursor |
| F7 | Partial retention | `sources/src/github/facts/collection.rs:361` | One bad record discards every earlier verified page |
| F8 | Partial retention | `sources/src/github/facts/collection.rs:293` | Deadline partial is rebuilt, then rejected by `finish_facts` |
| F11 | Identity of results | `sources/src/github/facts/collection.rs:279` | Resumed read publishes `complete` under the full-collection identity |
| F14 | Budget interaction | `sources/src/github/facts/collection.rs:213` | `maxAttempts: 1` makes both new families structurally unreadable |
| F15 | Contract regressions | `sources/src/github/mod.rs:883` | Catalog grammar dropped the PR listing and creation routes |
| F16 | Contract regressions | `sources/src/github/facts.rs:343` | Envelope reorder changes shipped facts bytes and its VersionTag |
| F19 | Contract regressions | `sources/src/github/facts/collection.rs:378` | Two names for one ceiling; failure facts drop the `limit` detail |
| F12 | Test coverage | `sources/tests/github_facts_contract.rs:1942` | New fixtures drop the session TempDirs before test bodies run |
| F21 | Test coverage | `mcp/tests/stdio_live_smoke.rs:409` | Recovery loop conflates the source cursor with recovery pages |
| F24 | Test coverage | `mcp/tests/support/profile_tls.rs:270` | Shared fixture matcher weakened to path-only for every test |
| F25 | Test coverage | `mcp/tests/stdio_mcp_contract.rs:3940` | Pre-existing recovery fence relaxed 1024 → 4096 silently |
| F26 | Repository gates | `scripts/ci-gates.py:138` | Permanent placement gate forked to a ticket-named copy |
| F27 | Type discipline | `sources/src/github/facts/collection.rs:57` | Coverage state is a `&'static str`, not a typed sum |
| F28 | Type discipline | `sources/src/github/facts/collection.rs:156` | `local_limit_error` is stringly-typed with a catch-all arm |
| F29 | Type discipline | `sources/src/github/facts/continuation.rs:94` | `SourceCursor::new` errors collapsed into a silent `Ok(None)` |
| F30 | Type discipline | `sources/src/github/facts/comment.rs:165` | Raw `u64` params replace the validated identifier newtypes |
| F31 | Documentation | `DESIGN.md:177` | Contract still says Facts do not exist for comments |
| F32 | Documentation | `docs/operating.md:186` | Acquisition-control and self-URL identity claims are now false |

---

## Dropped sibling guards

### F1. `crates/resourcefs-sources/src/github/facts/comment.rs:182`

`read_item` never compares the observed comment `id` to the addressed one — the guard
`conversation_comment` still enforces at `github/mod.rs:268` was not carried into the
facts family — and `request.commentId` is filled from the observed value, hiding the
substitution.

**Failure scenario (verified by probe).** Reading `pr://owner/repo/7/comments/9001/facts`
against an upstream answering with a different comment (id 424242, `node_id:
"IC_someone_elses"`, `issue_url` still pointing at PR 7) returns Ok with `resource:
"pr://owner/repo/7/comments/9001/facts"` but `request.commentId: "424242"`, `data.id:
"424242"`. The only cross-check is `validate_parent` on `issue_url`, which binds repo and
PR number, never the comment id. `identity::validate` (`identity.rs:27-29`) rejects
exactly this condition for the parent with `UpstreamIdentityMismatch`. The PR's own test
locks it in: `github_facts_contract.rs:1714-1729` rewrites the upstream id to
9007199254740993 and asserts the read succeeds with `data.id` equal to it.

### F5. `crates/resourcefs-sources/src/github/facts/comment.rs:152`

`CommentRecord.links` publishes the native `url`, `html_url` and `issue_url` with no
origin confinement, and `validate_parent` (`github/mod.rs:1011`) matches only path
segments with no host check — the `identity::validate_observed_link` guard the pull family
applies to every published link is never re-established for the comment family.

**Failure scenario (verified by probe on both the collection and singular routes).** An
upstream returning `{"issue_url":"https://evil.example/repos/owner/repo/issues/7",
"url":"https://evil.example/repos/owner/repo/issues/comments/9001",
"html_url":"file:///etc/passwd"}` passes `validate_parent` (its `[.., "repos", owner,
name, family, number]` pattern never inspects the host) and the values are published
verbatim as `data.links.issueUrl` / `apiUrl` / `htmlUrl`. `identity.rs:112-116` states the
invariant this violates in its own words — "publishing it puts an arbitrary URL
(`file:///…`, another host) in a field an agent reads as this deployment's" — and
`pull.rs` enforces it for `html_url`/`diff_url`/`patch_url`/`issue_url`/`_links`, plus a
userinfo rejection at `identity.rs:118-121`. DESIGN.md claims each record retains "the
verified parent PR identity and links". The comment tests only rewrite the repository path
segment, never the host.

### F6. `crates/resourcefs-sources/src/github/facts/collection.rs:351`

The facts collection drops `validate_comment_ids` — the duplicate-identity and
empty-login guard every other conversation-comment read in this adapter applies
(`github/mod.rs:380, 406, 545, 589` → `mod.rs:1107`) — so a duplicated comment id is
published as a complete, consistent collection.

**Failure scenario (verified by probe).** A comment deleted between the `page=1` and
`page=2` fetches makes GitHub's offset pagination re-serve one record across the boundary.
Fixture page 1 = ids `[1,2,3]` with `rel=next`, page 2 = ids `[3,4]`: the read returns
`state:"complete"`, `acceptedCount:5`, `data.records[*].id = ["1","2","3","3","4"]`. The
sibling `pr://owner/repo/7/comments` read of the same bytes refuses it with
`upstream_malformed` ("GitHub upstream field 'comment.id' contains a duplicate identity").
This is precisely the pagination inconsistency the `inconsistency`/`unknown` machinery
exists to report, and it is reported as `complete` instead. Same path: a record whose
`user.login` is `""` is emitted as `author.login: ""` here and rejected as malformed
there.

### F10. `crates/resourcefs-sources/src/github/facts/comment.rs:267`

`fetch_parent` returns only `(pull, endpoint)` and discards `response.cache_generation`,
and `collection::read` never re-checks the generation after building the document — two
guards that `pull::read` (`facts/pull.rs:259-267`) and the pre-PR `facts_resource` both
applied.

**Failure scenario.** Each fetch captures the generation at its own start and each
post-serialization check compares against that same response's value, so a bump strictly
*between* two fetches is invisible. Session S reads `pr://owner/repo/7/comments/facts`;
`GET /pulls/7` is served at generation G; a concurrent comment POST calls
`cache_remove_namespace("github-http")` bumping to G+1; the comments page is fetched at
G+1 and its check (`collection.rs:322`) compares G+1 to G+1 and passes. The published
`observed.parent`, every `data.records[*].parent`, and the identity validation that
accepted each comment as belonging to PR 7 all derive from a body the session declared
invalid, yet `state` reads `complete`. Separately, the collection has no post-`finish_facts`
generation check at all — with 900 records that serialization window is ~570 ms (measured
by `github_collection_production_budget`) — whereas
`facts_inflight_generation_change_refuses_publication` proves the pull family refuses
exactly this.

### F22. `crates/resourcefs-sources/src/github/facts/comment.rs:104`

`parent_facts` projects the parent PR's presence-aware `node_id`/`url`/`html_url` with
`skip_serializing_if = "Presence::omitted"`, but neither `read_item` nor
`collection::read` ever calls `Presence::unavailable` on them, so an omitted parent field
vanishes from the document without being named in `unavailableFacts`.

**Failure scenario.** `record()` (`comment.rs:135-142`) pushes only `comment.*` fields into
the unavailable list; the pull family names all 20 of its optional fields
(`pull.rs:165-170`). When an upstream PR response omits `node_id` or `html_url`,
`observed.parent.nodeId` and every record's `data.parent.nodeId` silently disappear while
`unavailableFacts` stays empty or lists only comment fields. DESIGN.md's envelope contract
is that a field is either present or named in `unavailableFacts` with reason `null` or
`omitted`; a client following it concludes the parent has no node id rather than that it
was not supplied — exactly the distinction the `Presence` machinery exists to preserve,
and which the same document type honours on the `pr://.../facts` route.

## Continuation handle

### F2. `crates/resourcefs-sources/src/github/facts/continuation.rs:79`

The continuation handle is plain base64url JSON with no integrity binding over `next`;
`CursorOwner::decode` compares only version/resource/origin/session, so a tampered handle
drives a credentialed request at a repository the operator never allowlisted.

**Failure scenario (verified by probe).** After a truncated read of an allowlisted
`pr://owner/repo/7/comments/facts`, base64-decode the handle, copy
`version`/`resource`/`origin`/`session` byte-for-byte, rewrite only `next`, re-encode.
`decode` (lines 72-79) passes all four equality checks; `confine_next` passes because
`is_repository_id_path` (`fetch.rs:504-515`) accepts
`<api_base>repositories/<any digits>/<suffix>`. Observed: `GET
/repositories/999999/issues/7/comments?per_page=100&page=2` was issued with the operator's
credential and returned Ok — repository 999999 was never seen by `authorize_repository`,
which only vetted `owner/repo` from the reference. The `/repositories/<id>/` allowance was
justified in `fetch.rs:459-463` for *provider*-issued Link headers; here it is applied to
caller-controlled input. `continuation_is_session_and_authority_bound` tampers with
`origin` but never with `next`.

### F20. `crates/resourcefs-sources/src/github/fetch.rs:490`

`confine_next` validates scheme/host/port/path but not userinfo or the query string, so a
cursor-supplied target carrying `user:pass@` passes confinement and reqwest converts it
into an appended `Authorization: Basic` header — precisely the header
`HttpRequest::with_header` refuses via `is_authority_or_framing_header`.

**Failure scenario (verified by probe: a `https://attacker:secret@host/...` target passed
confinement and was requested).** Replace `next` in a legitimate handle with
`https://attacker:secret@api.github.com/repos/owner/repo/issues/7/comments?per_page=100&page=2`
and re-encode; the `origin`/`session` digests are copied verbatim, not signed. `decode()`
matches, `confine_next` passes because `target.host_str()` is `api.github.com`,
`allowlist.authorize` also ignores userinfo, and `credentialed()` calls
`client.request(GET, url)` — reqwest strips the userinfo and **appends** `Authorization:
Basic ...`, after which the loop appends the configured GitHub credential, sending two
Authorization headers on one credentialed request (server-chosen identity) and writing
attacker-controlled text into the session cache key `format!("{accept}\n{url}")`.

### F23. `crates/resourcefs-sources/src/github/facts/continuation.rs:60`

The continuation cursor embeds an unsalted SHA-256 of the Path Session token in a plain
base64url JSON blob handed to the caller in the facts document, exposing a verifiable
oracle for the session secret and a stable cross-read correlator — while the module doc
claims "The handle carries no credential".

**Failure scenario.** `session: digest(source.session.token().as_str())` goes straight
into `Envelope`, is `serde_json::to_vec`'d and `URL_SAFE_NO_PAD`-encoded, and is published
both as `collection.continuation` in the document body and via
`resource.with_continuation(...)`. A caller base64url-decodes the handle with no key and
reads `{"version":1,"resource":"...","origin":"<sha256>","session":"<sha256 of the session
token>","next":"..."}`. `SessionToken::parse` (`session.rs:51`) accepts any externally
supplied token string, so a host that seeds a low-entropy or derived session token has it
recoverable offline; even with `SessionToken::generate`'s 128 random bits, the unsalted
digest is a stable identifier that confirms a guessed token and links cursors issued for
different resources back to the same session. An HMAC keyed on a per-session secret would
bind the envelope (fixing F2) without publishing a digest of the token itself.

## Admission arithmetic

### F3. `crates/resourcefs-sources/src/github/facts/collection.rs:130`

Page admission measures candidate documents with COMPACT JSON (`serialized_len` →
`serde_json::to_writer`) while `finish_facts` emits PRETTY JSON (`facts.rs:441`
`to_writer_pretty`) against the same ceiling, so pages the loop certifies as fitting blow
the cap at emission and the entire read fails with zero records.

**Failure scenario (verified by probe).** Two pages of 40 comment records with
`acquisition.maxRepresentationBytes = 69234`: the compact estimate reported 64,408 bytes
so both pages were admitted with `state="complete"`, then `finish_facts` pretty-printed
91,732 bytes, `CappedWriter` tripped, and the read returned a hard `limit_exceeded` with
ZERO records — instead of the honest partial the loop had just built (at cap 47,737 the
same fixture correctly yields `state:"incomplete"`, `acceptedCount:1`). Pretty printing
costs ~352 bytes per record at this nesting depth (+45%) against a fixed 512-byte reserve.
This falsifies DESIGN.md ("a page is admitted only while ... the resulting final
serialized representation ... fit") and the module docstring ("earlier verified pages
survive with explicit coverage"). The continuation-drop fallback at line 494 cannot recover
a percentage overshoot. Tests miss it because `representation_ceiling_includes_outcome_overhead`
uses 1-2 records, where the fixed reserve masks the delta.

### F17. `crates/resourcefs-sources/src/github/facts/collection.rs:386`

`pages_reserve` reserves a flat 512 bytes per page observation, but a single
`BodyObservation` retains upstream `etag`/`lastModified`/`date`/`selectedApiVersion`
strings bounded only by the 16 KiB `MAX_SOURCE_REQUEST_HEADER_BYTES` ceiling — and may be
paired with a second `revalidation` observation.

**Failure scenario (verified by probe).** Identical 5-record page, short `ETag` vs an 8 KiB
`ETag` (well under the retained-header ceiling): the published document grew from 7,516 to
15,703 bytes — 8,187 bytes of upstream-controlled growth against a 512-byte reserve, a 16x
under-reserve for one page. With 10 pages the ledger under-reserves by ~80 KB before the
`revalidation` observation is counted, so a read the admission arithmetic cleared is
refused by `finish_facts` at the same ceiling and — because the local-ceiling stop emits no
continuation (F4) — returns `limit_exceeded` with nothing published. The probe envelope at
line 267 also measures `unavailable_facts: Vec::new()`, so up to 16 coverage entries
(8 fields × 2 reasons, ~100 pretty bytes each) are appended with no budget at all, and
`OUTCOME_RESERVE_BYTES`'s own doc comment still says a continuation "is not yet issued by
this slice" while this PR issues ~350-570-byte handles from the same 512 bytes.

### F18. `crates/resourcefs-sources/src/github/facts/collection.rs:385`

`array_bytes = accepted_bytes + page_bytes.saturating_sub(2) + separators + 2` re-adds the
cumulative separator count and the array brackets on every page, so `localLimit.observed`
publishes a byte count no document ever had.

**Failure scenario.** `page_bytes` from `serialized_len(&page_records)` is S + (n_k − 1)
commas + 2 brackets; stripping 2 leaves this page's own commas in, and then `separators =
candidate_records - 1` (the CUMULATIVE count) is added on top, while `accepted_bytes`
already carries the previous iteration's brackets and separators. One page of n records
gives S + 2n against a true S + n + 1; two pages give S + 3n₁ + 2n₂. Ten 100-record pages
over-count by ~5,500 bytes plus 2 per page. The inline comment ("the array brackets, which
are re-added once in `array_bytes`") describes an invariant the code does not hold, since
`+ 2` is inside the loop. Effect: with a caller-set `maxRepresentationBytes` near the true
size a page that would fit is refused, and the `localLimit.observed` value handed to a
caller trying to pick a working ceiling is fabricated rather than measured. It also
partially masks the opposite-direction F3 error, so fixing either alone shifts the
truncation point unpredictably.

A third pattern is worth stating separately, because it changes what to do about the first
two: **the test suite is not thin, and it is not a passive detector that merely missed
things.** Forty-two tests pass, and they touch nearly every path that turned out to be
broken. See [Why the suite did not catch these](#why-the-suite-did-not-catch-these) at the
end of this document.

## Continuation issuance

### F4. `crates/resourcefs-sources/src/github/facts/collection.rs:382`

`while let Some(url) = pending.take()` empties `pending` at the loop head, and the two
local-ceiling breaks (records at 382, representation bytes at 402) never restore it, so a
locally truncated collection reaches the continuation match at line 427 with `pending ==
None` and names no continuation at all.

**Failure scenario (verified by probe, both ceilings).** The PR's own 600+401 fixture
yields `state="incomplete", acceptedCount=600, localLimit.kind="records",
continuation=None, resource.continuation()=None`; the representation-ceiling fixture yields
`state="incomplete", acceptedCount=1, continuation=None`. Records past the ceiling are
permanently unreachable even though the refused page's `url` is still live on the stack
(`facts_page` took `url.clone()`), and re-reading replays the identical truncation. This
contradicts DESIGN.md line 206, added in this same PR — a continuation is "named only for
a page that was never fetched (a local ceiling or a retryable attempt/deadline
exhaustion)" — and docs/operating.md ("A truncated collection names
`collection.continuation`"). Only the attempt/deadline branches (298, 315) ever issue a
handle, and those are the only ones the contract tests exercise.

### F9. `crates/resourcefs-sources/src/github/facts/collection.rs:334`

The cache-generation-mismatch (334) and malformed-page (345) breaks never restore
`pending`, even though both build `SourceUnavailable` errors that `retryable()` (line 137)
classifies as retryable — so those partials ship with no continuation and the caller is
told to retry with nothing to retry from.

**Failure scenario.** Both paths construct `failure(ErrorReason::UpstreamUnavailable)` /
`failure(ErrorReason::UpstreamMalformed)` but, unlike the `facts_page` arm at 309-319, omit
`if retryable(&error) { pending = Some(url); }` before `break`.
`github_facts_contract.rs:2061` exercises exactly this: 100 records then a `{malformed`
page yields `state:"incomplete"`, `failure.reason:"upstream_malformed"`, and no
continuation. A caller who could simply have retried that one page target must instead
restart from page 1 and re-spend the whole shared attempt and byte budget on data it
already holds. Three break paths in one loop disagree about resumability for errors of the
same category — a divergence that reads as intentional only because the degrade-or-reject
block is copy-pasted four times (293-303, 309-319, 328-335, 339-346; see Q4).

### F13. `crates/resourcefs-sources/src/github/facts/collection.rs:137`

`retryable()` classifies every `LimitExceeded` as retryable, but a per-page
`maxResponseBytes` breach is deterministic, so the read publishes a continuation handle
that can never advance.

**Failure scenario.** Page 2 of `pr://owner/repo/7/comments/facts` is a body over
`maxResponseBytes` (default 8 MiB). `fetch_controlled` → `HttpReadBudget::admit_body`
returns `LimitExceeded`/`ResponseBodyBytes`; the loop (309-319) sees `comments` non-empty
and `retryable` true, sets `pending = Some(url)` and publishes page 1 plus a `:cursor:`
handle for page 2. Every resume of that handle starts with `comments.is_empty()`, re-hits
the identical fixed ceiling and returns a hard `limit_exceeded` — a poison cursor the
caller is told to follow. Because `failure_facts` (line 96) also drops the `limit` detail,
the caller is never told which ceiling to raise.

## Partial retention

### F7. `crates/resourcefs-sources/src/github/facts/collection.rs:361`

A record that fails projection in a later page takes `Err(error) => return Err(error)`,
bypassing the partial-retention logic that every adjacent failure branch uses, so one bad
record discards every earlier verified page.

**Failure scenario (verified by probe).** Page 1 admits 100 records; page 2 contains one
record with `id` absent. `comment::record` → `native_field` returns `malformed_upstream`
and line 361 returns it directly: the read fails with `source_unavailable` and the 100
verified records are lost. Every sibling failure site (check_acceptance 293, facts_page
309, cache generation 329, page decode 340) first checks `comments.is_empty() ||
rejects_every_page(&error)`; the per-record arm skips it. DESIGN.md's rule is "An ordinary
later failure preserves verified earlier pages; a first unusable page is a typed error",
and this path applies the wrong half of it. `partial_retention_and_rejection_precedence`
covers a malformed *page*, never a malformed *record*.

### F8. `crates/resourcefs-sources/src/github/facts/collection.rs:293`

The deadline-exhaustion partial can never be published: the loop builds
`state:"incomplete"` plus failure facts plus a continuation, then `finish_facts` re-runs
`read.check_acceptance()` (`facts.rs:464`) against the same already-expired deadline and
returns Err, discarding everything.

**Failure scenario.** `check_acceptance` (`http/read.rs:134`) fails when
`self.deadline.remaining().is_zero()`, returning `deadline_error` with
`ErrorCategory::SourceUnavailable` — not in `rejects_every_page`, and in `retryable`. So
with pages already admitted the loop takes the honest-partial branch (`pending =
Some(url)`, `state = "incomplete"`, `recorded_failure`, `break`), issues a valid
continuation, and calls `finish_facts`, whose trailing `read.check_acceptance()?` evaluates
the same expired deadline (time only moves forward) and returns Err. The caller gets a hard
`source_unavailable`/`deadline_exceeded` instead of the verified pages plus a resume
handle. docs/operating.md, added in this PR, promises the opposite: "A later transport,
malformed or deadline failure keeps the verified earlier pages with `incomplete`
coverage." There is no deadline test for the collection.

## Identity of results

### F11. `crates/resourcefs-sources/src/github/facts/collection.rs:279`

A read resumed from a continuation handle publishes `state: "complete"` under the FULL
collection's canonical identity, with nothing in the document marking it as a tail.

**Failure scenario (verified by probe).** Reading
`pr://owner/repo/7/comments/facts:cursor:<handle>` after a truncated first page returns a
document with `resource: "pr://owner/repo/7/comments/facts"`, `collection.state:
"complete"`, `acceptedCount: 100`, first record id `"101"` — indistinguishable from a
complete read of a PR that has exactly 100 comments, and carrying the same
`canonical_reference()` as the first read. An agent that reads the resumed document alone
believes it holds the whole conversation-comment collection for PR 7 while records 1-100
are absent; nothing in the envelope (`request`, `collection`, `resource`) records that a
cursor was supplied.

## Budget interaction

### F14. `crates/resourcefs-sources/src/github/facts/collection.rs:213`

Both new resources need at least two attempts (the parent lookup is charged to the same
shared budget), so `acquisition.maxAttempts: 1` — a valid, documented control that works
for `pr://.../facts` — makes the whole conversation-comment fact family structurally
unreadable.

**Failure scenario.** `comment::fetch_parent` consumes the only attempt via
`fetch_controlled(..., Some(&mut ctx.budget))`; the first `facts_page` (or the
`issues/comments/{id}` fetch) then hits `remaining_attempts.checked_sub(1)` on 0
(`http/read.rs:99`) and returns `LimitExceeded`. In `collection::read`
`comments.is_empty()` is true, so it is a hard `return Err(error)` — even an empty
collection cannot be read. `ReadAcquisitionLimits::new(Some(1), ...)` is exercised as a
normal setting for pull facts at `github_facts_contract.rs:635`, and a profile-level
`"acquisition":{"maxAttempts":1}` is used at `stdio_mcp_contract.rs:3912`. An operator who
pins that profile silently loses both new resources, with nothing in docs/operating.md
warning of the two-attempt floor.

## Contract regressions

### F15. `crates/resourcefs-sources/src/github/mod.rs:883`

The rewritten `pr://` catalog grammar dropped the optionality around `/<number>`, so
`pr://<owner>/<repository>` (the repository PR listing) and
`pr://<owner>/<repository>/new` (the PR Creation Target) are no longer advertised even
though both routes are still parsed and served.

**Failure scenario.** Base spelling:
`pr://<owner>/<repository>[/<number>[/title|body|facts|comments|reviews|review-comments|diff]][:selector]`.
Head spelling makes `<number>` mandatory. `parse_pull_request_address` still maps `[owner,
repository]` to `PullRequestAddress::Collection` (`reference.rs:1315`), which
`pull_request_resource` serves at `github/mod.rs:435`, and `[owner, repository, "new"]` to
`PullRequestAddress::New`, which `GithubCreationTarget::parse` still routes at
`mutation.rs:187`. An agent that discovers routes from `rfs_read rfs://` (the documented
entry point) can no longer find how to list a repository's pull requests or how to create
one, and the new `catalog_advertises_comment_facts` test plus `stdio_mcp_contract.rs:4203`
assert the narrower spelling, locking the regression in. The sibling `issue://` entry on
line 877 kept its `[/<number>[...]]` optionality, indicating the change was accidental.

### F16. `crates/resourcefs-sources/src/github/facts.rs:343`

Making `Facts` generic with `#[serde(flatten)] body` moved `acquisition` ahead of
`request`/`observed`, silently changing the serialized bytes of the already-shipped
`pr://.../facts` document while `schemaVersion` stays `{major:1,minor:0}`.

**Failure scenario.** The base envelope order (`340d08b facts.rs:443-455`) is
schemaVersion, kind, resource, source, repository, request, observed, acquisition,
upstream, data, unavailableFacts. The head order — asserted by this PR's own new
`envelope_serializes_shared_fields_in_order` test — is schemaVersion, kind, resource,
source, repository, **acquisition**, request, observed, upstream, data, unavailableFacts.
Reading the same unchanged PR before and after the upgrade produces different bytes and
therefore a different content-derived `VersionTag` (AGENTS.md: "`VersionTag` — `sha256:`
plus 64 lowercase hex; it names exact content"), so a client holding
`pr://owner/repo/7/facts` sees a spurious change, while `schemaVersion` gives it no signal
that the envelope moved. The PR body claims it "reuse[s] the rfs-0n97 envelope".

### F19. `crates/resourcefs-sources/src/github/facts/collection.rs:378`

The published `collection.localLimit.kind` is the hand-typed literal `"records"` while the
typed error for the identical ceiling reports
`AcquisitionLimitKind::CollectionRecords.as_str()` = `"collection_records"`, and
`failure_facts` (line 96) drops the `limit` detail and collapses
`DelaySeconds`/`AtUnixSeconds` into one scalar.

**Failure scenario.** The PR's own `collection_admits_whole_pages_atomically` asserts both
spellings in one test: `facts["collection"]["localLimit"]["kind"] == "records"` for the
partial result and `error...limit().kind().as_str() == "collection_records"` for the whole
refusal, so a client matching on the machine vocabulary sees two different names for one
ceiling depending on whether the read was partially or wholly refused (the
representation-bytes literal happens to match its enum, hiding the divergence). Separately,
`FailureFacts` has no `limit` field, so a page refused by `maxAcceptedBodyBytes` embeds
`collection.failure = {category:"limit_exceeded", reason:"limit_exceeded"}` with no
`kind`/`bound`/`observed`, while the same refusal surfaced through `sanitize` carries the
full `LimitDetail` — and `retryAfterSeconds` cannot distinguish a relative delay from an
absolute Unix deadline, which `render/error.rs::DetailsOutput` (the existing projection
this duplicates) does distinguish.

## Test coverage

### F12. `crates/resourcefs-sources/tests/github_facts_contract.rs:1942`

`collection_fixture` (1942) and `comment_fixture` (1549) bind the `ScratchFixture` to
`_session` and return only `(listener, source)`, dropping the session store's `TempDir`s —
including the one holding the live session's `session_dir`/`objects_dir` — before any test
body runs.

**Failure scenario.** `build_source` returns `(GithubSource, ScratchFixture)`; the helper
drops the fixture at its own return, so `TempDir::drop` recursively removes the directory
`DiskSessionStorage::create` made (`session_storage.rs:184-196`), while the `GithubSource`
keeps the cloned `PathSession` pointing at the now-missing `objects_dir`. Every
collection/comment test therefore runs against a deleted session directory: the moment one
exercises Session Scratch — `session.retain` for the artifact recovery this PR documents in
operating.md, or a `ReadEngine` read of the 6 MB `github_collection_production_budget`
document — the write fails with a bare `NotFound` I/O error instead of testing the path.
The pre-existing `fixture()` helper at line 90 returns the session precisely to keep it
alive.

### F21. `crates/resourcefs-mcp/tests/stdio_live_smoke.rs:409`

The new MCP tests follow `continuationReference` in a concatenate-and-parse loop, but for a
truncated collection that field carries the source `:cursor:` handle (a whole second
document, plus new upstream egress), not an artifact-recovery page.

**Failure scenario.** `ReadEngine::read` puts `parts.continuation` — the `:cursor:` handle
— into `continuationReference` whenever the document did not overflow the display limit
(`read.rs:161-168`); only the overflow branch routes it to `sourceContinuationReference`.
Run `live_stdio_github_comment_facts_match_native_observation` against a PR with more than
nine pages of comments (or one that trips the record/byte ceiling): the loop follows the
cursor, appends a second complete facts envelope to `bytes`, and
`serde_json::from_str(&bytes)` fails with "trailing characters". The same loop in
`github_comment_facts_recover_without_reacquisition` (`stdio_mcp_contract.rs:4139`) would
silently issue extra credentialed acquisitions inside a block whose own assertion claims
"recovery cannot acquire again"; neither test checks `recoveryReference` to tell the two
continuation kinds apart.

### F24. `crates/resourcefs-mcp/tests/support/profile_tls.rs:270`

The shared fixture matcher is weakened to `response.path == target.split('?').next()` so
the new collection read's `?per_page=100&page=N` requests still match — relaxing query
verification for every profile-TLS contract test in the repo to accommodate one new family.

**Failure scenario.** Matching is ordinal (`responses.get(sequence - 1)`) plus path, so
with the query stripped a paginated read that requests `page=1` twice, requests `page=3`
when the fixture models `page=2`, sends the wrong `per_page`, or emits a cursor pointing at
the wrong offset now gets served the expected fixture body and every MCP-level assertion
passes. Only the raw request count remains as a signal. This is exactly the fixture-drift
class AGENTS.md's live-smoke section says motivated that section ("an adapter that could
not paginate a real repository while its fake stayed green"). `RecordedRequest.path` still
carries the full target, so the information needed to assert this exists and is simply no
longer used. The fix is to let the fixture table express query expectations rather than
have the matcher decide no fixture may ever care. (The `unwrap_or(target)` fallback on line
268 is also dead — `str::split` always yields at least one item.)

### F25. `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs:3940`

The pre-existing PR-facts recovery fence had its continuation-page budget relaxed from
`{"bytes":1024}` to `{"bytes":4096}` in commit `a129a45`, whose message documents only the
catalog/mutation/recovery additions and never mentions the change.

**Failure scenario.** `github_facts_stdio_reconstructs_native_json_without_reacquisition`
previously asserted that an oversized `pr://owner/repo/7/facts` document can be paged back
through MCP artifact recovery at a 1 KiB page budget; that assertion is gone. The
recovery-root read in the same test still uses `{"bytes":1024}` and passes, so the
relaxation is specific to the *continuation* pages. Either the flatten/envelope reordering
in `facts.rs` genuinely regressed 1 KiB continuation paging for pull facts (a user-visible
regression for any caller passing `limits.bytes <= 1024`), or a regression fence was
weakened for convenience — either way the 1 KiB continuation-page contract is no longer
covered by any test.

## Repository gates

### F26. `scripts/ci-gates.py:138`

The permanent placement gate is repointed from `scripts/module_shape.py` to a ticket-named
successor `scripts/module_shape_bfwa.py` that `importlib`-loads its predecessor and reads a
copy-and-append ledger `scripts/module-ledger-bfwa.json`, orphaning the shared ledger and
reversing the immediately preceding commit `dd03b13`.

**Failure scenario.** The chain is now five deep — `module_shape_bfwa.py` →
`module_shape.py` → `module_shape_base.py` → `.rfs-h212/oracles/module_shape.py` (still
reaching into a frozen ticket directory that `dd03b13` was supposed to escape).
`scripts/module-ledger.json` — 5 KB of ownership rows, protected-parent caps and
forbidden-dependency regexes — is now loaded by nothing, so regressions it records stop
being enforced while two contradictory policy files sit side by side and drift
independently (9 of 14 top-level keys are byte-identical today).
`module_shape_bfwa.py::main()` is a verbatim copy of its predecessor's, differing only in
the ledger filename, and it drops the argparse `choices=(...)` whitelist on `--stage`, so
an invalid stage raises `ValueError` instead of a usage error. The fork also raises the
`facts.rs` growth tripwire from 750 to 760 for a file that shrank to 574 lines.
`module_shape.py`'s own docstring warns that a permanent gate tied to ticket-scoped
evidence "breaks whenever that evidence is archived".

## Type discipline

### F27. `crates/resourcefs-sources/src/github/facts/collection.rs:57`

Collection coverage is carried as `state: &'static str` with values `"complete" |
"incomplete" | "unknown"` assigned at nine sites and re-read by string pattern in `match
(&pending, state)` at line 427 — the load-bearing continuation decision — against
AGENTS.md's "parse once into a typed sum; match on variants, never re-parse strings".

**Failure scenario.** The continuation rule at 427-431 — `(Some(_), "complete" | "unknown")
=> None` — is the safety decision that a resumable cursor is never named for a stalled or
complete traversal, and it is enforced by string literal. A typo at any of the nine
assignment sites (259, 279, 287, 300, 316, 332, 343, 376, 396) falls through to the
`(Some(target), _)` arm and issues a continuation for a collection DESIGN.md line 206 says
must not have one, with no compile error and no exhaustiveness check. Adding a fourth state
later silently changes that arm's behaviour instead of breaking the build. An `enum
CoverageState` with a snake_case `Serialize` gives the compiler the three-value vocabulary
DESIGN.md already fixes.

### F28. `crates/resourcefs-sources/src/github/facts/collection.rs:156`

`local_limit_error(kind: &'static str, ...)` re-parses the string into
`AcquisitionLimitKind` through a match with a catch-all `_` arm, computes a kind string it
immediately discards (`let _ = kind;` at 177), and swallows a `LimitDetail::new` failure
with `if let Ok(detail)`, emitting a `limit_exceeded` error with no `details` at all.

**Failure scenario.** A typo or a third limit kind at a call site (`"record"`, `"bytes"`)
compiles and falls through the catch-all, so the typed `LimitDetail` reports
RepresentationBytes with a records bound — the machine-readable error contradicts the human
message, and no test catches it because the string is discarded before it can be asserted
on. The dropped `Err` arm violates AGENTS.md's silent-failure discipline and DESIGN.md line
51's promise of `limit` (`kind`, positive `bound`, optional `observed`) on limit errors;
every other `LimitDetail::new` call site propagates (`facts.rs:448` uses `?`) or asserts
(`http/read.rs:301` uses `.expect`). The fix is `fn local_limit_error(kind:
AcquisitionLimitKind, bound: usize, observed: usize)` with the two call sites passing
variants, deleting the match, the `let _`, and the divergent hand-typed `LocalLimit.kind`
literals.

### F29. `crates/resourcefs-sources/src/github/facts/continuation.rs:94`

`CursorOwner::continuation` returns `Ok(None)` for every `SourceCursor::new` failure,
including the `invalid_reference` "not canonical base64url" case, with no log — collapsing
"too large to represent" and "this module encoded its own handle wrong" into one silent
outcome.

**Failure scenario.** `SourceCursor::new` (`reference/source_page.rs:43`) fails two
distinct ways — `LimitExceeded` "source cursor exceeds the reference byte ceiling" (66-69)
and `invalid_reference("source cursor must be canonical base64url")` (80-82) — and the `let
Ok(cursor) = ... else { return Ok(None) }` arm discards both. A corrupt handle produced by
this module then presents to the caller as a collection that simply had no continuation,
which is indistinguishable from a complete read. The very next arm in the same function
proves the intended discipline by filtering on category: `Err(error) if error.category() ==
ErrorCategory::LimitExceeded => Ok(None)` with `Err(error) => Err(error)`.

### F30. `crates/resourcefs-sources/src/github/facts/comment.rs:165`

New fact-family signatures take raw `u64` for the PR number and comment id even though the
validated newtypes `PullRequestNumber` and `ConversationCommentId` exist and the pre-PR
code carried them to the format site.

**Failure scenario.** AGENTS.md: "Never pass a raw `String`, `PathBuf`, or `u64` where the
domain has a type." The types exist at `reference.rs:471-472`; this PR strips them at the
dispatch (`facts.rs:559` `let number = number.get();`, then `comment::read_item(self,
repository, number, id.get(), &mut ctx)`), so `read_item` has two adjacent same-typed `u64`
parameters — `number` and `comment_id` — and a transposed call compiles.
`collection::read` (197) and `comment::record` (126) took the same downgrade. Given that
neither route validates the returned comment id against the requested one (F1), a
transposition would also not be caught at runtime.

## Documentation

### F31. `DESIGN.md:177`

The binding PR Facts contract paragraph still states that Facts accept no projection
selector and are unavailable for PR collections and comments — both falsified by this PR —
and was not updated in the same commit as AGENTS.md requires.

**Failure scenario.** AGENTS.md makes DESIGN.md the Behavior Contract and requires contract
changes to "update `DESIGN.md` in the same commit". DESIGN.md:177 still reads verbatim: "It
accepts no projection selector, including `:raw`. Facts are not available for issues, PR
collections, reviews, comments, or diffs." This PR adds `facts.rs:522` `if
reference.projection().is_some() && !(matches!(fact, PullRequestFact::Comments) &&
cursor.is_some())` — a projection selector IS now accepted on a Facts route — and adds
Facts for a PR collection and for comments, described in a new section twenty-three lines
below. The document readers are told to trust now asserts both that comment Facts do not
exist and that they do.

### F32. `docs/operating.md:186`

Two operator-facing guarantees in the same section are falsified by this PR: line 186
claims only PR Facts accept `acquisition` controls, and line 192 promises "the identity
check compares the object's own `url` against the endpoint that was requested" — which
neither comment route performs.

**Failure scenario.** Line 186: `github/mod.rs:818-826` now calls `reject_acquisition` only
when the address is not a Facts route, so `PullRequestFact::Comments` and `Comment(_)` also
accept the controls — and the section this PR adds at line 200 says so explicitly. Line
192: `identity::validate` enforces `same_url(pull.url, endpoint)` for the parent
(`identity.rs:30`), but `comment::read_item` moves `endpoint` into `fetch_controlled`
(`comment.rs:178-181`) and never re-checks it, and `collection::read` never checks any
page's self-URL. Read `pr://owner/repo/7/comments/9001/facts` where the upstream returns a
comment whose `url` is `https://api.github.com/repos/other/repo/issues/comments/42`:
`validate_parent` only inspects `issue_url`, so the mismatched self-URL is published
verbatim as `data.links.apiUrl` while the document still claims `request.commentId` 9001 —
the exact backstop line 192 promises callers they have.

---

## Quality tail (no functional impact)

Ranked below F32. Reuse, simplification and efficiency.

| # | Location | Issue |
|---|---|---|
| Q1 | `sources/src/github/facts/continuation.rs:45` | The whole `CursorOwner`/`Envelope` module duplicates `atlassian/jira/cursor.rs` field-for-field — same struct name, same `deny_unknown_fields` envelope of borrowed `Cow`s, same digest, same `URL_SAFE_NO_PAD` encode/decode, same version check, same error pair. The Jira copy already bounds the token against `MAX_PATH_REFERENCE_BYTES` before serializing; the GitHub copy has no pre-bound. A `version: 2` rollout has to be remembered twice. |
| Q2 | `sources/src/github/facts/collection.rs:96` | `FailureFacts` + `failure_facts()` re-derive the camelCase projection of `ResourceErrorDetails` that `mcp/src/render/error.rs::DetailsOutput` already implements as `From<&ResourceErrorDetails>`. Since `resourcefs-mcp` depends on `resourcefs-sources`, the one projection belongs beside `ResourceErrorDetails` in core. The copies have already diverged on retry guidance (see F19). |
| Q3 | `sources/src/github/facts/collection.rs:118` | `serialized_len`'s private `Counter` is the fourth hand-rolled byte-accounting writer in the crate — `facts.rs::CappedWriter` (same parent module), `atlassian/jira/cursor.rs::CursorBytes<W>` (already generic, already used with `io::sink()` purely to measure), `atlassian/wire/query.rs::QueryBytes`. |
| Q4 | `sources/src/github/facts/collection.rs:293` | The degrade-or-reject block is copy-pasted four times (293-303, 309-319, 328-335, 339-346) with the retry-requeue arm present in only two. Extract one `fn soften_or_reject(...)` or a `PageOutcome` enum. F9 exists only because of this duplication. |
| Q5 | `sources/src/github/facts/collection.rs:226` | The envelope-measuring `Facts { ... }` literal (226-268) and the real one (435-481) are near-identical ~45-line literals differing in five fields. They are the byte-admission contract: drift between them makes `envelope_bytes` an under-estimate and compiles cleanly. |
| Q6 | `sources/src/github/facts/collection.rs:411` | Every record is projected through `comment::record` twice — once per page for measurement (352, dropped at 366) and once over all of `comments` (412-424) — because `comments.extend(decoded)` invalidates the first pass's borrows. The second pass's `projection_unavailable` sink is write-only: filled, never merged, never read. ~1,000 wasted `Url::parse` calls, ~1,000 wasted `Vec` allocations in `validate_parent` and ~8,000 wasted pushes on a full collection. |
| Q7 | `sources/src/github/facts/comment.rs:147` | `parent_facts(pull, identity)` is embedded in and re-serialized for every record, producing byte-identical parent JSON 1,000 times though the envelope already carries it once at `body.observed.parent`. ~200 KB of duplicated JSON in the document — ceiling that could hold ~170 more real comments. Pre-encode once into a `RawValue`. |
| Q8 | `sources/src/github/fetch.rs:43` | `PageResponse` is a field-for-field copy of `FetchedResponse` minus `link` plus `next`, and building it forced `FetchedResponse::body` and `::link` to `pub(super)`, re-opening the raw `Arc<[u8]>` to the whole `github` module past the `body()` accessor. Have `facts_page` return `(FetchedResponse, Option<Url>)`. |
| Q9 | `sources/src/github/facts/comment.rs:31` | `ParentLinks` is a byte-identical duplicate of the private `facts.rs::Links`, and `CommentLinks` is that shape plus `issue_url`. Four link-shaped structs (`Links`, `ParentLinks`, `CommentLinks`, `pull.rs::PullLinks`) now emit the same `apiUrl`/`htmlUrl` keys. |
| Q10 | `sources/src/github/facts/continuation.rs:97` | The continuation reference is built by `format!("{}:cursor:{}", canonical.requested(), cursor.as_str())` and re-parsed, re-implementing the typed `PathReference::pull_request(address, Some(ProjectionSelector::from_source_cursor(cursor)))` that `facts.rs:527` already calls and the Jira twin uses. A change to the separator spelling breaks only this hand-built string, and only on collections large enough to paginate. |
| Q11 | `core/src/reference/pull.rs:27` | A second hand-rolled `str::find(":cursor:")` pre-split plus a hardcoded allowlist in core grammar. `reference/jira.rs:398-431` has the identical shape and `jira.rs:443-450` a third copy inside `canonical_record_identity`; the two have already diverged (jira handles `:offset:` and takes `.min()`, pull handles neither). Move marker set and eligibility onto the address family. |
| Q12 | `core/src/resource.rs:19` | `MAX_COLLECTION_RECORDS` is the only `MAX_*` in `resource.rs` with no core consumer — exported purely so `github/facts/collection.rs:368` can compare against it, while `MAX_TEXT_BYTES` and friends are all enforced by `limits.rs`/`read.rs` validators. It also drags a public `AcquisitionLimitKind::CollectionRecords` variant into the Behavior Contract's finite vocabulary for a bound no caller can set. |
| Q13 | `mcp/src/acquisition.rs:67` | The new `AcquisitionLimitKind::CollectionRecords => return error.message()` arm in `describe_limit_rejection` is unreachable (all three call sites validate caller `acquisition` input, which can never produce that kind), and it hand-maintains a caller-writable/local-policy split that the `AcquisitionField` enum ten lines below already encodes. A `const fn caller_field(self) -> Option<&'static str>` makes the table total. |
| Q14 | `sources/tests/github_live_smoke.rs:311` | The new live test spawns `gh api -H "X-GitHub-Api-Version: 2022-11-28"` inline twice with `.expect(...)` instead of calling `native_gh(token, endpoint)`, the helper at line 53 of the same file used by the sibling test at 286. `native_gh` exists to return `None` when `gh` is absent, so a runner with `RFS_LIVE=1` and a token but no `gh` panics where its neighbour skips. The pinned API-version header is now spelled in four places across two crates. |
| Q15 | `sources/tests/github_facts_contract.rs:1913` | `collection_fixture` (1913) and `comment_fixture` (1516) each re-copy the whole router preamble of `fixture_with_session` (87) — the same `TlsListener::serve_request_router` setup, the same GET guard, the byte-identical 8-line Host-header `find_map`, the same `NATIVE.replace("@API@", ...)` templating — differing only in their route tables. Three independent copies in one file. |
| Q16 | `sources/src/github/facts/collection.rs:213`, `:322`, `facts.rs:437` | Efficiency: the `pulls/{number}` parent lookup is fully awaited before the first page URL is even constructed though the two requests are independent (~150-300 ms per read, on the 30 s shared deadline; needs a reserve-then-reconcile permit split since `HttpReadBudget` is non-cloneable). An extra `session.cache_generation(...)` per page takes the session-wide `inner.admission` mutex where one comparison before and after the loop detects the same window. `CappedWriter` starts at `Vec::new()` though `candidate_bytes` just computed a tight estimate and discarded it — a 16 MiB document is grown through ~24 reallocations and ~32 MiB of memcpy. |

---

## Why the suite did not catch these

Mostly the wrong *kind* of test, not the wrong *amount* — with a subset where the tests
were themselves the defect. The suite is not thin: 42 tests pass, and they execute nearly
every path that turned out to be broken.

The distinguishing symptom of an oracle problem rather than a coverage problem is a test
that runs the defective line and passes. That is not a gap closed by writing more tests of
the same kind; more tests drawn from the same oracle just add more passing assertions next
to the bug.

### Tests that stood directly on the bug and asserted past it — F1, F3, F4, F9, F19

`collection_admits_whole_pages_atomically` and `representation_ceiling_includes_outcome_overhead`
both drive the exact truncation path in F4; they assert `localLimit.kind` and
`acceptedCount` and never look at `continuation`. `github_facts_contract.rs:2061` runs the
malformed-page fixture in F9 and asserts `state` and `failure.reason`, not resumability.
`representation_ceiling_includes_outcome_overhead` exercises the precise
gate-passes-then-serializer-fails sequence of F3 and asserts only the error category. F19
is the sharpest: the PR's own test asserts `"records"` and `"collection_records"` in one
body without noticing they name the same ceiling. These are pass-by-omission, not absence.

### The oracle came from the implementation, not the contract — F4, F7, F8, F11, F13

Each of these contradicts a sentence in `DESIGN.md` or `docs/operating.md` *written in this
same PR*. The contract text is more precise than the tests meant to enforce it — "a
continuation is named only for a page that was never fetched (a local ceiling or a
retryable attempt/deadline exhaustion)"; "a later transport, malformed or deadline failure
keeps the verified earlier pages with `incomplete` coverage." Nobody derived assertions
from those sentences; the assertions were derived from watching the code run. F31 and F32
are the same failure one level up: the docs and the code drifted and nothing compares them.

### No differential test against the sibling route — F1, F5, F6, F10, F22

All five are "the human route refuses this; the facts route publishes it." One test — the
same fixture bytes through `pr://owner/repo/7/comments` and
`pr://owner/repo/7/comments/facts`, asserting both accept or both refuse — catches four of
them mechanically, without anyone having to anticipate duplicate ids, hostile hosts, or
comment-id substitution.

### Fixtures too small to expose the arithmetic — F3, F17, F18

All three are correct-ish at 1-2 records and wrong at 100. There *is* a 900-record test
(`github_collection_production_budget`) but it asserts wall-clock timing, so the one large
fixture in the suite is pointed at the wrong property.

### Adversarial input enumerated one field — F2, F20, F23

`continuation_is_session_and_authority_bound` has exactly the right shape for F2: decode
the envelope, tamper, assert refusal. It tampers with `origin` and stops. The one field it
does not touch is `next` — the only field that becomes a URL the process then requests.
F20 and F23 sit in the same blind spot. Exhaustive per-field tampering would have found F2
and F20 without either being conceived of in advance.

### Cases where the tests were the defect — F1, F12, F15, F24, F25

Not a detector that missed something; an artifact that encoded, relaxed, or locked in the
wrong behaviour.

- **F12** — the new fixtures drop the session `TempDir`s before any test body runs, so the
  harness itself is broken.
- **F24** — the shared matcher was weakened to path-only for *every* profile-TLS test to
  accommodate one new family.
- **F25** — a pre-existing 1 KiB regression fence was relaxed to 4 KiB in a commit whose
  message does not mention it.
- **F15** — a new test asserts the *narrowed* catalog grammar, locking the regression in.
- **F1** — the test actively asserts the buggy behaviour: it rewrites the upstream comment
  id and asserts the read succeeds with the substituted value.

No quantity of tests catches a test that encodes the defect as intent. That needs review of
the test diff, which is where four of these five surfaced.

### Not test-shaped at all — F16, F26, F31, F32

F16 needs golden bytes for the shipped envelope (an artifact, not an assertion). F26, F31
and F32 are process and documentation.

### The live-smoke irony

`AGENTS.md` documents this exact failure mode as the reason the live-smoke section exists —
"an adapter that could not paginate a real repository while its fake stayed green." This PR
does ship a live row. But its continuation assertion sits inside an `if state ==
"incomplete"` branch that cannot fire for the small PR it targets, so the one instrument
built specifically to catch fixture drift was gated off by its own fixture choice.

### Cheapest changes, by findings covered

| Change | Covers |
|---|---|
| One sibling-route differential test over shared fixture bytes | F1, F5, F6, F10 |
| Whole-document assertions on the truncation paths, not selected keys | F4, F9, F11, F13, F19 |
| One ≥100-record fixture on the ceiling tests | F3, F17, F18 |
| Exhaustive rather than single-field envelope tampering | F2, F20 |
| Golden bytes for the shipped envelope | F16 |

Five changes, roughly fifteen of the thirty-two findings.
