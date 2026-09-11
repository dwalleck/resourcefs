# Design: rfs-jrz7 — immutable commit and exact source facts

## Route and inputs

**Status: approved by requester on 2026-09-09; proceeding to budgeted plan and checkpointed build.**

Empirical route; source baseline `635ab170ab57542c18272921298d575da2f8b08a`, branch `feat/rfs-jrz7`, worktree `../resourcefs-wt-rfs-jrz7`. The complete behavior set is **route.md, T4 rows 1–13**, adopted in full. spec.md: N/A — behavior is explicit in the ticket and that T4 set; exact spelling/placement is this design's responsibility. `evidence.md` and `probe.py`/`probe-result.json` supply P1: REST 2022-11-28 pinned commit, three complete nonrecursive trees and blob agree with local Git objects. P2 (future grammar) and P3 (implementation enforcement) are correctly classified as feature claims, not empirical evidence. rfs-0n97's existing schema/errors/controls are landed; the local tracker's older in_progress status is stale.

This is additive capability, not removal of a safety constraint. Existing repository/origin authority, cancellation, cache generation, output recovery, human projections and mutation grants remain authoritative. Exhaustive ResourceAddress matches must migrate; no wildcard fallbacks or compatibility aliases.

### Selected address and schema contract

- `github://<owner>/<repo>/commits/<commit>/facts`
- `github://<owner>/<repo>/source/<commit>/<path>/facts`

`GithubRepositoryIdentity` retains its existing canonical owner/repository policy. New core `GithubCommitId` is exactly 40 lowercase hexadecimal characters; branches, abbreviated IDs, uppercase SHA spellings, synthetic merge refs and unsupported object formats fail construction. This type identifies commit operands, not VersionTags. New `GithubSourcePath` stores validated decoded UTF-8 segments; it is not WorkspacePath and does not inherit platform filesystem normalization. New `GithubAddress` is a closed sum of Commit and Source, both carrying the repository and commit, Source additionally carrying the path; ResourceAddress gains Github.

Split structural slashes and remove the final `/facts` before segment decoding. Encoded source segments contain RFC3986 unreserved literal bytes or valid `%HH` escapes; canonical emission uses uppercase escapes. Decode once, require valid UTF-8, reject empty/`.`/`..`/NUL or decoded slash. Unicode, spaces, backslashes and representable control bytes are encoded, not normalized; literal reserved path characters must be escaped. A literal `%2F` filename uses `%252F`; a `facts` filename uses `/facts/facts`. SHA and owner/repository operands are never percent-decoded into new separators. Source selectors are not another content API: as with existing singular facts, projections on these facts fail unsupported_projection; real MCP display recovery uses artifacts.

Reuse the current owned envelope and `schemaVersion: {major: 1, minor: 0}`. New kinds are `github.commit` and `github.source`. Native IDs remain decimal strings and Presence preserves supplied null, absent and actual values. Consumer examples reject unknown majors while allowing additive fields. No public wire DTOs or generic fact schema framework.

Commit body:
- `request`: repository and `commitSha`.
- `observed`: validated `commitSha`, `treeSha`; envelope repository.observed contains independently acquired native repository identity.
- `data`: `sha`, `treeSha`, `parents` (SHA plus supplied links), `message`, native Git `author`/`committer` (name/email/date with supplied presence), optional/null GitHub `authorAccount`/`committerAccount`, and `links` (supplied API/HTML links). Preserve provider message exactly; it is not reconstructed raw Git commit bytes.
- `upstream`: bounded per-acquisition observations identifying repository and commit responses; keep body and revalidation observations separately, as the existing envelope does. Do not flatten unrelated responses into one ETag.

Source body:
- `request`: repository, `commitSha`, decoded `path`; canonical Resource remains escaped.
- `observed`: validated commit, root tree, containing tree and terminal object SHA/mode/type/path.
- `data`: `commitSha`, root `treeSha`, `containingTreeSha`, `path`, `mode`, `objectType`, `objectSha`, supplied `sizeBytes` when available, and a typed `content` sum.
- `content` Available serializes `state: "available"`, `encoding: "base64"`, measured `decodedSizeBytes`, and complete `bytesBase64`. Unavailable serializes `state: "unavailable"`, finite `reason` and applicable sanitized `failure`/`limit`, never bytes. Unsupported serializes `state: "unsupported"` and finite `reason`, never regular-file bytes. Reason vocabulary: `decoded_size_limit`, `acquisition_failed`, `non_regular_object`, `unsupported_object_mode`, `unrepresentable_path`. These are representation outcomes, not a second operational error enum.
- Submodule target remains `objectSha` with mode 160000/type commit; symlink remains mode 120000/type blob. Final directories are unsupported regular-file content but retain verified tree identity. Committed LFS pointer bytes are ordinary regular bytes, not classified or hydrated through LFS services.
- `upstream`: observations for each actually acquired repository/commit/tree/blob, bounded by the same attempt ceiling; attach them to their object IDs. Do not retain every complete response body merely to serialize provenance.
- No `collection` or source continuation for either singular resource. Internal tree fragments do not become independently usable partial source records. Retention is permitted only after the requested terminal entry has been verified.

### Acquisition and integrity

1. Validate the typed request, configured deployment and repository permission before any lookup. Acquire `/repos/<owner>/<repo>` through the existing controlled fetcher to obtain native repository identity, then `/repos/<owner>/<repo>/commits/<full-sha>`. The commit endpoint retains supplied GitHub accounts; Git Data commit endpoint alone would lose them. Validate repository returned full_name/ID and all supplied authority-bearing links, exact requested SHA and tree/parent identities. Never fetch returned links or substitute another repository.
2. Source reads walk nonrecursive `/git/trees/<tree-sha>` responses by exact decoded segment, using only selected immutable object IDs in URLs. No caller filename enters an HTTP path. Verify requested tree SHA, valid mode/type combinations, unique raw UTF-8 entry names and complete/non-truncated tree before selecting a child. Reconstruct the Git tree object serialization (Git byte ordering, tree mode 40000, NUL separators and binary object IDs) and check its SHA-1 using existing `aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY`; lossy path representation cannot silently validate as another tree. A malformed or contradictory tree rejects the read, even when an apparent requested entry appears early. Unknown Git modes have an explicit unsupported outcome only when the complete object identity can still be verified. Do not silently omit an entry to make a tree hash fit.
3. Intermediate entries must be directories. Never traverse a symlink or submodule. Terminal non-regular entries yield unsupported content with verified native metadata. A path absent from a complete verified tree is not_found; truncated/unverifiable trees cannot prove absence. A provider-unrepresentable path that is detectable is refused explicitly; a tree hash contradiction is an integrity failure, not guessed absence.
4. For a regular file whose supplied verified tree-entry size exceeds the effective decoded cap, return metadata-only decoded_size_limit and do not request its blob. The size is upstream metadata, not a claim that unseen bytes were locally counted. If size is absent, retain absence and attempt only bounded blob acquisition.
5. Otherwise GET `/git/blobs/<blob-sha>` through the same controlled fetcher. Validate exact SHA, encoding, base64 alphabet/padding and independently calculated decoded length before allocating a decoded buffer. Require decoded length to match supplied blob/tree sizes, and hash accepted bytes using Git's `blob <length>\0` header to compare the object ID. No lossy text conversion; produce canonical base64 of complete accepted bytes. 4 MiB and 4 MiB+1 have equal encoded lengths, so encoded-length comparison alone is never the control. Oversized content has no accepted partial bytes.
6. Ordinary blob-stage acquisition failure after verified terminal metadata yields content unavailable with sanitized failure facts if the honest final document fits; cancellation, repository/identity/link violations or contradictory blob identity invalidate the entire current resource. Earlier repository or parent-tree observations alone are not a usable source result. Representation failure is limit_exceeded, not truncated JSON or fake recovery.
7. One FactsRead/BoundedRead/HttpReadBudget covers repository lookup, commit, every tree, blob, retries, waits and revalidation. With no retries a path with d segments costs d+3 requests (repository, commit, d trees, blob); therefore the 10-attempt ceiling can refuse deep paths. This is explicit bounded behavior, not an unlimited tree traversal promise. A metadata-only oversized/non-regular result omits blob acquisition. Each tree is bounded by existing response/body/deadline limits and at most 1,000 entries; no pagination/fallback disguises tree truncation or entry cap.
8. Extend existing ReadAcquisitionLimits with positive lower-only `max_decoded_bytes` (4,194,304 hard default), MCP/profile `maxDecodedBytes`, and AcquisitionLimitKind::DecodedContentBytes (`decoded_content_bytes`). Extend existing constructor, callers and schemas atomically; no second controls object or compatibility constructor. Existing families do not decode regular-file content and must not acquire a new hidden work path. Preserve their established output contract: the added effective field is present where the new source family uses it, not a gratuitous rewrite of human output. No new ErrorCategory or ErrorReason is required. Update DESIGN.md's bounded vocabulary in the same implementation commit.
9. Reuse capped serialization with exact pretty-JSON byte accounting, per-response provenance and honest outcome included. Check cache generation throughout the chain and again **after** serialization; check operation/deadline acceptance before returning. Failed revalidation never becomes stale success. Origin-bound credential handling, principal-scoped sessions and all-false read mutation grants remain unchanged.

## Input shapes

| Shape | Covered by |
|---|---|
| Commit/Source address variants; existing Catalog/Workspace/Artifact/Local/Https/Issue/PullRequest/Jira variants | C2, C10 |
| Empty/short/uppercase/nonhex/full commit; branch or synthetic-ref spelling; authorized/denied/renamed/fork repository | C2, C3, C7 |
| Empty/root/nested paths; ASCII/Unicode/space/reserved characters/backslash/control byte/facts filename; percent nesting, malformed escapes, invalid UTF-8, encoded slash/NUL/dot traversal; absent/present selector | C1, C2, C4 |
| Commit parent lists empty/one/multiple/duplicate; native account and author/time/message/link properties missing/null/value; zero/negative/malformed native IDs or wrong JSON type | C3, C9 |
| Empty/single/multiple trees; duplicate names with identical or different IDs, file/directory sort collision, missing or false/true truncation, nonrepresentable name, unknown mode, contradictory mode/type/ID/link | C4, C7 |
| Regular 100644/executable 100755; tree 040000; symlink 120000; submodule 160000; unsupported mode; intermediate non-directory | C4, C5 |
| Empty/text/binary/non-UTF-8/CRLF/mixed-newline/LFS-pointer blob; valid/malformed base64; absent/null/negative/contradictory size; wrong object identity | C5, C7 |
| Decoded zero/exact-cap/cap+1; equal encoded lengths; lower caller/operator cap; zero/negative/above-hard/wrong-type controls | C5, C6 |
| Attempts/deadline/body/accepted-body/representation exact and over; deep path and wide tree; allocation under representative maximum shape | C6 |
| First lookup/commit/tree failure; verified entry then ordinary blob failure; cancellation or integrity violation at every stage and after serialization | C7, C8 |
| Fresh/cached/304/failed revalidation; generation changes between responses and during serialization; principal change with new session | C8 |
| Per-response optional headers, unknown upstream values, acquisition clock interval; identical Git ID with changed representation provenance | C9 |
| Direct public library, configured CLI check/catalog, real MCP compact/overflow/artifact recovery; live gates absent/present | C10 |
| Production module placement, visibility and protected-parent growth | C11 |
| PR enumeration/comparison collections and their continuations | N/A — separate verified tickets rfs-iktl and rfs-e5cv; singular paths add no collection cursor |
| Corporate credentials/native Windows/Tauri/capture storage | N/A — outside ResourceFS ownership; not acceptance claimed by this issue |

## Placement

| Capability | Owner | New seam | Forbidden |
|---|---|---|---|
| Validated commit/path/address grammar | core reference/github.rs, integrated by reference.rs | Concrete typed family at existing PathReference seam | Git object acquisition/hash/JSON or platform path normalization in core |
| Decoded acquisition dimension | existing core acquisition.rs/error.rs and existing MCP input/profile adapters | Extend existing control interface; no new trait | Parallel limit/error taxonomy or silent clamping |
| Native commit acquisition and facts | sources github/facts/commit.rs | Private acquire returning verified commit plus observations; private read at existing SourceAdapter seam | Alternate HTTP client, branch/ref fallback, wire DTO exports |
| Git tree/blob validation | sources github/facts/object.rs | Private concrete validated object types/helpers consumed by source.rs | HTTP, sessions, envelope serialization, generic provider trait |
| Source acquisition and facts | sources github/facts/source.rs | Private read using FactsRead, commit acquire and object validator | LFS/raw-download/contents fallback; traversal in parent facade |
| Dispatch/envelope/transport/catalog | existing github/mod.rs, facts.rs, fetch.rs and catalog interface | Existing interfaces; dispatch and context generalization only | New object responsibility bodies in parent/transport |
| Tool/profile/recovery integration | existing MCP acquisition/profile/catalog/read owners | Existing five tools and SourceResource | Sixth tool, inspector, duplicated recovery or Cyril client |

## Module shape

### Current cluster and baseline inventory

Physical line counts are concentration signals, not architecture proof; baseline is the pinned source above. Existing source-neutral and integration modules remain the owners of their current responsibility. Counts below were measured with wc -l; code/consumer inventory was independently mapped by read-only scouts. Before exported-symbol edits, LSP references and the compiler must enumerate every migration caller.

| Module/path | Baseline lines | Interface and callers | Clusters/dependency direction | Change |
|---|---:|---|---|---|
| core/src/reference.rs | 2000 | PathReference/ResourceAddress used by engines/adapters/MCP; public parser contracts | Syntax, canonicalization, typed dispatch; no provider I/O | retain protected parent; add family declaration/arm and reuse the existing lexical primitives; hoist the exact Jira RFC3986 encoder here for both children rather than copy it |
| core/src/reference/github.rs | 0 | GithubAddress/CommitId/SourcePath through public PathReference | New segment grammar/validated identifiers; core primitives only | create |
| core/src/reference/jira.rs | 454 | Jira grammar using shared lexical helpers | Existing provider syntax; no Git grammar | retain; import the relocated encoder without changing Jira encoding behavior |
| core/src/resource.rs | 720 | SourceResource and canonical identity validation | Exhaustive typed identity validation, no HTTP | retain; new match arm only |
| core/src/acquisition.rs | 132 | ReadAcquisitionLimits constructor/getters/intersection, all source/MCP callers | Validated source-neutral dimensions | deepen existing responsibility |
| core/src/error.rs | 293 | ResourceErrorDetails/AcquisitionLimitKind | Finite stable sanitized operational facts | deepen existing dimension vocabulary |
| sources/src/github/mod.rs | 1170 | GithubSource at SourceAdapter; library/MCP/canonical catalog tests | Dispatch, authority, catalog and established human/mutation behavior | retain protected parent |
| sources/src/github/facts.rs | 638 | Internal facts dispatch/context/envelope/serialization | Shared acquisition context, presence/native-ID/schema primitives | retain protected parent; generalize typed request dispatch only |
| sources/src/github/facts/identity.rs | 590 | Existing private identity/link helpers | Native PR/repository/link validation | retain; reuse finite helpers without object logic |
| sources/src/github/facts/collection.rs | 771 | Existing FailureFacts/failure_facts/rejects_every_page policy reused by source.rs | Sanitized subordinate acquisition failure classification | retain existing owner; widen internal visibility only, never copy classifiers |
| sources/src/github/fetch.rs | 628 | fetch_controlled used by facts and legacy reads | HTTP/cache/response/status behavior | retain unchanged |
| sources/src/configuration/github.rs | 262 | GithubConfig/GithubDeployment | Source identity, repository policy, lower limits | retain unchanged interface except extended limits type |
| sources/src/github/facts/commit.rs | 0 | Private acquire/read; callers Source and fact dispatch | Repository/commit native identity, presence and projection | create |
| sources/src/github/facts/object.rs | 0 | Private verified tree/blob decode; caller source.rs | Strict provider validation using gix-object/gix-hash for Git representation/order/modes/hashing and existing base64 for content | create |
| sources/src/github/facts/source.rs | 0 | Private read; caller fact dispatch | Bounded selected-path acquisition, outcome retention and source projection | create |
| core/src/lib.rs | 83 | Public domain re-exports | Existing public crate surface | retain; export validated address identifiers |
| core/src/read.rs | 369 | ReadRequest and engine projection handling | Typed source dispatch; public read contract | retain; Github match integration |
| core/src/discovery.rs | 1341 | Source-kind and discovery record identity | Typed identity/discovery, no object acquisition | retain; classify Github records without inventing commit discovery |
| sources/src/compiled.rs | 489 | CompiledSources read/resolve/search | Routes configured adapters | retain; Github dispatch and deliberate unsupported discovery/search where not provided |
| sources/src/github/mutation.rs | 1319 | Existing mutation target classification | Explicit mutation grants and operations | retain; reject immutable Github mutation targets |
| sources/src/http/read.rs | 349 | begin_read_with_limits, BoundedRead/HttpReadBudget | Source-neutral transport bounds | retain; pass the sixth validated constructor dimension through unchanged, no decoded-content work here |
| mcp/src/acquisition.rs | 151 | Closed AcquisitionInput DTO, visitor, schema, into_limits and rejection mapping | Shared tool/profile validation adapter; no provider dependencies | deepen all four coordinated field/visitor/conversion/error-name spots |
| mcp/src/server/read.rs | 154 | ReadInput and real dispatch | Protocol validation and existing engine invocation | retain; existing DTO consumption, no Git logic |
| mcp/src/profile/model.rs | 1832 | GithubSourceProfile embeds AcquisitionInput | Profile configuration representation | retain existing embedding, no duplicate limit fields |
| mcp/src/profile/schema.rs | 49 | Generated profile_schema_json consumed by CLI | Existing schema generation | retain; regenerate published schema fixture through real CLI |
| scripts/module_shape.py | 432 | Existing non-production placement gate | Ledger-driven path/symbol/dependency/fingerprint checks | deepen existing gate for C11, no parallel policy convention |
| scripts/module-ledger.json | 416 | Existing owner/cap/fingerprint inventory | Approved production owner manifest | add four owners and reviewed dispatch deltas, preserve other families |
| scripts/ci-gates.py | 160 | Repository-owned complete gate | Gate roster and ignored-test budget classification | register applicable new production-budget rows, not live rows |

The core/source/mcp shorthand above expands to `crates/resourcefs-core`, `crates/resourcefs-sources`, and `crates/resourcefs-mcp`. Schema fixture is `crates/resourcefs-mcp/tests/fixtures/server-profile-v1.schema.json`; its contract remains generated-schema compatibility, not incidental prose. Primary prior-art test surfaces are core `tests/acquisition_contract.rs`/`tests/github_reference_contract.rs`, sources `tests/github_facts_contract.rs`/`tests/github_adapter_contract.rs`/`tests/github_live_smoke.rs`, and MCP schema/profile/stdio contracts plus `tests/stdio_live_smoke.rs`. Existing constructors in test fixtures migrate with the public constructor; production `profile/convert.rs` consumes the unchanged AcquisitionInput conversion seam and needs no duplicate mapping.

Paths in this inventory use `crates/resourcefs-` before core/sources. Requester-approved dependency amendment: gix-object0.64.1 and gix-hash0.26.2, default features disabled and SHA-1 enabled, own Git object format and hashing; existing base64 supplies content decoding. Construct recognized EntryKind modes explicitly so REST040000 becomes native40000; never authorize a mode through lossy kind() classification. Existing aws-lc-rs remains for its existing consumers. SHA-1 serves provider Git identity compatibility, not a new collision-resistant security promise. VersionTag stays SHA-256 of representation bytes. The independent oracle remains Git plumbing, never the chosen crate.

### Alternatives and seam tests

A. **One private immutable module**: `immutable::read(address, ctx)` hides commit/source/object handling together. Caller is one dispatch call; no generic adapter. Lowest interface count, but native commit schema, source outcome handling and Git object hashing change for different reasons in one growing file. Rejected for locality/concentration, not merely line count.

B. **Selected: commit family + source family + pure object validator**: dispatch calls `commit::read` or `source::read`; source reuses `commit::acquire` and calls object validators. Native commit validation/projected presence has one owner; source traversal/retention has one owner; byte-level Git serialization/identity has one owner. More private interfaces than A, but each hides an independent responsibility and is directly exercised through public SourceAdapter reads. Deleting any of these modules moves actual identity/schema/traversal complexity into its caller; none merely forwards.

C. **Generic Git object adapter with transport trait**: callers provide a repository/object backend and ask `GitObjects::read(commit,path)`. Can support local Git and GitHub adapters, but only GitHub is production; local Git is an independent oracle, not a requested runtime provider. Reject the hypothetical seam, extra caller knowledge and risk of moving provider authorization away from existing GithubSource.

All external callers/tests cross existing PathReference/SourceAdapter/SourceResource seams. Internal object decoding needs no injectable trait. One owner per responsibility and primary public contract test family preserve locality. Existing HTTP adapter is reused unchanged; core remains provider-I/O-free. Core grammar has its intrinsic syntax ownership, not a pretend second provider adapter.

### Proposed module ledger and protected parents

The inventory plus Placement is the proposed ledger; approval applies to both. All new family interfaces are private except validated core domain identifiers/address variants. No exported SourceAdapter method or SourceResource representation type is added. Tests must exercise public parser/source/read interfaces; the shape oracle is non-production and is not a behavioral test inspecting source text.

| Protected parent | Baseline responsibilities | Allowed change | Forbidden change | Exit condition |
|---|---|---|---|---|
| core/reference.rs | Existing grammar and dispatch | declaration/re-export, Github match/parse delegation | Segment decoder, Git object types/logic | New grammar body resides in reference/github.rs |
| core/resource.rs | Canonical SourceResource identity | Exhaustive Github identity arm | Object acquisition/decoding | Identity arm delegates typed canonical reference |
| sources/github/mod.rs | Source dispatch/authority/catalog/human reads | Github routing, catalog examples | Commit/tree/blob visitors/traversal/base64 | No new object responsibility body |
| sources/github/facts.rs | Envelope/context/serialization/family dispatch | Shared request sum/context init, source-specific effective limit reporting, family calls | Git mode/hash/tree/blob or source outcome bodies | Family logic only in named children; existing caps respected |
| sources/github/fetch.rs and HTTP substrate | Transport/cache/budget | Existing behavior retained; http/read.rs forwards new validated decoded dimension when constructing effective limits | Git object/domain imports or decoding in transport | No object-domain change; decoded cap survives substrate intersection |
| MCP tool/profile facades | Existing mapping/validation/tool dispatch | Acquisition field/schema and registered scheme integration | Git traversal/JSON fact construction | Only existing seam consumption |

C11 extends the existing `scripts/module_shape.py` and `scripts/module-ledger.json`, not a second issue-local placement policy. This is the existing-seam exception to the workflow's preferred issue-local oracle. The non-production gate compares the tree/diff against this ledger, discovers default/upstream branch, requires the four new owner paths, restricts core imports, rejects object symbols in protected parents and enforces private source-family interfaces while preserving existing owner caps/fingerprints. Add an immutable stage in the existing stage mechanism if needed; no permanent gate is disabled to admit the change. Plan owns numerical growth tripwires and exact permitted parent deltas; unchanged-ownership estimate overruns return to plan, responsibility/interface changes return to design approval. MCP acquisition.rs currently has a 200-line cap and facts.rs a 700-line cap; use existing helper/visitor conventions rather than allowing these parents to absorb new clusters. Final independent review reconstructs production shape in a fresh context before seeing this ledger, then compares and resolves every mismatch.

## Claims

- C1: The proposed encoded segment grammar can name all enumerated valid path identities without structural ambiguity.
- C2: The production typed parser accepts only the approved immutable routes and preserves canonical path identity.
- C3: Commit facts preserve native metadata while validating requested repository/commit/tree/parent identities.
- C4: Source selection follows only a complete validated immutable tree chain to the exact requested entry.
- C5: Content outcomes preserve Git meaning and return only complete verified regular bytes within the decoded ceiling.
- C6: Every acquisition and final representation obeys the existing shared limits plus the lower-only decoded dimension.
- C7: Authority/identity/cancellation failures override retention while ordinary post-entry blob failure preserves only verified metadata.
- C8: Cache and final acceptance cannot publish stale or cancelled success after revalidation/generation changes.
- C9: Owned schema and per-response provenance preserve absent/null/value knowledge separately from Git and representation identity.
- C10: Both Resources work through public library and configured real MCP recovery without changing existing human reads or adding tools.
- C11: Production responsibilities and interfaces remain within the approved module ledger.

## Falsification

Each implementation row is discharged by Main at the checkpointed-build slice implementing it; plan assigns those slices. All fixtures use independent Git plumbing/hand-specified expected identity and a request-observing upstream, not echo assertions. Absence assertions always include a positive permitted-request/control run. A matching error category alone never identifies the guard: check request sequence, error reason/details and controlled input reaching that stage.

| # | Claim / shapes | Falsifier and alternate-cause control | Independent oracle | Named mutation and expected red | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|
| C1 | Grammar feasibility; encoded paths/selectors | Run the standalone grammar witness; any unequal roundtrip or accepted refused spelling falsifies it. Positive valid paths prevent an always-reject decoder passing. | Python standard URI encoder plus hand-authored decoded identities; scanner decoder is separate | In falsify_grammar.py replace return result with return result.replace("%2F", "/"); literal-percent path reports C1 roundtrip failure | Retain falsify_grammar.py as design oracle; production fence is C2 | milliseconds | PASS — run log below |
| C2 | Typed grammar and canonical identity | Public parser exact roundtrips/refusal matrix and canonical SourceResource acceptance; HTTP listener confirms encoded filenames never become route operands. Ordinary valid path reaches acquisition as positive control. | Fixed expected Unicode/byte identities and observed request paths, independent of core decoder | reference/github.rs: decode an already-decoded segment a second time; `%252F` source must fail exact identity assertion | core/tests/github_immutable_reference_contract.rs | focused parser run | PENDING — Main, checkpointed-build grammar slice |
| C3 | Native immutable commit facts | Serve distinct requested/branch-tip/synthetic/fork IDs and presence matrix, assert exact native values and typed mismatch failures. Correct requested commit succeeds with same optional-field shape so missing metadata cannot explain rejection. | Git commit fixtures authored via git plumbing; explicit native account/presence table | facts/commit.rs: omit requested SHA equality check; wrong-SHA response becomes success and fence turns red | sources/tests/github_immutable_contract.rs commit cases | focused loopback run | PENDING — Main, checkpointed-build commit slice |
| C4 | Complete tree chain and exact selection | Nested trees, Git ordering, duplicates, truncation, lost path bytes and non-directory intermediate shapes; compare selected entry and request chain. Correct complete tree containing same target succeeds, distinguishing wrong selection from fixture absence. | git ls-tree/cat-file/mktree object database and independent request observer | facts/object.rs: skip computed tree SHA comparison; lossy-path tree with plausible claimed SHA wrongly succeeds | sources/tests/github_immutable_contract.rs tree cases | bounded Git/loopback run | PENDING — Main, checkpointed-build source slice |
| C5 | Exact bytes/nonregular/oversized outcomes | Roundtrip empty/binary/newline/LFS and exact-cap/cap+1; assert matching mode/type, no bytes for unsupported/unavailable, no forbidden GET after positive ordinary-blob control. For the independent decoded-bound pair omit optional tree/blob size metadata so no earlier size guard can explain refusal; both complete base64 bodies fit the response limit and have equal encoded lengths. Separately test reported-oversized metadata skips the blob. | Git hash-object/cat-file plus binary byte corpus; arithmetic decoded lengths independent of encoder | facts/object.rs: use base64 encoded length threshold instead of decoded length; the size-absent cap+1 equal-encoded-length response wrongly yields available bytes and reports C5 | sources/tests/github_immutable_contract.rs content cases | boundary buffers up to 8 MiB | PENDING — Main, checkpointed-build source slice |
| C6 | Controls/shared budgets/final bytes | Exact/over lower caller/operator, attempts including repo/parents/retries, deadline across send/body/wait, tree-entry cap, response/cumulative/pretty-JSON bounds; measure allocations on same public path and record actual high-water, without claiming caps are RSS guarantees. Controls allow identical below-bound scenario. | Independent server attempts/timestamps, counting allocator and serialized byte counter outside production admission code | facts/source.rs: give each tree a fresh HttpReadBudget; a deep path's independently counted eleventh request turns fence red | sources/tests/github_immutable_budget_contract.rs, runner inventory registration | bounded maximum-shape measurements | PENDING — Main, checkpointed-build budget slice |
| C7 | Retention and integrity priority | Ordinary blob transport failure returns verified metadata only; early failure errors; denied repo/origin/link, changed SHA and cancellation at each hop yield no Resource. Listener control proves allowed egress before denied path tests. | Independently logged methods/URLs and controlled cancellation trigger; fixed expected metadata | facts/source.rs: classify identity failure as ordinary content-unavailable after verified entry; wrong blob SHA returns facts and fence turns red | sources/tests/github_immutable_contract.rs safety/outcome cases | loopback fault matrix | PENDING — Main, checkpointed-build source slice |
| C8 | Cache/final generation acceptance | Fresh/304/error and generation change after serialization with no stale publication; successful revalidation control rules out cache disabled/fixture failure. Principal-switched session is separately constructed. | Server response sequence and independent operation/session controller | facts/source.rs: remove final post-serialization generation check; controlled serialization-race resource wrongly returns | sources/tests/github_immutable_contract.rs lifecycle cases | deterministic race run | PENDING — Main, checkpointed-build lifecycle slice |
| C9 | Versioned schema/provenance/presence | Missing/null/value native fields, distinct per-response headers and times, representation SHA-256; unsupported-major consumer rejection. Change only observed metadata and confirm representation tag changes without changing Git ID. | Hand-authored expected facts and separate SHA-256 over returned bytes | facts/commit.rs: serialize absent native authorAccount as null; expected absent-vs-null consumer assertions fail | sources/tests/github_immutable_contract.rs schema cases | focused consumer run | PENDING — Main, checkpointed-build commit/source slices |
| C10 | Full library/config/real stdio/recovery and legacy | Real rfs check/catalog then read commit/source, force small display and reconstruct artifacts byte-exact before parsing; observe recovery causes no new upstream requests after an initial positive request. Live public commit/source smokes at exact revision; gates absent skip not pass. Existing human native diff exact-byte/truncated-body checks remain. | Independent stdio client, upstream request log and Git bytes; consumer rejects unknown schema majors | sources/src/compiled.rs: route ResourceAddress::Github reads to unsupported_projection instead of GithubSource; configured stdio read fails while direct adapter positive control succeeds | existing MCP stdio fact contracts/live files plus adapter live rows, extended with new paths | real binary + credential-gated public live runs | PENDING — Main, checkpointed-build integration/live slice |
| C11 | Module shape | Existing standalone path/import/symbol/visibility/diff census extended for new owners; deliberate forbidden placement identifies exact owner and C11 rather than broad size failure, with valid child placement as positive control | Non-production source/diff ledger oracle independent of behavioral implementation | Add Git tree decode helper to github/facts.rs instead of object.rs; gate reports C11 protected-parent responsibility violation | scripts/module_shape.py with scripts/module-ledger.json plus existing architecture gates | source census | PENDING — Main, checkpointed-build every affected slice and fresh-context final review |

C1 PASS is design feasibility only; C2 is the independent production parser obligation. No implementation, live or final-gate row inherits C1/P1's PASS. Mutation/restoration evidence is required at the applicable checkpoints, with localized negative controls; no fence-risk waiver requested.

## Non-goals and future work

Permanent non-goals: new tools/provider traits/HTTP clients, branch-tip or synthetic-merge substitution, symlink/submodule traversal, LFS hydration, chunked blob acquisition, binary transport redesign, output-recovery reinvention, lossy paths/bytes, remote writes and corporate credential/native/UI/storage acceptance. These violate the ownership or exact-evidence contract; no speculative follow-up tickets.

Separate verified intended families: rfs-iktl owns PR commit/native-file enumeration; rfs-e5cv owns direct-tree comparison and old/new navigation. This issue implements the immutable resources they can consume, not their collection semantics. Existing PR collection retention/cursors stay unchanged and remain covered by their established tests. No tracker closure or publication of those families is authorized here.

## Falsifier run log

- `python .rfs-jrz7/falsify_grammar.py`, isolated worktree, 2026-09-09: exit0; C1 PASS —9 exact roundtrips,19 refusals, encoded-colon/structural-selector distinction. Full output:falsify-grammar-result.txt. This was pre-implementation feasibility evidence; S1 production/mutation evidence is recorded separately in plan.md.
- Empirical P1 command/result is owned exclusively by evidence.md; retain its pointer rather than duplicating its changing record here.
- Self-review: complete T4 behavior pointer and integration inventory; all reachable shapes mapped; every claim has falsifier/oracle/mutation/fence/cost/status; cheapest C1 ran; no FAIL; every remaining row names Main/checkpointed-build; no approved-risk N/A rows. Existing module gate is reused rather than introducing a competing placement policy.

## Approval

Requester approval: **"Approved"**, 2026-09-09, in direct response to this design and its grammar, maxDecodedBytes control, module ledger and stated bounded-traversal tradeoff.

Approval scope: address/schema spellings; lower-only maxDecodedBytes dimension; complete-tree/hash validation and bounded depth/width consequences; module ledger/protected parents and all falsifiers. Approved risk acceptances: **None** (no test/fence/live-gate waivers). Implementation planning and production edits are authorized through the workflow; no shipping authorization is implied.

2026-09-10 amendment: requester selected **"Use gix-object and gix-hash"** after disclosure of the narrow SHA-1 dependency graph, strict-policy boundary and required build/cargo-deny verification. This authorizes the object-layer dependency amendment above, not weakening behavior, budgets, oracle independence or fences.

2026-09-10 publication authorization: requester selected **"Authorize draft PR stack"**. Publish checkpointed increments as draft PRs; no merge or issue-closure authority is granted.
