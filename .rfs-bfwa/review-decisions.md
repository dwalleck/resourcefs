# PR10 review decisions and bounded repairs

Base assessed: `acd02a9b427d07b6139f30d217f685cfe0131a57`; repair base: `70c60638e59267166269da98863d29cc8be1d8ec` (only tracker storage differs).
User approval: "Okay, lets address all the issues following your recommendations". No push/merge authorization. Main owns final proof.
Evidence states below are direct source/contract checks; Verified may establish a narrower fact than the original review. No original reported probe is relabeled as a fresh run. F18 additionally has an independent executed recurrence check: ten pages of 100 records overcount by 5,499 bytes.

| finding-id | finding | reviewer | evidence-state | evidence | decision | fix | note |
|---|---|---|---|---|---|---|---|
| F1 | `read_item` never compares the observed comment id to the addressed one | PR10 automated review | Verified | Pinned source `sources/src/github/facts/comment.rs:182`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F5 | Comment links published verbatim with no origin confinement | PR10 automated review | Verified | Pinned source `sources/src/github/facts/comment.rs:152`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F6 | `validate_comment_ids` dropped; duplicates published as `complete` | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:351`; approved design/contract | Modify | R1; implemented; fresh proof below | Detect duplicate identities but preserve empty native actor strings. |
| F10 | Parent fetch's `cache_generation` discarded; no post-serialize re-check | PR10 automated review | Verified | Pinned source `sources/src/github/facts/comment.rs:267`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F22 | Parent `Presence` fields vanish without an `unavailableFacts` entry | PR10 automated review | Refuted | Pinned source `sources/src/github/facts/comment.rs:104`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Availability inventory is explicitly selected, not exhaustive (DESIGN.md:188). |
| F2 | `next` is unsigned; a tampered handle drives off-allowlist egress | PR10 automated review | Verified | Pinned source `sources/src/github/facts/continuation.rs:79`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F20 | `confine_next` ignores userinfo; reqwest appends `Authorization: Basic` | PR10 automated review | Verified | Pinned source `sources/src/github/fetch.rs:490`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F23 | Unsalted SHA-256 of the session token is published in the cursor | PR10 automated review | Verified | Pinned source `sources/src/github/facts/continuation.rs:60`; approved design/contract | Modify | R1; implemented; fresh proof below | Artifact token is public identity, not signing material; use independent private random key. |
| F3 | Admission measures compact JSON; emission is pretty | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:130`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F17 | 512-byte per-page reserve against a 16 KiB retained-header ceiling | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:386`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F18 | `array_bytes` double-counts commas and re-adds brackets per page | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:385`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F4 | Local-ceiling truncation names no continuation | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:382`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F9 | Two retryable break paths strand the page URL | PR10 automated review | Refuted | Pinned source `sources/src/github/facts/collection.rs:334`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Fetched malformed pages are explicitly nonresumable (DESIGN.md:206); retain that behavior. |
| F13 | Deterministic body-size breach yields a poison cursor | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:137`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F7 | One bad record discards every earlier verified page | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:361`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F8 | Deadline partial is rebuilt, then rejected by `finish_facts` | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:293`; approved design/contract | Modify | R1; implemented; fresh proof below | Actual deadline expiry remains a hard failure; documentation corrected, no acceptance bypass. |
| F11 | Resumed read publishes `complete` under the full-collection identity | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:279`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F14 | `maxAttempts: 1` makes both new families structurally unreadable | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:213`; approved design/contract | Modify | R1; implemented; fresh proof below | Shared parent-plus-child budget correctly requires two attempts; document, never exempt parent. |
| F15 | Catalog grammar dropped the PR listing and creation routes | PR10 automated review | Verified | Pinned source `sources/src/github/mod.rs:883`; approved design/contract | Modify | R1; implemented; fresh proof below | Restored repository listing and facts grammar; /new remains write-only, not an advertised readable resource. |
| F16 | Envelope reorder changes shipped facts bytes and its VersionTag | PR10 automated review | Verified | Pinned source `sources/src/github/facts.rs:343`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Order changes bytes, not JSON schema compatibility; tags include acquisition observations (DESIGN.md:194). |
| F19 | Two names for one ceiling; failure facts drop the `limit` detail | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:378`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F12 | New fixtures drop the session TempDirs before test bodies run | PR10 automated review | Verified | Pinned source `sources/tests/github_facts_contract.rs:1942`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F21 | Recovery loop conflates the source cursor with recovery pages | PR10 automated review | Verified | Pinned source `mcp/tests/stdio_live_smoke.rs:409`; approved design/contract | Modify | R2; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F24 | Shared fixture matcher weakened to path-only for every test | PR10 automated review | Verified | Pinned source `mcp/tests/support/profile_tls.rs:270`; approved design/contract | Accept | R2; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F25 | Pre-existing recovery fence relaxed 1024 → 4096 silently | PR10 automated review | Refuted | Pinned source `mcp/tests/stdio_mcp_contract.rs:3940`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Existing reconstruct_with_limits retains 1024-byte limits on every continuation (stdio_mcp_contract.rs:2582-2653,3953-3954). |
| F26 | Permanent placement gate forked to a ticket-named copy | PR10 automated review | Verified | Pinned source `scripts/ci-gates.py:138`; approved design/contract | Modify | R3; implemented; fresh proof below | Frozen dependency predates PR10; preserve inherited checks while moving executable machinery to permanent scripts. |
| F27 | Coverage state is a `&'static str`, not a typed sum | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:57`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F28 | `local_limit_error` is stringly-typed with a catch-all arm | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:156`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F29 | `SourceCursor::new` errors collapsed into a silent `Ok(None)` | PR10 automated review | Verified | Pinned source `sources/src/github/facts/continuation.rs:94`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F30 | Raw `u64` params replace the validated identifier newtypes | PR10 automated review | Verified | Pinned source `sources/src/github/facts/comment.rs:165`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F31 | Contract still says Facts do not exist for comments | PR10 automated review | Verified | Pinned source `DESIGN.md:177`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| F32 | Acquisition-control and self-URL identity claims are now false | PR10 automated review | Verified | Pinned source `docs/operating.md:186`; approved design/contract | Modify | R1; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| Q1 | The whole `CursorOwner`/`Envelope` module duplicates `atlassian/jira/cursor.rs` field-for-field | PR10 automated review | Refuted | Pinned source `sources/src/github/facts/continuation.rs:45`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Jira stores a native token with different ownership semantics; no shared unsigned cursor framework. Bounded encoding is included in R1. |
| Q2 | Duplicated error-detail wire projection drops typed detail | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:96`; approved design/contract | Modify | R1; implemented; fresh proof below | Preserve the complete typed error payload in the source wire projection; no source-to-MCP dependency or generic error framework. |
| Q3 | `serialized_len`'s private `Counter` is the fourth hand-rolled byte-accounting writer in the crate | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:118`; approved design/contract | Modify | R1; implemented; fresh proof below | One CappedWriter/serialize_facts implementation serves io::sink measurement and Vec emission with checked byte accounting. |
| Q4 | Repeated degrade-or-reject branches obscure retention policy | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:293`; approved design/contract | Modify | R1; implemented; fresh proof below | Centralized hard-refusal and fresh-read retry classification; malformed data and authority failures retain distinct policies. |
| Q5 | Separate measurement and publication envelope literals can drift | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:226`; approved design/contract | Modify | R1; implemented; fresh proof below | One build_facts constructor and shared serializer govern admission and emission; CandidatePage pairs records with provenance. |
| Q6 | Every record is projected through `comment::record` twice | PR10 automated review | Verified | Pinned source `sources/src/github/facts/collection.rs:411`; approved design/contract | Modify | R1; implemented; fresh proof below | Validate each record once, then borrow it for bounded page-level measurement and final projection; no serialized-record cache. |
| Q7 | Repeated per-record parent projection costs wire bytes | PR10 automated review | Verified | Pinned source `sources/src/github/facts/comment.rs:147`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Per-record parent is required C10; RawValue does not remove repeated wire bytes. |
| Q8 | PageResponse duplicates FetchedResponse storage and widens access | PR10 automated review | Verified | Pinned source `sources/src/github/fetch.rs:43`; approved design/contract | Modify | R1; implemented; fresh proof below | PageResponse wraps the complete FetchedResponse plus next, preserving the existing body/link access boundary. |
| Q9 | ParentLinks duplicates the existing Links shape | PR10 automated review | Verified | Pinned source `sources/src/github/facts/comment.rs:31`; approved design/contract | Modify | R1; implemented; fresh proof below | Reuse the existing common link projection instead of adding another parent-link shape. |
| Q10 | Continuation spelling is manually formatted and reparsed | PR10 automated review | Verified | Pinned source `sources/src/github/facts/continuation.rs:97`; approved design/contract | Accept | R1; implemented; fresh proof below | Use the typed continuation constructor and propagate non-limit errors. |
| Q11 | PR and Jira cursor grammar use similar pre-split logic | PR10 automated review | Verified | Pinned source `core/src/reference/pull.rs:27`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Jira offsets and PR cursor eligibility deliberately differ; no generic parser rewrite. |
| Q12 | Core owns the collection record ceiling without a core consumer | PR10 automated review | Verified | Pinned source `core/src/resource.rs:19`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Core record-ceiling ownership is expressly approved by C18/C19 and module ledger. |
| Q13 | CollectionRecords error-description fallback is unreachable from current caller limits | PR10 automated review | Verified | Pinned source `mcp/src/acquisition.rs:67`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Explicit exhaustive fallback is correct; no generic caller-field mapping. |
| Q14 | Live tests bypass the existing native_gh helper | PR10 automated review | Verified | Pinned source `sources/tests/github_live_smoke.rs:311`; approved design/contract | Accept | R2; implemented; fresh proof below | Apply the assessed root-cause repair, not broader speculative changes. |
| Q15 | Comment and collection fixtures duplicate router construction | PR10 automated review | Verified | Pinned source `sources/tests/github_facts_contract.rs:1913`; approved design/contract | Modify | R1; implemented; fresh proof below | Reuse one fixture router/source constructor and retain its scratch owner through each test. |
| Q16 | Parent-first acquisition was proposed for parallelization | PR10 automated review | Refuted | Pinned source `sources/src/github/facts/collection.rs:213`, `:322`, `facts.rs:437`; approved design/contract | Reject | N/A — no repair warranted by the assessed claim | Parent-first validation and intermediate generation checks are authority behavior, not presumed redundant work. |

## R1 — Facts authority, admission and publication
Finding IDs: all non-rejected source findings plus F12/F14/F15/F31/F32 and directly related quality repairs. Owning slices S2–S4; C3–C15/C17, amended in design.md. Caller inventory and exact command receipts are completed from writer reports before commit.
Paths: github/facts.rs; facts/{comment,identity,collection,continuation,pull}.rs; github/{mod,fetch}.rs; HTTP authority boundary; existing source contracts; manifests; DESIGN.md and operating.md. Existing owners remain; no new core dependency. Independent private source-session MAC key, typed identities/outcomes, one final-context serializer, one generation baseline, unchanged final deadline.
Expected proof: wrong ID/link/duplicate and wrong-parent precedence; tampered next/userinfo zero egress; exact-cap long-metadata partial with valid refused-page cursor; scoped tail; deadline/generation publication refusal; fixture retention and source budgets. Existing production budget limits retained: collection local release processing <=1s, existing memory limits unchanged. Cursor bounded by 64KiB reference ceiling.
Evidence disposition: previous behavioral PASS for changed validators/serializer/cursor/fixtures is invalidated. New targeted regressions, red/restored-green and production budgets required. Provider pagination/shape premise retained from original evidence because endpoints and native shape unchanged; live paths still rerun after assembly.
Checkpoint: PASS — source runtime, compilable mutations/restoration, production budgets and corrected complete repository gate.

### R1 fresh regression observations

- Pre-fix production `70c6063` with the new transport test only:
  `cargo test -p resourcefs-sources --test http_substrate_contract userinfo_url_is_refused_before_egress -- --exact --nocapture`
  fails at the required rejection: the real loopback server returned HTTP 200 for
  the credential-bearing URL. This is behavioral RED, not a simulated URL parser.
- Initial test compilation exposed a fixture API mistake (`requests()` returns a
  vector); corrected to `is_empty()`. That compiler failure is not regression proof.
- Initial redirect test used a default-port Location against a dynamic-port
  allowlist, so the old code rejected a different origin. Its category-only failure
  is not an adequate egress fence; the fixture must name the actual allowed port
  before that regression can supply RED/restoration evidence.

## R2 — MCP recovery, strict fixtures and live oracle reuse
Finding IDs: F21/F24/Q14 plus F15 consumer assertions. Owning S5/S6, C13/C16/C20. Paths: MCP stdio tests/live tests/profile TLS support and source github_live_smoke.rs.
Expected proof: inline and spilled partials reconstruct one document without source reacquisition; source cursor is traversed separately; wrong query target fails fixture; live helpers retain missing-gh behavior; existing 1KiB continuation fence remains.
Checkpoint: PASS — fresh recovery/fixture/live execution and corrected complete repository gate; core artifact engine unchanged.

## R3 — Permanent placement gate
Finding IDs: F26. Owning S0/C18 and established permanent-gate obligation. Paths: permanent scripts and ledger; ticket wrapper/duplicate ledger removed.
Expected proof: all transitive executable checker imports in permanent scripts; retained old/new policy; invalid stage usage error; old forbidden dependency/new unauthorized owner/protected-parent growth mutations red, restoration green.
Checkpoint: PASS — permanent-only old/final source execution, policy mutations/restoration, stage validation and corrected complete repository gate.

## Final integration
Main runs affected source/HTTP/core/MCP tests, named mutations and restoration, release production budgets, credential-gated GitHub adapter + real stdio live paths, python scripts/ci-gates.py, and isolated assembled design/security review. Exact command/source/results replace pending receipts before final completion. No production fixes committed before applicable proof passes.

## Requested test-suggestion recheck

The user requested a fresh read of the review during implementation. Its new
"Why the suite did not catch these" section (lines 706–801) adds five explicit
suggestions; they were not in the earlier 696-line copy. No finding IDs changed.

| Suggestion | Disposition | Required proof |
|---|---|---|
| Shared-fixture sibling differential test | Modify | Independently authored outcomes for shared wrong-ID, wrong-parent and duplicate-first-page invariants. Preserve deliberate facts differences for empty native fields and later partial retention; do not use the weaker human link validator as an oracle. |
| Whole-document truncation assertions | Accept with F9 correction | Assert state, scope, retained IDs, full typed failure/limit data and continuation presence/absence together; actually follow locally refused-page continuations. Fetched malformed data remains nonresumable. |
| At least 100 records at serialized ceilings | Accept | Multi-page nested pretty representation with long ETag, revalidation and availability metadata; exact/one-byte-lower boundaries, not just a wall-clock budget. |
| Every cursor envelope field tampered | Accept | Valid clone round-trip control plus independently changed version/source/API/resource/target/tag; numeric repository and userinfo targets acquire nothing. |
| Shipped-envelope golden bytes | Reject | F16 did not establish member-order compatibility. Version Tags identify exact observation-inclusive bytes; do not replace a rejected requirement with a new golden pin. |

The live continuation branch must also execute, not remain conditional on a small
fixture. Native evidence collected 2026-09-09: `gh api repos/rust-lang/rust/pulls/159628
--jq '{number,comments,review_comments,state}'` observed a closed PR with 122
conversation comments. This count is discovery evidence, not a permanent assertion.
`gh api --include 'repos/rust-lang/rust/issues/159628/comments?per_page=100&page=1'
--jq 'length'` returned HTTP 200, 100 records and the real next target
`https://api.github.com/repositories/724712/issues/159628/comments?per_page=100&page=2`.
The existing adapter and stdio live rows will use the two-attempt parent-plus-first-page
budget, require a continuation, recover each document independently, and consume that
provider-issued numeric-repository target against native page observations.

### R2 deterministic recovery/fixture proof (2026-09-09)

- `cargo test -p resourcefs-mcp --test stdio_mcp_contract github_comment_facts_ -- --nocapture`:
  three passing rows: existing full-document recovery, inline partial and spilled
  partial. The inline row was strengthened to call `recover_artifact_document` on
  the initial result itself, not bypass that helper with direct JSON parsing.
- Mutation: remove the helper's non-`artifact://` stop. The exact inline row
  fails with JSON `trailing characters`, proving two source segments were
  concatenated. Restore the stop: all three rows pass.
- Fixture protocol mutation: change only the initial native Link from page 2 to
  page 3 while the scripted tail remains page 2. The exact inline row fails with
  `not_found` / HTTP 404. With that wrong query retained, weaken the matcher to
  compare paths after stripping both queries: the row incorrectly passes.
  Restore both the exact matcher and original Link: all three rows pass again.
- These checks ran against pre-repair source production, isolating the MCP/test
  helper behavior. Final assembled-source rerun and credential-gated live proof
  remain required; none of the historical checkpoint PASS claims covers them.

## Fresh assembled repair receipts (2026-09-09)

### Runtime and suggested-test coverage

| Obligation | Executed evidence |
|---|---|
| Original-production reproduction | New source corpus on `70c6063`: 23 passed, 14 failed, 2 ignored. Compiler/fixture mistakes are excluded; schema-first failures are not claimed as isolated proof of later assertions. |
| Source regression corpus | `cargo test -p resourcefs-sources --test github_facts_contract -- --nocapture`: 39 passed, 2 ignored. |
| Shared-route identity | `human_and_facts_comment_routes_share_identity_guards_not_partial_policy` shares valid/wrong-ID/wrong-parent native records and duplicate pages across real human/facts routes. It explicitly preserves the later-duplicate difference: human refusal versus facts verified prefix, unknown coverage and no cursor. |
| Whole-document outcomes | Record/representation/response/aggregate/attempt stops assert retained IDs, scope, coverage, exact typed dimension/details and cursor semantics; malformed fetched pages assert no cursor. Safe record/representation/aggregate continuations are actually read. |
| Pretty-byte boundaries | Three private facts tests pass. The exact-cap row uses 100 nested records, Unicode/nulls, two provenance observations and long metadata, independently encodes the same literal document, compares exact bytes/hash, and refuses one byte less. The public metadata row uses actual two-page acquisition, 8 KiB ETag, 304 revalidation and availability. |
| Final-outcome rollback | `collection_final_failure_rolls_back_last_verified_page` derives the candidate ceiling from public JSON, admits two 100-record pages, observes a third-page failure, trims page 2 plus its provenance/availability, and actually resumes at 101 through 201. Cancellation likewise occurs on page 2, after page 1 is verified. |
| Cursor fields and public-key forgery | `continuation_is_session_and_authority_bound` checks clone success, foreign contexts, each envelope field and tag, numeric-repository/userinfo target tampering, and HMAC attempts using the actual public artifact token and its SHA-256 digest. Rejections acquire nothing. |
| Shipped member-order golden | Rejected as assessed under F16; no incidental byte-order pin added. Exact serialization/hash/cap behavior remains tested. |
| HTTP authority | Initial URL and same-allowlisted-port redirect userinfo rows pass. Native-Link userinfo is a hard permission refusal, not a soft later-page invalid-reference failure. Facts budgeted reads refuse redirects altogether. |
| MCP | Three `github_comment_facts_` rows and `catalog_advertises_github_repository_and_comment_facts_routes` pass. Inline helper execution is real, not bypassed by direct parsing. |
| Live branch execution | Adapter and real stdio rows each traversed 100 + 22 records at rust-lang/rust#159628 with independent `gh` observations. Exact commands and native target are in `evidence.md`. |

### Compilable mutation receipts

All source mutants compiled and reached a behavioral assertion; none is credited
for a compiler error. Mutations were individually restored before the next.

| Mutation | Selected test / observed failure |
|---|---|
| Bypass `hmac::verify` | `continuation_is_session_and_authority_bound`: foreign-session cursor returned a successful resource. |
| Emit/count compact JSON instead of pretty JSON | `github::facts::tests::final_facts_measured_bytes_and_hash_fit_cap_but_one_less_refuses`: independent pretty-byte equality failed. |
| Disable first-page and cross-page duplicate guard | `human_and_facts_comment_routes_share_identity_guards_not_partial_policy`: duplicate facts page returned success. |
| Treat per-response oversize as resumable | `collection_response_and_aggregate_limits_keep_distinct_full_details`: impossible fresh-read cursor appeared. |
| Disable terminal representation rollback | `collection_final_failure_rolls_back_last_verified_page`: expected fitting prefix became `limit_exceeded`. |
| Concatenate source cursors during artifact recovery | Exact inline MCP row: JSON `trailing characters`; restoration gives three passing recovery rows. |
| Wrong native next query, then path-only fixture matching | Exact matcher fails HTTP 404; stripping queries masks that error; restore matcher and Link gives three passing rows. |
| Forbidden `serde_json` dependency in core | Permanent checker rejects inherited C1 dependency violation. |
| New trait in collection owner | Permanent checker rejects unapproved generic seam. |
| New `github/extra.rs` owner | Permanent checker rejects path outside approved ledger. |
| Added responsibility inside protected HTTP body | Permanent checker rejects protected-body change. |

The restored source suite passed after MAC/serialization/duplicate/poison-cursor
mutations. The final repository gate owns the last rollback restoration and
final private/MCP reruns as well as the unchanged release budget limits.

### Permanent gate and independent reviews

`python scripts/module_shape.py --stage live` passes with one permanent ledger.
A disposable current-source copy with only `scripts/module_shape.py`,
`module_shape_base.py` and `module-ledger.json` also passes without frozen ticket
directories. Invalid explicit stage is usage exit 2; invalid ledger default is
input failure exit 1. All four structural mutations above were restored and
the clean disposable tree passed.

Final owner sizes after formatting: facts 627/700, identity 459/460, collection
767/850. `CandidatePage` pairs candidate records with their required provenance;
`WebCommentRoute` carries validated domain numbers. Clippy passes without lint
suppression; measurement and emission share one capped writer, not a new
generic cursor or error framework.

Independent security review found two actionable details, both corrected:
standard `/issues/N` web links carry identity, and native-Link userinfo must
remain a hard whole-read refusal. Its facts-redirect concern was withdrawn
because budgeted facts reads already refuse redirects. Independent gate review
found no retained-policy defect. Final admission review found no production
regression and identified the two test gaps now closed by the page-2
cancellation and actual terminal-rollback tests above. Reviewers ran no tests.

Full repository gate and local commit receipts are recorded below on completion.

Commit ordering is R3 gate migration, R1 source/contract/owned-policy repair, R2
consumers/live fixtures, then final receipts. The intermediate R3 policy retains
the old required symbols and the single `validate` identity entry point; its
permanent-only checker passed against archived `70c6063` source. R1 atomically
introduces the new required symbols and `validate_comment_links` entry point.
This is a verified migration checkpoint, not a compatibility shim or source
presence heuristic. The full working tree always retains the final strict policy.

Post-smoke cleanup removed the credential-in-memory live runner and both owned
disposable source/shape-policy trees. No production test or release budget was
removed. No standalone changelog exists; the source contract, operating guidance
and this per-finding repair ledger carry the behavior-change record.

### Complete-gate correction

The first full run completed in 1,557 seconds. Placement, formatting, every
ignored production budget and dependency vetting passed. It found two redundant
`as_bytes().len()` calls in the final rollback test and the HTTP library name
inside the new rejection assertion's message. The existing architecture fence
scans that token even in strings, so both workspace modes rejected the message;
no second client or dependency edge had been introduced.

Changed the assertions to use string byte length directly and describe refusal
before HTTP egress. No lint or architecture restriction was disabled. The full
all-target/all-feature Clippy command and all seven architecture tests then
passed. The corrected complete gate is rerunning.

R1 also carries the three-line existing MCP native-fixture correction: synthetic
comment IDs must update their recognizable HTML anchors. Recovery-helper and
new-consumer-test changes remain R2; this separates compatible commits without
shipping a knowingly false fixture alongside the stronger identity contract.

### Final verification

`python scripts/ci-gates.py` completed with **exit 0**, printing
`All repository gates passed.` The supervised run
`rfs-pr10-final-gate` took approximately 21 minutes 55 seconds.
Placement, formatting, all-target/all-feature Clippy, debug/release workspace
suites, all 14 classified ignored production budgets and cargo-deny passed.
This includes final rollback restoration, all private serialization guards,
the source/HTTP/core/MCP regressions and strict fixture recovery.

The R1 compatibility checkpoint also ran
`cargo test -p resourcefs-mcp --test stdio_mcp_contract github_comment_facts_recover_without_reacquisition -- --exact --nocapture`
with the original MCP test/support files plus only the three-line native HTML
anchor correction: **1 passed**. The fully verified R2 files were restored
byte-for-byte afterward. No intermediate commit needs a broken consumer fixture.

### Local commit receipts

| Commit | Checkpoint | Changed lines |
|---|---|---:|
| `2e3e615` | R3: permanent-only gate migration retaining the old source policy | 815 |
| `df5da90` | R1: source behavior, source regressions, contract, strict owner policy and required existing MCP fixture correction | 3,857 |
| `8a28dd0` | R2: recovery consumers, exact-target fixtures and live oracle paths | 862 |

Each behavior increment stays below the 4,000-line review boundary. This final
receipt commit contains only `evidence.md` and `review-decisions.md`.
All 48 findings are accounted for: 38 accepted/modified repairs and 10 explicit
rejections. All applicable proof is complete; no push, PR, merge or tracker
closure was performed. The primary checkout was not modified.
