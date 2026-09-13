# PR #12 code review (feat/rfs-jrz7 → main, head `6e1c55c`)

Automated review run on 2026-09-10 at effort `max`. Twelve agents ran — ten independent
finder angles (A–E functional, F–J quality) plus a verifier pass and a Phase 3 gap sweep.
The PR head was checked out into a scratch worktree (since removed; the main working tree
was never touched) and the top findings were confirmed by compiling and running targeted
probes against it.

**The PR is green.** The workspace builds clean, clippy is clean, `scripts/module_shape.py`
passes, and the full core + sources suite is green. Nothing below is a build break — every
finding is a latent or semantic defect.

**Scope.** 37 changed files, 2,929 additions / 102 deletions.
Merge-base `635ab17`, head `6e1c55c`.
<https://github.com/dwalleck/resourcefs/pull/12> — *feat: immutable GitHub addressing and
decoded acquisition controls*.

## What is in this document

The review ran under a `≤15 findings` output cap, so the fifteen findings in the first
half are a ranked survivor set, not the full yield. This document carries the complete
inventory.

| Bucket | Count | Where |
|---|---|---|
| Raw candidates before dedup | 78 | 8 each from angles A–I, 2 from J, 5 from the sweep |
| **Reported findings** | **15** | [F1–F15](#index-by-theme) — severity order, F1 most severe |
| Refuted with counter-evidence | 5 | [R1–R5](#refuted-candidates) |
| Folded as duplicates of a reported finding | 12 | [Folded duplicates](#folded-duplicates) |
| Trimmed below the cap — functional | 11 | [T1–T11](#below-the-line--functional) |
| Trimmed below the cap — quality tier | 25 | [Q1–Q25](#below-the-line--quality-tier) |

The verifier produced six verdicts rather than new candidates. One below-the-line item
(**T3**) carries **UNVERIFIED** status and is marked with the probe that would settle it;
everything else below the line was either confirmed by execution or is a cleanup with no
runtime claim to verify.

## The through-line

Three patterns account for nearly all of what follows.

1. **Guards measure the wrong quantity.** `GithubSourcePath::new` checks the encoded
   *path* against the 64 KiB *reference* ceiling, ignoring the 74–204 byte `github://…/facts`
   envelope that must also fit (F1). The owner-uniqueness check counts declarations over
   raw source while the guard directly above it counts over production-masked source (F7).
   The capacity hint sizes an encoded buffer from a decoded byte total (F13). In each case
   a sibling in the same tree already measures it correctly — `SourceCursor::validate`
   subtracts its own prefix; the census above uses `declarations`.

2. **Fences that do not fence.** Three tests state a purpose their assertions cannot
   deliver. The mounted family fence passes identically with and without the guard it was
   written to protect, because an unrelated catch-all produces a byte-identical error (F2).
   The C19 encoded-separator fence still justifies itself by a panic contract this same PR
   deleted (F9). The allocation budget hardcodes the cheapest segment shape, so the
   parser's real worst case — ~32,700 segments in the same 64 KiB — is never measured (F14).

3. **Workarounds promoted instead of causes fixed.** `CALL` matching `Github(` inside the
   enum-variant pattern `ResourceAddress::Github(_)` was first patched with a hard-coded
   `{"Query"}` allowlist; on its second occurrence the workaround was promoted to a ledger
   table plus a validator rather than fixed in the regex (F15). The gate's new transition
   machinery carries two structurally unsatisfiable checks (F4, F5) and one stale pinned
   entry corrected only by a runtime patch (F12).

## Index by theme

| # | Theme | Location | One line |
|---|---|---|---|
| F1 | `github://` bounds | `core/src/reference/github.rs:66` | Path ceiling ignores the `github://…/facts` envelope |
| F8 | `github://` grammar | `core/src/reference/github.rs:159` | Parser accepts selectors both identity validators reject |
| F11 | `github://` grammar | `core/src/reference/github.rs:216` | Encoded backslash survives into a validated segment |
| F13 | `github://` bounds | `core/src/reference/github.rs:55` | Encode hint 3x short; ceiling checked after materialization |
| F2 | Fences that don't fence | `sources/tests/github_adapter_contract.rs:735` | Mounted family fence passes without the guard |
| F9 | Fences that don't fence | `core/tests/path_reference_contract.rs:588` | C19 rationale cites a panic contract this PR deleted |
| F14 | Fences that don't fence | `sources/tests/github_facts_contract.rs:1554` | Budget pins the cheapest shape; assertion cannot bite |
| F3 | Capability contract | `sources/src/compiled.rs:174` | Read and mutate arms make contradictory claims |
| F4 | Module-shape gate | `scripts/module_shape_base.py:297` | Obsolete-helper check keys on old names — unsatisfiable |
| F5 | Module-shape gate | `scripts/module_shape_base.py:311` | Owner outside the watch list counts as "found 0" |
| F6 | Module-shape gate | `scripts/module_shape.py:469` | Ledger-driven `git show` unguarded; aborts the whole gate |
| F7 | Module-shape gate | `scripts/module_shape.py:332` | Owner census counts `cfg(test)` declarations |
| F10 | Module-shape gate | `scripts/module_shape.py:342` | Uniqueness narrowed and 1,651x slower |
| F12 | Module-shape gate | `scripts/module_shape_base.py:105` | Pinned policy still owns a deleted symbol |
| F15 | Module-shape gate | `scripts/module_shape_base.py:540` | `wiring_calls` table masks a tokenizer bug |

## `github://` grammar and bounds

### F1. `crates/resourcefs-core/src/reference/github.rs:66`

`GithubSourcePath::new` bounds only the encoded path against `MAX_PATH_REFERENCE_BYTES`,
ignoring the 74–204 byte `github://<owner>/<repo>/source/<40-hex>/ … /facts` envelope, so
it blesses typed paths that can never become a `PathReference` — the exact inverse of the
invariant its own comment states.

**Failure scenario (verified by execution against the PR head).**
`GithubSourcePath::new(vec!["a".repeat(65536)])` succeeds; the resulting
`GithubAddress::Source.canonical_reference()` is 65,610 bytes; `PathReference::github(address,
None)` then returns `Err(LimitExceeded, "Path Reference exceeds 64 KiB")`. Every canonical
path length in `65,332..=65,536` is affected.

The comment two lines above says *"a typed path that cannot fit a Path Reference must not
exist, or the full-reference parser would refuse a value this type accepted"* — which is
precisely what happens. The PR's own test
`typed_source_paths_stay_within_the_reference_ceiling` asserts the 65,536-byte construct
succeeds ("exact ceiling is inclusive") without ever feeding it to `PathReference::github`,
so the test pins the hole open.

The sibling `SourceCursor::validate` (`reference/source_page.rs:65`) already subtracts its
prefix and returns `LimitExceeded`; this constructor returns `InvalidReference` for the
same ceiling, so the two disagree on category as well as on arithmetic.

### F8. `crates/resourcefs-core/src/reference/github.rs:159`

Every `github://` reference in this grammar is a `/facts` document, which accepts no
selector, yet `parse_github_reference` splits and stores a projection — producing
`PathReference`s that both identity validators reject unconditionally.

**Failure scenario (verified by execution).** `github://owner/repo/commits/<sha>/facts:raw`,
`…/a.rs/facts:12-20` and `…/facts:page:2` all parse and round-trip with `Some(projection)`.
But `canonical_identity` (`discovery.rs:1259`) and `validate_canonical_identity`
(`resource.rs:642`) both hard-require `projection().is_none()` for `Github`, so such a
reference can never be a Resource identity or a discovery record.
`PathReference::github(address, Some(selector))` (`reference.rs:1188`) is a public
constructor that only ever produces unusable values.

Both sibling parsers post-validate — `parse_jira_reference` rejects cursors off
collections, `parse_pull_request_reference` rejects a cursor off a comment collection — and
`GithubSource::search` (`github/mod.rs:918`) refuses selectors on Facts documents outright,
with a comment explaining why.

**The shipped evidence contradicts itself.** `.rfs-jrz7/falsify_grammar.py:57-62` asserts
`x/facts:raw` must be REFUSED and `falsify-grammar-result.txt` records
`"C1 selector distinction PASS"`, while `.rfs-jrz7/compare_grammar.py:37` hard-codes the
accept-with-projection answer as expected. No test asserts either behavior.

### F11. `crates/resourcefs-core/src/reference/github.rs:216`

`github://` is the only path-bearing family whose parse never calls
`validate_percent_encoding`, so encoded backslashes survive into a validated segment;
`validate_decoded_segment` spells the containment rule only for `/`.

**Failure scenario (verified by execution).**
`github://owner/repo/source/<sha>/..%5C..%5C..%5Cetc%5Cpasswd/facts` parses to the single
segment `..\..\..\etc\passwd`, and `C%3A%5CWindows%5Csystem32` to `C:\Windows\system32` —
while `local://a%5Cb`, `rfs://workspace/root/a%5Cb` and `a%5Cb` are all rejected with
"Path Reference must not contain an encoded separator". The explicit rejection of `.` and
`..` shows containment is the intent; it simply does not cover the Windows separator
spelling.

**Why this is latent, not exploitable today.** Backslash preservation is a recorded,
defensible decision (`.rfs-jrz7/review-decisions.md` S1-R2-F13: Git names are byte strings)
and there is no consumer of `segments()` yet. But `GithubSourcePath` is `pub` and
re-exported from `lib.rs`, carries no invariant against a downstream join, and its type doc
("Git names preserve backslashes and control bytes") reads as a guarantee rather than a
warning. At minimum the doc should state that segments must be compared against Git tree
entry names and never joined to a filesystem or cache path.

### F13. `crates/resourcefs-core/src/reference/github.rs:55`

The capacity hint sizes the ENCODED buffer from the DECODED byte total (3x short for any
escaped byte), and the ceiling is checked only after the whole canonical string is
materialized, so the public constructor allocates unboundedly before rejecting.

**Measured.** A 60,074-byte escape-heavy reference (20,000 × `"%20"`) takes 1.748 ms versus
707 µs for an identically sized unescaped one, because the hint is 20,000 while the output
is 60,000 — two reallocations, 120,000 bytes allocated and ~60,000 memcpy'd to produce a
60,000-byte string. Separately, `GithubSourcePath::new(vec!["b".repeat(4 MiB)])` allocates
4,194,304 bytes and spends ~10 ms encoding byte-by-byte before returning the ceiling error.
The type is `pub` and re-exported; only the internal `::parse` path is bounded upstream by
`validate_reference_input`.

**Both fixes are free.** Reject on `decoded_bytes > ceiling` at line 55 first (sound,
because `canonical.len() >= decoded_bytes` always), and count non-unreserved bytes in the
validation loop at line 52 that already touches every byte. Note the `+ segments.len() - 1`
term is safe only because of the `is_empty()` guard three lines away.

## Fences that don't fence

### F2. `crates/resourcefs-sources/tests/github_adapter_contract.rs:735`

The mounted half of the paired family fence is inert: it passes identically with and
without the new `ResourceAddress::Github(_)` arms, so it cannot detect the fold-back
regression it was written to catch.

**Failure scenario (mutation applied and run).** Delete
`ResourceAddress::Github(_) => Err(github_family_unreadable())` from `CompiledSources::read`
(`compiled.rs:132`) and fold `Github` into the `Issue | PullRequest` arm — the mutation
`.rfs-jrz7/mutation-review2.sh` applies. On a mounted build `self.github_source()?`
succeeds, `GithubSource::read_resource` falls to `_ => return Err(unsupported_github_projection())`
(`github/mod.rs:792`), which is `ErrorCategory::UnsupportedProjection` with no details
(`mod.rs:1157-1162`) — byte-identical to what the test asserts. No HTTP request is made, so
the fixture's `panic!("unserved family")` router never fires. `GithubSource::search` has the
same catch-all at `mod.rs:928`, so the search half is equally inert. Only the unmounted
sibling goes red.

The test also omits the controls-bearing read and the resolve assertion its sibling makes,
so `compiled.rs:172`'s mutation arm is never exercised on a mounted registry at all —
contradicting the PR body's claim that "paired mounted/unmounted fences pin it".

### F9. `crates/resourcefs-core/tests/path_reference_contract.rs:588`

The C19 encoded-separator fence's stated rationale still asserts the panic contract this PR
deleted, making its "the test completing at all is itself the evidence" claim vacuous.

The doc says `percent_decode` *"indexes `bytes[index + 1]` under an
`expect("percent escapes were validated")`, so a form that reaches it without prior
validation aborts the process rather than returning `invalid_reference`"*. This PR replaced
both `expect`s with `ok_or_else`, so the fence's whole justification for not being a family
list is now false. The PR rewrote the two source comments about this same invariant
(`reference.rs:1046-1049` and `1602-1605`) and missed this one.

**Concretely untested as a result.** `github://owner/repo/source/<sha>/a%/facts` — the
former panic input — is now rejected correctly, but nothing pins it. The guard table gained
no `github://` row even though `github://` is now the only family that reaches
`percent_decode` unvalidated (see F11), so its `PreservesEncoding` / `RejectsSeparator` /
`RejectsMalformed` rows have no entry for the one family that most needs them.

### F14. `crates/resourcefs-sources/tests/github_facts_contract.rs:1554`

Nothing bounds the segment count, and the new budget hardcodes the cheapest 1,024-segment
shape, so the parser's real worst case is never measured and the heap assertion cannot bite.

**Failure scenario.** With `SEGMENT_COUNT = 1_024` and `total_bytes = 65_536` the reference
is one 63,410-byte segment plus 1,023 two-byte ones (~1,049 allocations measured). The
adversarial 64 KiB reference `github://o/r/source/<sha>/x/x/x/…/facts` yields ~32,700
segments — ~32,700 separate `String` allocations plus a 32,700-element `Vec<String>`,
roughly 32x the per-segment cost — and is exactly what an untrusted-reference DoS would
pick.

The assertion `incremental_heap <= 16 * EXACT_BYTES + 1_048_576` is 2.1 MiB against a
measured actual peak of 417 KB, so it cannot detect a regression of that magnitude either.
`GithubSourcePath::new` then walks every segment twice more — `validate_decoded_segment`'s
`contains('/')` and `contains('\0')` scans duplicate work `percent_decode` already did.

## Capability contract

### F3. `crates/resourcefs-sources/src/compiled.rs:174`

The `MutationAdapter` arm advertises `github://` as "read-only" while the `SourceAdapter`
arm 42 lines above says the same family is "not served by the compiled sources" — one
address, two contradictory capability claims in the same build.

**Failure scenario.** `rfs_edit` on `github://owner/repo/commits/<sha>/facts` returns
`UnsupportedMutation` — *"github:// Resources are read-only; immutable GitHub facts accept
no mutation"* — which tells the caller a read is the supported route. The same caller then
issues `rfs_read` on the identical reference and gets `UnsupportedProjection` — *"github://
Resources are not served by the compiled sources"*.

The `Https` and `Jira` read-only arms this one is modeled on are truthful because those
families really are readable. `github://` is readable on no build in this increment, so the
mutation refusal promises a capability that does not exist and sends the agent into a dead
loop.

## Module-shape gate

### F4. `scripts/module_shape_base.py:297`

The new "obsolete shared helper" check keys on OLD symbol names while `historical_owners`
is rekeyed to NEW names, so any `relocated_symbols` row that keeps the symbol name (a pure
ownership move) is permanently unsatisfiable.

**Reproduced by execution.** Adding
`{"parse_jira_address": {"name": "parse_jira_address", "owner": "crates/resourcefs-core/src/reference/jira.rs"}}`
to `relocated_symbols` makes the gate emit:

```
inherited C1 FAIL crates/resourcefs-core/src/reference/jira.rs: obsolete shared helper
parse_jira_address must migrate to its approved owner
```

— pointed at the symbol's own approved owner. Lines 275-277 `del historical_owners[old]`
then re-add the same key, and line 297 then sees the name in
`transition.relocated_symbols` for a declaration sitting exactly where the ledger says it
belongs. There is no ledger spelling that clears the gate; the two keyspaces (old-name →
relocation, new-name → owner) are conflated in one `name` variable.

### F5. `scripts/module_shape_base.py:311`

`relocated_counts` is only incremented for declarations found inside `watched`
(`CORE/reference/`, `SOURCES/atlassian`, `SOURCES/http`) plus `CORE/reference.rs`, so a
relocation whose approved owner lives anywhere else reports "found 0" even when the symbol
is declared exactly once and correctly placed.

**Reproduced by execution.** Pointing the relocation at `canonical_identity` owned by
`crates/resourcefs-core/src/discovery.rs` — a file `module_shape.py` already manages as a
`PARENTS` entry, and where `fn canonical_identity` is declared exactly once — yields:

```
inherited C1 FAIL crates/resourcefs-core/src/discovery.rs: shared helper canonical_identity
requires one owner, found 0
```

The check silently conflates "declared once" with "declared once inside this historical
watch list", and the only way to satisfy it is to edit the pinned policy. Today's single row
(`encode_rfc3986_segment` → `reference.rs`) happens to land inside the set, so it passes by
luck. `module_shape.py:218-224` validates `wiring_calls` paths but adds no equivalent
validation for `relocated_symbols` owners.

### F6. `scripts/module_shape.py:469`

The new C11 protected-parent block validates the pinned commit id and probes the commit
object, then leaves the `git show <pin>:<path>` on ledger-supplied data unguarded, so a path
absent at its own valid pin aborts the entire gate with an anonymous oracle-input failure.

**Failure scenario.** Point the `protected_parents` row at a path that does not exist at
commit `635ab170` — e.g. a later increment renames `github/facts.rs` to
`github/facts/tree.rs` while keeping the row. `inherited.git()` raises `ValueError`, it
escapes `check()` uncaught, and `main()`'s blanket handler prints:

```
C02 FAIL oracle input: git show 635ab170...:crates/.../tree.rs failed (exit 128)
```

— with no C11 claim, no row identity, and every other check in the run abandoned. The two
guards immediately above (the 40-hex regex at `:461` and the `cat-file -e` at `:466`, both
wrapped in try/except) exist precisely because this row is ledger data rather than a module
constant; the third read was left out.

### F7. `scripts/module_shape.py:332`

The rewritten owner-uniqueness check counts declarations with `node_spans` over the RAW
source while the guard immediately above it uses production-masked `declarations`, so
declarations inside inline `#[cfg(test)] mod` blocks — deliberately excluded from the
ownership census everywhere else — now count and produce a false failure.

**Reproduced.** Appending `#[cfg(test)] mod zz_probe { fn parse_pull_request_reference() {} }`
to `crates/resourcefs-core/src/reference/pull.rs` (an owner row whose symbols include that
name) makes the gate emit:

```
C02 FAIL crates/resourcefs-core/src/reference/pull.rs: owned symbol
parse_pull_request_reference must have exactly one declaration
```

— even though `production()` blanks that body and the production census sees exactly one.
Latent today: none of the 17 `ledger['owners']` rows currently has an inline `cfg(test)`
body (`github/facts.rs` uses a bodyless `#[path] mod tests;`). But the gate forbids a normal
Rust idiom in exactly the files it is meant to protect.

### F10. `scripts/module_shape.py:342`

Owner uniqueness was narrowed from "same name anywhere in the owner's directory" to "same
name AND byte-identical fingerprint anywhere", which loses two whole classes of duplicate
and costs a measured 1,651x more time.

**What it stopped catching.**

- `node_spans`'s pattern captures only `(fn|struct|enum|const|type)` while `inherited.DECL`
  also matches `trait`, so a trait declaration bearing an owned symbol's name yields no span
  and can never be flagged.
- `fingerprint` includes literal spellings, so a copied body whose only difference is one
  error message, or `Err(())` vs `Ok(())`, no longer matches — and divergent copies are the
  dangerous kind, since they drift. Under the old rule ANY declaration of that name under
  the owner's directory failed regardless of body.

**Cost (measured on the real 103-file workspace).** The old loop is 1.038 ms; the new one is
1,714 ms, because `node_spans` re-parses whole files per (symbol × candidate) pair — 130
calls over only 24 distinct files, e.g. `github/facts/identity.rs` parsed 22 times at
21.9 ms each. A path-keyed memo (shared with `check_parent_nodes:241` and the
`protected_parents` loop at `:471`) cuts it roughly in half.

### F12. `scripts/module_shape_base.py:105`

The pinned policy still asserts ownership for `encode_jira_segment`, a symbol this PR
deleted; the correction lives only in a runtime ledger patch applied when `transition` is
not `None`, and both signatures default it to `None`.

`HISTORICAL_OWNERS` maps `encode_jira_segment` → `crates/resourcefs-core/src/reference/jira.rs`,
a declaration the PR removed. `check_historical(root, stage, transition=None)` and
`check(root, repository, stage, transition=None)` both default to `None`; with `None` the
copied dict keeps the dead entry, `relocated_counts` is empty, and neither the "obsolete
shared helper" failure nor the one-owner count runs — so nothing enforces that
`encode_rfc3986_segment` has exactly one owner or that `encode_jira_segment` stays deleted.

Latent because `module_shape.py:192` always passes a transition. But the PR's own comment
(`module_shape_base.py:29-32`) says ceilings and ownership belong "in the pinned policy, not
through a ledger relaxation" — and this correction does exactly the opposite.

### F15. `scripts/module_shape_base.py:540`

The whole new `wiring_calls` ledger table is an allowlist compensating for a tokenizer bug:
`CALL` matches `Github(` inside the enum-variant pattern `ResourceAddress::Github(_)`.

`CALL = re.compile(r"\b([A-Za-z_]\w*)\s*(?:!\s*)?\(")` captures a match arm as a function
call. The line directly above is proof this is the second occurrence of the same workaround
— `allowed_calls = {"read_query", "Query"} if path == … else {"Query"}` already hard-codes a
variant name for exactly this reason — and `.rfs-jrz7/plan.md` records that the first
integrated run "misclassified `ResourceAddress::Github` patterns as new calls".

Rather than fixing the regex on the second occurrence, the workaround was promoted to a
ledger table plus a validator (`module_shape.py:218-224`). Every future family must now add
its variant name to `wiring_calls`, for every protected parent that matches on it, in every
stage — and the next family discovers the need the same way this one did, through a false
failure. Excluding `::`-qualified PascalCase identifiers in the regex retires both
workarounds and the table.

---

# Below the line

Everything from here down was surfaced by the review but did not make the reported 15.

## Refuted candidates

Five candidates were killed by counter-evidence. They are recorded here so the same
hypotheses are not re-raised on the next increment.

### R1. `crates/resourcefs-sources/src/compiled.rs:132` — Github read arm never calls `reject_acquisition`

*Raised by Angle E.* The claim: the Github arm returns `Err(github_family_unreadable())`
without consulting `acquisition`, so a controls-bearing read loses
`ErrorReason::AcquisitionControlsUnsupported`, which `source.rs:8-14` calls "the one
outcome the contract forbids and no test would catch."

**Counter-evidence.** `CompiledSources::read` already has three arms that refuse before
consulting acquisition — the unmounted `self.https_source()?`, `self.atlassian_source()?`
and `self.github_source()?` calls at `compiled.rs:122-131`, where `?` fires first. The
verifier ran them: on an unmounted build, `https://example.com/a` *with*
`Some(&ReadAcquisitionLimits::default())` returns `SourceUnavailable`, reason `None`;
`issue://` and `pr://` behave identically. Only the Catalog arm calls `reject_acquisition`,
because that arm actually serves a resource. Nothing is silently dropped — the whole read
is refused, with the same `ErrorCategory` that `reject_acquisition` itself uses
(`source.rs:22`), and nothing in the tree branches on `AcquisitionControlsUnsupported`.
Already dispositioned in `.rfs-jrz7/review-decisions.md` as S1-R2-F9.

### R2. `crates/resourcefs-sources/src/compiled.rs:288` — `UnsupportedProjection` is the wrong category for "family not served"

*Raised by Angle C.* The claim: `github_family_unreadable()` reuses the category the tree
uses for genuine selector failures, so a client that drops its `:raw` selector and retries
gets the identical error.

**Counter-evidence.** The premise "everywhere else it means a bad selector" is false in
pre-existing code — `catalog_discovery_unsupported` (`discovery.rs:1331-1335`),
`HttpsSource::glob` (`https.rs:203-207`), `GithubSource::glob` (`github/mod.rs:958-962`)
and `workspace_glob_required` (`filesystem.rs:1306-1311`) all use it with no selector
involved. `DESIGN.md:196` is decisive: it assigns `unsupported_projection` to a build not
configured to serve Facts. Every alternative is worse — `NotFound` would assert the commit
does not exist upstream; `SourceUnavailable` is reserved for "no source configured" and
would re-create the mounted/unmounted ambiguity this repair exists to remove;
`InvalidReference` is false since the reference parses. Already dispositioned as S1-R2-F8.

### R3. `crates/resourcefs-core/src/discovery.rs:72-78` — `github://` glob falls through to `GlobSource::Workspace`

*Raised by Angles C and E.* The claim: `DiscoveryAdapter` has two methods and only `search`
got a Github arm, so `rfs_glob("github://owner/repo/source/<sha>/**")` reaches
`FilesystemSource::glob` and dies in `workspace_glob_scope` (`filesystem.rs:1242`) with a
Workspace message instead of the family refusal.

**Counter-evidence.** The verifier probed a live `CompiledSources` and got the
byte-identical error for `https://example.com/**`, `jira://TEST/**`,
`issue://owner/repo/1/**` and `pr://owner/repo/1/**`. `GlobTarget::new` is untouched by
this diff (the `discovery.rs` changes are only the `GlobEntry` arm at `:384` and
`canonical_identity` at `:1259`), and glob never dispatches on address at all, so there was
no arm to add. The behavior is already pinned as accepted for another family at
`crates/resourcefs-mcp/src/server/jira_query_tests.rs:492-496`. The *category* matches the
family refusal; only the message differs. Pre-existing message-quality wart shared by all
five remote families.

### R4. `crates/resourcefs-sources/tests/github_adapter_contract.rs:740` — the "mounted" fence may not actually mount the GitHub source

*Raised during Phase-2 verification.* The claim:
`CompiledSources::new(filesystem, artifacts, local, None, Some(github), None)` might bind
`Some(github)` to the wrong positional parameter.

**Counter-evidence — clean.** `compiled.rs:30-37` is
`new(filesystem, artifacts, local, https: Option<HttpsSource>, github: Option<GithubSource>, atlassian: Option<AtlassianSource>)`;
the three Options are distinct types and `fixture_source` returns
`(TlsListener, GithubSource)`, so a transposition would be a compile error, not a silent
misbind. The paired unmounted fixture passes `None, None, None` and is genuinely
github-free. (The separate *inertness* defect in that same test is reported as **F2**.)

### R5. `fuzz/fuzz_targets/path_reference.rs:21` — the fuzz oracle asserts any `%2f`/`%5c` input must be rejected

*Raised by Angles A and C.* The claim:
`github://owner/repo/source/<sha>/back%5Cslash/facts` parses `Ok` (the PR's own test pins
it as valid) while the fuzz target asserts `first.is_err()`, so the target panics on a
shape reachable from the seed grammar.

**Counter-evidence.** Run at this head: `https://example.com/a%5Cb` → `Ok`,
`https://example.com/a%2Fb` → `Ok`, `jira://site/search/a%5Cb` → `Ok`. The oracle is
already false on `main` for at least two families, and
`crates/resourcefs-core/tests/path_reference_contract.rs` explicitly pins those HTTPS forms
as `PreservesEncoding`. `fuzz/Cargo.toml` declares its own workspace, so
`cargo check --workspace` never builds it and nothing under `scripts/` or `.github/`
invokes it. Stale pre-existing oracle rot that this PR widens by one family — not a defect
it introduced.

## Below the line — functional

Eleven functional candidates survived verification but ranked below the cap.

### T1. `crates/resourcefs-core/src/discovery.rs:1259` and `crates/resourcefs-core/src/resource.rs:642` — Github identity arms reject every projection

Both validators hard-require `projection().is_none()` for Github, while the
Issue/PullRequest arms three lines below route through `github_record_identity` and the
Jira arm through `jira_record_identity`, both of which admit a `:page:<n>` continuation as
canonical.

The parser mints `github://owner/repo/commits/<sha>/facts:page:2` today (verified to
round-trip). No live break — `CompiledSources` refuses the family before any record or
`SourceResource` is built. It becomes a break the moment S2/S3 makes commit/source Facts
readable with pagination: a paginated github Facts page would be unable to name itself,
because `SourceResource::text(canonical, ..)` and `SearchRecord::new` would both reject the
page-bearing reference `pr://` uses without complaint. One line to decide now.

*Raised by Angle C. Trimmed:* the narrower forward-compat half of **F8**'s seam, with no
reachable consequence in this increment; listed separately because the fix is in a
different file and is a design decision, not a parser guard.

### T2. `scripts/module_shape.py:327` — the new `continue` suppresses the cross-file "belongs in" diagnostic

The old code ran the namespace scan unconditionally, so relocating `parse_github_reference`
out of its owner produced *both* "required owned symbol … is missing" and "symbol …
belongs in `<owner>`". The new code reports only the first, so the gate says a symbol
vanished without saying where it went. Combined with **F10** (the workspace scan now needs
an identical fingerprint), a symbol that moved *and* was lightly edited produces no
location information at all.

*Raised by Angle B. Trimmed:* a diagnostic-quality regression — the gate still fails, just
less helpfully — so it ranks below every finding that causes a false pass or false failure.

### T3. `scripts/module_shape_base.py:275` — a bad `relocated_symbols` row raises before the ledger-shape validation runs — **UNVERIFIED**

`check_historical` raises `ValueError` on an unknown historical owner symbol; that call
sits at `module_shape.py:192`, ahead of the new per-claim validation at `:198-224`. A
ledger typo therefore aborts the run with
`C02 FAIL oracle input: unknown historical owner symbol 'x'` — mislabelling a C11-stage
ledger defect as C02 and suppressing every other ledger-shape failure the new block would
have listed. Because `git()` also raises `ValueError` (`module_shape_base.py:132`), an
unrelated Git failure reports through the same anonymous channel.

*Raised by Angle D.* **UNVERIFIED:** the mechanism is plain from call ordering, but no bad
old-name was ever injected to observe the message. **To confirm:** add
`relocated_symbols["immutable-grammar"]["not_a_real_symbol"] = {...}` and check the emitted
claim label. Same class as **F6**, at a different site.

### T4. `scripts/module_shape_base.py:33` and `:72` — the `reference.rs` tripwire was over-raised and is written twice

The increment needed +53 lines; the ceiling was raised +70 (2010 → 2080) while the file is
2053, leaving 27 lines a later change can consume with no placement review — the exact
thing the new comment says must not happen. Gate output on the PR head:
`C12 crates/resourcefs-core/src/reference.rs: lines=2053 maximum=2080` and
`C1 …: 2089 -> 2053, maximum 2080`.

Because the number lives in both `PARENTS` (`:33`) and `HISTORICAL_PARENTS` (`:72`), a
future increment that raises only one gets a green C12 and a red C1 (or vice versa) with no
indication the dicts disagree. `HISTORICAL_PARENTS`'s companion `before` element (2089) is
now *larger* than its own maximum, producing the nonsense observation above. Pinning
2053–2060 restores the previous ~10-line tightness.

*Raised by Angles B and I. Trimmed:* confirmed by running the gate, but it is policy slack
and duplication with no incorrect verdict today.

### T5. `crates/resourcefs-core/src/reference.rs:685` — `ResourceAddress` is a public enum without `#[non_exhaustive]` in a workspace pinned at 1.0.0

Adding the `Github` variant is a semver-major break shipped without a version change. No
crate carries `publish = false`, the workspace version is `1.0.0`, and no enum in `crates/`
carries `#[non_exhaustive]` (checked — the only hits are `finish_non_exhaustive()` Debug
impls). Any external consumer writing an exhaustive `match` on
`resourcefs_core::ResourceAddress` — the only way to use it — fails to compile against the
new release. The workspace demonstrates the blast radius: one variant forced edits in
`read.rs`, `resource.rs`, `discovery.rs` (×2), `reference.rs`, `compiled.rs` (×3) and four
test files.

*Raised by the sweep. Trimmed:* real and verified, but a repo-wide pre-existing pattern
this PR extends rather than introduces, with no runtime consequence for a standalone MCP
server binary.

### T6. `crates/resourcefs-core/src/reference/github.rs:17` — asymmetric normalization between the repo half and the SHA half

`GithubRepositoryIdentity::new` lowercases owner and repository
(`reference.rs:400-401`), so `github://Owner/Repo/commits/<sha>/facts` is accepted and
canonicalised; `github://owner/repo/commits/0123456789ABCDEF…/facts` is an
`InvalidReference`. A user who normalises the repo half has no signal the SHA half is
subject to a stricter rule. Separately, `GithubCommitId::new` measures bytes with
`str::len()` while its message says "characters" — currently masked by the ASCII-hex scan
rejecting multibyte input first, so it is a latent trap only if that byte class is widened.
The PR's own `immutable_grammar_rejects_aliases_and_unrepresentable_operands` locks the
inconsistency in.

*Raised by Angle D. Trimmed:* deliberate design (immutable identity is canonically
lowercase) and pinned by test, so it is a UX/consistency judgment rather than a defect.

### T7. `crates/resourcefs-core/src/reference/github.rs:103` — `GithubAddress` has public struct-variant fields, so no invariant can attach to the combination

`GithubCommitId` and `GithubSourcePath` each use private-fields-plus-`new`; `GithubAddress`
does not, so `GithubAddress::Source { .. }` is constructible by any downstream crate with
no validation hook. This is the structural reason **F1**'s length invariant is
unenforceable — there is no place to say "this address fits a Path Reference", and no way
to add a bound later without a breaking API change. Three call sites already treat
`canonical_reference()` as an identity that must re-parse (`reference.rs:1188`,
`discovery.rs:1259`, `resource.rs:642`), an invariant the type cannot promise.

*Cleanup:* `GithubAddress::commit(..)` / `GithubAddress::source(..)` constructors returning
`Result`, fields private. *Raised by Angle E. Trimmed:* absorbed in effect by **F1**, which
names the concrete consequence; kept because the deeper fix is a different edit.

### T8. `crates/resourcefs-sources/src/compiled.rs:288` — `github://` is parseable and dispatchable but described by no `SourceCatalogEntry`

`GithubSource::catalog_entries` (`github/mod.rs:877-892`) still lists only `issue://` and
`pr://`, so the `rfs://sources` catalog — the documented discovery entry point — never
mentions the family. `crates/resourcefs-mcp/src/server.rs:527` tells clients "Start with
rfs_read of rfs:// to discover mounted sources", so a client following the documented flow
can never learn `github://` parses, yet the parser accepts it and returns a refusal that
presumes the caller knew it existed.

*Raised by Angle E. Trimmed:* the omission is deliberate and correct for an increment that
serves nothing ("not advertised by this increment"); `CompiledSources::new` enforces the
scheme rule per adapter, not per family, so the build is self-consistent.

### T9. `crates/resourcefs-sources/tests/github_facts_contract.rs:1574` — a core-parser budget placed in a sources HTTP-facts test binary

`immutable_reference_parse_budget` measures `PathReference::parse` — pure
`resourcefs-core`, no network — but lives 4,000 lines into the sources crate's facts
contract. `cargo test -p resourcefs-core` never exercises it; `scripts/ci-gates.py` selects
it by package and `--test github_facts_contract`, dragging reqwest/rustls/tokio into a
core-parser measurement; and its `incremental_heap` is read from `FACTS_PEAK_HEAP`, that
binary's own `#[global_allocator]` counter, so its heap figure is not comparable to the
sibling `reference_parse_budget` (`github_reference_contract.rs:339`) its own comment
calibrates against. The PR already created
`crates/resourcefs-core/tests/github_immutable_reference_contract.rs`; the budget belongs
there.

*Raised by Angles C and F. Trimmed:* placement and comparability, not correctness — ranked
below **F14**, the substantive defect in the same test.

### T10. `scripts/module_shape.py:301` — the C11 claim label is keyed off one hard-coded stage name

`claim = 'C11' if row['stage'] == IMMUTABLE_STAGE else 'C02'` means S2's and S3's stages
will report C02 for rows `plan.md` says are C11, unless someone widens the constant. The
label belongs on the ledger row, not in a module constant.

*Raised by Angle I. Trimmed:* cosmetic mislabelling with no wrong verdict.

### T11. `scripts/module-ledger.json:4` — `protected_parents` introduces a third pinned baseline

Three pinned baselines now answer the same question ("did an untouched body change?")
against three trees two days apart: `module_shape_base.BASELINE` = `fad4cf2c`,
`ledger['baseline']` = `7de8d147`, and this row = `635ab170`. `github/mod.rs` is censused
against the middle one via a hard-coded block at `module_shape.py:429-448`;
`github/facts.rs` against this one via a generic loop at `:449-471`; no check compares them.
The single row is for a file this PR does not modify, with `changes: {}`, so it proves
nothing today and exists for S2/S3 — by which time the baselines will have drifted further
apart.

*Cleanup:* fold the hard-coded `GITHUB` parent into `protected_parents` as an ordinary row
and give every protected parent one baseline source. *Raised by Angle I. Trimmed:*
altitude/maintainability with no wrong verdict today; **F15** took the altitude slot because
it names a one-line root-cause fix rather than a restructuring.

## Below the line — quality tier

Cleanup, not defects. All trimmed unless noted.

### Reuse (Angle F)

**Q1. `crates/resourcefs-core/src/reference/github.rs:17`** — `GithubCommitId::new` is a
second, independently spelled definition of "a Git commit SHA";
`crates/resourcefs-sources/src/github/facts/identity.rs:389 fn sha` already validates
40-lowercase-hex. Two spellings of one rule (`matches!(byte, b'0'..=b'9' | b'a'..=b'f')` vs
`b.is_ascii_digit() || (b'a'..=b'f').contains(&b)`); when GitHub's SHA-256 object format
lands, the facts validator could accept a `head_sha` the address type refuses. *Cleanup:*
have `sha()` validate through the newly-public `GithubCommitId::new`. Angle I noted
`GithubRepositoryIdentity` — the other half of the same identity — is already a core newtype
consumed by 13 files in `resourcefs-sources`, so the crate boundary is not the obstacle.
Rejected in the PR's record as S1-R2-F17.

**Q2. `crates/resourcefs-core/src/reference/github.rs:202`** — `parse_encoded_segment` plus
the re-encode in `GithubSourcePath::new` re-implements the codec
`jira.rs:369 parse_canonical_jira_segment` already packages. Three copies of "percent_decode,
then encode into a fresh String" (`jira.rs:359`, `jira.rs:369`, `github.rs:56`); github's
omits the `canonical != input` equality check both jira copies perform, which is why the two
families already answer "is `%7e` valid?" differently — jira refuses it as non-canonical,
github silently rewrites it to `~`, and tests on both sides lock the divergence in.
*Cleanup:* lift it into `reference.rs` beside `encode_rfc3986_segment`, exactly as this PR
lifted `encode_jira_segment`.

**Q3. `crates/resourcefs-core/src/reference/github.rs:215`** — `validate_decoded_segment` is
the third copy of the crate's single-path-segment rule; `LocalName::new`
(`reference.rs:228`) and `validate_jira_opaque_segment` (`jira.rs:110`) both already reject
empty / `.` / `..` / separator. Its NUL branch is dead on the parse path because
`percent_decode` (`reference.rs:1795`) already rejects NUL. Three validators, three
messages, three different extra rules — a hardening applied to the traversal rule lands in
one or two and misses the family that feeds a path at a Git tree.

**Q4. `crates/resourcefs-core/src/reference/github.rs:159`** — `parse_github_reference` is
the third hand-written copy of "split the trailing selector, parse it, then parse the
address" (`jira.rs:386-395`, `pull.rs:42-49`), in a third spelling (`map_or_else` with a
closure returning `Ok` vs the siblings' `match`). *Cleanup:* one
`split_projection(input) -> Result<(&str, Option<ProjectionSelector>)>` in `reference.rs`.

**Q5. `crates/resourcefs-core/src/reference/github.rs:170`** — the prefix strip duplicates
`parse_github_body` (`reference.rs:1275`), which was written parameterized on the prefix for
exactly this reason and emits the identical `"malformed GitHub reference"` string, now
literally duplicated in two files. *Cleanup:* extract the strip half so `github://` shares it
with `issue://` and `pr://`.

**Q6. `crates/resourcefs-core/src/reference/github.rs:66`** — the ceiling calls
`invalid_reference` where the module's `limit_exceeded()` (`reference.rs:2023`) names this
exact failure and is what `validate_reference_input` uses for the same constant. One
ceiling, two `ErrorCategory` answers, both pinned by the PR's own tests
(`github_immutable_reference_contract.rs:164` asserts `InvalidReference`, `:184` asserts
`LimitExceeded`). **Folded into F1**, whose failure scenario names the category split.

**Q7. `crates/resourcefs-sources/tests/github_facts_contract.rs:1554`** —
`immutable_reference_with_size` rebuilds the maximum-size `github://` reference and
re-asserts the over-ceiling `LimitExceeded` refusal already written in
`crates/resourcefs-core/tests/github_immutable_reference_contract.rs:170`. Two builders, two
arithmetic derivations; change the canonical spelling and one silently stops exercising the
boundary. Overlaps **T9**.

**Q8. `scripts/module_shape.py:204`** — the stage-roster validation writes the same four-line
`row['stage'] not in known_stages` block five times, and `Transition.__init__` (`:150-157`)
adds a third copy of the setdefault/update section loop. A sixth ledger section gets a fifth
copy — or gets forgotten, which is the exact silent skip the block's own comment says it
exists to prevent. `ledger['owners']` uses direct indexing while its two neighbours use
`.get(…, {})`, a difference carrying no meaning. *Cleanup:* one loop over a
`(section, label)` table. Also raised by Angle G.

> **Angle F explicitly cleared:** `github_family_unreadable()` is *not* triplicated (`read`
> and `search` share the helper; `resolve` is a different category inlined exactly like the
> Https and Jira arms above it) — only its name breaks the crate's `unsupported_*`
> convention. `canonical_reference()`'s `format!` is not meaningfully duplicated across
> families.

### Simplification (Angle G)

**Q9. `crates/resourcefs-core/src/reference/github.rs:116-133`** —
`GithubAddress::repository()`, `commit()` and `path()` have **zero call sites anywhere in
the workspace**, production or test (grepped and confirmed). 18 lines of or-pattern matching
no test exercises; a third variant forces you to extend two or-patterns nobody reads, and a
mistake there is uncaught. *Cleanup:* delete all three. This was the runner-up for the F15
slot.

**Q10. `crates/resourcefs-core/src/reference/github.rs:39`** — `canonical` is a pure
injective function of `segments`, yet all four derived traits walk both fields. Every
equality/hash on a path the type allows to reach 64 KiB does ~2x the byte work and stores up
to 4x the bytes; the derived `Ord` orders by decoded segments first and canonical second — a
different order than `as_str()` lexicographic — so the type ships two orderings and nothing
says which is intended. Angle H measured the storage half: a 1,024-segment parse does 1,049
allocations vs 18 for the same 64 KiB in one segment, retaining 220,069 bytes for a
65,536-byte input (3.36x = requested + canonical + 24,576 bytes of String headers + segment
bytes), all held alive by any long-lived `PathReference`. *Cleanup:* drop `PartialOrd, Ord`
(nothing orders this type and `GithubAddress` isn't `Ord`) and hand-delegate
`PartialEq`/`Hash` to `canonical` alone.

**Q11. `crates/resourcefs-core/src/reference/github.rs:176`** — collapse the
`if body.is_empty()` early return, the `splitn(5)` + let-else, and the follow-up
`match (kind, segments.next())` into one match over the five-tuple with a single
`_ => Err(...)`. The grammar is currently readable only by holding three constructs at once,
and spends four `invalid_reference` sites on two near-identical spellings no test
distinguishes; `if body.is_empty()` is already unreachable as a distinct outcome (an empty
body yields `Some("")` for owner and the let-else refuses it one line later with the same
category).

**Q12. `crates/resourcefs-core/src/reference.rs:982`** — the sixth verbatim copy of
`return Ok(Self { requested, address: X(a), projection, selector_candidate: None, selector_error: None, local_candidate: None })`.
Three `None` fields restated six times; change a default and the compiler says nothing. The
issue and pr branches have already diverged, inlining `canonical.clone()` + `format!` where
github and jira call `remote_requested`. *Cleanup:* one private
`fn remote(address, projection) -> Self` that all six branches call.

**Q13. `scripts/module_shape.py:434`** — `before_nodes` is assigned and never read; only
`after_nodes` is used at `:443`, and the other call site at `:469` discards both. Any linter
flags this (F841). *Cleanup:* `check_parent_nodes` should `return after_nodes`.

**Q14. `scripts/module_shape_base.py:549`** — delete `main()` and the `__main__` guard
entirely; the module is loaded only via importlib from `module_shape.py:33` and is never
executed (verified: no `.github/`, Makefile, justfile, `scripts/` or docs caller). Twelve
lines whose only behaviour is to raise, plus a subtle regression the diff introduced — the
guard was `raise SystemExit(main())` and is now `main()`, so the message escapes only
because `main` raises rather than returns; if anyone ever makes it return a message, the
script exits 0 silently. Angle I added that the dead CLI leaves nine unreachable
`transition is None` branches (lines 272, 280, 297, 345, 362, 411, 510, 524, 539) and four
dead `stage ==` branches (343, 435, 480, 490), plus a module docstring documenting
`--root`/`--repository` flags that no longer exist.

**Q15. Small notes below Angle G's own eight:**

- `scripts/module_shape.py:450` — `if row['stage'] not in known_stages: continue` is dead:
  `policy.active` is `stages[:i+1]`, always a subset of `known_stages`, and the next line
  already covers the case.
- `scripts/module_shape_base.py:278-282` — the `relocated_counts` comprehension re-tests
  `transition is not None` four lines after the `if transition is not None:` block that
  could just set the key inside the loop it already runs; line 297 re-tests it per
  declaration inside the hot loop, where it is loop-invariant.
- `crates/resourcefs-core/src/discovery.rs:387` — the added blank line after `};` is pure
  diff noise.

### Efficiency (Angle H — all figures measured, release build, counting allocator)

**Q16. `crates/resourcefs-core/src/reference/github.rs:147`** — `canonical_reference()`'s
`format!` has no capacity hint: **6 allocations / 196,732 bytes for a 64 KiB source
address**, 3x the output, because `format!` starts at capacity 48, grows to 65,530 for the
path, then doubles to 131,060 for the trailing `/facts`. That single call is 43% of the
458,906 bytes a whole parse allocates. Even an 85-byte realistic reference costs 5 allocs /
294 bytes / 562 ns. *Cleanup:* build once into `String::with_capacity(exact)` and store it in
`GithubAddress`, returning `&str`.

**Q17. `crates/resourcefs-core/src/reference.rs:1270`** — `remote_requested`'s
`projection.map_or(base.clone(), …)` evaluates the default eagerly, so every
projection-less parse copies the whole canonical reference and drops the original.
*Measured:* patching it to move instead of clone drops one 64 KiB parse from 506,843 to
441,307 bytes (exactly one full copy) and peak live heap from 416,665 to 351,129. The
85-byte case goes 18 allocs / 663 B → 17 / 578 B. The helper is on the jira/issue/pr
per-MCP-request parse paths too (`server.rs:540/617/696`, `server/read.rs:87`), so the fix
pays four times. Raised independently by Angle A. **Note:** deleting the clone alone would
permanently retain the 131,060-byte over-allocated buffer from Q16, so the two fixes go
together.

**Q18. `crates/resourcefs-core/src/discovery.rs:1260`** — the identity checks allocate a full
canonical reference purely to feed `==`: 6 allocations / 196,732 bytes for a single boolean
on a 64 KiB address; 5 allocs / 294 bytes / 562 ns on an 85-byte one. `canonical_identity`
runs once per discovery result row (`SearchRecord::new:321`, `GlobEntry::new:397`) and per
diagnostic (`:893`); `validate_canonical_identity` (`resource.rs:642`) runs on every
`SourceResource` construction. Latent for github (the family is refused), but the arm is
copied verbatim from the Issue/PullRequest arms that are hot now, and `canonical_identity`
then allocates a second copy at `:1283` via `requested().to_owned()`. *Cleanup:* an
allocation-free `matches_canonical(&str) -> bool`.

**Q19. `crates/resourcefs-core/src/reference.rs:1050`** — `validate_percent_encoding` and
`percent_decode` now scan the same bytes twice and hex-decode each `%XX` twice for every
`local://` and workspace reference (parsed once per MCP request at `server.rs:617` and
`server/read.rs:87`). The same back-to-back pair appears at `reference.rs:1282-1283`
(`parse_github_body`) and `jira.rs:370-371`, both on the identical slice with no work
between. Angle I framed the same thing as an ownership problem — the new comments at
`reference.rs:1046-1049` and `1602-1605` claim the prior pass "exists for that separator
rule" while lines 1752-1761 still re-implement the whole escape grammar, and the callee's new
`Err` branch is reachable from only 1 of 6 call sites (`github.rs:212`), so the
defence-in-depth is unreachable and untested for the other five. *Cleanup:* fold the
separator rule into `percent_decode` behind a flag and delete the pre-pass.

**Q20. `scripts/module_shape.py:471`** — the `protected_parents` loop costs two subprocesses
per row (`cat-file -e` + `git show`); the `cat-file -e` is redundant since the `git show` two
lines later fails identically. Halves the I/O. (See **F6** for the *correctness* consequence
of that same unguarded `git show`.)

### Altitude (Angle I)

**Q21. `crates/resourcefs-sources/src/compiled.rs:132`** — derive family servability from the
source registry that already builds the catalog (`catalog_metadata()` returns
`Vec<&dyn SourceCatalogMetadata>`, each declaring a `SourceCatalogEntry { scheme, .. }`, and
`compiled.rs:57-59` already states the rule the arms re-implement by hand) instead of
hard-coding three `github_family_unreadable()` arms in three trait impls. When S2 lands,
`.rfs-jrz7/review-decisions.md` F8 says only the **read** arm moves back to routing — the
search arm at `:343`, the mutation arm at `:172`, and the catalog / DESIGN.md / CONTEXT.md
rows (F10, deferred to P13/P14) are remembered by nothing, and the existing fence still
passes because it asserts the refusal. Every future family repeats all four edits. Strongest
altitude runner-up; lost the slot to **F15**, which names a one-line root-cause fix.

**Q22. `crates/resourcefs-core/src/reference.rs:982`** — replace the seventh copy-pasted
`starts_with(PREFIX)` branch with a family-dispatch table
(`const REMOTE_FAMILIES: &[RemoteFamily { prefix, parse }]` whose rows live in each child
module) plus a `ResourceAddress::canonical_reference()` / `kind()` method, so a new family
adds one row in its own file and zero lines to `reference.rs`. *Counted:* `parse` now has
**7** near-identical prefix branches; landing one family required editing **11** exhaustive
`ResourceAddress` matches (8 production, 3 test) plus a `lib.rs` re-export. The compiler
catches the production matches but not the semantic drift between them — `discovery.rs:1259`
and `resource.rs:642` are the same conjunct written twice per family. Overlaps **T4** (the
tripwire raise is a symptom of this).

**Q23. `scripts/module_shape_base.py:33`** — collapse the ceiling to one constant; it is
currently written twice in the same file. See **T4**.

**Q24. `scripts/module-ledger.json:411` (`relocated_symbols`)** — a JSON table that
runtime-patches a Python constant, making a fourth ownership table alongside `owners`,
`HISTORICAL_OWNERS` and `HISTORICAL_JIRA_TRANSPORT`, with a *narrower* uniqueness scope
(name-only, three directories) than the workspace-wide identical-body rule this same PR added
for `owners`. **F4**, **F5** and **F12** are its functional consequences.

### Conventions (Angle J)

**Q25. `.rivets/issues.jsonl` (rfs-jrz7 row)** — the ticket-store update rode in `5d941fb`,
the **first** of the PR's nine commits, and no later commit touches the store.
`docs/agents/issue-tracker.md:15` states verbatim: *"When a change spans several commits, the
ticket store rides with the final one."* `git log 635ab17..6e1c55c -- .rivets/issues.jsonl`
returns only `5d941fb`. The durable note it carries says "Full repository gate and
incremental review in progress before first checkpoint publication" — a state the eight
subsequent commits superseded. *Low confidence:* Angle J would not block on it, since the
note is a progress record rather than the closure the rule's rationale targets, and the
ticket is deliberately left `in_progress`.

> **Angle J's clean results, recorded so they are not re-litigated:** `cargo fmt --check`
> clean; `clippy -D warnings` exit 0; `module_shape.py` `C02 PASS`; the one new `#[ignore]`
> (`immutable_reference_parse_budget`) *is* registered in `scripts/ci-gates.py:18` and
> `BUDGETS - seen` is empty; all growth tripwires satisfied (`reference/github.rs` 225 vs max
> 450; `discovery.rs` 1347 vs 1367); no `[workspace.lints]`, `clippy.toml` or
> `#![deny(missing_docs)]` exists, so `expect_used` / `arithmetic_side_effects` /
> `missing_docs` break no configured lint, and the `.expect("writing to a String cannot fail")`
> is the verbatim relocated body of the pre-existing `encode_jira_segment`; `pub fn github`
> lacking a doc comment matches its four undocumented siblings; the live-smoke rule does not
> bite because this increment adds no network surface; the DESIGN.md update obligation is
> scoped verbatim to `ErrorCategory` and the `ErrorReason` / `LimitDetail` vocabulary, none of
> which changed; no `unwrap_or_default`, `.ok()`, `let _ =`, `unsafe`, `thiserror` or second
> error enum in the added lines. Only the root `AGENTS.md` governs — there is no `CLAUDE.md`
> anywhere and no per-directory doc.

## Folded duplicates

Twelve candidates were absorbed into a reported finding rather than dropped. Eight of the ten
angles independently found **F1**, which is the strongest signal in the run.

| Candidate | Raised by | Absorbed into |
|---|---|---|
| `github.rs:66` ceiling | A1, B4, C1, D1, E1, F6, G (note), J1 — 8 of 10 angles | **F1** |
| `github.rs:66` build-then-reject unbounded allocation | E5, H7 | **F13** |
| `github.rs:55` capacity hint | A4, C6, D2, G3, H6 | **F13** |
| `github.rs:159` projections accepted | B5, E2 | **F8** |
| `.rfs-jrz7/compare_grammar.py:37` vs `falsify_grammar.py:57-62` self-contradiction on `x/facts:raw` | Sweep S4 | **F8** |
| `github.rs:216` `%5C` | B1, E7 | **F11** |
| `github.rs:83` no segment-count bound | D8 | **F14** |
| `github_adapter_contract.rs:735` missing `resolve` / controls assertions | A8 | **F2** |
| `module_shape_base.py:311` relocated_counts scope | A7, D3 | **F5** |
| `module_shape.py:332` raw-vs-production | A3, D5 (partial), verifier C5 | **F7** |
| `module_shape.py:342` fingerprint narrowing / trivial-accessor false positives | B3, D5 (partial) | **F10** |
| `module_shape.py:341` measured 1,651x slowdown | H1 | **F10** |

Angle A's stated regression scenario for the folded `github_adapter_contract.rs:735` item —
moving the Github arm below `Issue | PullRequest` in `resolve` — is not actually possible,
since that would leave the match non-exhaustive and fail to compile. The omission itself is
real and is named inside **F2**'s failure scenario.
