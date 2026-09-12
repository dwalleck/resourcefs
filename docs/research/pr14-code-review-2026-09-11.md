# PR #14 code review (feat/rfs-jrz7-source → main, head `882b4ef`)

Automated review run on 2026-09-11 at effort `max`. Thirteen agents ran — ten independent
finder angles (A–E functional, F–J quality), a two-batch verifier pass over the sixteen
strongest candidates, and a Phase 3 gap sweep. The PR head was checked out into a scratch
worktree (since removed; the main working tree was never touched) and the load-bearing
findings were confirmed by compiling and running targeted probes against it.

**Scope.** 53 changed files, 4,722 additions / 120 deletions.
Merge-base `b9aa182`, head `882b4ef`.
<https://github.com/dwalleck/resourcefs/pull/14> — *feat(github): verified immutable source
Facts (rfs-jrz7 S3)*.

## What is in this document

This review ran **uncapped** at the reviewer's request, so unlike the PR #12 and #13
records there is no "below the line" section: every surviving finding is reported in
severity order, F1 most severe.

| Bucket | Count | Where |
|---|---|---|
| Raw candidates before dedup | ~80 | 8 each from angles A–I, 2 from J, 7 from the sweep |
| **Reported findings** | **50** | [F1–F50](#index-by-theme) — severity order |
| Refuted with counter-evidence | 5 | [Refuted candidates](#refuted-candidates) |
| Verified clean and worth recording | 11 | [Verified clean](#verified-clean) |

Sixteen candidates went through a dedicated verifier; the sweep added seven more after
the finder angles closed.

## Merge gate

Two findings would block the merge on their own:

- **F1** — a deadline refusal is published as a *successful* partial Facts document,
  contradicting `DESIGN.md:230` and `docs/operating.md:230`, both added by this same PR.
  The new contract file contains zero deadline or `Retry-After` coverage.
- **F2** — a path eight components deep can never return content, and nine deep cannot be
  read at all, against a hard attempt ceiling no configuration can raise. The PR's own
  `deepBlobLimit` and `deepBeforeTerminalLimit` fixtures encode this as expected behaviour.

## The through-line

Four patterns account for most of what follows.

1. **One predicate decides too much.** `ordinary_blob_failure` collapses "retain a partial
   result" and "reject the whole read" into a single boolean driven by
   `invalidates_retention`, whose allowlist names three error kinds. Everything it does not
   name — deadline exhaustion (F1), response-byte overflow (F5), cache-generation
   invalidation (F18) — is retained and published as `acquisition_failed`, and the one thing
   it does name too eagerly (F6) discards verified identity that DESIGN says may be kept.

2. **Bounds that no operator can reach.** The 10-attempt ceiling is spent one attempt per
   path segment (F2), `MAX_TREE_ENTRIES` is a private constant duplicating an exported core
   one (F3, F34), and both are reported under limit kinds that carry no caller-facing field
   name — so the operator is told a limit was hit and handed no knob.

3. **The new family diverges from its siblings.** `identity::required()` is skipped where
   `commit.rs` documents why it is mandatory (F8); `unavailableFacts` is hard-coded empty and
   `Presence::Null` is collapsed into `Omitted` in the one family whose purpose is byte-exact
   fidelity (F12); the strict link validator is used where three siblings use the lenient one
   (F11); `endpoint` is used where every other site uses `expected_object_url` (F36).

4. **Tests that pin the shape but not the verdict.** Thirteen structurally different identity
   mutations assert only `details().is_some()` (F20); the last `ErrorCategory` assertion on any
   `github://` read was deleted by this PR (F21); the production budget gate measures a
   one-segment path against a one-entry tree (F23); the `maxDecodedBytes` intersection gate
   reads a family that never emits it (F22).

## Index by theme

| Theme | Findings |
|---|---|
| [The retention funnel](#the-retention-funnel) | F1, F5, F6, F8, F10, F11, F17, F18 |
| [Bounds no operator can raise](#bounds-no-operator-can-raise) | F2, F3, F34 |
| [Published unverified, lost silently](#published-unverified-lost-silently) | F4, F9, F12 |
| [Capability contract and provenance](#capability-contract-and-provenance) | F7, F15, F16, F25, F26, F33, F41 |
| [Fences that don't fence](#fences-that-dont-fence) | F14, F20, F21, F22, F23, F27 |
| [Cost: CPU, allocations, dependencies](#cost-cpu-allocations-dependencies) | F13, F28, F29, F30, F31, F32, F44 |
| [Module shape and CI gates](#module-shape-and-ci-gates) | F24, F48, F50 |
| [Reuse, simplification, altitude](#reuse-simplification-altitude) | F19, F35, F36, F37, F38, F39, F40, F42, F43, F45, F46, F47, F49 |

---

# The retention funnel

### F1. `crates/resourcefs-sources/src/github/facts/source.rs:361`

**`ordinary_blob_failure` classifies a deadline-exceeded blob fetch as retainable, so a
deadline refusal is published as a successful partial Facts document — contradicting
`DESIGN.md:230` and `docs/operating.md:230`, both added by this same PR.**

Blob GET returns 429/503 with `Retry-After: 120` while 29s of the 30s logical deadline
remain. `retry_wait_fits` (`http/mod.rs:140-143`, `wait < remaining`) is false, so
`fetch_bounded_attempts` returns `deadline_error(...)` = `SourceUnavailable`/`DeadlineExceeded`
with the deadline **not** expired. `invalidates_retention` (`collection.rs:248`) matches only
`Cancelled | PermissionDenied | UpstreamIdentityMismatch`, so `ordinary_blob_failure` is true
and the error becomes `content.state:"unavailable"`, `reason:"acquisition_failed"`,
`failure.reason:"deadline_exceeded"`. The final `ctx.read.check_acceptance()` (`source.rs:277`)
passes because `deadline.remaining()` is 29s, not zero. Zero deadline or `Retry-After` tests
exist in `github_source_contract.rs`.

### F5. `crates/resourcefs-sources/src/github/facts/source.rs:361`

**The same funnel retains `LimitExceeded`/`response_body_bytes`, publishing partial Facts JSON
where `DESIGN.md:378` states "Response/admission/representation overflow fails without
publishing partial Facts JSON".**

`maxDecodedBytes` (4 MiB) and `maxResponseBytes` (8 MiB) are independent caller dimensions and
base64 inflates by 4/3 — the PR's own `decodedBoundary.exact` records `decodedSizeBytes`
4194304 / `encodedLength` 5592408. Set `maxResponseBytes` to 4194304 (legal) and read that
path: the pre-check at `source.rs:287` passes, the blob fetch trips `response.truncated()`
(`fetch.rs:306-329`) returning `LimitExceeded` + `LimitDetail{response_body_bytes}`,
`ordinary_blob_failure` is true, and the read succeeds with `acquisition_failed` instead of the
typed refusal the caller could act on by raising `maxResponseBytes`. No fixture covers it;
`github_source_contract.rs` never mentions `maxResponseBytes`.

### F6. `crates/resourcefs-sources/src/github/facts/source.rs:362`

**The same funnel is over-broad in the other direction: a `PermissionDenied` on the blob fetch
rejects the whole read and discards already-verified terminal metadata, which `DESIGN.md:230`
says may be preserved for an ordinary blob acquisition failure.**

`classify_facts_status` (`fetch.rs:118-137`) maps 401, and 403 without
`x-ratelimit-remaining: 0` or `Retry-After`, to `PermissionDenied`/`UpstreamDenied`.
`invalidates_retention` matches `PermissionDenied`, so `source.rs:313` propagates and `?` at
`source.rs:202` discards verified `commitSha`/`treeSha`/`containingTreeSha`/`mode`/`objectType`/
`objectSha`. Reachable via a GitHub App installation token expiring (1h TTL) between the last
tree response and the blob fetch, an OAuth token revoked mid-read, a SAML SSO session lapsing
(403 + `X-GitHub-SSO`, no `Retry-After`), or a secondary-limit 403 with remaining > 0. The
identical situation with a 503 yields a retained partial result; no test exercises 401/403 on
a blob.

### F8. `crates/resourcefs-sources/src/github/facts/source.rs:162`

**`source.rs:162` and `:318` call `identity::require_object_link` without the
`identity::required()` presence pre-check that `commit.rs` performs and documents, so an absent
or null tree/blob `url` yields `not_found`/`upstream_identity_mismatch` where `DESIGN.md:210`
mandates malformed.**

`commit.rs:363-369` states the rule — "Presence and identity answer different questions: an
absent link is a malformed document… presence is required here to keep this read's verdicts
consistent" — and calls `identity::required(&commit.url)?` before `require_object_link`. The new
source paths omit it, and `require_object_link`'s `_ =>` arm catches `None`. A
`GET /git/trees/:sha` or `/git/blobs/:sha` response that omits the top-level `url` key (or sets
it null) is reported as `not_found`/`upstream_identity_mismatch`; the commit family under the
identical condition returns `source_unavailable`/`upstream_malformed`. One-line fix.

### F10. `crates/resourcefs-sources/src/github/facts/source.rs:186`

**An intermediate path component that resolves to a blob, symlink or submodule returns
`source_unavailable`/`upstream_malformed`, eleven lines after the sibling missing-name case
correctly returns `not_found`/`upstream_not_found_or_hidden`.**

`github://owner/repo/source/<sha>/run.sh/anything/facts` against the shipped fixture: `run.sh`
is a `100755` blob, `index + 1 != len`, disposition != `Tree`, and
`failure(ErrorReason::UpstreamMalformed)` (`facts.rs:450`) yields `ErrorCategory::SourceUnavailable`
— the category `DESIGN.md:56` reserves for outages and malformed data, and which
`profile/model.rs:747` buckets as `ProfileErrorKind::Io`. The tree passed full SHA
reconstruction immediately before, so nothing is malformed. A permanent user path error is
reported as a transient upstream problem and retried forever. `.rfs-jrz7/design.md:41` agrees
with `not_found`. No test asserts `NotFound` or `UpstreamNotFoundOrHidden` anywhere in
`github_source_contract.rs`.

### F11. `crates/resourcefs-sources/src/github/facts/source.rs:356`

**`validate_optional_object_link` fails the whole read on an unparseable or query/fragment-bearing
entry url — the opposite of its own doc comment and of three sibling validators — and this PR
widens that from a handful of links to every entry of every traversed tree.**

The doc says "opaque provider values stay lossless" (`identity.rs:215-216`), but
`matches_object_url` uses `Url::parse(value).is_ok_and(...)` (`identity.rs:187-194`), so "not a
URL at all" lands in the same arm as "names someone else's object".
`validate_optional_comment_link` (`identity.rs:135-137`) and `validate_observed_link`
(`identity.rs:429-433`) both `return Ok(())` for unparseable values, the latter with a comment
stating the intended rule. `validate_entry_links` (`source.rs:347-357`) applies the strict form
to up to 1,000 entries per tree × path depth, so one sibling entry with a relative url or a
`?ref=x` suffix — in a directory you are only passing through — kills the read with
`source_unavailable`/`upstream_identity_mismatch`. No current `api.github.com` shape triggers it,
so this is a contract/robustness divergence rather than a live outage.

### F17. `crates/resourcefs-sources/src/github/facts/source.rs:348`

**`validate_entry_links` expects a submodule entry (type `"commit"`) to carry
`git/commits/{sha}` in the *containing* repository — definitionally wrong, since a submodule
commit belongs to a different repo — and the fixture invents exactly that URL, so the contract
suite validates a shape GitHub never sends.**

Verified against `api.github.com/repos/git/git/git/trees/master`: GitHub omits both `url` and
`size` for `type: "commit"` entries, so `Presence::Omitted` falls into
`validate_optional_object_link`'s `Ok` arm and no read fails today. But
`tests/fixtures/github_immutable.json` line 13 hard-codes
`"url": "@API@repos/owner/repo/git/commits/cdf95a…"`, synthesized by `build_git_corpus.py` to
match the code's own wrong expectation — so the `submodule` contract case
(`github_source_contract.rs:434`) tests an impossible shape and leaves the real one (url absent)
untested. The day GitHub populates that field in any spelling, every path under any directory
containing a submodule becomes unreadable, because the check runs over every entry of every
traversed tree.

### F18. `crates/resourcefs-sources/src/github/facts/collection.rs:248`

**Cache-generation invalidation is inexpressible by `invalidates_retention` — `fetch.rs` raises
it as `SourceUnavailable`/`UpstreamUnavailable`, identical to an ordinary outage — so the
predicate that now claims to answer "reject the whole read?" contributes nothing for that case.**

A concurrent mutation bumps the `github-http` generation while the blob is revalidating with
304. `invalidates_retention` returns false, `ordinary_blob_failure` returns true, and the
invalidation is written into the document as an ordinary `acquisition_failed`. The whole read is
still refused today, but only indirectly, by the post-serialization `check_generation` at
`source.rs:276` — `DESIGN.md:230`'s "generation invalidation rejects the whole read" is enforced
by a separate mechanism, not by the predicate named for it. Any reordering that moves
`finish_facts` after the final checks, or a future caller of `invalidates_retention` without a
trailing `check_generation`, silently publishes an invalidated read.

---

# Bounds no operator can raise

### F2. `crates/resourcefs-sources/src/github/facts/source.rs:146`

**One shared attempt per path segment against an unraisable 10-attempt hard ceiling makes any
path 8 components deep permanently content-less and any path 9 deep unreadable.**

Attempts = 1 repo + 1 commit + N trees + 1 blob. `MAX_HTTP_READ_ATTEMPTS = 10`
(`http/read.rs:11`) and `validate()` refuses anything above the hard cap, so `maxAttempts` is
lower-only. `src/main/java/com/example/app/Service.java` (N=8) exhausts the budget at the blob
stage: the read **succeeds** with `content.state:"unavailable"`, `reason:"acquisition_failed"`
forever — the PR's own `deepBlobLimit` fixture and
`source_facts_keep_one_attempt_budget_through_deep_blob_stage` bake this in. N=9
(`deepBeforeTerminalLimit`) fails the whole read with `limit_exceeded`. `GithubSourcePath::new`
bounds encoded bytes, not segment count, so the address space is wider than the reader.
`docs/operating.md` frames the unraisable ceiling as a tuning knob.

### F3. `crates/resourcefs-sources/src/github/facts/object.rs:124`

**`MAX_TREE_ENTRIES = 1000` is an un-tunable hard constant applied to every tree on the path
including the root, reported under a limit kind that deliberately has no caller-facing field,
and it has no test.**

Any repository with a >1000-entry directory anywhere on the path (generated protos, `icons/`,
`locales/`, a monorepo `packages/`) makes every file at or below it permanently unreadable with
`LimitExceeded` / "GitHub tree exceeds the bounded entry limit". The kind is
`AcquisitionLimitKind::CollectionRecords`, which `describe_limit_rejection`
(`mcp/src/acquisition.rs:70-72`) explicitly refuses to map to a control ("no caller-facing field
name to report"), so the operator is told a limit was hit with no knob. Unlike the six
advertised dimensions it is not in `acquisition.limits`, and unlike every other tree-validation
branch it has no contract test.

### F34. `crates/resourcefs-sources/src/github/facts/object.rs:28`

**`MAX_TREE_ENTRIES` duplicates the exported core constant `MAX_COLLECTION_RECORDS` (same value)
and reports breaches under the very `AcquisitionLimitKind` that constant owns.**

`pub const MAX_COLLECTION_RECORDS: usize = 1_000` lives at `resourcefs-core/src/resource.rs:19`,
is re-exported from `lib.rs:72`, and is already imported by `facts/collection.rs:14`, which uses
it with `AcquisitionLimitKind::CollectionRecords` at lines 634-647 — exactly the kind
`object.rs:126` tags its own breach with. An operator tuning the record ceiling edits the core
constant, every collection family moves, and the tree walk keeps its private 1,000 — emitting a
`limit.kind == collection_records` error whose bound contradicts what every other family reports
for that same kind, which is the field `collection::retryable` and `describe_limit_rejection`
read back.

---

# Published unverified, lost silently

### F4. `crates/resourcefs-sources/src/github/facts/source.rs:287`

**The pre-fetch decoded-size refusal is driven by the tree entry's `size`, which Git tree objects
do not contain and the reconstructed tree SHA therefore does not verify; the unverified number is
then published as `data.sizeBytes` and `limit.observed` alongside hash-verified identity.**

A Git tree entry is `<mode> SP <name> NUL <20-byte oid>` and nothing else (gix-object
`write.rs`), and `validate_tree` builds `Entry { mode, filename, oid }` — no size. A
compromised/proxying API or a stale GHE mirror returns `"size": 99999999` for a 12-byte blob:
`read_blob` returns `DecodedSizeLimit` at line 291 without ever issuing `git/blobs/`, so
`validate_blob`'s exact-size cross-check (`object.rs:279-286`) — the only thing that could
contradict it — never runs. The caller gets a successful document asserting the file is too
large, with `limit.observed: 99999999` presented as an observation. Under-stated sizes are
caught; over-stated (content-censoring) ones are not.

### F9. `crates/resourcefs-sources/src/github/facts/object.rs:161`

**Tree entry names round-trip through a JSON `String`, so one non-UTF-8 filename anywhere in a
traversed directory makes that whole tree fail SHA reconstruction and blames the provider for a
representational limit on our side.**

Git permits any bytes except NUL and `/` in an entry name.
`native!(NativeTreeEntry { path: String, … })` forces UTF-8, and
`BString::from(name.as_bytes())` (`object.rs:161`) hashes whatever survived. Either GitHub
scrubs the bytes to U+FFFD — the reconstructed hash differs, `object.rs:188-191` returns
`integrity()` = `source_unavailable`/`upstream_identity_mismatch` — or serde rejects the body and
`source.rs:159` returns `upstream_malformed`. A latin-1 `café.txt` in the **root** tree makes
every path in the repository unreadable, with an error that reads as "GitHub is lying" and never
resolves on retry. `.rfs-jrz7/plan.md:105` lists this shape; the only shipped mutation
(`UnrepresentableTreeName`) trips the `/` check instead.

### F12. `crates/resourcefs-sources/src/github/facts/source.rs:268`

**`github.source` always publishes `"unavailableFacts": []` and collapses `Presence::Null` with
`Presence::Omitted` for `sizeBytes`, dropping the absent/null/present distinction every sibling
family preserves.**

`Facts.unavailable_facts` (`facts.rs:354-364`) has no `skip_serializing_if` and `source.rs:268`
is a literal `Vec::new()`, so every document asserts nothing was unavailable (visible in the PR's
own `.rfs-jrz7/mutation-c4-tree.txt`). Meanwhile `size_bytes: entry.size.value().copied()`
(`source.rs:196`) maps both `Null` and `Omitted` to `None`, and `skip_serializing_if` drops the
key either way. Siblings keep such fields as `Presence<T>` under
`skip_serializing_if = "Presence::omitted"` (`commit.rs:82-111`, eight fields) plus a `missing!`
entry recording "null" vs "omitted". `DESIGN.md:206`: "absent, null and present metadata remain
distinct." A consumer cannot tell "GitHub reported null", "GitHub omitted it", or "this schema
has no sizeBytes" — in the one family whose purpose is byte-exact fidelity.

---

# Capability contract and provenance

### F7. `crates/resourcefs-sources/src/github/mod.rs:823`

**`maxDecodedBytes` is advertised on every acquisition-bearing read but is
accepted-and-silently-ignored by PR/comment/review/inline/commit Facts, and is not even echoed
back in the emitted envelope.**

`is_facts` puts `pr://…/facts` in the accepted set, so `reject_acquisition`
(`core/src/source.rs:15-28`, an all-or-nothing per-Resource check) is skipped; the value is
intersected into `ctx.limits` at `facts.rs:631` and then read by nothing outside
`facts/source.rs`; `acquisition_at` hard-codes `max_decoded_bytes: None` with
`skip_serializing_if`, and only `source.rs:218` patches it back.
`rfs_read {"path":"pr://owner/repo/7/facts","acquisition":{"maxDecodedBytes":1024}}` succeeds,
bounds nothing, and returns five limit keys. `DESIGN.md:376` says controls on an unsupporting
Resource must fail with `unsupported_projection`/`acquisition_controls_unsupported`; the PR body
claims "No increment advertises a control it cannot honor", true only at increment granularity.
`stdio_mcp_contract.rs:4012` tests only invalid values against `pr://`, never a valid one.
Note `.rfs-jrz7/review-decisions.md:21` records this as knowingly deferred.

### F15. `crates/resourcefs-sources/src/compiled.rs:177`

**`github://` mutation and search still answer `UnsupportedProjection` under a justification
comment the PR invalidated, while the two sibling read-only mounted families in the same match
answer `UnsupportedMutation`.**

`compiled.rs:177-180` says the refusal "names the same fact the read and search arms name — this
build serves no `github://` route", but the read arm (`compiled.rs:137-143`) now dispatches every
`ResourceAddress::Github(_)` to the mounted adapter. `https://` (`compiled.rs:170`) and `jira://`
(`compiled.rs:173`) return `UnsupportedMutation` with a "Resources are read-only" message;
`github://` returns `UnsupportedProjection`. `DESIGN.md:200` and `:216` both now say
Commit/Source Facts are "read-only regardless of mutation grants", i.e. exactly the https/jira
shape, and `DESIGN.md:52` lists unsupported mutation as a category. A client that branches on
`unsupported_mutation` to mean "read-only, stop retrying" instead treats `github://` as an
unsupported reference. `github_adapter_contract.rs:777` pins the current behaviour.

### F16. `crates/resourcefs-sources/src/compiled.rs:132`

**The comment surviving the deleted Source arm now states the opposite of the code: it claims a
spelling is refused before the mount is consulted and that one address cannot answer differently
depending on configuration.**

The arm it annotates refuses nothing — `self.github_source()?` **is** the mount lookup
(`compiled.rs:269-271`, returning `SourceUnavailable` / "no GitHub source is configured").
`GithubAddress` has exactly two variants, so no unserved github spelling remains for the sentence
to describe. The PR's own rewritten tests prove the inverse: `compiled_sources_contract.rs` now
asserts `SourceUnavailable` for the unmounted source spelling while
`github_adapter_contract.rs:797` asserts `DeploymentIdentityUnavailable` for the mounted one —
two answers for one address, decided by configuration. The next maintainer either preserves a
rule the code no longer follows or "fixes" the code back.

### F25. `docs/operating.md:186`

**The operator-facing acquisition paragraph still lists five dimensions and omits immutable
source Facts from the Resources that accept controls, contradicting the new section 31 lines
below it in the same file.**

Line 186 reads "Omitted dimensions use the hard defaults before intersection: 10 attempts,
30000 ms, 8388608 response bytes, 16777216 accepted body bytes, and 16777216 representation
bytes" (`maxDecodedBytes`/4194304 missing) and ends "…and immutable commit Facts support these
controls". An operator sizes `maxResponseBytes`/`maxAcceptedBodyBytes` for a 4 MiB blob, never
sets `maxDecodedBytes`, and every source read silently uses the undocumented 4 MiB default —
returning `content.state` "unavailable" against a bound the one doc page they read does not
mention.

### F26. `crates/resourcefs-sources/src/github/mod.rs:891`

**The `github://` catalog entry crams two grammars into one field separated by `" | "` while the
example still shows only the commit spelling, and the new spelling's hardest rule —
per-component percent-encoding — gets no worked example.**

Sibling entries (`issue://`, `pr://`) express alternatives with in-line `[a|b|c]` brackets, and
the catalog's own legend uses `" | "` to mean alternative selector, so the rendered
scheme/grammar/example line is ambiguous. `SourceCatalogEntry::new` (`catalog.rs:29`) only
round-trips `example` through `PathReference::parse`; `grammar` is unvalidated free text. An
agent discovering sources via `rfs://sources` sees an `<encoded path>` operand with no example,
builds `…/source/<sha>/docs/my file.txt/facts` or `…/docs%2Fguide.txt/facts`, gets
`invalid_reference`, and the catalog offers nothing to correct it — while the commit spelling
beside it has a copyable example. The only assertion is `catalog_text.contains("/source/")`.

### F33. `crates/resourcefs-sources/src/github/facts.rs:409`

**`acquisition_at` — documented as "One acquisition observation shared by measurement and final
emission" — hard-codes `max_decoded_bytes: None`, producing a value that is wrong for the only
family that has a decoded bound, repaired by one caller after the fact.**

Of nine callers of the shared `acquisition(ctx)` helper, exactly one binds it `mut` and patches
the field (`source.rs:218`). The next family that decodes content (raw blob projection,
attachment Facts, artifact content) copies the `commit.rs` shape, forgets the one-line mutation,
and silently emits an acquisition envelope omitting a limit it is actually enforcing — a
provenance record that lies about what governed the read, with no compiler or test signal
because `None` is a valid serialization. Deeper fix: pass the governing dimension set into
`acquisition_at` so the struct is correct by construction, or always emit all six and take the
minor schema bump (`SchemaVersion` is already major/minor).

### F41. `crates/resourcefs-sources/src/github/facts.rs:610`

**`GithubAddress` exposes no `repository()` accessor, so the PR widens the outer match and adds
an inner exhaustive variant match purely to reach a field both variants already carry — and
`mod.rs:825` enumerates the same variant list redundantly.**

Three places now list `GithubAddress` variants: `facts.rs:610` (to extract `repository`),
`facts.rs:677` (the genuine dispatch), and `mod.rs:825`'s `is_facts` — the last of which is pure
redundancy, since `parse_github_address` mandatorily strips `/facts`, so every `GithubAddress` is
a Facts address by construction. A third variant compiles only after editing `facts.rs:610` and
`mod.rs:825`; forget `mod.rs:825` and the new spelling routes to the non-facts path where
`reject_acquisition` fires, turning a supported read into `acquisition_controls_unsupported` —
the exact confusion recorded as S1-R2-F9 in `.rfs-jrz7/review-decisions.md`. Add
`GithubAddress::repository()` in core beside `canonical_reference()`, and collapse `mod.rs:825`
to `Github(_)`.

---

# Fences that don't fence

### F14. `crates/resourcefs-sources/tests/github_facts_contract.rs:67`

**New in this PR: `pause_c8_serialization` blocks for up to 5 seconds inside
`GlobalAlloc::realloc` and performs re-entrant allocation there, in a process-wide
`#[global_allocator]`.**

`realloc` (line 126) calls `pause_c8_serialization`, which does `trigger.send(true)` on a
`std::sync::mpsc::Sender` — allocating queue blocks, re-entering `FactsCountingAllocator::alloc`
— then `release.lock()` and `recv_timeout(5s)`, parking the thread mid-`realloc`. It is armed by
two allocation internals at once: serde_json allocating the parsed content `String` at exactly
`content.len()`, and `RawVec` growth routing through `Allocator::grow`/`realloc` rather than
alloc+copy+dealloc. If either changes, the assertion at line 5119 fails with nothing pointing at
the cause; if the controller thread stalls, a test thread sits blocked inside the allocator for
5s per attempt. A `serialize_facts` seam with a test-visible hook would give the same coverage
without instrumenting the global allocator.

### F20. `crates/resourcefs-sources/tests/github_source_contract.rs:531`

**`source_facts_verify_tree_and_blob_identity_before_retention` drives thirteen structurally
different identity mutations and asserts only `error.details().is_some()` — pinning neither
category nor reason, and despite its name never asserting that no partial Facts were published.**

`WrongTreeSha`, `WrongTreeLink`, `TruncatedTree`, `DuplicateTreeName`, `UnrepresentableTreeName`,
`UnselectedTreeName`, `ContradictoryModeType`, `WrongBlobSha`, `WrongBlobLink`,
`WrongBlobEncoding`, `WrongBlobContent`, `MalformedBlobBase64` and `WrongBlobSize` are
indistinguishable to this test. A tree-link contradiction degrading from a typed refusal to a
soft `acquisition_failed` retention, or a blob-integrity failure being reclassified as
`not_found`, keeps the suite green — while `DESIGN.md` tells callers to match on category/reason
rather than messages. It also asserts no request counts, so it cannot tell "refused before the
blob GET" from "refused after".

### F21. `crates/resourcefs-sources/tests/github_adapter_contract.rs:792`

**The PR dropped the last `ErrorCategory` assertion on any `github://` read; no test now pins the
category of the deployment-identity refusal for either github spelling.**

The rewrite removed `assert_eq!(read.category(), ErrorCategory::UnsupportedProjection)` and
`assert!(read.details().is_none())`, leaving only `assert_ne!(read.message(), search.message())`
and a reason check. A repo-wide grep for `DeploymentIdentityUnavailable` across `crates/*/tests/`
returns exactly two hits, neither asserting `.category()`. Change `facts.rs:622-630` to build
that refusal as `SourceUnavailable` while keeping the reason and the full suite stays green —
silently contradicting `DESIGN.md:196` and `docs/operating.md:169`, which document
`unsupported_projection` / `deployment_identity_unavailable`.

### F22. `crates/resourcefs-sources/tests/github_facts_contract.rs:926`

**The canonical per-dimension intersection gate was padded with a trailing `None` but its
assertion loop still lists five field names, and because it reads PR Facts (which never emit
`maxDecodedBytes`) it structurally cannot cover the new dimension.**

No adapter-level test anywhere exercises caller > operator for `maxDecodedBytes`:
`github_source_contract.rs:979` is operator-only and both MCP cases use caller <= operator. A
regression in which the per-call value is passed through instead of intersected — anything that
reorders or drops the 6th argument in `facts_resource`'s
`.intersect(acquisition.copied().unwrap_or_default())` — lets a caller raise the operator's
decoded-content ceiling to the 4 MiB hard cap with the whole suite green. Only
`ReadAcquisitionLimits::intersect`'s own core unit test covers the arithmetic.

### F23. `crates/resourcefs-sources/tests/github_facts_contract.rs:4934`

**The new `immutable_source_production_budget` gate measures a 1-segment path against a 1-entry
tree, so it certifies the cheapest possible shape and is structurally blind to the two costs this
design introduces.**

It asserts `listener.requests().len() == 4` for `…/source/{sha}/payload/facts` against a fixture
tree holding exactly one entry, so the N-1 extra serial round-trips never happen,
`validate_entry_links` runs once instead of 1,000 times, and `validate_tree`'s two 1,000-capacity
Vecs plus HashSet plus sort are no-ops. The fixture's `native_content` is
`format!("{base64}\n")` — one newline — where real GitHub wraps at 60 chars (~93,206 newlines).
At 160 MiB for a 4 MiB payload the heap limit is 38× loose. Add a case at the worst admitted
corner: 7 segments through 1,000-entry trees with wrapped blob content, asserting the request
count.

### F27. `scripts/module-ledger.json:119`

**The new forbidden-token fence for `facts/object.rs` omits six tokens its only sibling
pure-validation module fences, and the gix tokens are fenced only in `commit.rs` and `source.rs`,
leaving the rest of `resourcefs-sources` unguarded.**

`object.rs`'s doc comment claims it "knows nothing about HTTP, sessions or facts envelopes", but
its regex lacks `Serialize`, `Serializer`, `serde_json`, `finish_facts`, `CacheMetadata` and
`SessionCacheWrite` — all six of which `facts/identity.rs` (ledger line 122) forbids for the same
claim. Meanwhile `github/fetch.rs`, `github/mod.rs`, `facts/collection.rs`, `http/*.rs` and
`compiled.rs` can import `gix_object`/`gix_hash` with no gate: the new `architecture_contract`
test proves only that core and MCP are clean. A later increment deriving `Serialize` on
`VerifiedTree` and emitting it straight into the envelope, or moving blob hashing into
`fetch.rs`, passes every gate while the advertised containment is lost.

---

# Cost: CPU, allocations, dependencies

### F13. `crates/resourcefs-sources/src/github/facts/source.rs:281`

**The largest synchronous CPU work in the adapter — multi-megabyte base64 decode, SHA-1 over
4 MiB, up to ten tree reconstructions, and serialization of a ~5.6 MB document — runs directly on
the async task with no `spawn_blocking` and no yield point, against the repo's own established
convention.**

`github/mod.rs:942`, `artifact.rs:156`, `filesystem.rs:745`, `https.rs:189` and `local.rs:276`
all wrap comparable work in `tokio::task::spawn_blocking`; the new source path does not. The MCP
binary runs a multi-threaded `#[tokio::main]`, so a 4 MiB source read pins one worker for the
whole decode/hash/serialize window and stalls every other tool call scheduled on it (seconds in
the unoptimized Functional tests gate). It also defeats the wall-clock backstop at
`github/mod.rs:846-851`: `tokio::time::timeout` cannot pre-empt synchronous code, and its
1-second "representation assembly" margin was sized for small Facts JSON, not megabytes.

### F28. `crates/resourcefs-sources/tests/github_source_contract.rs:133`

**The boundary fixture router regenerates the 4 MiB payload from scratch on every blob request and
then copies the 5.6 MB base64 string two or three more times, inside a test that is not
`#[ignore]`d and therefore runs in the debug CI gate.**

`boundary_blob` runs `STANDARD.encode(vec![byte; size])` per `/git/blobs/` hit;
`default_fixture_response` then calls `replace_api` (line 74), which recurses into every string
including the 5.6 MB base64 content and runs `text.replace("@API@", …)` — where `@` is not in the
base64 alphabet and the marker can never occur — allocating another full copy; `body.to_string()`
serializes a third; `Mutation::MalformedOversizedTail` adds a fourth.
`source_facts_accept_exact_decoded_cap_and_reject_cap_plus_one` executes this three times in the
unoptimized Functional tests gate, with tests running in parallel. The release budget gate is
`#[ignore]`d and measures only the adapter side, so nothing points at the fixture router as the
slowest, most memory-hungry part of the suite.

### F29. `Cargo.toml:26`

**`gix-object` + `gix-hash` drag in 28 new transitive crates — a full datetime/timezone stack
(`jiff`, `jiff-tzdb`), an embedded-logging framework (`defmt` ×3, `heapless`, `hash32`) and a
progress-bar crate (`prodash`) — to compute two SHA-1 hashes.**

`comm` on origin/main vs PR-head `Cargo.lock`: 311 → 339 packages, 0 removed. The chains are
`gix-object → gix-actor → gix-date → jiff → {defmt, jiff-tzdb-platform, portable-atomic}`, and
`gix-object → gix-features → prodash`. None is reachable: the code calls exactly
`WriteTo::write_to`/`loose_header` for `Tree` and `BlobRef` and `ObjectId::from_hex`.
`default-features = false` + `features = ["sha1"]` does not prune them. The workspace already
depends on `sha2` (RustCrypto), so `sha1 = "0.10"` reuses `digest`/`cpufeatures`/`cfg-if` and adds
**one** package; the canonical encodings are ~25 lines. For a workspace whose own `Cargo.toml`
comments justify dropping reqwest's charset feature to keep the surface small, 28 packages for two
hashes is out of character. Licences verified clean against `deny.toml`; confirm cargo-deny
advisories were actually run against the new lockfile — `unmaintained = "all"` with `ignore = []`
makes any RUSTSEC advisory in that subtree a hard red.

### F30. `crates/resourcefs-sources/src/github/facts/source.rs:342`

**`validate_entry_links` performs ~6 heap allocations and 2 full URL parses per tree entry — up to
1,000 per tree, ~7,000 per deep path — for a field that is never retained, never serialized and
never dereferenced.**

Per entry: `format!("git/{segment}/{sha}")`, `source.endpoint`'s own `format!`, `api_base.join`
producing a `Url`, `Url::parse` of the entry's url, and two `origin()` calls each cloning the
host. ~6,000 allocations + 2,000 parses for a 1,000-entry tree; ~42,000 + 14,000 across a
7-segment path. `VerifiedEntry.url` is read at exactly one site in the crate (`source.rs:356`) —
it is not in `SourceObserved`, not in `SourceData`, and `read_blob` builds its own endpoint from
`terminal.object_sha`. Cheaper: validate only the entry `find()` selects (the tree hash already
authenticates the whole entry set), or hoist the loop-invariant prefix and compare suffixes — one
allocation for the whole read, zero URL parses.

### F31. `crates/resourcefs-sources/src/github/facts/object.rs:290`

**`validate_blob` allocates a full 4 MiB decoded buffer that is dead the instant it is hashed,
pushing peak live bytes to ~15 MiB for a 4 MiB payload.**

At the default cap: response body ~5.51 MiB (still owned by `read_blob`'s response), `compact`
5.42 MiB of capacity (`retain` edits in place and never shrinks, so ~93 KB of removed newlines
stay as slack for the lifetime of the `VerifiedBlob`), and `decoded` 4.00 MiB — all live
simultaneously at lines 290-297. `decoded` is used only for a length re-check that
`exact_decoded_size` already derived and for `hash_object`, then dropped. `hash_object` is already
a streaming sink (`io::sink` at line 237): write `loose_header(Kind::Blob, decoded_size)` and
`io::copy` from `base64::read::DecoderReader` with a stack buffer, removing the 4 MiB allocation
(~-27% peak). Separately `finish_facts` starts from `Vec::new()` and doubles to ~5.6 MB while
`bytes_base64` is live; `with_capacity` removes a 12 MiB transient.

### F32. `crates/resourcefs-sources/src/github/facts/object.rs:137`

**`validate_tree` materializes a full `VerifiedEntry` (up to five owned `String`s) for all 1,000
entries when the caller consumes exactly one, copies each name a second time into a `BString`,
then walks the entries three more times.**

The loop builds two parallel Vecs — `git_entries` (each `Entry` carrying
`BString::from(name.as_bytes())`, a copy of a `String` `verified` already owns) and `verified`
(name, mode, object_type, object_sha, plus `Presence<String>` url) — roughly 6 live allocations
per entry, ~6,000 per tree, ~42,000 across a 7-segment path. The caller then does `.find(…)` and
keeps **one**. Afterwards: a 1,000-entry HashSet pass, a `sort_unstable` over heap-pointer
`BString` comparisons, and the serialization into the hasher. The hash reconstruction genuinely
needs every (mode, name, oid), but not a second owned copy: build only `git_entries`, pass the
wanted segment name in, construct the single `VerifiedEntry` for the match, and use
`BString::from(name.into_bytes())` to move rather than copy.

### F44. `crates/resourcefs-sources/src/github/facts/object.rs:319`

**The final base64 quantum is decoded twice, `compact` retains ~93 KB of newline slack for the
lifetime of the `VerifiedBlob`, and the post-decode length re-check is unreachable.**

`exact_decoded_size` runs `STANDARD.decode_slice` on the last 4 bytes purely to validate trailing
bits, then `validate_blob` decodes the whole string including that quantum. `String::retain` never
reallocates, so the ~93,206 stripped newlines of a real 60-char-wrapped 4 MiB blob remain as
capacity slack in the `String` that is stored in `VerifiedBlob` and serialized. And
`if decoded.len() != decoded_size` (line 293) cannot fire, since `exact_decoded_size` derived that
length from the exact encoded shape. Each is individually trivial; all three disappear if the
streaming-decode rewrite in F31 is adopted.

---

# Module shape and CI gates

### F24. `crates/resourcefs-sources/src/http/read.rs:333`

**The one line this PR adds puts `http/read.rs` at exactly its module-shape growth tripwire
(`lines=350, maximum=350`) without raising the budget, so the next edit of any kind fails CI.**

Verified by running the gate on the PR head:
`C12 crates/resourcefs-sources/src/http/read.rs: lines=350 maximum=350`. `wc -l` is 350 at head,
349 on origin/main; the ceiling lives at `scripts/module_shape_base.py:98` and is byte-identical
on both refs — `module_shape_base.py` is not in the PR diff at all. The check is
`if lines > maximum: fail(…)` (`module_shape_base.py:455-457`). Any commit adding one physical
line, including a comment or blank line, fails with
"growth tripwire: lines=351 maximum=350; placement review required", with no in-file warning that
the budget was exhausted here.

### F48. `crates/resourcefs-mcp/tests/architecture_contract.rs:97`

**`git_object_dependencies_stay_in_source_adapter` is the fourth copy of the cargo-metadata
preamble in the file and hard-codes one dependency pair, where the file already factors its other
shared logic into helpers and the test above it states the general rule.**

The identical 8-line block (`MetadataCommand` → `BTreeMap` of `workspace_packages` →
`dependency_names`) appears at `:17-25`, `:260-268` and `:364-372`, so the new test makes four
full `cargo metadata` resolutions per run of this binary; the file already has `dependency_names`,
`files_containing_token` and `offending_paths`, so the missing sibling is a
`workspace_dependencies()` helper. More importantly the fence is per-dependency-name: the next
native-format codec added to `resourcefs-sources` is covered by nothing until someone appends two
more string literals. `enforces_dependency_direction` above it already expresses the general form
(confine provider formats to wire modules by path); extending that, driven from the manifest
rather than a literal, covers gix and everything after it.

### F50. `scripts/ci-gates.py:35`

**The PR adds two more literal rows to `EXCLUDED`, a list that grows by construction on every
increment because this repo requires an env-gated live smoke test for every network source.**

Every real member of `EXCLUDED` already matches a mechanical predicate — the name starts with
`live_` or contains `::live_` — except two explicitly-named child-server hosts. So every increment
that adds a live smoke must also edit a Python literal in a second file, and forgetting it fails
CI with "Unclassified ignored test" rather than anything about the change. `EXCLUDED` carries no
completeness obligation (only `BUDGETS` is checked for missing rows at `ci-gates.py:129`), so
replacing it with the `live_` convention plus the two convention-breaking entries removes the
per-increment edit. `BUDGETS` should stay a literal precisely because `missing = BUDGETS - seen`
gives it a job a convention cannot do.

---

# Reuse, simplification, altitude

### F19. `crates/resourcefs-sources/src/github/facts/commit.rs:391`

**`validate_commit` now returns `Vec<usize>` parent indices that are always exactly
`0..parent_values.len()`, `acquire` discards three of four returns, and `read` re-resolves each
index through an error branch that cannot fire — replacing a construction-time guarantee with a
convention.**

Line 390-399 pushes `index` unconditionally with no filtering; `acquire` does
`let (_, _, _, parent_indices)` (line 199); `read` re-derives `parent_values` and does
`.get(index).ok_or_else(|| failure(UpstreamMalformed))` (line 245) — unreachable dead code that
reads as a real guard. The old code built `CommitParentFacts` in the same loop that validated the
links, making mismatch impossible by construction; now `CommitParentFacts` is buildable anywhere
in the module from an unvalidated `&NativeParent`, and `AcquiredCommit` exports `parent_indices`
to `source.rs`, which ignores it. If `validate_commit` ever starts skipping a parent, indices
shift silently and `read` publishes the wrong parents' SHAs and links.

### F35. `crates/resourcefs-sources/src/github/facts/object.rs:124`

**Hand-rolls the `LimitDetail` + `ResourceError(LimitExceeded).with_details(…).with_limit(…)` chain
that `collection::local_limit_error` already encapsulates in a sibling module of the same crate.**

`fn local_limit_error(kind, bound, observed) -> ResourceError` sits at `facts/collection.rs:257`
with an identical body and two existing call sites (`:636`, `:670`); making it `pub(super)` is a
one-word change. `describe_limit_rejection` (`mcp/src/acquisition.rs:54`) and
`collection::retryable` both branch on `(category, reason, limit.kind())`, so two independent
constructors of that triple means a future change — adding `retry_guidance`, say — lands in
`collection.rs` and leaves the tree path emitting a differently-shaped error. Note `object.rs`
uses `LimitDetail::new(…)?` where `collection.rs` uses `.expect(…)`, converting a
compile-time-positive bound into a second untested error path.

### F36. `crates/resourcefs-sources/src/github/facts/source.rs:355`

**`validate_entry_links` builds the expected object URL with `GithubSource::endpoint` (the
fetch-URL constructor) instead of `identity::expected_object_url`, which every other
link-validation site in the facts layer uses — and the two differ in their error arms.**

`identity::expected_object_url` (`identity.rs:165`) is the documented expected-link constructor;
`commit.rs:388` builds the identical `git/trees/{sha}` expectation with it, and
`inline.rs`/`review.rs` follow suit. The joins are byte-identical but `endpoint` yields
`ErrorCategory::InvalidReference` with no `ResourceErrorDetails` while `expected_object_url`
yields `failure(UpstreamMalformed)`. A repository name that breaks the join reports a
caller-blamed `invalid_reference` with no machine-readable reason from the tree path and an
upstream-blamed `upstream_malformed` from the commit path, for the same input — and a future
change to how expected URLs are spelled (an Enterprise path-prefix fix) must be applied twice.

### F37. `crates/resourcefs-sources/src/github/facts/source.rs:58`

**The manual `Serialize` for `SourceContent` matches `self` three times for one object — a
field-count match, the body match, and an inner match inside the shared unsupported arm that picks
the reason string via a `_ =>` fallback.**

Adding a variant means editing three places, and the inner `_ => "unsupported_object_mode"` means
the third can be silently forgotten: a new variant routed into that arm serializes with a wrong
reason and no compiler complaint, publishing an incorrect fact rather than failing to build. The
declared field count is only a hint to serde_json, so a miscount is invisible in tests.
`serializer.serialize_map(None)` — the pattern this codebase already uses for a variable-shape
object (`impl Serialize for NativePerson`, `commit.rs:63-77`) — deletes the count match entirely;
splitting the shared arm makes every variant named exactly once and a new one a compile error.

### F38. `crates/resourcefs-sources/src/github/facts/object.rs:112`

**`validate_tree` destructures the same `Presence` twice with unreachable `else` arms, then makes a
second full pass over `verified` for the duplicate-name check, followed by an explicit
`drop(names)` that is a no-op.**

`identity::sha(&native.sha)?` (line 112) already proves `Presence::Present`, yet lines 116-119
re-destructure with an else arm whose `observed_sha` half can never fire; the same shape repeats
per entry (line 140 vs 141-154). std's HashSet drop glue is `#[may_dangle]`, so `drop(names)` at
line 181 is not a borrow-check requirement — verified by compiling the same shape on editions 2021
and 2024 — but it reads as one and future editors will be afraid to touch it. Fold the uniqueness
check into the main loop (`if !names.insert(name.as_str())`) and delete the extra pass, the drop,
and two unreachable pattern arms.

### F39. `crates/resourcefs-sources/src/github/facts/object.rs:324`

**`exact_decoded_size` ends with a six-line fallible checked-arithmetic chain in which no link can
fail, advertising four failure modes the function does not have.**

`checked_div(4)` has a nonzero constant divisor; `len` was proved a multiple of 4 at line 306;
`groups*3 < len` so the multiply cannot overflow (a slice cannot exceed `isize::MAX`);
`padding <= 2` was enforced at line 314 and `padding > 0` implies `len >= 4` hence `decoded >= 3`;
`len == 0` gives `padding == 0`. The `malformed()` returned here is unreachable and untestable, so
anyone chasing "which input makes this return malformed at the tail?" finds nothing, and the next
person adding a branch inherits a `Result` tail to keep threading.
`Ok(encoded.len() / 4 * 3 - padding)` with a one-line comment naming the two guards above says the
same thing.

### F40. `crates/resourcefs-sources/src/github/facts/source.rs:200`

**`source::read` carries `terminal` as `Option<Terminal>` set inside the descent loop and unwraps it
afterwards with an `ok_or_else` that can never fire, attributing a local invariant violation to the
upstream provider.**

`GithubSourcePath::new` refuses an empty segment list (`core/src/reference/github.rs:70-75`) and
`parse` refuses an empty string, so the loop always runs and its final iteration always assigns. A
reader of this 70-line loop must go to another crate to answer "can `terminal` be `None`?", and the
phantom `UpstreamMalformed` would blame GitHub if the invariant ever broke. Splitting the walk with
`path.segments().split_last()` — ancestors in a loop, last segment once after — makes `terminal` a
plain value and removes the Option, the `ok_or_else` and the `index + 1 != len` test. The 0-or-1
`BTreeMap` built via `blob.into_iter().map(…).collect()` at line 239 is the same habit.

### F42. `crates/resourcefs-sources/src/github/facts/source.rs:368`

**`decoded_limit` adds a fifth hand-written `LocalLimit { kind: <kind>.as_str(), bound, observed }`
literal; the shared wire type still has no constructor and nothing type-checks that `kind` came
from `AcquisitionLimitKind`.**

`collection::failure_facts` (`collection.rs:189-217`) already holds the canonical
`LimitDetail → LocalLimit` projection, and the same struct literal is re-typed inline at
`collection.rs:645`, `:680` and `:751`. Because `kind` is a raw `&'static str`, a call site can
pass a hand-typed `"decoded_content_bytes"` that diverges from
`AcquisitionLimitKind::DecodedContentBytes.as_str()` (`core/src/error.rs:143`) with no test
noticing. A `LocalLimit::new(kind: AcquisitionLimitKind, bound: u64, observed: Option<u64>)` on the
type serves all five sites.

### F43. `crates/resourcefs-sources/src/github/facts/source.rs:148`

**`check_generation` takes the session's global admission mutex once per path segment, three lines
before `fetch_controlled` takes the same mutex to read the same value.**

`super::check_generation` (`facts.rs:422-438`) awaits `source.session.cache_generation(…)`, locking
`inner.admission` to read one u64; `fetch_controlled` does exactly the same read under the same lock
as its first act (`fetch.rs:224-227`), and `commit::accept_generation` at `source.rs:158` is the
check that actually decides, using the value the fetch read. Across a 7-segment path that is 8
redundant lock round-trips plus extra await points. The pre-check's window is strictly wider than
the post-fetch check's, so it catches nothing `accept_generation` would miss; if the intent is to
fail fast and save one of the scarce 10 attempts, say so and read the generation once before the
loop.

### F45. `crates/resourcefs-sources/tests/github_source_contract.rs:741`

**Two ~60-line concurrency harnesses are line-for-line identical except for eight lines — the
subtlest code in the new test file, duplicated.**

`source_facts_generation_change_between_each_response_refuses_publication` (`:741-800`) and
`source_facts_cancellation_at_metadata_and_verified_blob_is_whole_read_refusal` (`:825-884`) differ
only in the `_session` binding, four expect strings, the interruption (`cache_remove_namespace` vs
`operation.cancel()`), and the expected (category, reason). The `Notify` + mpsc +
`Arc<Mutex<Receiver>>` + spawn + double `timeout(3s)` barrier now exists twice, so a flake fix, a
timeout bump, or a change to `source_for_with_router`'s signature must be applied in both — and a
fix applied to one leaves a test that still flakes for a reason someone already diagnosed. The
revalidation-counter prologue at `:561-575` and `:652-666` is a second, 14-line instance.

### F46. `crates/resourcefs-sources/tests/github_facts_contract.rs:4845`

**`immutable_source_budget_fixture` constructs `ImmutableSourceBudgetOracle` twice for one value —
first with a placeholder `expected_base64: String::new()`, then again via functional-update syntax
after the payload is encoded.**

For 30 lines the oracle is in a state that is a lie, and the only thing preventing a reader (or a
future `assert!` inserted between) from trusting it is that nobody touches it yet. Adding a field
means remembering the placeholder-then-overwrite dance, and functional update silently carries
forward whatever the first literal set, so a future placeholder never overwritten compiles clean.
Bind `decoded_size` and `byte_value` as locals first, build `payload` and `expected_base64`, then
construct once with every field real — about 14 lines shorter and one fewer struct literal.

### F47. `crates/resourcefs-sources/tests/github_source_contract.rs:60`

**Third byte-identical copy of `host_from_head` in the sources test tree, while the shared support
module that owns `FixtureRequest::head()` has no such helper.**

The function already exists character-for-character at `tests/github_facts_contract.rs:142` and
`tests/github_immutable_contract.rs:27`. Its home is `tests/support/tls.rs`, which this file already
imports and which defines `FixtureRequest` with `head()` at `:295-311` — a `pub fn host(&self)`
there would serve all three files and the 7 call sites in `github_facts_contract.rs`. The helper
`.expect("Host header")`s on a parse of the raw request head; if `TlsListener`'s head framing
changes, three files must be fixed, and the one that is missed panics inside a fixture-router
closure where it surfaces as a hung or aborted TLS accept rather than a named assertion.

### F49. `crates/resourcefs-mcp/tests/stdio_live_smoke.rs:122`

**`write_source_profile` inlines the GitHub source row verbatim instead of calling the
`github_source()` helper defined 50 lines above it in the same file.**

`fn github_source(repository, max_attempts)` at `:70` carries the doc comment "The GitHub source
row both live profiles mount", and `write_profile_with_attempts` (`:85`) and `write_commit_profile`
(`:108`) both call it; the new `write_source_profile` (`:122`) re-types the same six keys by hand.
When the profile schema changes a source row — the credential shape, a new required key — two of
the three live profiles update through the helper and the new source smoke keeps mounting the stale
shape. Since it is `#[ignore]`-gated behind `RFS_LIVE=1`, the drift only surfaces during a live run.

---

# Refuted candidates

Five candidates were dropped with counter-evidence.

1. **Entry names `.`, `..`, `.git`** — guarded by `GithubSourcePath::validate_decoded_segment`.
2. **Missing pre-flight guard before the blob fetch** — `check_acceptance` already runs inside
   `fetch_bounded_attempts`.
3. **The "pull request" error message leaking** — `sanitize` rewrites it.
4. **`not_found` + `upstream_identity_mismatch` as an illegal pairing** — `DESIGN.md:210` blesses
   it. Replaced by the real adjacent defect, the missing `identity::required()` presence pre-check
   (**F8**).
5. **Asymmetry in the MCP `maxDecodedBytes` wiring** — verified symmetric across deserialize,
   duplicate detection, schema, intersection and rejection mapping.

# Verified clean

Checked and found correct; recorded so the next review need not re-derive them.

- gix's `tree::Entry: Ord` really implements Git's `/`-suffix `name_order`, so `sort_unstable`
  reconstructs canonical trees.
- `hash_object`'s header-before-body ordering and its `gix_hash::io::Write` usage are correct.
- base64 0.22 `STANDARD` is `RequireCanonical` with trailing bits rejected, and
  `exact_decoded_size` cannot disagree with `STANDARD.decode`.
- `parse_mode` cannot truncate and handles gix's `040000`/`40000` internal hack.
- No `u64 → usize` narrowing anywhere in the new code.
- Nested `#[serde(flatten)]` with the hand-written `SourceContent` serializer is sound.
- All ~39 `ReadAcquisitionLimits::new` call sites got the 6th argument in the right slot.
- `scripts/module_shape.py` passes on this head (see F24 for the margin).
- `deny.toml` licences need no new entry for the gix subtree (see F29 for the advisory caveat).
- Live smokes are correctly `#[ignore]`d and env-gated.
- The workspace builds clean and the full core + sources suite is green; nothing above is a build
  break.
