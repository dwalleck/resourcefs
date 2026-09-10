# Plan: rfs-jrz7

Approved input: design.md, requester “Approved”, 2026-09-09. Empirical route and complete behavior: route.md T4; P1 evidence remains valid against pinned 635ab170. C1 grammar design feasibility is already PASS; production comparisons remain owed. This plan declares no slice complete.

## Partition and execution contract

Three independently-green slices, three review increments in dependency order:

| Increment | Slice | Changed lines incl. artifacts/tests/fixtures | Churn margin | Total | Independently mergeable result |
|---|---|---:|---:|---:|---|
| immutable-addressing | S1 | 2300 | 460 | 2760 | Public validated immutable syntax and six-dimensional acquisition controls with all callers migrated; unsupported source behavior remains explicit until its family is registered |
| immutable-commit | S2 | 2200 | 440 | 2640 | Configured immutable commit facts through public library and real stdio, independent of source content |
| immutable-source | S3 | 3200 | 640 | 3840 | Complete exact source path, outcomes, limits, recovery and live proof |

Sum7700 +1540 margin (20%, new edge-case fixtures and structural integration churn) =9240. >4000 requires this partition. Each boundary requires a draft PR under checkpointed-build. On2026-09-10 requester selected "Authorize draft PR stack": push checkpointed increments and open draft PRs; do not merge or close the issue. Discover upstream from Git remote HEAD, not a hardcoded branch. No PR increment relies on code in a later increment to compile or verify its delivered contract.

Main owns integration/checkpoints/artifacts. Writers work in isolated worktrees with disjoint ownership and skip all validation; Main runs the proof after integrating each completed atomic slice. This transfers, not waives, all tests/mutations/budgets. No subsequent slice begins while an earlier checkpoint fails. Named mutation red/restored-green receipts and nine gate states live in this plan's owning slice record; full command logs may be retained alongside it.

## Module growth ledger

Baseline counts are physical-line signals copied from design's measured inventory; production responsibility is enforced independently by the existing module gate. Ranges include only approved ownership, never permission to move a body into a facade.

| Module (under crates/resourcefs-* unless scripts/) | Baseline | Projected final | Responsibility/interface change | Protected-parent rule |
|---|---:|---:|---|---|
| core/src/reference.rs | 2000 | 2020–2080 | Github declaration/re-export/typed dispatch | No family grammar implementation |
| core/src/reference/github.rs | 0 | 250–450 | Validated CommitId/SourcePath/GithubAddress | Own segment grammar, no I/O/JSON |
| core/src/lib.rs | 83 | 85–90 | Re-export domain identifiers | No implementation |
| core/src/resource.rs | 720 | 722–735 | Canonical Github identity match | No provider work |
| core/src/read.rs | 369 | 370–380 | Projection identity branch | No provider work |
| core/src/discovery.rs | 1341 | 1345–1365 | Explicit source-kind/identity matching | No new discovery responsibility |
| sources/src/compiled.rs | 489 | 495–510 | Github adapter dispatch | No fact construction |
| sources/src/github/mod.rs | 1170 | 1180–1230 | Fact detection/catalog/routing | No object/grammar logic |
| sources/src/github/mutation.rs | 1319 | 1320–1330 | Explicit immutable-target refusal | No writes added |
| sources/src/github/facts.rs | 638 | 650–700 | Request dispatch/context/reporting | No Git objects/source outcome bodies |
| sources/src/github/facts/identity.rs | 590 | 590–610 | Reuse helpers; only necessary visibility | No new object responsibility |
| sources/src/github/facts/commit.rs | 0 | 300–500 | Native commit acquire/read/projection | Private concrete family |
| sources/src/github/facts/object.rs | 0 | 350–550 | Pure tree/blob/mode/hash/encoding validation | No HTTP/session/envelope |
| sources/src/github/facts/source.rs | 0 | 350–550 | Bounded traversal/retention/projection | Private concrete family |
| mcp/src/server/read.rs | 154 | 154–160 | Existing input consumption if needed | No Git logic |
| mcp/src/profile/model.rs | 1832 | 1832–1840 | Existing AcquisitionInput embedding | No parallel DTO |
| mcp/src/profile/schema.rs | 49 | 49 | Generated schema reused | No hand schema implementation |
| scripts/module_shape.py | 432 | 440–510 | Existing placement gate extended | No competing issue gate |
| scripts/module_shape_base.py | 548 | 540–570 | Consume explicit successor wiring policy/shared-codec relocation; own the pinned parent ceilings | Library-only support module: the ledger gate is the one entry point |
| scripts/module-ledger.json | 416 | 450–550 | Four owner entries/approved fingerprints | Keep existing owner rules |
| scripts/ci-gates.py | 160 | 165–185 | New ignored-budget inventory | Live roster remains separate |

Constructor-callsite changes outside these owner bodies are mechanical sixth-argument migrations; enumerate exact callers through LSP references and textual safety-net before edits, record them below. Existing test modules migrate their constructors without changing unrelated expectations. Configuration GithubConfig embeds the same limits type unchanged. Existing acquisition/profile/generated schema fixtures and public docs update with their governing interface slice.

## Slice S1: Typed immutable addresses

**Claim IDs:** C1/C2. C11's final conformance is owned by S3, with the mandatory shape gate at every slice. The decoded acquisition dimension lands with S3, the first increment that decodes regular-file content, so no increment advertises a control it cannot honor.
**Expected behavior:** Public PathReference parses/canonicalizes Commit/Source exactly as approved, newtypes reject invalid values, every ResourceAddress caller is exhaustive without wildcard escape, and the unserved github:// family is refused identically with and without a mounted GitHub source or caller acquisition controls. Existing adapters explicitly reject unavailable projections; no stub facts or success fallbacks.
**Oracle:** Hand-authored decoded path/invalid-input table plus independent standard-library encoding from C1; paired mounted/unmounted compiled-source runs for the family refusal. HTTP operand observation from C2 is a cross-path recheck under S3/C4, where source acquisition exists; it cannot stand in for this public parser proof.
**Stress fixture:** All C1 paths/refusals plus lowercase full SHA versus uppercase/short/ref names, %252F, U+FFFD as valid Unicode, backslash/control-byte filenames; 1024-segment bounded syntax specimen. The unserved-family refusal is exercised with and without caller acquisition controls so its category cannot depend on the control set.
**Regression fence:** core/tests/github_immutable_reference_contract.rs; sources/tests/compiled_sources_contract.rs and tests/github_adapter_contract.rs for the family refusal.
**Named mutation:** C1 witness double-decodes percent literal; C2 production segment decoder double-decodes percent literal, yielding different path/refusal; the compiled family arm routes to the GitHub source again, so an unmounted build answers `source_unavailable` for a family no source serves.
**Complexity/production scale:** Segment parse/canonicalization O(n) input bytes, O(n) retained UTF-8; positive limits O(1). Maximum accepted fixture: a complete encoded reference of exactly 65,536 bytes, including scheme/operands/suffix, with 1024 segments; 65,537 bytes must retain the existing limit_exceeded refusal. Allocation growth <=16 times input plus 1 MiB and <=1 second release wall time. Rationale: linear syntax work bounded by existing MAX_PATH_REFERENCE_BYTES, with ample CI scheduling headroom and no new public ceiling.
**Wall budget/phase:** Always-on parsing above: <=5 ms per maximum fixture (release), ~70x tighter per byte than the sibling small-reference budget; a control-construction phase bounded at 1 microsecond average. No provider latency in this slice.
**Module shape:** Create core reference/github.rs; protected reference.rs +<=80, resource.rs +<=15, read.rs +<=11, discovery.rs +<=24, compiled.rs +<=21, mutation.rs +<=11; interfaces are the approved typed address sum; the sixth limit dimension arrives with S3. Append the explicit successor stage and run `python scripts/module_shape.py --root . --stage immutable-grammar` → correct owner placement, shared-codec relocation and no forbidden dependencies. Preserve all prior stages and inherited obligations; pass the approved2080 reference-parent tripwire through both inherited budget checks rather than filtering their failures; parent ceilings live in the pinned policy, not in ledger relaxations.
**Files:** core reference.rs/reference/github.rs/lib.rs/resource.rs/read.rs/discovery.rs; sources compiled.rs; the exact typed-constructor callers from impact analysis; named tests; scripts/module-ledger.json/module_shape.py/module_shape_base.py; this issue artifacts. Source family is not exposed in catalog before implemented.
**Estimate:** 1–3 hours engineering signal, not gate.
**Diff estimate:**2300 changed lines plus460 churn margin, including workflow artifacts, requested dependency evaluation and grammar/limit fixtures. Updated from2100+420 after measured pre-commit2677 lines: requester-directed crate-resolution evidence and incremental review records account for the increase; ownership and partition boundaries are unchanged.
**PR increment:** immutable-addressing.
**Commands and expected results:**
- `cargo test --locked -p resourcefs-core --test github_immutable_reference_contract` → exact valid identity roundtrips and refused aliases, including `%252F` unchanged after one decode.
- `cargo test --locked -p resourcefs-mcp --test schema_contract` and affected acquisition/profile tests → schema and accepted/rejected real DTO inputs agree.
- `python .rfs-jrz7/compare_grammar.py` → public Rust parser agrees item-by-item with independent standard-library encoding and hand-authored refusal/selector expectations; temporary example is removed.
- `cargo test --release -p resourcefs-sources --test github_facts_contract immutable_reference_parse_budget -- --ignored --exact --nocapture` → parser time and tracked allocation hold under the tightened release bounds; a debug run reports the wrong profile instead of passing.
- Named mutations → corresponding fence red, restored fence green.
- Existing shape gate plus affected cargo checks → no missing match arms, no owner drift.

### S1 implementation observations

- Production/oracle comparison: PASS, 39 identities/refusals/selector cases via `compare_grammar.py`. Covers Unicode, controls/backslash, once-decoded percent literals, nonminimal/lowercase escapes, commit-ID refusal, and encoded versus structural selectors. Initial build caught an accidentally omitted ResourceAddress re-export; restored the existing export before this successful run.
- Reuse review removed avoidable split/collect/join/re-split path copying, duplicate decoded-segment validation, pure accessor/constructor aliases, and redundant existing refusal arms. GitHub syntax now retains borrowed path input until the one decoder.
- Shape baseline refused missing child and missing relocated codec as expected. First integrated run misclassified ResourceAddress::Github patterns as new calls; corrected through an explicit path-scoped wiring-call allowance, not whole-function exemptions. The gate and named placement mutation subsequently passed; see checkpoint evidence below.

## Slice S2: Complete immutable commit metadata Resource

**Claim IDs:** C3/C9; commit-specific portion of C10 is verified here, final C10 owner S3.
**Expected behavior:** Authorized configured repository/full commit read returns complete owned JSON with exact requested/observed IDs, parents, native identities/times/accounts and presence; wrong identities/links fail. Commit kind is discoverable through catalog/CLI and public library/real stdio; human reads unchanged.
**Oracle:** Independent Git fixture object database and hand-authored native account/presence/response-observation table; compare public read output and actual upstream requests, not private DTOs.
**Stress fixture:** Multiple distinct parents, absent/null/value properties, malformed types/duplicate JSON fields/native zero IDs, wrong SHA/repository/link origin, metadata larger than display cap, revalidation/error/cancellation controls; positive valid fixture at same shape.
**Regression fence:** sources/tests/github_immutable_contract.rs commit/schema cases, existing MCP stdio facts contracts and adapter live smoke rows added here.
**Named mutation:** C3 remove exact requested-SHA comparison (fixture's supplied links stay correct so link guard cannot explain refusal); C9 coerce absent authorAccount to null; exact assertions turn red then restored green.
**Complexity/production scale:** O(b+p) decode/project over bounded response bytes b<=8 MiB and parent entries p within that body; repository+commit plus retries share <=10 attempts/30s, admitted bodies<=16MiB, final JSON<=16MiB. Release maximum commit fixture local CPU wall <=2s; high-water tracked allocation <=128MiB over measured baseline. Rationale: bounded raw payload, presence-aware decoding and one serialized result without retained generic JSON tree.
**Wall budget/phase:** Always-on acquisition <=30s shared logical deadline; local decode/project <=2s fixture measurement. No background phases.
**Module shape:** Create private facts/commit.rs acquire/read; facts.rs +<=62, mod.rs +<=60 total branch tripwire; no new parent body. `python scripts/module_shape.py --root . --stage immutable-commit` → allowed commit child, preserved legacy fingerprints/owner obligations.
**Files:** sources github/facts/commit.rs/facts.rs/github/mod.rs/compiled.rs, identity.rs only helper visibility if necessary; named source/MCP tests and fixture rows; GitHub catalog/CLI documentation; scripts/module-ledger.json/module_shape.py; plan evidence record.
**Estimate:** 2–4 hours signal.
**Diff estimate:** 2200 changed lines.
**PR increment:** immutable-commit.
**Commands and expected results:**
- `cargo test --locked -p resourcefs-sources --test github_immutable_contract` → requested versus synthetic/fork IDs and all presence cases match independent expected values; zero forbidden egress with positive permitted control.
- A real `rfs check`/stdio read against independent fixture → complete parseable commit facts, artifact recovery byte-equal to acquired representation, no second acquisition during recovery.
- `cargo test --locked -p resourcefs-sources --test github_adapter_contract` → existing native diff still exact and truncation refused.
- Adapter/stdio gated commit live row against pinned public revision → same commit/tree/parents as Git oracle; skips are not PASS.
- C3/C9 mutation/restoration and shape/CPU/allocation measurements → localized red/restored green and bounded values.

## Slice S3: Complete exact source acquisition and assembled proof

**Claim IDs:** C4/C5/C6/C7/C8/C10/C11; revalidate C1–C3/C9 when integration changes their assumptions.
**Expected behavior:** Exact source bytes/objects through validated tree chain with all approved outcomes, shared controls, integrity overrides, cache lifecycle and real MCP recovery. The complete ticket contract holds, not merely the happy path.
**Oracle:** Git ls-tree/cat-file/hash-object/mktree independent of implementation and REST probe; explicit byte corpus; independent upstream listener/cancellation controller/allocator/stdio client. Per-response provenance and final SHA-256 are computed outside production admission.
**Stress fixture:** Design's full tree/mode/path/base64/size matrix; exact decoded cap/cap+1 with absent sizes to isolate decoded guard; reported oversize skips blob; deep path's eleventh request blocked; 1000/1001 entries, equal-prefix file/directory Git ordering, invalid/lossy name causing tree hash mismatch, truncated tree with target present, source missing only on complete tree, terminal symlink/submodule/directory/LFS, ordinary post-entry blob failure versus cancellation/identity failures; revalidation/generation race after serialization.
**Regression fence:** sources/tests/github_immutable_contract.rs and github_immutable_budget_contract.rs; actual MCP stdio facts/recovery and live rows; existing module-shape gate extended. Every new fence is introduced with the source behavior, not a later hardening slice.
**Named mutation:** C4 bypass tree hash; C5 encoded-length-only bound; C6 reset budget per tree; C7 demote identity failure to unavailable; C8 remove post-serialization generation check; C10 map compiled Github read to unsupported; C11 move tree decoder into facts.rs. Each requires its positive control and localized red/restored green. Technical corrections preserving oracle meaning are recorded under design approval semantics.
**Complexity/production scale:** For d path segments and tree widths w_i<=1000, traversal decode/hash O(sum entry-name bytes + sum w_i log w_i) plus O(B) blob/base64 work; <=10 physical attempts including repository/commit/trees/blob/retries, <=30s shared deadline, <=8MiB response, <=16MiB accepted bodies/final JSON, <=4MiB decoded content. Sort stores bounded entries; discard complete raw trees after verification except bounded observations. Release maximum loopback fixture local CPU <=3s, tracked allocation high-water <=160MiB over baseline, no wire/RSS guarantee. Rationale: two bounded response/decode representations and one bounded output with generous allocator/serde headroom, measured rather than assumed.
**Wall budget/phase:** Always-on acquisition 30s shared logical deadline with explicit lower caller/operator limits; local bounded tree/blob projection <=3s maximum fixture; cancellation/final acceptance must reject regardless of time spent. No background phase.
**Module shape:** Create facts/object.rs (pure Git validation) and facts/source.rs (source acquisition/projection), final private owner interfaces per design; no added parent responsibility. `python scripts/module_shape.py --root . --stage immutable-source` and full gate → all four owner entries and exact protected-parent census hold. Fresh-context reviewer reconstructs final module map before approved ledger disclosure.
**Files:** sources github/facts/object.rs/source.rs/facts.rs/github/mod.rs; core acquisition.rs/error.rs decoded dimension with its MCP field, generated schema fixture, transport forwarding and contract tests (repartitioned here from S1 so the control lands with the increment that enforces it); named source tests/fixture support; MCP stdio fact/recovery/live tests; existing library examples/catalog/profile docs; scripts/module-ledger.json/module_shape.py/ci-gates.py and live-smoke roster only if discovery requires it; owning issue evidence/tracker record.
**Estimate:** 3–6 hours signal.
**Diff estimate:** 3200 changed lines.
**PR increment:** immutable-source.
**Commands and expected results:**
- `cargo test --locked -p resourcefs-sources --test github_immutable_contract` → Git-oracle byte/object equality, exact requested paths, correct unsupported/unavailable outcomes and integrity rejection.
- `cargo test --locked --release -p resourcefs-sources --test github_immutable_budget_contract -- --include-ignored` → exact/over dimensional assertions and measured CPU/allocation bounds, no eleventh request.
- Real stdio source read with tiny display bound → artifact chain reconstructs exact JSON and decoded Git blob; recovery performs no upstream acquisition; separate successful initial read proves request observer is active.
- `python scripts/ci-gates.py` → complete repository-owned local gate; required ignored budgets registered, no failures waived.
- `scripts/live-smoke.sh` with RFS_LIVE=1 and token through existing mechanism → adapter and actual rfs stdio immutable paths agree with authorized public Git oracle; exact source revision recorded separately; absent gates are SKIP not PASS.
- C4–C8/C10/C11 named mutations → each appropriate fence red then restored green. Final reviewer reconstruction/ledger comparison → no unresolved mismatch.

## Self-review and initial critique

All eleven design claims have one final owning slice; shared precursor/integration checks are explicit, not separate claim completion records. Every slice has fourteen mandatory fields, its fences/mutations, independently observable interface, stress shape, measured cost bound, ownership/diff budget and increment. No proposed loop cost assumes constant tree width or free parents. The source SHA-1 work stays in sources and uses existing dependency; paths never use platform filesystem normalization. No waived risk or implicit corporate/native acceptance. Separate intended families remain verified rfs-iktl/rfs-e5cv, not implementation omissions.

Initial critique resolved: C2's HTTP request observation requires S3 acquisition and is therefore rechecked under C4/C10, while S1's decisive parser oracle is independent literal path identity. C6 includes earlier control-constructor migration but cannot be judged complete until actual S3 decoding/transport measurements. Stage names/line fingerprints in the existing shape gate must be adapted without relaxing unrelated ownership. Publication authorization is a real boundary prerequisite, explicitly recorded above.

Reuse-audit correction: reference.rs::validate_reference_input already enforces a 64 KiB complete-reference ceiling. The original 128 KiB accepted parser measurement would have contradicted that established contract; the fixture above now exercises the existing exact/over boundary. This is a technical verification correction preserving approved behavior, not a new limit decision.

## Checkpoint records

No completed slice yet. Each record will carry caller analysis, helper reuse, symmetry audit, exact proof commands/results/source state, nine gate judgments, mutation/restoration receipts, budget measurements and evidence-validity decisions. Required source/fixture corrections stay in the owning slice record; no duplicated mutable proof ledger.

### S1 pre-implementation impact analysis

LSP references run on the indexed main checkout at the identical 635ab170 source baseline: acquisition constructor returned 33 references, ResourceAddress returned 192. Absolute sibling-worktree lookups incorrectly returned zero; reported to tool QA and recovered through the indexed identical source. A textual constructor safety net additionally found the test helper at github_facts_contract.rs:195, proving LSP alone was incomplete. No code had changed in either checkout when compared.

Constructor callers (baseline lines; sixth argument migrates atomically):
- core/tests/acquisition_contract.rs:15,28,35,43,56,64,72.
- mcp/src/acquisition.rs:35.
- sources/src/http/read.rs:327.
- sources/src/http/read_acquisition_tests.rs:11,62,127,193,365,384.
- sources/src/github/facts_tests.rs:186.
- sources/tests/github_facts_contract.rs:195,670,743,865,875,886,897,1050,2529,2611,2787,2820,2911,3050,3245,3315.
- sources/tests/github_live_smoke.rs:364.

Load-bearing ResourceAddress match owners: core reference.rs (parse, constructors, canonical identity), read.rs:120–127, resource.rs:623–650, discovery.rs:378–386 and1237–1269; sources compiled.rs:99–135,148–170,289–322; GitHub mod.rs:771–804,822–830,916–931; mutation.rs:62–118 and175–205. Existing let-else callers keep explicit rejection semantics and need no invented fallback. Exhaustive test matches include core discovery_engine_contract.rs:283–290, path_reference_contract.rs:308–320 and read_engine_contract.rs:80–117; compiler checking after integration finds any remaining sites. New GithubCommitId/GithubSourcePath/GithubAddress methods have no preexisting callers; their callers are introduced with the typed family. Existing SourceAdapter::read signature is unchanged.

S1 RED baseline: `cargo test --locked -p resourcefs-core --test github_immutable_reference_contract` in isolated grammar writer checkout at 635ab170 with only the new regression test added returned exit101. `immutable_source_references_keep_encoded_path_identity` failed at the public PathReference::parse call with InvalidReference / unsupported Path Reference scheme. The test exercises an existing public interface and compiled successfully; this is behavioral RED, not an absent-symbol compile failure. Raw run artifact: artifact://1028 (durable outcome recorded here). No production edits preceded it.

### Mandatory reuse inventory — user-directed audit

User instruction: “As we design these new modules, lets ensure similar code doesn't already exist. That really hurt us on the last PR.” Audit results are implementation inputs, not optional review suggestions.

| Proposed S1 operation | Existing owner/symbol | Decision |
|---|---|---|
| RFC3986 unreserved uppercase encoding | reference/jira.rs::encode_jira_segment | Move the exact codec into the existing shared reference lexical-helper owner and rename to encode_rfc3986_segment with LSP; migrate Jira and Github to it. Do not add another encoder. Workspace/local encode_component has different semantics and stays unchanged. |
| Safe single-pass percent decode | reference.rs::percent_decode/decode_hex | Extend the existing decoder to return invalid_reference for malformed/truncated escapes instead of unchecked indexing/expect. New Github segments invoke it once after literal-alphabet validation; existing callers retain their stricter separator validator. No second escape scanner. |
| Repository and selector syntax | GithubRepositoryIdentity, projection_candidate_split, ProjectionSelector | Reuse directly. Split structural route first; never reuse parse_github_body's whole-body decoding for source paths. |
| Git path semantics | No matching existing domain type | New GithubSourcePath is justified: WorkspacePath, LocalName and workspace segment checks normalize/reject valid Git backslash/control-byte names. Reuse lexical primitives, not filesystem policy. |
| Full commit operand | Existing fixed-hex validation pattern; no matching commit domain identifier | New GithubCommitId with its own semantic errors; do not invent a generic identifier framework for a short lexical check. Upstream source SHA validation stays in its current layer. |
| Decoded controls and MCP field | ReadAcquisitionLimits::validate/intersect/Default; AcquisitionInput/read_field/into_limits/describe_limit_rejection | Extend existing machinery. Reuse shared profile/tool DTO and generated schema. No second limits parser or serializer. |
| Parser contract fixtures | Existing github_reference_contract/path_reference_contract patterns; C1 independent table | Reuse public parser seam and table conventions; no new harness. New test file is family-specific behavior, not a duplicated parser or fixture server. |

Additional S1 module inventory: core/src/reference/jira.rs baseline454, projected440–452; only replace its private codec with the shared helper import/call. This is shared lexical-helper consolidation within the existing core reference responsibility, not a new family parser body in the protected parent. The approved Github family/SourceAdapter interfaces and responsibility clusters are unchanged. Parent reference.rs already owns decoding and encoding primitives; this maintenance does not authorize moving Jira/Github family grammar bodies there. Existing Jira behavior must be verified unchanged, including its canonical query/segment tests.

| Proposed source operation | Existing owner/symbol | Decision |
|---|---|---|
| Sanitized failure facts and fatal-retention policy | collection.rs::FailureFacts/failure_facts/rejects_every_page | Reuse in place with the smallest pub(super) visibility; rename the shared predicate to invalidates_retention through LSP when its second caller is introduced. Keep one serializer/policy; do not copy into source.rs or move bodies into the protected facts.rs parent. Collection's cursor retryable policy remains collection-only. |
| Repository/native SHA/link validation | identity.rs::validate_repository/sha plus expected_object_url/require_object_link/validate_optional_object_link | Expose or minimally factor existing helpers within the same owner. Do not reconstruct repository identity from a second validator or force PR-specific route parsing onto commit URLs. |
| Native account/presence decoding | facts.rs::Presence/NativeId/native!/Actor/Repository | Reuse types and visitor macro; only native Git author/committer wire shape is new. |
| Context and final publication | facts.rs::FactsRead/facts_resource/finish_facts/check_generation/sanitize | Generalize existing typed dispatch/context initialization once, no second setup path or capped writer. Preserve all post-serialization checks. |
| Acquisition and observations | fetch.rs::fetch_controlled/FetchedResponse/BodyObservation; HttpReadBudget | Reuse one budget per read. Legacy json/fetch and cursor retry helpers are the wrong seam. New source-specific provenance aggregation contains existing observations, not copied raw response payloads. |
| Git tree/blob serialization, Git byte ordering, mode/type handling | User-approved gix-object/gix-hash; no existing production object implementation | Use the specialized crates for Git format semantics, with strict ResourceFS acceptance policy around them. Existing base64 STANDARD remains the blob codec, not URL_SAFE_NO_PAD cursor math. |
| Request/TLS/session test infrastructure | sources/tests/support/tls.rs, certificates.rs, session_support; MCP tests/support/stdio.rs and profile_tls.rs | Reuse established #[path] helpers and per-family fixture builders, not another HTTP observer or fixture framework. Reuse existing artifact reconstruction helper within the owning stdio test file. |
| Streaming canonical Git object hashing | gix_object::WriteTo::{loose_header,write_to}; gix_hash::io::Write::new(std::io::sink(), Kind::Sha1) and writer.hash.try_finalize() | Registry source confirms a crate-owned hashing writer: hash the crate-generated header and stream serialization into the sink, without a second custom hash writer or materialized tree-byte buffer. Collision-detection errors remain failures. This API inspection is not yet S3 compilation proof. |

Source-audit recommendation to lift failure helpers into facts.rs is deliberately rejected: its 700-line protected-parent rule and envelope/dispatch ownership remain load-bearing. Minimal visibility reuse retains the current helper owner and avoids both duplication and facade growth. A truly required ownership change would return to design approval, not be smuggled in as cleanup. Add collection.rs to S3's touch inventory for visibility/neutral predicate naming only (baseline771; projected771–776); no retention behavior change. Every source writer must consult this table, and final review explicitly searches for duplicate validators/encoders/error policies/fixture infrastructure.

### Specialized Git crate evaluation — requester follow-up

Requester asked whether crates already cover Git tree decoding. No production object decoder exists yet. `gix-object=0.64.1` and `gix-hash=0.26.2`, with default features disabled and SHA-1 enabled, fit the intended narrow object layer without importing full gix or git2. Public Tree/Entry/WriteTo APIs cover representation/serialization; tree::name_order supplies Git's directory-aware byte ordering. EntryKind converts known modes to native spellings (directory40000); do not pass REST040000 through the byte-preserving internal mode parser, or use lossy kind() to authorize unknown modes. ResourceFS must still validate provider JSON, full tree integrity, duplicate/path/mode/type evidence, authority and budgets.

Published manifests declare Rust1.85 (workspace floor1.88) and MIT OR Apache-2.0. A temporary external Rust1.88 Cargo package successfully resolved the SHA-1-only graph:68 packages across targets,28 package names absent from the current repository lockfile. This is not a measured final workspace lockfile delta or binary/build-size estimate. The sha1 feature adds sha1-checked, rather than using aws-lc-rs. Installed aws-lc-rs1.18.0 already exposes digest::SHA1_FOR_LEGACY_USE_ONLY; no cryptographic implementation needs to be written. Complete metadata is retained in git-object-dependency-audit.json. No project manifest or lockfile changed; no crate compilation or cargo-deny pass claimed for this candidate.

Sources: https://docs.rs/gix-object/0.64.1/gix_object/ ; https://docs.rs/gix-object/0.64.1/src/gix_object/tree/mod.rs.html ; https://docs.rs/crate/gix-object/0.64.1/source/Cargo.toml ; https://docs.rs/crate/gix-hash/0.26.2/source/Cargo.toml.orig . Recommendation: specialized object/hash crates for format semantics, with our strict acceptance policy around them; dependency/design decision remains explicit before object implementation.

Decision resolved2026-09-10: requester selected "Use gix-object and gix-hash". S3 implements strict validation atop these crates, not a new hand-written Git codec; root/source Cargo manifests and Cargo.lock become intentional S3 changes. Existing behavioral claims, independent Git oracle and budget/fence obligations remain. Dependency adoption still owes assembled build, cargo-deny and exact Git-format fixture proof.

### S1 checkpoint evidence

- Affected functional contracts: PASS at the repartitioned S1 head; the decoded-control contracts and their schema fixture moved to S3 with the dimension, so no S1 leg depends on an unenforced control.
- Review round 2 re-verification: PASS, `python3 scripts/ci-gates.py` reported "All repository gates passed." on the repair commits; mutation-review2.txt holds the routing, ceiling, budget, claim, baseline, ledger-shape and ownership mutants red/restored-green; qualification-s1.txt records the exact command and result.
- S1 parser falsifier/oracle: PASS,39 production/oracle cases via compare_grammar.py; C2's HTTP operand/SourceResource acceptance checks remain assigned to S3, not claimed here. C1 design witness retained and rechecked.
- Stress/production budgets: PASS, release exact65536-byte reference with1024 segments,100 iterations; the tightened bounds, the exact 65537-byte refusal and the control-construction phase are recorded in mutation-review2.txt. Provider-latency budget N/A for this syntax slice.
- Regression/mutation/restoration: PASS; mutation-c1.txt, mutation-c2-parser.txt and mutation-review2.txt retain compiled red and restored-green outcomes. The decoded-control mutation moved to S3 with the dimension it governs. The parser-test repair was a compile fix: ResourceAddress has no canonical_reference method; the replacement uses a fixed expected canonical identity, not a changed behavioral expectation. The redundant constructor-derived equality was removed after the existing as_str identity assertion, not because it failed.
- Module placement/mutation/restoration: PASS; mutation-c11-placement.txt includes successful cargo check of the misplaced decoder, exact C11 protected-parent decode_git_tree#1 refusal, and restored-green gate. Initial mutant incorrectly passed; existing parent-node comparison was reused for the missing facts.rs boundary. No numeric failure substituted for responsibility enforcement. Codec ownership lookup uses historical_owners; reference.rs2044<=2080, HTTP read350<=350.
- Restoration runner repair: preserving source mtime initially reused a mutant Cargo artifact. Subsequent runs restore exact backed-up bytes with a fresh mtime, force recompilation naturally, and record real restored-green results. No source changes were lost.
- Impact-analysis recheck: rust-analyzer ready; main-checkout constructor definition line19 returns33 references. Assembled-worktree grep confirms migrated constructors and Github match sites across all three crates; an earlier empty sibling-worktree LSP response was never treated as exhaustive.

### S1 returned checkpoint judgment

| Gate | State | Evidence/applicability |
|---|---|---|
| 1 Affected unit tests | PASS | Full all-feature debug and release functional legs passed after F1 removal; qualification-s1.txt. |
| 2 Assigned falsifiers | PASS | C1 witness and S1 production grammar cases; C2 HTTP operands explicitly belong to S3, not this syntax increment. |
| 3 Stress fixture | PASS | Unicode/once-decoded identity matrix and exact65536/65537 boundary with1024 segments. |
| 4 Implementation versus oracle | PASS |39 fixed URI/refusal/selector cases; removing an unused alternate entry point does not alter the exercised PathReference path. |
| 5 Approved module shape | PASS | Explicit immutable-grammar owner/parent census and compiled misplaced-decoder red control; parent-node check reused. |
| 6 Production budgets | PASS | Release parser and control-construction measurements above under the tightened bounds; runner recognized/executed immutable_reference_parse_budget. Network-latency phase N/A: no acquisition in S1. |
| 7 Regression fence | PASS | Parser and typed-constructor contracts plus the paired mounted/unmounted family-refusal contracts; full debug/release functional legs. |
| 8 Named mutation | PASS | C1/C2/C11 logs plus the review-round-2 routing, ceiling, label and duplicate-ownership mutants; each mutant changes the governed observable and compiles where applicable. |
| 9 Restored fence | PASS | Exact-source backup restoration plus successful fresh compilation/test results in the same logs. |

Repository quality: python scripts/ci-gates.py completed all legs; only Formatting failed after the F1 edit reordered imports. cargo fmt --all followed by cargo fmt --all -- --check passed. Retain all other constituent results (clippy, debug/release functionality, complete ignored-budget inventory/execution, cargo-deny, placement): formatting changes no semantics, dependency graph or fixtures. qualification-s1.txt records this disposition; the original aggregate exit1 is not relabeled exit0. Earlier900s orchestration timeout was too short; its last test passed independently in19.96s, and the extended run completed in1141.43s.

Incremental review F1 is resolved by deleting the unused unbounded GithubAddress::parse API. Review round 2 is resolved by the S1 rows of review-decisions.md: the control surface moved to the increment that enforces it, the family refusal no longer depends on configuration, and the placement gate pins parent ceilings, classifies claims per check, validates ledger policy shape and compares owner declarations workspace-wide. No publication/merge of S2/S3 behavior is claimed. Final full-feature conformance, platform and live proof remain outstanding.

Publication checkpoint: S1 committed as `5d941fb` and pushed to `feat/rfs-jrz7`; draft PR [#12](https://github.com/dwalleck/resourcefs/pull/12) targets the discovered upstream default branch `main`. Post-commit fetch found no upstream movement from `635ab170`; no merge/rebase or proof invalidation was necessary. S2 continues on stacked branch `feat/rfs-jrz7-commit`. No merge or ticket closure is authorized. The initial Actions watcher returned a connection error; subsequent authoritative run queries confirmed both [34435883243](https://github.com/dwalleck/resourcefs/actions/runs/34435883243) and [34435861926](https://github.com/dwalleck/resourcefs/actions/runs/34435861926) completed successfully at the exact S1 head.

### S2 returned checkpoint judgment

Delivered: immutable commit Facts through the existing SourceAdapter, compiled source registry, catalog, configured profile and real binary stdio. The shared context dispatch matches the existing address sum; no mirror enum, second setup/envelope or duplicate SHA validator remains. Final physical lines: commit owner405, facts parent671<=700, GitHub facade1175. Staged increment:1938 changed lines across24 files, within S2's2200-line estimate. Source-file acquisition and crate adoption are not claimed by S2.

| Mandatory gate | Verdict | Evidence |
|---|---|---|
| 1 Affected unit tests | PASS | All-feature debug/release functional legs; ten new immutable contracts, existing adapter/Facts tests and actual stdio path. |
| 2 Assigned falsifiers | PASS | C3/C9 compiled red and exact-restored green; commit part of C10 runs real stdio and live upstream. |
| 3 Stress fixture | PASS | Exact 8MiB native commit, distinct parents, presence/malformed/identity cases and Enterprise API prefix. Existing shared cache/revalidation/cancellation controls remain covered by the existing Facts suite; source-chain C8 remains S3-owned. |
| 4 Implementation versus oracle | PASS | Native/Git tree, ordered parents and exact 69-byte message comparison plus actual adapter/stdio comparison at the same pinned revision; independent hand-authored presence fixtures. |
| 5 Approved module shape | PASS | immutable-commit owner census, exact reused identity seam and facts parent allowances; existing C11 protected-parent mutation remains applicable to the unchanged declaration-rejection logic. |
| 6 Production budgets | PASS | 8388608 native bytes, 8390709 owned bytes; 16245553ns wall and 33575607 incremental heap bytes, below 2s/128MiB. |
| 7 Regression fence | PASS | Wrong identity/link, native presence, malformed values, Enterprise prefix and actual artifact recovery; full functional/release legs retained. |
| 8 Named mutation | PASS | mutation-c3-commit.txt and mutation-c9-commit.txt. The initially masked C3 mutant was diagnosed and isolated, not misreported as red. |
| 9 Restoration | PASS | Exact backed-up production bytes restored with fresh mtimes; restored C3/C9 tests pass. |

Full runner completed in1135.01s with only a test-setup needless-borrow lint failure. The unnecessary String copy was removed; exact formatter, full Clippy gate and affected release budget then passed. Other successful legs remain valid; the original aggregate exit1 is not relabeled exit0. Complete disposition: qualification-s2.txt. Incremental review S2-F1/F2 is resolved; live rows and credentials-free output are recorded in evidence.md/live-commit-results.txt. No S2/S3 merge or issue closure is authorized.

### S3 returned checkpoint judgment

Delivered: exact immutable source Facts through the configured SourceAdapter and real MCP binary, with the approved gix-object/gix-hash adoption, independent Git corpus, complete-tree/blob validation, one shared acquisition budget and narrow verified-metadata retention. The `github://` catalog has one family entry, not duplicate roots. Final physical lines: facts parent683<=700, commit440, object330, source374, collection772 and GitHub facade1175. Production traversal/hash responsibilities remain in the two private child owners; no second transport, encoder, error policy, envelope or fixture framework was introduced.

Caller/reuse reconciliation: the mandatory inventory above remains governing. Shared `commit::acquire` is consumed by commit.rs:233 and source.rs:139. The neutral `collection::invalidates_retention` policy is consumed by collection.rs:547/594 and source.rs:362; its old-family semantics are unchanged. Sibling-worktree LSP references returned empty despite these compiled callers, so that result was not treated as exhaustive; the bounded private-module text search and assembled compilation provide the fallback. New object/source functions have only their owning private-family callsites. Existing identity, body observation, capped serialization, operation guard and HTTP cache/budget helpers are reused.

Symmetry: commit and source share repository/commit authorization, requested identity and link checks, acquisition accounting and final acceptance. Source deliberately diverges only after verified terminal metadata: an ordinary blob failure or decoded bound can preserve metadata without bytes; malformed/integrity/link, denial, cancellation, deadline and generation failures still refuse the read. Unsupported terminal objects are explicit states, never followed or converted through another provider. Errors remain typed ResourceError values with the existing bounded operational details; no silent fallback or alternate logging pipeline was added.

Technical proof corrections preserve approved behavior, architecture and oracle meaning: source rows live in `github_source_contract.rs`; allocation/race/budget rows reuse the existing instrumented `github_facts_contract.rs` rather than duplicating its allocator in a proposed budget file. The catalog startup defect, initially masked C4 fixture, volatile elapsed-time representation fixture and equivalent Clippy repairs were resolved in this slice. Fresh C8 remove/move-before mutations cover the changed controller. Dependency adoption is intentional lockfile/manifests growth under the requester's recorded approval, not drift.

| Mandatory gate | Verdict | Evidence |
|---|---|---|
| 1 Affected unit tests | PASS | Full runner debug/release functional legs, plus repaired release commit/source21 and C8/budget2; qualification-s3.txt. |
| 2 Assigned falsifiers | PASS | C4–C8/C10/C11 named experiments; C2 exact once-decoded path and actual HTTP/stdio acceptance; C1/C3/C9 retain unchanged meaning with assembled parser/commit/live regression coverage. |
| 3 Stress fixture | PASS | Independent Git corpus: full-tree ordering/integrity,1000/1001, binary/special modes/LFS, exact decoded cap/cap+1 without size metadata, deep shared budget, cache lifecycle and serialization race; evidence.md. |
| 4 Implementation versus oracle | PASS | Git-plumbing fixture identities/bytes, separate gix adoption experiment, and enabled native adapter/real stdio comparisons at the pinned public revision; evidence.md. |
| 5 Approved module shape | PASS | immutable-source gate, C11 misplaced decoder red/restored green, and blind-first independent reconstruction/ledger comparison; blind-module-map.md and design-conformance-review.json. |
| 6 Production budgets | PASS | Full registered roster; final source5594189 native/5595590 owned bytes,25132893ns,22407536 tracked incremental heap bytes below3s/160MiB; qualification-s3.txt. Shared logical deadline/attempt and decoded limits have exact refusal fixtures. No wire/RSS guarantee was planned. |
| 7 Regression fence | PASS | Source/cache/cancellation/cap/identity matrix, real configured stdio recovery with no reacquisition, final serialization controller and existing old-family suites. |
| 8 Named mutation | PASS | mutation-c4-tree.txt, mutation-c5-decoded.txt, mutation-c6-source-budget.txt, mutation-c7-retention.txt, mutation-c8-generation.txt, mutation-c8-position.txt, mutation-c10-stdio.txt and mutation-c11-source-placement.txt. Initial masked C4 is recorded rather than counted as red. |
| 9 Restoration | PASS | Exact source-byte restoration with fresh mtimes; restored fences green. Changed C8 fence re-proven after lint cleanup. |

Local assembled quality is resolved through the completed runner and bounded repair, not an invented aggregate exit0: the original runner's only failed gate was Lints; full Clippy and all newly affected checks then passed. Exact final source state and disposition: qualification-source-state.json and qualification-s3.txt. Final live roster was rerun after cleanup:13 real network PASS,5 Jira environment SKIP; live-roster-results.txt. The glossary's existing Facts Resource and Version Tag definitions remain accurate, so CONTEXT.md is intentionally unchanged; DESIGN.md and the operating guide now describe source behavior. No existing changelog was found.

This is the final planned independently mergeable draft increment, stacked on `feat/rfs-jrz7-commit`. Published-head platform CI remains a publication check; no merge or ticket closure is authorized.
