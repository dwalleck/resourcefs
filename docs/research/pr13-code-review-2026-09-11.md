# PR #13 code review (feat/rfs-jrz7-commit → main, head `1cfa020`)

Automated review run on 2026-09-11. Seventeen agents ran: ten independent finder
angles (A–J), six verifier passes, and one gap sweep. Finder agents read a
detached worktree pinned at the reviewed head; verifiers re-derived each decisive
claim from source and the vendored crate sources, and the parent re-proved the
top finding against the live GitHub API. No agent edited any file.

**The PR is green.** The full repository gate (`python scripts/ci-gates.py`) passes
on this head — module placement, clippy, all-feature debug and release tests, the
ignored production budgets, and `cargo deny`. Nothing below is a build break.

**Scope.** 26 files, +1952/-142. Merge-base `09cef66`, head `1cfa020`.
<https://github.com/dwalleck/resourcefs/pull/13> — *feat: expose immutable GitHub
commit facts* (slice S2 of rfs-jrz7).

This head is the conflict-resolution merge of `origin/main` into the slice branch.
Six files conflicted; the resolution preserved both sides and is verified in the
[merge resolution](#merge-resolution) section.

## What is in this document

This document carries the complete inventory.

| Bucket | Count | Where |
|---|---|---|
| **Reported findings** | **13** | [F1–F13](#index-by-theme) — severity order, F1 most severe |
| Refuted candidates | 13 | [R1–R13](#refuted-candidates) |
| Below the line | 14 | [T1–T14](#below-the-line) — each with its raiser and evidence state |

Evidence states for findings: **Verified** (reproduced by execution, or by reading
source with the deciding lines quoted) and **Partially-confirmed** (the conclusion
survives but a stated mechanism does not). Every finding names the agent that
raised it and the verifier that re-checked it. Below-the-line items carry the same
attribution individually rather than in aggregate.

Two claims in an earlier draft of this document were withdrawn after
re-verification — see [R12](#refuted-candidates) and the corrections recorded
against F5 and F13. That is the reason each item below names its evidence state.

## The through-line

Four patterns account for nearly all of what follows.

1. **An expectation built from a name where the provider supplies a different
   spelling.** `validate_account` reconstructs each account's API and web links
   from the login, then demands exact path equality (F1). GitHub's own payload for
   a bot account does not use those paths. The same construction drops a `.`/`..`
   login entirely because the URL crate skips those segments (F12).

2. **Fences that cannot fail, and evidence that outruns its row.** A null-presence
   assertion an *omitted* key satisfies (F3); a "real binary over stdio" claim
   whose deterministic row spawns the test harness (F4); a budget row blind to a
   *loosened* ceiling while pinning the tightened one (F13); live rows whose
   non-skipped status is prose-asserted rather than receipt-observable (T13).

3. **The merge kept the weaker half of two mechanisms.** Main's owner-uniqueness
   rule iterated every same-named declaration in the counterpart file; the branch
   had already replaced that with a single-subject comparison that skips a
   counterpart declaring the name twice. The resolution kept the branch's shape,
   so the increment is **strictly weaker than main** on the permanent placement
   gate (F2).

4. **Claims that outlived the capability they describe.** The family-refusal
   sentence still says `github://` is not served, and this increment *pins* it in
   two fences (F7); the still-unserved `/source/` spelling answers three ways and
   no fence covers it (F9); the operating guide's control-route list was not
   extended although its `DESIGN.md` twin was (F11).

## Index by theme

### Account identity

#### F1. `crates/resourcefs-sources/src/github/facts/commit.rs:377-392` — app/bot accounts fail the commit read · **Verified** · P1

*Raised by Angle C and Angle G independently; re-proved by the parent against the live API.*

`validate_account` rebuilds each supplied account's links from the login:

```rust
let expected_api = append_path_segments(api, &["users", login])?;
let expected_web = append_path_segments(web, &[login])?;
identity::validate_optional_object_link(&account.url, &expected_api)?;
identity::validate_optional_object_link(&account.html_url, &expected_web)?;
```

`identity::matches_object_url` demands `actual.path().eq_ignore_ascii_case(expected.path())`,
and `Url::path()` returns the **percent-encoded** serialized path (url-2.5.8
`lib.rs` doctest: `Url::parse("…/việt nam").path()` → `"/countries/vi%E1%BB%87t%20nam"`).

GitHub's real payload for an app account does not use either expected path:

```console
$ gh api repos/actions/checkout/commits/e8d4307… \
    --jq '{login: .author.login, url: .author.url, html_url: .author.html_url}'
{"login":"dependabot[bot]",
 "url":"https://api.github.com/users/dependabot%5Bbot%5D",
 "html_url":"https://github.com/apps/dependabot"}
```

The constructed API path is `/users/dependabot[bot]` (`[`/`]` are outside the
path-segment encode set), and the constructed web path is `/dependabot[bot]`
against the observed `/apps/dependabot`. **Both** comparisons fail, so
`validate_account` returns `Err(UpstreamIdentityMismatch)` and the whole read
aborts with `source_unavailable` — for every commit authored *or* committed by a
GitHub App bot (dependabot, github-actions, renovate, pre-commit-ci, snyk, …),
even when SHA, tree, parents and every other link agree with the request.

Blast radius is bounded on the other side: **organization owners are fine** —
`repos/actions/checkout` returns `owner.url = https://api.github.com/users/actions`
and `html_url = https://github.com/actions`, both of which match. Only the
`[bot]`/`/apps/` shape diverges. This is the increment's only defect that changes
behaviour on ordinary upstream data.

The suite cannot see it: the ten immutable fixtures build account links as
`{api}/users/{login}` and `{web}/{login}` for bracket-free logins only, and the
credential-gated live row pins a human-authored commit.

*Recommended:* stop deriving account links from the login. Accept the account's
own route family (deployment origin plus empty credentials, plus the provider's
documented `/apps/<slug>` profile) or compare percent-decoded path segments, and
add a fixture with a `dependabot[bot]`-shaped account.

#### F12. `crates/resourcefs-sources/src/github/facts/commit.rs:389-396` — a `.`/`..` login silently vanishes from the expected URL · **Verified mechanism** · P3

*Raised by Angle C; its wider form refuted by the gap sweep; narrowed by the parent.*

`append_path_segments` calls `path_segments_mut().pop_if_empty().extend(segments)`.
In url-2.5.8 (`src/path_segments.rs:247-249`) `extend` **skips** any segment equal
to `.` or `..`:

```rust
if matches!(segment, "." | "..") {
    continue;
}
```

So a hostile or broken upstream sending `"login": ".."` makes the expected API
object `<api>/users` — the user *collection* — and a supplied `account.url` equal
to that collection URL, with `html_url` absent (a case
`validate_optional_object_link` deliberately passes), would satisfy the fence
while `data.authorAccount.login` is published as `".."`. The repository identity
cannot reach this: `validate_github_identity_part` already rejects dot segments
(`crates/resourcefs-core/src/reference.rs:446-456`). The *account* login comes
straight from the upstream payload, so it is reachable from upstream data alone.

*Refuted, wider form:* hostile text cannot name a cross-authority object — logins
are percent-encoded through `extend`, so `"//foreign.invalid/"` cannot define a
matching expectation. The dot-segment skip is the residual: a hardening item, not
a reachable exploit, and it needs the upstream to be hostile *and* the account's
`html_url` to be absent or null.

*Recommended:* refuse a `.`/`..` login with `upstream_malformed` before building
the expected URLs.

### The placement gate

#### F2. `scripts/module_shape.py:299-322, 366-374` — the owner-uniqueness rule is strictly weaker than main · **Verified** · P2 (latent)

*Raised by Angle I; independently identified by the parent during the conflict resolution; verified by a dedicated re-derivation.*

`declaration_subject` returns `None` as soon as a path declares the symbol more
than once:

```python
spans = [span for (name, _), span in production_nodes(path).items()
         if name == symbol]
if len(spans) == 1:
    return fingerprint(codes[path][slice(*spans[0])], inherited)
if spans:
    return None
```

The counterpart comparison is `declaration_subject(other, symbol) == subject`, so
a counterpart that already declares the name twice yields `None == <tuple>` —
never true — and a verbatim copy of an owned declaration placed in that file is
**silently accepted**. The `continue` that skips can only fire for the *owner*
path, never for a counterpart.

The review base does not have this hole. `git show 09cef66:scripts/module_shape.py`
iterates every same-named span of the counterpart:

```python
for (name, _), span in production_nodes(other).items():
    if name == symbol and fingerprint(codes[other][slice(*span)], inherited) == body:
        fail(other, f'symbol {symbol} belongs in {path}', claim)
```

Origin: the weakening entered on the **branch** (commit `4b3af28`), not in the
merge resolution — the branch already compared `declaration_subject(sources[other], symbol)`
at `64e1f56`. The resolution faithfully kept that shape. Relative to `09cef66`,
therefore, this patch accepts trees main rejects.

Reachability, measured independently by the parent and by the verifier:

- **0** files under `crates/*/src` declare any one of the 122 ledger-owned symbol
  names twice — the hole is latent, not a live miss.
- **48** files declare *some* name twice or more (parent's count over the same
  census; the verifier counted 42 on a slightly different basis), so the
  precondition "the counterpart already declares this name once" is ordinary
  rather than exotic.
- 17 (owned-symbol, counterpart) pairs declare the symbol exactly once today; one
  copy into any of them makes that pair silently exempt.

Other checks do not close it: the directory-scope name rule only covers the
owner's own directory, and `check_parent_nodes` only covers the protected parents
(`sources/src/github/facts.rs`, `sources/src/github/mod.rs`). A copy into
`sources/src/github/fetch.rs`, `compiled.rs`, or any core file is defended only by
the weakened comparison.

*Recommended:* restore the counterpart all-span iteration and fall back to the
`native!` invocation only when the counterpart has no span for the symbol.

### Fences and evidence

#### F3. `crates/resourcefs-sources/tests/github_immutable_contract.rs:390` — the null half of the account-presence fence cannot fail · **Verified** · P2

*Raised by Angle E; verified.*

`present["data"]["committerAccount"].is_null()` is satisfied just as well by an
*omitted* key: `serde_json`'s string indexing returns `Value::Null` for a missing
key, and `commit.rs:109` skips the key entirely when the value is
`Presence::Omitted`. A commit-local regression collapsing a present-null account
into an omitted key would leave all ten immutable contracts, the MCP row, and the
sources live row green — the live row uses the same `is_null()` conflation.

The distinction is real and required (`DESIGN.md:206`: "absent, null and present
metadata remain distinct"), and the PR family fences it correctly with a
key-presence idiom (`github_facts_contract.rs:410` asserts `.get(key).is_some()`
and `:424` asserts the `unavailableFacts` reason). No commit-route test asserts
`unavailableFacts` at all.

`mutation-c9-commit.txt` proves only the opposite direction (an absent account
becoming null) and must not be cited for this half.

*Recommended:* assert key presence before `is_null()`, and record the presence
reason in `unavailableFacts` as the sibling family does.

#### F4. `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs:152-181` — the "real binary" stdio row runs the test harness · **Verified** · P2

*Raised by Angle E; re-proved by the parent.*

`start_profile_with_https_root_and_env` spawns `std::env::current_exe()` — the
compiled `stdio_mcp_contract` test binary — with `--ignored --exact
profile_https_test_server`, which calls `serve_profile_with_https_root` directly.
It does not spawn `binary()` = `env!("CARGO_BIN_EXE_resourcefs")`, which every
other start path in the file uses.

`.rfs-jrz7/evidence.md:64` records "Real `resourcefs` binary over MCP stdio:
repository then commit under two attempts, owned schema and native values,
selector/mutable-operand refusal, catalog exposure, and byte-exact artifact
recovery", and `qualification-s2.txt:36` repeats "Real-binary stdio commit read,
catalog and byte-exact recovery without reacquisition: PASS". Both describe this
deterministic row's content. Only the credential-gated
`live_stdio_github_commit_facts_match_native_observation` starts the shipped
binary — and it asserts none of the selector/catalog/refusal specifics.

The harness route is deliberate and defensible: the fixture CA is unselectable
through the shipped CLI, fenced by
`github_profile_trust_reaches_real_stdio_reads_only_when_explicit`. The defect is
the evidence text, not the row.

*Recommended:* correct the two evidence sentences. The bypassed surface is CLI
argv/profile plumbing and process identity, not the facts pipeline.

#### F13. `crates/resourcefs-sources/tests/github_facts_contract.rs:1665-1698` — the commit budget row is blind to a loosened ceiling · **Partially-confirmed** · P3

*Raised by Angle B; corrected by the verifier — the conclusion survives, the stated mechanism does not.*

The row cannot notice a **raised or deleted** acquisition ceiling. The mechanism
first reported here was partly wrong and is withdrawn:

- **Withdrawn:** "two responses of exactly 8 MiB each saturate the 16 MiB
  cumulative cap." The second response is the 317-byte repository fixture, so
  total admitted is 8 388 925 B — 49.9 % of the cap — and the cumulative ceiling
  never binds on this path. That saturation scenario belongs to
  `http::read_acquisition_tests::verified_http_bodies_share_cumulative_admission_without_reset`.
- **Withdrawn:** "no assertion is a function of an enforced bound." The fixture
  body is exactly the effective response ceiling, so the row *does* pin bound
  **inclusivity** at the response boundary: lowering either response bound below
  8 MiB, or flipping `>` to `>=`, turns it red through the truncation refusal.

What survives: `acceptedBodyBytes` is identical with or without the cumulative
check (the counter is accumulated regardless of enforcement), and
`output_bytes <= OUTPUT_LIMIT` compares an 8.39 MB document against a 16 MiB
literal it cannot reach — 50.0 % of it — so the assertion can never fail for this
fixture. `requests().len() == 2` bounds observed requests, not attempted ones;
nothing asserts `attemptedRequests`.

Severity is therefore test-strength, not defect: every removable enforcement point
on this path is fenced by another row (the operator-limit rows,
`facts_body_and_cached_readmission_exact_plus_one`,
`facts_representation_includes_escaping_and_envelope`,
`http_bounds_contract::fetch_ceiling_boundary`, and the retry row).

*Recommended:* accept as-is (the roster covers the removal), or add one `cap + 1`
refusal leg to this row so it reads `acquisition.limits` rather than a literal.

### Shared-helper and dispatch quality

#### F6. `crates/resourcefs-sources/src/github/facts/commit.rs:79-86` — `ParentLinks` re-implements the shared `facts::Links` · **Verified** · P2

*Raised by Angle F and Angle G; verified with repo precedent.*

`ParentLinks` is field-for-field the existing `facts.rs:211-216` `Links`: the same
camelCase `apiUrl`/`htmlUrl` pair with the same
`skip_serializing_if = "Presence::omitted"`. `Links` is precisely the type the
existing `parent::ParentFacts` already uses for a parent object's links, so this
increment serializes identical JSON through a second, renamed definition.

This is a **re-introduction of an already-ordered fix**: `.rfs-bfwa` Q9 ordered
exactly this duplicate removed in favour of the shared `Links`, and repair
`df5da90` did so for `comment.rs`. `CommitLinks` legitimately stays distinct — it
adds `commentsUrl`.

The placement gate cannot see it: its duplicate rule compares declarations of the
*same ledger symbol name*, and `ParentLinks` is not in `commit.rs`'s ledger row.

*Recommended:* delete `ParentLinks` and use `super::Links`.

#### F10. `crates/resourcefs-sources/src/github/facts.rs:630-641` — the second address match re-derives the first match's guarantee · **Verified** · P3

*Raised by Angle F and Angle G; verified.*

`facts_resource` destructures `reference.address()` twice. The second match's
`let PullRequestResource::Facts(fact) = resource else { … }` cannot fire (the first
match already refuses a non-`Facts` pull-request resource), and the
projection/cursor condition is recomputed after being enforced. The route decision
for the new `GithubAddress::Commit` family therefore had to be written into three
places in `resourcefs-sources` (both matches plus `GithubSource::read`'s `is_facts`
predicate at `github/mod.rs:823-831`) and a fourth at `compiled.rs:135-140`. A
drift between them changes which error a caller observes rather than being refused
at one boundary.

*Recommended:* bind the family decision where the address is first destructured
and let `is_facts` share that classification.

#### F5. `crates/resourcefs-sources/src/github/facts/identity.rs:199-213` — a contradicted or absent required link answers `not_found` · **Verified** · P2

*Raised by Angle A; verified; the message half of the original claim **withdrawn** — see R12.*

`require_object_link` answers `ErrorCategory::NotFound` with reason
`UpstreamIdentityMismatch` for a required link that is absent (or null) and for one
that names a foreign object. It carries no `HttpStatus` and no `access_ambiguity`,
while a genuine 404 from the transport gets `UpstreamNotFoundOrHidden` **plus**
`AccessAmbiguity::MissingOrAccessHidden` (`fetch.rs:138-152`). So a document that
was successfully fetched is reported to the caller as "does not exist", with a
`details.reason` that says the opposite.

Within one read the verdict is also inconsistent: every *other* absent required
commit field answers `upstream_malformed` — an absent repository `url` is
`upstream_malformed` because `require_repository` pre-checks presence, while an
absent commit `url` at `:336` is `not_found`.

**Withdrawn:** the original claim that this path leaks the pull-request-specific
message to the caller. `read_facts` applies `sanitize`, which rewrites every
`NotFound` message to the constant "GitHub facts read failed" while preserving
category and details. The prose defect exists in `identity.rs:207` but is not
caller-visible on this route. See R12.

*Recommended:* split presence from identity at `:336` (so absence stays
`upstream_malformed`), and decide the category question explicitly: either answer
an identity mismatch for a contradicted link, or record in `DESIGN.md` that the
comment family's `not_found` verdict extends to required links.

### Claims that outlived the capability

#### F7. `crates/resourcefs-sources/src/compiled.rs:285-297` — the family-refusal sentence is now false, and this increment pins it · **Verified** · P3

*Raised by Angle D and Angle J; verified.*

`github_family_unreadable()` still emits "github:// Resources are not served by
the compiled sources" and its doc comment still says commit and source Facts "are
registered by the increments that acquire them; until then the same refusal
answers on every build". After this increment a configured build **does** serve
`github://…/commits/<sha>/facts`; the helper is now reached only from the mutation
arm (`compiled.rs:179`) and the search arm (`:347`). On a mounted build the
sentence is reachable through the ordinary MCP `rfs_search` path and is simply
untrue. The category stays right; the sentence and the comment do not.
`github_adapter_contract.rs:789-793` asserts `mutation.message() == search.message()`
for a commit reference, i.e. the increment pins the stale sentence.

Related scope note: the design inventory assigns `github/mutation.rs` the
permanent immutable-target refusal, and `plan.md`'s ledger gives it a line budget,
but neither S2's nor S3's file list names the file and it contains no refusal text
at this head. The permanent refusal has no owning slice.

*Recommended:* reword the sentence and the doc comment to the claim the family can
still make ("this build accepts no writes to `github://` Resources"), and assign
the permanent refusal to a slice.

#### F9. `crates/resourcefs-sources/src/compiled.rs:135-137` — the still-unserved `…/source/…` spelling answers three ways · **Verified** · P3

*Raised by the gap sweep.*

The read arm routes the **whole** `ResourceAddress::Github(_)` variant to the
configured source, while servability is sub-address: `is_facts` matches only
`GithubAddress::Commit`. So one address family, one spelling
(`github://owner/repo/source/<sha>/<path>/facts`), answers three ways:

| Build / input | Answer |
|---|---|
| unmounted | `SourceUnavailable` — "no GitHub source is configured" |
| mounted | `UnsupportedProjection` via the projection refusal |
| mounted, caller `acquisition` controls | `UnsupportedProjection` + reason `acquisition_controls_unsupported` |

Two of the three are mount- or control-dependent — exactly the class S1-R2-F8/F9
removed for the family. The gap sweep confirmed no fence covers the source
spelling in either mount state or with controls; both routing fences exercise only
the commit spelling.

No data or security consequence: the route is unadvertised (the catalog lists only
the `/commits/` shape, asserted by `stdio_mcp_contract.rs:4299-4306`) and S3
serves the spelling. This is interface honesty and fence coverage, not a reachable
defect.

*Recommended:* either serve or refuse the source spelling at the same boundary as
the commit spelling, and fence both spellings in both mount states.

#### F8. `docs/operating.md:203-209` — per-call acquisition controls are documented for the commit route but untested there · **Verified** · P2

*Raised by Angle E; verified.*

`docs/operating.md` documents the commit route with `"acquisition": {"maxAttempts": 2}`
and `DESIGN.md:343` extends the contract to all GitHub Facts. Production honours
it: `is_facts` includes `GithubAddress::Commit` so controls are not rejected, and
`facts.rs:611-615` intersects them with operator policy before acquisition.

No test at any level exercises acceptance on the commit route: the ten immutable
contracts and both live commit rows pass `None`; the MCP commit row sends only
display limits and never asserts `facts["acquisition"]["limits"]` (the sibling PR
row does). The only controls-bearing commit read is on an *unmounted* registry,
where the control never reaches an adapter.

*Recommended:* add a commit case mirroring the PR row's effective-`maxAttempts`
assertion.

#### F11. `docs/operating.md`, `DESIGN.md`, `.rfs-jrz7/*` — recorded claims the tree does not support · **Verified / Partially-confirmed** · P3–P4

*Raised by Angle J and Angle I; verified per item.*

1. **`docs/operating.md:186`** — *partially-confirmed.* Still enumerates the
   controls-bearing routes as "Singular PR Facts, conversation-comment collection
   Facts and singular conversation-comment Facts", omitting the commit Facts route
   this increment added and documents 20 lines later. The parallel `DESIGN.md`
   wording *was* generalized in the same increment. Two corrections to the
   original claim: the five control dimensions and defaults are identical in both
   documents (only the route list is stale), and the list already omitted the
   review-Facts routes before this increment, so only the commit-Facts omission is
   patch-introduced.
2. **`.rfs-jrz7/qualification-s2.txt:35`** — *confirmed.* Records
   "github_adapter_contract: 24 passed". The file has 28 test attributes, 3
   `#[ignore]`d, i.e. **25** runnable tests at every revision of this increment. No
   raw output for that focused run is retained, so the count is unverifiable and
   inconsistent with the file. The error understates passes and hides no gap.
3. **`DESIGN.md:127`** — *confirmed.* The increment adds the `github.commit` row
   and the catalog entry, but "Included address families" still omits `github://`;
   a repo-wide grep finds the family mentioned only in the new route sentence.
   Documentation-only: no gate or test reads the list.
4. **`.rfs-jrz7/plan.md:225`** — *partially-confirmed.* Reports the facade as 1175
   physical lines (it is 1174 by `wc -l` and by the gate's own count), and "1938
   changed lines across 24 files". 24 files is exactly the first S2 commit's file
   count (the increment is 26), and 1938 is the insertion count against the S2 fork
   point `6e1c55c` (against the review base it is 1952 insertions, 142 deletions).
   No limit is crossed either way: the facade stays under its 1498 protected
   limit, and the increment stays inside its 2200-line estimate.
5. **`.rfs-jrz7/evidence.md:67-74`** — *partially-confirmed*; recorded below the
   line as **T13**, because its fix is to the evidence rather than to a claim
   inside this list.

## Refuted candidates

Kept because a wrong refutation silently drops a real bug; these are claims that
were raised and then killed with counter-evidence.

| # | Claim | Refutation |
|---|---|---|
| R1 | `commit.rs`'s manual `api_base.join(format!("repos/{owner}/{repo}"))` duplicates `GithubSource::endpoint` | `endpoint(repo, "")` yields a trailing-slash path that GitHub's `repository.url` does not carry, so the exact-link comparison needs the manual join. *(Raised by the parent during the conflict resolution; refuted by Angle F.)* |
| R2 | The `Url::join` calls can drop the API or web path prefix | `api_base` is normalized to end with `/` at construction (`github/mod.rs:165-168`), and `validate_web_origin` requires `path() == "/"` (`configuration/github.rs:108-117`). |
| R3 | Organization-owned repositories fail like bot accounts | `owner.url` is `/users/<login>` and `owner.html_url` is `/<login>` for orgs; both match the expectation. Only the `[bot]`/`/apps/` shape diverges. |
| R4 | Credential or token text reaches an emitted fact, an error, or a recovery reference | The serialized field set carries no header or credential echo; `OriginCredential`'s secret has neither `Debug` nor `Display`; composition failures are generic sentences; the profile reports only that a credential could not be resolved. |
| R5 | A `github://` commit read is an authorization bypass | `authorize_repository` runs before any egress for every facts read; search and mutation refuse without network access; both request URLs are built only from allowlist-validated identity bytes (`.`/`..` and non-`[A-Za-z0-9._-]` rejected, lowercase-folded) plus a 40-lowercase-hex SHA. |
| R6 | SSRF or credential-bearing redirect on the two new requests | Both fetches pass `Some(&mut budget)`, which forces `RedirectBehavior::Refuse`; the client is built with `Policy::none()`, so a 3xx is an upstream failure and the credential is never re-sent. |
| R7 | Cancellation between the two fetches can still publish or retain state | `check_acceptance` runs before each attempt's egress; a mid-request cancellation discards the response; publication is gated twice. |
| R8 | A cached repository body can be paired with the wrong request or generation | The cache key is `{accept}\n{full url}`; invalidation bumps the namespace generation and drops every entry under one lock; both `accept_generation` and the final generation check guard the pairing. |
| R9 | An over-limit document is truncated and labelled recoverable | The source never truncates: the capped writer turns a cap breach into a typed `LimitExceeded`. Ceilings are monotone (16 MiB representation < 64 MiB artifact < 256 MiB session) and both cut shapes continue losslessly. |
| R10 | This increment changed a shared helper used by another family without updating that family's tests | The ledger lists exactly `facts.rs` (`read_facts`, `facts_resource`), `github/mod.rs` (`read`, `catalog_entries`) and the new `commit.rs`; the `is_facts` edit is additive for the commit spelling. |
| R11 | The repaired C3 equality mutation is still masked by non-SHA link checks | Re-deriving the fixture against `commit.rs` shows the neutered equality guard is the sole possible refusal source, and the MinimalLinks positive control differs by the SHA field alone. The mutant's RED is a real behavioural red. |
| R12 | `require_object_link`'s pull-request-specific prose reaches a commit caller | `read_facts` maps every error through `sanitize` (`facts.rs:529-558`), which for `NotFound` replaces the message with the constant "GitHub facts read failed" while preserving category and details. `ErrorOutput` (`mcp/src/render/error.rs:17-24`) then copies that sanitized message verbatim. **This was reported as a finding in the first draft of this document and withdrawn after re-verification**; a verifier had confirmed it by inspecting the unsanitized helper. Two finder angles had independently refuted it. |
| R13 | The commit budget row's cumulative ceiling saturates at two 8 MiB responses | The second response is the 317-byte repository fixture; total admitted is 8 388 925 B, 49.9 % of the cap. The saturation scenario belongs to `read_acquisition_tests::verified_http_bodies_share_cumulative_admission_without_reset`. The row's *conclusion* survives on a different mechanism — see F13. |

## Below the line

Each item carries its raiser and its evidence state. "Below the line" is not a
severity claim: **T3** is a confirmed measured defect that this review recommends
against fixing here, and **T12** is a claim that was largely refuted.

### T1. `commit.rs:206-211` — `macro_rules! missing` duplicates `facts/pull.rs:164-166` · Angle F · **Verified**

Identical 3-line macro expanding `Presence::unavailable` over a field list; both
feed the same `Vec<Unavailable>`. Sharing needs an accumulator argument, which is
why the copies have not converged. A change to how unavailability is recorded must
find every copy (two macros plus hand-written calls in `comment.rs`, `review.rs`,
`inline.rs`).

### T2. `tests/github_immutable_contract.rs:26-45` — fixture machinery copied from the sibling contract · Angle F · **Verified, scope narrowed**

`host_from_head` is verbatim against `github_facts_contract.rs:82-90`,
`read_reference` differs only by an unused `limits` parameter, and the
substrate/credential/mount sequence repeats `github_facts_contract.rs:150-174`;
the budget test inlines the same commit object a third time. **Narrowed:** the
finder's proposed home (`tests/support/jira.rs`) is family-specific and not
reusable, and the new file *does* reuse the shared TLS/session support — so the
remedy is a GitHub-family support module, not `jira.rs`.

### T3. `commit.rs:34-40` — the native commit message is copied where the response outlives a legal borrow · Angle H · **Verified with arithmetic**

`native!(NativeCommitData { message: String })` decodes through
`Presence<String>` → `String::deserialize`, which allocates even when the slice
reader offers `visit_borrowed_str`. The 8 387 706-byte message is therefore
reallocated out of a body the projection outlives, costing **24.98 %** of the
row's measured 33 575 607 B peak. The repo already uses the proposed
`#[serde(borrow)] Cow<'a, str>` pattern (`facts/continuation.rs:118-126`,
`atlassian/jira/cursor.rs:20-30`).

**Why below the line rather than a finding:** the fix is not a field-type swap. It
needs a lifetime-parameterized type or a new `native!` arm; adding an arm changes
the invocation text the placement gate keys on (the owner row would fail with
"required owned symbol … is missing"), and `facts.rs` is at 671 lines against its
700-line tripwire. That is a design change with a gate accommodation, not a review
repair. Output bytes, semantics and the 128 MB budget are unaffected.

### T4. `tests/github_immutable_contract.rs:304` — the WrongSha row's name implies links the fixture omits · Angle E · **Partially-confirmed**

The fixture removes `html_url`, `comments_url` and `commit.url` for
`FixtureMode::WrongSha` to isolate the equality mutant. Every link the fixture
*does* supply is correct, so the name is not false — but nothing records the
deliberate omissions, and re-adding the links would silently re-mask the equality
mutant, the exact failure S2-F2 repaired.

### T5. `scripts/module_shape.py:319` — `codes.get(path, '')` is redundant · Parent · **Verified**

`production_nodes` already yields `{}` for a path with no production code, so the
guard duplicates its own protection. Harmless; noted so it is not mistaken for a
required defence.

### T6. `scripts/module_shape.py:299` — the `declaration_subject` docstring claims "Tokens and literals" · Parent · **Verified**

`fingerprint` recovers literal *spellings* from the text it is given, but
`declaration_subject` now passes production-masked text, in which literals are
blanked. The subject is tokens only. The behaviour matches the rule the review
base already proved (its `body` is also taken from `codes`), so nothing is broken
— the docstring, written when the function read raw `sources`, misleads the next
reader of the gate.

### T7. Ignored-row registration · Gap sweep · **Refuted — recorded as a positive**

All 32 `#[ignore]` rows are classified; unclassified rows *fail* the gate rather
than being skipped silently, missing `BUDGETS` rows fail it too, and both new
`live_*` rows match `scripts/live-smoke.sh`'s selector. Silent-skip is not
reachable through classification.

### T8. `compiled.rs` — `UnsupportedProjection` as the category for an unserved family · Angle D · **Verified, pre-existing**

Previously dispositioned as S1-R2-F2/R2; the increment changes the *message*'s
truth (F7), not the category.

### T9. `tests/github_facts_contract.rs:30-38` — budget-row execution discipline and wall headroom · Gap sweep · **Verified mechanism; Unverified for flake probability**

The counting allocator's heap counters are process-global, so running the rows the
way their ignore messages invite (`--ignored` without `--exact --test-threads=1`)
can let a sibling multi-MiB row land inside the measured window and breach the
bound — a false red locally while CI is green. The direction is fail-safe (the
peak is stored before measuring). The wall bounds run on a three-OS matrix with
~6.4× headroom on the tightest row. **Settling the flake probability needs one
execution of the budgets leg on macOS and Windows**, which this review could not
run.

### T10. `commit.rs:294-303` — `accept_generation` is a second spelling of the generation rule · Angle F · **Verified**

`comment.rs:180-182`, `inline.rs:296-298`, `review.rs:207-209` and
`collection.rs:563-565` state the same rule inline against a saved generation,
while `facts.rs` owns the bookkeeping through `establish_generation` and
`check_generation`. The commit read genuinely needs the check (its second request
has no parent object to compare against), but it leaves the policy with two
spellings and no single owner. Confidence from the raiser: 0.45.

### T11. `stdio_live_smoke.rs:101-124` — `write_commit_profile` duplicates `write_profile_with_attempts` · Angle F · **Verified**

The same GitHub source object differing only in repository name, session
`cacheDirectory`, and the omitted HTTPS source; both feed the same `check --probe`
and `Server::start` paths. A profile-schema change must be applied twice. The rest
of the new MCP tests reuse the existing harnesses correctly.

### T12. `facts.rs:507-521` — the facts writer takes no capacity hint · Angle H · **Partially-confirmed, benefit refuted**

The writer really is unreserved (`CappedWriter::new(Vec::new(), cap)`) and
serde_json really does emit one write per escape-free run. **Refuted:** the escape
density does not cause the peak — the escape-free fixture's own document already
exceeds 8 MiB, so the doubling happens regardless; the registered
`github_facts_production_budget` row already exercises ~1.05 M fragmented writer
calls at the same scale through the same writer; and the proposed message-length
hint is a sound lower bound yet still 3 003 B short of the document, so it does
**not** remove the doubling. Net: an allocation-churn tidy-up whose benefit for
the peak-heap case it was proposed for is zero, and the writer predates this
increment.

### T13. `.rfs-jrz7/evidence.md:67-74` — the live rows' non-skipped status is prose-asserted · Angle J · **Partially-confirmed**

Both live rows begin `let Some(token) = live_token() else { return };`, which
prints a skip notice and returns green when `RFS_LIVE`/`GITHUB_TOKEN` are unset.
Because libtest captures output from passing tests, the retained receipt
(`1 passed; 0 ignored; 4 filtered out`) is identical for a real run and for a
skip, so "No row skipped" is asserted in evidence.md prose rather than observable
in the receipt. The only receipt-side signal is the recorded ~2 s duration (a skip
returns in microseconds). Also: a genuine live run silently omits its
native-observation comparison when the `gh` CLI is unavailable, so the receipt
cannot prove that half either. This pattern is deliberate and pre-existing across
the repository's live rows, which is why it is below the line here.

### T14. `.rfs-jrz7/commit-oracle-result.json` and the catalog claim · Angle J · **Substance verified, attribution overstated**

The oracle artifact is reproducible byte-for-byte — tree `c4402464…`, ordered
parents `0dae3b64…`/`04ddca39…` and the 69-byte message all match
`git cat-file commit 635ab170…`, and the live provider returns the same values.
The catalog entry is mounted-source-gated in code and asserted in tests at both
mount states. **Correction:** neither `docs/operating.md` nor `DESIGN.md` states
the entry is mount-gated, so the original claim's documentation attribution was
overstated while its substance is right. Nothing needs fixing.

## Merge resolution

This head is the merge of `origin/main` (the S1 round-3 repairs) into the slice
branch. Six files conflicted. The resolution kept both sides, and the verification
below is what makes that claim checkable rather than asserted:

- **`scripts/module_shape.py`** — main's production-masked, memoised spans were
  kept, and the branch's `native!` subject was rebuilt on top of them. Proven by
  re-running main's own A/B harness (`.rfs-jrz7/mutation-review3.py`) on the
  merged gate: all nine recorded cases reproduce their recorded verdicts,
  including the five that must still bite, and a fresh macro probe fails a
  verbatim `native!` copy while a reshaped same-named invocation stays green. The
  residual weakening relative to main is **F2**, which the branch introduced
  before the merge; the harness's "reviewed" column fails on the merged tree for
  reasons unrelated to any case (the old gate does not know this increment), so
  only its "repaired" column is evidence here.
- **`compiled.rs`, `module-ledger.json`** — auto-merged; the retired
  `wiring_calls` table stays retired and the increment's owner rows survive.
- **The three contract test files** — the unmounted read reports the missing
  profile entry while discovery and mutation keep the family's
  configuration-independent refusal; the parent's mutation claim is restored
  against the search message rather than the (now different) read message. The
  commit production budget joins the parent's adversarial parser budget and the
  superseded single-argument reference builder is dropped.
- **`plan.md`, `review-decisions.md`** — S1 round-3 records precede the S2 ones.

Full gate on the merged head: **passed** (module placement, clippy, debug and
release workspace tests, ignored production budgets, `cargo deny`).

## Disposition

No code was changed by this review; verification is not authorization to repair.
Ranked repair plan:

1. **F1** — the only finding that breaks the increment's stated behaviour on real
   upstream data. Start with a `dependabot[bot]`-shaped fixture, then relax the
   account-link expectation.
2. **F2, F6** — restore the counterpart all-span iteration; delete `ParentLinks`.
   Both are "we already ordered this fixed" regressions.
3. **F3, F4, F5, F8, F13** — fence, category and evidence repairs; each is small
   and each converts a claim into something the suite can falsify.
4. **F7, F9, F10, F11, F12** — wording, dispatch and doc accuracy.
5. **T1–T6, T9–T14** — hardening and cleanup, or explicit acceptance. **T3** is
   the one to decide deliberately: it is a confirmed measured cost whose fix needs
   a design change and a placement-gate accommodation, so it belongs in a
   follow-up rather than this review's repair round.
