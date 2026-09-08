# Design: rfs-0n97 — deployment-qualified PR facts

## Route and inputs

**Status: approved by requester on 2026-09-08. No production edits at approval.**

Empirical route at `7de8d1477af6be825bada9015a6996b3f2917c7d`, isolated worktree `/home/dwalleck/repos/resourcefs-wt-rfs-0n97-impl`, branch `feat/rfs-0n97`. The complete behavior set is **route.md, T4 rows 1–11**, adopted without weakening or repeating the completed interview. `spec.md: N/A — behavior fully explicit in those rows`.

Inputs: the named Rivets issue; `route.md`; `evidence.md` and `probe_provider.py` with both retained JSON observations; the approved sibling `resourcefs-wt-github-resource-contract/docs/github-resource-contract-spec.md`, decision ledger and ADR-0007. P1 freshly passed independent Python versus gh/jq identity observations under REST 2022-11-28. P2 retains applicable rfs-45ww native-diff/conditional/status evidence, not a source-level compatibility fence. Existing-interface inspection and new-feature numerical targets are classified separately, not sold as runtime proof. Corporate ghe.com/native Windows acceptance is not established here.

The change adds capability and migrates an existing read interface; it does not remove an authority, cancellation, output-recovery or mutation invariant. Two structural constraints must transition explicitly: protected orchestration tokens and the global new-module inventory. Their replacement retains the old dependency/ownership obligations rather than disabling them.

## Selected interface and representation

### Source-neutral read controls

Keep the existing seam and return type:

```rust
async fn read(
    &self,
    reference: &PathReference,
    operation: &OperationGuard,
    acquisition: Option<&ReadAcquisitionLimits>,
) -> Result<SourceResource, ResourceError>;
```

`ReadRequest` gains owned `acquisition: Option<ReadAcquisitionLimits>`. Migrate all seven production implementations, all direct callers and test adapters atomically; no legacy overload, compatibility shim or extra provider trait. Discovery's internal reads explicitly pass no override. OperationGuard remains cancellation/commit state.

Core owns a validating `ReadAcquisitionLimits` with private fields, positive dimensions, getters, hard defaults and componentwise intersection. Dimensions are `max_attempts`, `timeout`, `max_response_bytes`, `max_accepted_body_bytes`, and `max_representation_bytes`; timeout uses `Duration` in Rust. Construction rejects zero and above-hard values, rather than clamping invalid input. No serialization or provider dependency reaches core.

MCP exposes an optional closed `acquisition` object, separate from existing display `limits`, with optional positive integer fields:

- `maxAttempts` (hard maximum 10)
- `timeoutMs` (30,000)
- `maxResponseBytes` (8,388,608)
- `maxAcceptedBodyBytes` (16,777,216)
- `maxRepresentationBytes` (16,777,216)

Omitted members use hard defaults. A supplied empty object requests those defaults; explicit null, negative/fractional/wrong-type values, unknown members, zero and above-hard values are invalid protocol input. Validation precedes root refresh/acquisition in both real dispatch and generated-tool paths. The Rust constructor produces the corresponding typed error; MCP maps input-validation failures to invalid_params, preserving existing precedence.

Only PR `/facts` implements these overrides in this slice. Explicit overrides on other resources return `unsupported_projection` with reason `acquisition_controls_unsupported`, including direct adapter calls; absent overrides retain existing behavior. This does not pretend that local files or older network projections implement the new five-dimensional acquisition contract. GitHub profile operator `acquisition` uses the same validated dimensions. Effective values intersect caller, GitHub operator and applicable substrate ceilings; none can raise a lower operator bound.

### Deployment configuration

One GitHub deployment remains configured per instance. Add validated `GithubDeployment` in the existing `configuration/github.rs` owner. Its constructor consumes optional API-base and web-origin inputs; its internal sum distinguishes public GitHub, documented Enterprise Cloud pairing, and existing custom-base compatibility.

- Default/public `https://api.github.com/` derives `https://github.com`; an explicit contradictory web origin fails.
- `https://api.<tenant>.ghe.com/` derives `https://<tenant>.ghe.com`. Require the documented single tenant-label pairing, HTTPS origins, standard origin form, and no Enterprise Server `/api/v3` path for this class; reject conflicting web/API inputs before credentialed requests.
- Preserve previously accepted custom API bases for existing human reads. A custom base does **not** acquire a guessed web identity. Machine facts require an explicitly supplied validated web origin for this compatibility class; otherwise fail `unsupported_projection` / `deployment_identity_unavailable`. This is not a claim of Enterprise Server support or a deployment registry.

Profile adds optional `webOrigin` and `acquisition`, retaining `apiBaseUrl`. Public defaults remain usable without new fields. `GithubConfig::new` takes the validated deployment in place of raw optional API base and takes validated operator acquisition limits; migrate every constructor caller. Existing credential/grant/repository validation stays authoritative. The existing API-base accessor delegates to the validated deployment, not a duplicated string cache.

Configured `sourceId`, API origin and web origin qualify facts. Credential references, token material and principal identifiers do not enter the document. Native links are inert observations, never authority to fetch a fork or new origin. Metadata about a head repository supplied by an authorized PR is retained without acquiring that repository. Independent fork-content access remains separately authorized.

### Owned PR machine schema

Return `application/json` UTF-8 through SourceResource, not an exported provider DTO. Exact initial schema is `schemaVersion: {major: 1, minor: 0}` and `kind: "github.pull_request"`; outer contractVersion and REST version remain independent. Consumer examples reject unsupported majors and tolerate additive optional fields. ResourceFS produces this schema; it does not add a public generic schema framework or pretend to control every external consumer.

Top-level fields:

| Field | Exact meaning |
|---|---|
| `schemaVersion`, `kind` | Owned version and distinct singular PR fact family. |
| `resource` | Canonical `pr://owner/repository/number/facts` string. |
| `source` | `sourceId`, `deployment: {webOrigin, apiOrigin}` from validated configuration. |
| `repository` | Requested canonical owner/name plus supplied observed native repository facts, kept distinct rather than overwritten. |
| `request` | Requested canonical repository and repository-scoped PR number. |
| `observed` | Validated native PR identity and actual base/head commit/repository identities. |
| `acquisition` | `startedAtUnixMs`, `completedAtUnixMs`, monotonic `elapsedMs`, `restApiVersion: "2022-11-28"`, effective `limits`, and `usage` with attempted requests and admitted body bytes. |
| `upstream` | Body-observation metadata and, when applicable, separate revalidation metadata: supplied ETag, Last-Modified, Date, selected-version header and original native times/links retain their own meanings. |
| `data` | PR fields described below. |
| `unavailableFacts` | Bounded entries from a finite known field vocabulary for required knowledge the provider omitted or explicitly made unavailable. This is not an operational-error taxonomy. |

Native numeric object IDs and repository-scoped PR numbers serialize as canonical **decimal strings**, avoiding JavaScript precision loss; Rust validates positive u64 identities. Commit SHAs validate as lowercase full 40-hex for this explicitly selected initial object format. Node IDs remain supplied opaque strings, not decoded or synthesized. Original native spelling/fullName/links remain distinct from canonical request identity.

`data` contains `id`, `nodeId`, `number`, `title`, `body`, `state`, `author`, `createdAt`, `updatedAt`, `closedAt`, `mergedAt`, `links`, and `base`/`head` with `refName`, actual `commitSha`, owning `repository` and explicit repository availability. Preserve additionally supplied draft/merged flags; do not fabricate their absence. Unknown native state values remain strings. A private presence-aware decoder distinguishes absent, explicit null and present values; it does not reuse human rendering defaults or globally tighten the old human PR decoder. Supplied null remains null; omitted optional properties remain absent; an empty string remains empty. Native timestamps remain supplied strings, separate from acquisition clock values.

Required identity operands—positive native PR ID/number, a valid requested-parent link, and actual full base/head SHAs—must validate before publication. Missing/malformed required identity cannot become a successful unidentified Resource. Non-identity omissions are explicit unavailable knowledge. A null/deleted fork repository is valid and is never replaced with the base repository. Any supplied repository or parent identity contradicting the requested PR rejects the read. Do not fetch a synthetic merge ref or substitute merge_commit_sha.

Do not place VersionTag or final serialized-byte usage inside the bytes they describe. The outer VersionTag hashes exact final JSON; consumers can measure its byte length. Acquisition/revalidation metadata can change a facts VersionTag even when provider content and commit pointers did not. This does not change legacy human projection revalidation semantics.

### Acquisition, cache and errors

Use the existing substrate, conditional cache, origin credentials and single retry policy. Extract the current GitHub fetch/cache implementation into private `github/fetch.rs`; do not build a facts-only parallel cache or HTTP client. Retain media-type-separated keys, session authority/generation checks, cache quota behavior, validator handling, and no stale fallback after failed revalidation.

One facts read shares its deadline and noncloneable attempt/body ledger. Budgeted requests continue to refuse redirects. The resolved workspace/all-feature reqwest graph contains neither http2 nor http3, and the pinned ProtocolNacks classifier cannot retry without those features. Preserve that existing protocol selection with a resolved-feature census; do not enable protocols solely for proof or change current client/mutation retry behavior on an unreachable premise. Confirm enabled-transport physical requests and substrate retries with independent server logs, not just a logical-call counter. A future protocol change must invalidate/reprove this premise before passing the fence.

A PR facts read normally needs one PR response, or its permitted retry/revalidation; it does not manufacture parent/fork calls to use ten permits. Lower-one-attempt tests exercise the real public read. Shared substrate tests exercise the ledger's hard boundary independently. No promise of ten useful pages or eight retries is introduced.

Response-body admission applies the effective cap during body reading. Cumulative admission charges bodies used for decoding, including a revalidated cached body once; the 304 body itself is zero. Current lower limits also constrain reused cached bodies, so a prior high-limit read cannot bypass them. These are admitted-body counters, not wire-byte counters. A rejected response keeps its consumed attempt/time. Finish decode, validation and bounded serialization before final cancellation/deadline acceptance; no late result publishes.

Serialize directly into a cap-enforcing writer/Vec; do not create an unrestricted full document and truncate or validate after publication. Include every envelope byte and JSON escaping expansion. Avoid an additional serde_json::Value tree and duplicate body copies where the existing content ownership allows moves/borrows. Numerical caps are not peak-memory promises; measure actual allocations separately in the implementation checkpoint.

Extend existing ResourceError with optional private, typed bounded details and accessors. Keep its stable categories and detail-less constructor; no second operational error enum. Core detail values represent reason, HTTP status, access ambiguity, safe parsed retry/reset guidance and an optional limit fact (dimension/effective bound/observed or requested value where known). Limit and reason vocabularies are finite; no arbitrary metadata maps, headers, URLs, provider bodies or unbounded prose enter details.

Initial reason vocabulary: `invalid_acquisition_limit`, `acquisition_controls_unsupported`, `deployment_identity_unavailable`, `repository_not_authorized`, `upstream_not_found_or_hidden`, `upstream_denied`, `upstream_rate_limited`, `upstream_unavailable`, `upstream_malformed`, `upstream_identity_mismatch`, `transport_failure`, `limit_exceeded`, `deadline_exceeded`, `cancelled`. Limit dimensions distinguish attempts, response body, accepted bodies, representation and elapsed time. Extend only when a distinct first-slice outcome actually requires it, with the same category and boundedness obligations.

404 remains `not_found` plus missing-or-access-hidden ambiguity; 401 and nonsignaled 403 are permission denial, not proof of credential expiration; 429 and signaled primary-limit 403 are source unavailable/rate limited. Safe guidance is parsed numerically/date-wise, never inferred from message text. Unclassified failures retain honest generic categories/reasons. Current facts errors use bounded sanitized messages; raw decode/error bodies cannot leak via structured or text output.

One MCP-owned error serializer maps ResourceError details for read, discovery and mutation error envelopes, removing duplicate category/message DTOs without changing absent-detail wire output. Profile configuration failures retain existing ConfigurationError/ProfileError handling. Do not leak rmcp or schema types into core/sources.

## Input shapes

The following disjoint groups cover the inputs this slice changes. Independent optional fields use the complete stated presence/type matrix; a cross-product does not gain a new semantic case merely by combining independent omissions.

| Shapes | Status |
|---|---|
| R1: every existing PullRequestResource variant plus Facts; root/collection/new/item; valid canonical owner/repo/positive number, case aliases, malformed number, zero/overflow, extra segment; `/facts` versus existing `/diff`, `/diff/N`, selectors; source dispatch Workspace/Artifact/Local/Https/Jira/Issue/PullRequest/Catalog. | C03; existing unchanged grammar cases retain their fences. Commit/path escaping operands are N/A here: rfs-jrz7/rfs-e5cv. |
| D1: public default/explicit matching/mismatch; ghe tenant matching/mismatch/wrong scheme/port/path/userinfo/query/fragment/invalid tenant; custom API with/without explicit valid web identity; required/optional and normal/degraded mount. | C04, C17. |
| A1: acquisition absent, empty object, each member absent/present, null/wrong type/negative/fraction/zero/one/exact hard/above hard/overflow; caller below/equal/above a valid operator ceiling; independent unequal dimensions; supported facts versus unsupported resources. | C03, C08. |
| I1: PR ID/number zero/positive/overflow/malformed/missing; node ID absent/null/value; requested and observed case-equivalent identities versus wrong parent/owner/repo/number; distinct head/base/fork/synthetic merge identities; full40/short/uppercase/nonhex SHA. | C05, C06. |
| P1: each optional body/author/time/flag/link/ref/repository field absent, explicit null, correct value or wrong JSON type; strings empty/ASCII/Unicode/escaped/newline/large; known and unknown state; author/repository objects with complete or partial optional native fields; head repo null/deleted; empty/multiple distinct/multiple duplicate link keys or object properties. Required identity missing/malformed is a rejection; duplicate recognized identity keys are rejected rather than last-value-wins. | C05, C06, C12. Arbitrary record arrays are N/A: this is singular; rfs-bfwa owns collection matrices. |
| H1: fresh200; unchanged304 with valid cache, changed200, 304 without usable cache, generation/principal change, failed revalidation; absent/valid/malformed validators/time/version headers; provider time different from HTTP time and acquisition time. | C07, C15. |
| T1: no retry, permitted enabled-transport/retryable-status retry, exhausted one-attempt limit, boundary shared budget; refused same/foreign-origin redirect; denied repo/fork/credential-unowned target; positive authorized egress controls. Protocol-NACK retries are N/A — unreachable in the resolved production feature graph; C09 mechanically fences that premise rather than enabling protocols for a test. | C09, C14, C15. |
| T2: deadline during send/body/wait/decode/serialization; absent/usable/too-long retry guidance; cancellation before egress, mid-read and immediately before acceptance; live versus cancelled session. | C10, C11, C15. |
| B1: body empty/valid/exact effective/plus one/truncated/invalid UTF-8 or JSON; admitted total exact/plus one; reused cache under lower limits; representation below/exact/plus one including Unicode escaping and envelope overhead. | C12, C13. |
| E1: 401, plain403, rate-signaled403, hidden404,429,503,other5xx, transport, malformed identity/JSON, limits and explicit cancellation; safe/malformed/missing status/retry/reset facts; malicious provider error text/header values. | C15. |
| O1: small inline document, exact inline boundary, multipage output recovery, cancellation/expiry during recovery; source continuation absent for singular facts; major1 plus unknown optional field/native state, unsupported major. | C06, C13, C16. |
| L1: native unpaged diff with valid UTF-8 exact bytes, missing terminal newline, truncated body, invalid UTF-8; Accept captured independently; human text projections and mutation refusals with grants both absent and enabled. | C03, C18. |
| V1: public authorized live source/MCP run versus absent gate; loopback real stdio under test-only trust versus ordinary shipped launch; fixture authorization/log positive controls and real public native links. | C14, C16, C17, C19, C20; C20 alone owns fixture-trust selection and propagation. |

## Placement

| Capability | Owner | Seam and forbidden ownership |
|---|---|---|
| Canonical Facts variant | core/reference.rs | Existing typed reference parser/canonical renderer; no HTTP/JSON or provider DTOs. |
| Acquisition values and operational detail facts | core/acquisition.rs; core/error.rs | Pure validating values through existing read/error seam; no provider state, credentials, serde_json, rmcp or policy engine. |
| Deployment pairing/operator controls | sources/configuration/github.rs | Existing validated configuration interface; no sign-in lifecycle, network probing or profile syntax. |
| Native decode and owned facts serialization | sources/github/facts.rs | Private concrete read entrypoint reached by GithubSource; no public provider trait, human fallback, or transport/cache duplication. |
| Native identity/link validation | sources/github/facts/identity.rs | One private `validate` entrypoint returns six borrowed validated operands; no HTTP, cache, clock, or serialization responsibility. |
| Fetch/cache/observation metadata | sources/github/fetch.rs | Private existing cache/fetch seam extracted once and reused by human/facts/mutation consumers; no owned facts schema or review verdicts. |
| Physical requests, deadlines, admission | sources/http/{mod.rs,read.rs} | Existing substrate and BoundedRead; no GitHub/Jira JSON, source identity or provider error prose matching. |
| MCP acquisition syntax | mcp/acquisition.rs | Shared concrete DTO/validation bridge used by read and profile; no network or owned GitHub schema. |
| MCP read parsing/dispatch | mcp/server/read.rs | Private child methods; callers/tests still tools/call and ReadEngine. Parent retains macro tool registration and dispatch wiring only. |
| Safe error JSON/schema | mcp/render/error.rs | One private serializer from ResourceError; no competing operational error type or source-specific classification. |
| Profile/launch/catalog | Existing profile model/convert/mount and launch; GithubSource catalog | Existing owners only; no deployment policy in schema generator, no GitHub grammar hardcoded in MCP. Test CA is inaccessible through production profile/CLI/env. |

## Module shape

### Alternatives and selected seams

**Read controls:** (A, selected) explicit optional controls on SourceAdapter::read, e.g. `source.read(&path, &guard, Some(&limits))`; hides validated lowering and admission while callers keep their current source seam. Seven real adapters already exist; unsupported explicit control has an honest error. (B) enlarge OperationGuard with acquisition state, e.g. `guard.with_limits(...); source.read(&path, &guard)`; trivial common caller but mixes cancellation/commit authority with read policy and touches unrelated mutation/discovery semantics. (C) add a `FactProvider`/`BoundedSource` extension trait and a second call path; flexible dispatch but one real new adapter and duplicated consumption contract. Reject B/C for ownership and hypothetical-seam cost.

**GitHub fact/fetch ownership:** (A) put facts in the existing parent next to human read arms; caller trivial but concentrates decode/schema/ledger/cache in a 1,498-line orchestrator. (B) put one private facts entrypoint behind the source and continue sharing parent cache methods; hides semantics but leaves all conditional metadata changes in the parent. (C, selected) private facts module plus extraction of the existing fetch/cache cluster into private fetch.rs; source caller remains one dispatch, human and machine paths reuse one acquisition owner, and schemas evolve without changing human DTOs. No generic private trait is introduced. The extraction is substantive: deleting fetch.rs puts conditional-cache/metadata machinery back into multiple projection owners; deleting facts.rs puts native/owned-schema/identity complexity back into the parent.

**MCP placement:** (A) extend server/render parents inline; easiest caller but new parsing/detail responsibilities grow both orchestrators. (B, selected) extract read parsing/dispatch to server/read.rs, share acquisition syntax between read/profile, and put common error serialization in render/error.rs; real stdio tests cross the same tool interface and parents retain only their existing other tools and registration. (C) create a general tool middleware/context framework; maximizes hypothetical extension but broadens every tool without a second concrete requirement. B has the smallest necessary responsibility moves.

Deployment/core-error/HTTP changes fit existing concrete owners; N/A — no competing public seam. Apply deletion, same-interface test, real-adapter and locality tests to each selected owner above. The new MCP acquisition DTO has two actual consumers (read and profile), and deleting it duplicates closed-object/null/numeric-schema handling. Private facts/fetch modules are concrete, not generic seams requiring invented adapters.

### Current inventory and module ledger

Counts below are measured at the source base. `P/L` means physical lines / nonblank lexical production lines from the active nae2 `production()` mask. L masks comments/literals and ordinary inline cfg(test) modules; it is **not** compiler-expanded LOC and does not exclude cfg(all(test, unix)) or test-support features. P is the active growth-tripwire metric. Full path prefixes: C=`crates/resourcefs-core/src/`, S=`crates/resourcefs-sources/src/`, M=`crates/resourcefs-mcp/src/`.

| Module (P/L baseline) | Interface / owns / dependencies | Adapters and test interface | Change / forbidden |
|---|---|---|---|
| C source.rs (20/10) | SourceAdapter::read; pure core + async_trait | Seven implementations; read_engine_contract and direct source tests | Deepen controls parameter; no provider trait. |
| C read.rs (365/318) | ReadRequest/ReadEngine/TextLimits; source invocation, recovery, seen regions; core only | dyn SourceAdapter; read_engine_contract | Deepen request/wiring only; no acquisition implementation. |
| C error.rs (78/66) | Stable category/ResourceError plus typed details; std/core | All adapters; public error-value and MCP contracts | Deepen values; no JSON/provider response parsing. |
| C reference.rs (1942/1650) | Typed grammar/canonical form; core/url | All source consumers; github_reference_contract | Retain owner/add Facts; no acquisition. |
| C lib.rs (78/74) | Public contract exports | External consumers | Wiring only. |
| C acquisition.rs (new) | Validated limits/intersection | Rust consumers, MCP conversion, sources; public control contracts | Create pure owner; no deadlines/HTTP engines. |
| S github/mod.rs (1498/1354) | Mount/source/read/discovery/catalog orchestration and existing projections | GithubSource; github_adapter_contract/live | Split fetch; facts dispatch/catalog only; no new decoder/serializer bodies. |
| S github/fetch.rs (new) | Existing conditional fetch/cache plus observation metadata | Human and facts/mutation paths; public adapter/cache contracts | Create by extraction/deepen; no parallel cache or facts schema. |
| S github/facts.rs (new) | Private PR facts read, presence and owned schema | GithubSource::read; github_facts_contract | Create; no reqwest, generic provider trait or human rendering. |
| S github/facts/identity.rs (new) | Native identity and recognizable-link consistency | facts.rs through one `validate` function; same public adapter contracts | Private child; borrowed result only, no HTTP/cache/clock/serialization. |
| S github/wire.rs (390/362) | Legacy private wire DTO/decode | Human/mutation consumers; github_wire_contract | Retain; no new facts DTOs or global nullability tightening. |
| S github/render.rs (245/229) | Legacy text/order rendering | Human consumers; adapter/wire contracts | Retain unchanged; no machine serialization. |
| S github/mutation.rs (1317/1264) | Existing mutation routing/authority/cache invalidation | MutationAdapter; github_mutation_contract | Wiring for extracted fetch and explicit Facts refusal only; no new write capability. |
| S http/mod.rs (1491/954) | Sole reqwest/policy/credential/body/header owner | HttpSubstrate; http_substrate/http_bounds contracts | Deepen existing metadata/effective-cap bodies only; no provider semantics or retry-policy change. |
| S http/read.rs (194/172) | Shared deadline/attempt/retry/admission ledger | Existing BoundedRead consumers; read_tests + adapter request observations | Deepen controls; no separate retry clock in GitHub. |
| S http/request.rs (228/193) | Validated request/header/redirect/replay intent | All HTTP consumers; existing request contracts | Retain unless mechanical read-limit plumbing requires it; no provider decode. |
| S http/extract.rs (356/246) | Existing HTML projection | HttpsSource; http_extract_contract | Retain unchanged. |
| S configuration/github.rs (127/105) | GithubRepository/GithubConfig/GithubDeployment validation | Embedder/profile; configuration_contract | Deepen same owner; no credentials lifecycle or schema DTO. |
| S configuration/mod.rs (91/75) | Config exports/error/shared validation | Configuration consumers | Export wiring only. |
| S configuration/https.rs (266/223) | Existing HTTPS URL validation reused by GitHub | Config consumers; configuration_contract | Retain unchanged; do not weaken URL policy for fixtures. |
| S compiled.rs (471/284) | Catalog and concrete source dispatch | Six mounted source instances; compiled_sources_contract | Controls forwarding/catalog override refusal only. |
| S artifact.rs (319/280) | Immutable session artifacts/recovery | SourceAdapter; artifact_adapter_contract | Signature/explicit override refusal only. |
| S filesystem.rs (2963/2696) | Contained filesystem/root authority | SourceAdapter; filesystem contracts | Signature/explicit override refusal only; no filesystem refactor. |
| S https.rs (221/167) | Existing HTTP raw/reader-mode reads | SourceAdapter; HTTPS contracts/live | Signature/explicit override refusal only. |
| S local.rs (400/342) | Session-local scratch reads/mutations | SourceAdapter; local contracts | Signature/explicit override refusal only. |
| S atlassian/jira.rs (292/276) | Existing Jira read/discovery facade | SourceAdapter; Jira contracts | Signature/refusal/internal-call migration only; child owners unchanged. |
| S catalog.rs (563/193) | Generic validated catalog representation | CompiledSources; catalog contracts | Retain unchanged; GitHub grammar remains source-owned. |
| S lib.rs (78/75) | Existing source/config exports | External consumers | Export wiring only. |
| M acquisition.rs (new) | Closed acquisition DTO, schema and validated conversion | Read/profile; real MCP/profile contracts | Create; no provider or transport behavior. |
| M server.rs (1431/1317) | Tool registration/session/root lifecycle/other tool dispatch | Real stdio | Split read owner; only imports/registration/delegation change. |
| M server/read.rs (new) | Existing ReadInput/display validation/read dispatch plus acquisition | Real tools/call and ReadEngine | Create by extraction/deepen; no facts decoder/HTTP. |
| M render.rs (1577/625) | Existing read/mutation/discovery rendering | Actual tool outputs | Extract common error DTO/conversion; no new classification body. |
| M render/error.rs (new) | One safe category/message/details serializer + schema | All three tool-result families | Create; no message parsing/duplicate error taxonomy. |
| M profile/model.rs (1815/1498) | Profile DTO/static metadata/into_parts | Profile/CLI schema; profile/schema contracts | Add GitHub fields/typed conversion wiring only. |
| M profile/convert.rs (269/257) | DTO→validated configurations | Profile loader; profile_contract | Construct deployment/limits; no duplicate policy. |
| M profile/mod.rs (266/222) | Check/mount orchestration and substrate construction | Launch; profile/stdout contracts | Thread test trust to GitHub; no second HTTP client. |
| M launch.rs (242/203) | Launch admission/trust routing | Shipped launch and test-support | Reuse one internal HTTP-trust choice for HTTPS/GitHub; no external selector. |
| M test_support.rs (27/17) | Gated existing serve_profile_with_https_root | Reexecuted actual stdio test harness | Retain helper; HTTPS root also reaches GitHub's HTTPS substrate. |
| M lib.rs (21/16) | Private module declarations and gated test support | Embedder/CLI | Wiring only. |
| M profile/check.rs (595/501) | Static/dependency/secret/TCP checks | rfs check/profile contracts | Retain owner/behavior; pairing is validated configuration, not a successful TCP probe claim. |
| M profile/schema.rs (49/43) | Generated schema from ProfileDocument | Actual CLI schema | Retain generation; update expected published schema, not duplicate fields manually. |

### Protected parents and exit conditions

| Parent | Allowed | Forbidden | Checkable exit |
|---|---|---|---|
| C reference.rs | Facts variant/parser/canonical match | Wire/HTTP implementation/new infrastructure | Existing 1,989-line cap; old unrelated symbols/bodies retained. |
| C read.rs / source.rs | Request field and explicit forwarding/interface | Acquisition engine or provider knowledge | No provider/deadline implementation imports; documented interface census. |
| S github/mod.rs | Fetch extraction, facts dispatch/catalog and needed imports | New facts DTOs/decoder/serializer/cache duplicate | Net shrink from 1,498; moved fetch ownership unique; no Facts schema body. |
| S github/mutation.rs | Match refusal and moved-helper references | New mutation/retry/authority semantics | Unrelated bodies unchanged; Facts never resolves a write target. |
| S http/mod.rs | Fetch metadata and bounded-reader effective-limit plumbing | New provider knowledge, client retry-policy change or new responsibility modules hidden in facade | Keep inherited physical ceiling/net-shrink rule; exact approved function-change allowlist. |
| S atlassian/jira.rs | Signature/refusal/internal read call | Jira query/browse body changes | Keep inherited cap/owners; exact changed-symbol allowlist. |
| M server.rs | Read extraction/imports/tool registration delegation | New request processing, profile or transport bodies | Net shrink from 1,431; all unrelated production tokens retain old freeze. |
| M render.rs | Common error extraction/calls | Provider status classification | No duplicate category/message/detail DTOs; no new responsibility body. |
| M profile/model.rs | Two GitHub fields and into_parts/static metadata wiring | Pairing/network/error-policy implementation | Allowed symbol/field delta only; no new loops/transport imports. |
| Export/config/mount facades | Named declarations/construction/trust forwarding | Extra providers, generic middleware or credentials lifecycle | Census matches the ledger; existing trust cannot be selected by CLI/profile/env. |

Physical growth tripwires for new owners are provisional placement signals: core/acquisition 300; github/facts 750; github/fetch 650; MCP acquisition 200; server/read 400; render/error 300. The budgeted plan owns exact per-checkpoint projections, existing-owner deltas and measurements. Exceeding a tripwire requires responsibility review, not padding/splitting; unchanged ownership permits a justified plan correction, changed ownership requires reapproval.

### Enforced successor transition

Add `.rfs-0n97/oracles/module_shape.py` and a data manifest; make the runner invoke this successor instead of the incompatible bare nae2 query invocation. Preserve inherited h212 and all unchanged nae2 assertions.

The current checker has no overlay parameter. Add an explicit typed/data policy input for approved transition deltas, not failure-string filtering. The successor supplies only this ledger's new paths, extracted owners, and exact affected declarations/functions. Default historical invocation keeps its existing behavior. The successor verifies these deltas and calls the inherited checks with the narrow transition policy. Specifically:

- Extend, do not replace, the global admitted-file inventory.
- Replace server's blanket freeze only for the named read extraction/import/registration nodes; compare every remaining production token against the existing baseline.
- Admit only named HTTP/control/grammar plumbing changes; preserve parent ceilings, net-shrink, forbidden dependencies, unique symbol ownership and all untouched bodies.
- Keep old Jira/query/request-intent guarantees and test-child mounting restrictions; no broad `github/**` or `mcp/**` exemption.
- Discover upstream/default refs with the existing mechanism; retain pinned baseline evidence explicitly. No hardcoded main/origin assumption.
- Report C02 plus exact path/symbol/delta. Mutations for a new parent body, forbidden dependency, unlisted file and changed unrelated frozen body must each turn red; restoring source turns green.

No new behavioral production test reads source text. The structural oracle is non-production. Final conformance requires fresh-context reconstruction by a nonimplementing reviewer, followed by comparison with this ledger.

## Claims and falsification

Each row's falsifier names its decisive control. **All PENDING rows are owned by Main at checkpointed-build, assigned to the implementing checkpoint by plan.md.** A baseline PASS does not discharge the assembled-tree rerun. Named test files below are intended permanent owners for genuinely uncertain identity/limit/authority/error behavior, not a request to pin wording or private DTO layout.

| ID / claim | Shapes | Falsifier and confounder control | Independent oracle | Named mutation → expected red | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|
| C01: Core/profile/source dependency direction remains intact. | Placement | Run actual dependency fence; confirms existing allowed provider serde is present so an empty scan cannot explain success. | cargo metadata plus source ownership census, independent of runtime decode | Add actual serde_json decoding in C error.rs and its dependency → source-neutral-core diagnostic. | Existing architecture_contract::enforces_dependency_direction | Seconds after build | PASS — baseline run below; assembled rerun required |
| C02: Approved module ownership and protected-parent constraints remain enforced. | Placement | Successor census plus four source mutations; each must fail its own path/rule, then restore green; baseline-only file count cannot substitute. | Git baseline/diff/symbol census independent of production behavior | Put facts serializer body in S github/mod.rs; separately unlisted file/foreign import/unrelated frozen body edit → localized C02 failures. | Issue-local successor module_shape.py and inherited assertions | Seconds | PENDING — Main, structural checkpoint |
| C03: Facts route is canonical/read-only and explicit unsupported overrides never disappear. | R1,A1,L1 | Parse/canonical/read/refuse matrix through public interfaces; authorized legacy read/mutation-field positive controls isolate routing rather than blanket denial. | Expected path cases and independent upstream request log | Route Facts to Aggregate, omit one concrete adapter override refusal, or permit Facts mutation target → wrong shape/result or observed write-target acceptance. | Core github_reference_contract; public source override and facts mutation-refusal contracts | Seconds | PENDING — Main, interface/facts checkpoints |
| C04: Qualified deployment configuration cannot pair the wrong corporate web/API authority. | D1 | Public/ghe/custom corpus through Rust and actual profile conversion; matching pairs succeed and mismatches fail before credential resolution/egress. | Explicit hostname-pair corpus and positive listener | Remove ghe pair comparison in configuration/github.rs → mismatched profile accepted. | configuration_contract + profile_contract deployment cases | Seconds | PENDING — Main, configuration checkpoint |
| C05: PR facts retain actual distinct identities and reject contradictions. | I1,P1 | Independent server supplies unequal PR/head/base/fork/merge identities and wrong-parent variants; assert exact owned facts and request operands, with valid near-neighbor fixture succeeding. | Hand-authored native fixture plus external JSON comparison, supported by live P1 observations | Replace head SHA/repository with base or merge identity in facts.rs; omit parent equality check → wrong identity accepted. | github_facts_contract identity cases | Seconds | PENDING — Main, facts checkpoint |
| C06: Presence, precision and owned schema version meanings survive consumption. | P1,O1,I1 | Parse complete returned JSON independently; null/absent/empty/unknown and >2^53 native IDs remain distinguishable. The same external consumer succeeds on actual major1 and additive fields but explicitly refuses production-emitted major2. | External Python/jq consumer and explicit presence/type matrix | In facts.rs default nullable body to empty, cast ID via f64, coerce unknown state, or change emitted schemaVersion.major from1 to2 → value mismatch or normally successful consumer fails explicitly as unsupported major. | github_facts_contract presence/precision; retained actual-consumer compatibility contract | Seconds | PENDING — Main, facts checkpoint |
| C07: Provenance distinguishes body observation, revalidation and native times without stale rescue. | H1 | Fixture sends distinct PR time/Last-Modified/Date, changed200/304/error/generation transitions; a normal valid304 succeeds, failed revalidation never publishes stale bytes. | Server transcript and independent final-byte SHA256 | Overwrite body timestamp from304 or return cached body on failed revalidation in fetch.rs → wrong provenance/stale success. | github_facts_contract conditional provenance cases | Seconds | PENDING — Main, fetch checkpoint |
| C08: Valid caller controls lower operator policy; malformed controls fail before acquisition. | A1 | Exercise every dimension independently below/equal/above operator and hard maxima through Rust and real tools/call; valid authorized request proves listener can observe egress. | Explicit min arithmetic and real listener plus protocol response | Use max instead of min, accept zero, or validate only generated helper not dispatch → wrong effective limit/egress. | Public acquisition value contracts and stdio acquisition cases | Seconds | PENDING — Main, controls/MCP checkpoints |
| C09: Physical attempts obey one ledger under the fenced transport feature set. | T1 | Server-observed requests for lower-one retry and shared hard budget, with retry-allowed positive controls. Separately resolve all-feature dependency graph and require no http2/http3; do not claim an unreachable NACK experiment. | Independent HTTP server transcript plus Cargo's resolved feature graph | Skip attempt charge before retry in http/read.rs → excess observed request; separately enable reqwest http2 in workspace Cargo.toml → C09 feature-premise census fails. | HTTP budget regression, public facts attempt case and issue-local resolved-feature oracle | Seconds | PENDING — Main, transport checkpoint |
| C10: One deadline covers send, body, waits and final acceptance. | T2 | Distinct barriers/delays at each stage and a completing control; relaxed unrelated byte/attempt caps prevent another guard explaining termination. | Independent server timestamps plus monotonic elapsed observation | Restart deadline on retry or omit final deadline check → late accepted result. | HTTP/facts deadline boundary contracts | Seconds | PENDING — Main, transport/facts checkpoints |
| C11: Explicit cancellation cannot publish a late current result. | T2,O1 | Cancel at controlled egress/body/final-acceptance barriers; same data without cancel succeeds, and earlier completed independent read remains usable. | Operation lifecycle and real response barrier, not source counter | Omit final operation-active check in facts read → cancelled read succeeds. | Public source cancellation and engine/session contracts | Seconds | PENDING — Main, facts/MCP checkpoints |
| C12: Effective response and cumulative admitted-body limits cannot be bypassed by cache reuse or ledger reset. | B1,P1 | Real PR/cache exact/plus-one per-body and lower-limit cases, with competing caps relaxed. Separately drive the production shared ledger through multiple real HTTP bodies: two individually admissible 8 MiB bodies reach16 MiB, then a one-byte admission fails; also use smaller unequal bodies and lowered totals. No extra PR fetches are invented. | Independent server byte vectors/summed counts; positive individual admissions prove per-body guard is not the cause. | Use substrate default instead of effective cap, omit cache admission, or reset accumulated accepted bytes before each admission in http/read.rs → oversize/cache or third-body acceptance contradicts oracle. | HTTP multi-body shared-ledger contract plus github_facts_contract admission cases | Seconds | PENDING — Main, transport/facts checkpoints |
| C13: Complete serialized JSON including metadata fits the effective representation cap. | B1,O1 | Construct JSON-escaping/envelope cases at exact/plus one while response cap is safely higher; parsed accepted bytes fit, over-bound result is error, not truncated JSON. | External serializer/byte counter over expected values | Count only body/data bytes or check cap before envelope completion → oversize success. | github_facts_contract representation boundary | Seconds | PENDING — Main, facts checkpoint |
| C14: New read paths cannot enlarge authority or perform remote writes. | T1,V1 | Allowed and denied repository/origin/credential/redirect/fork targets; authorized listener positive control and normal request barrier make zero forbidden requests decisive. | Independent multi-origin method/credential logs | Follow returned head link or attach origin credential to redirected target → forbidden request/method observed. | GitHub/HTTP authority contracts and real stdio fixture | Seconds | PENDING — Main, source/MCP checkpoints |
| C15: Error categories/details and retry guidance are bounded, honest and machine-derived. | E1,H1,T1,T2 | Status/header matrix with identical misleading prose, plus secret sentinels and valid success/retry controls; assert safe details, distinct ambiguity and request timestamps. | Explicit status/header policy table and server log | Parse message for rate status, label404 absent, leak raw body, or ignore applicable retry wait → category/detail/sentinel/timestamp failure. | GitHub error contracts, HTTP retry contracts, actual MCP error output | Seconds | PENDING — Main, error/transport/MCP checkpoints |
| C16: Actual MCP recovery reconstructs the acquired JSON, not a new upstream read. | O1,V1 | Force real inline limits below facts size, follow actual recovery chain, compare all bytes then parse; initial authorized GET is positive control, recovery adds no provider requests. | Independent byte collector/parser and upstream transcript | Drop recovery chunk, reread source during recovery or confuse source continuation → byte/request mismatch. | stdio GitHub facts recovery contract | Seconds | PENDING — Main, MCP checkpoint |
| C17: Published profile/tool/catalog contracts expose the implemented feature. | D1,V1 | Run actual CLI schema/check and tools/list/catalog against the implemented fields/grammar; invalid neighboring inputs fail and valid ones reach the intended source. TCP probe alone is not acceptance. | CLI/JSON-RPC process outputs and independent accepted/rejected input corpus | Omit acquisition schema field or catalog Facts grammar → concrete protocol/config assertion fails. | schema/profile/catalog and stdio contracts | Seconds | PENDING — Main, final feature checkpoint |
| C18: Legacy native unpaged diff remains media-correct and byte-exact, refusing truncation. | L1 | Public source read; independent server captures native Accept and serves newline-sensitive diff then truncation/invalid UTF-8; valid control succeeds. | Original server byte vector and captured Accept | Switch Accept to JSON, rebuild from file patches or ignore truncation → media/byte/refusal failure. | github_adapter_contract dedicated native diff case | Seconds | PENDING — Main, fetch checkpoint |
| C19: Reusable live evidence exercises actual source and MCP facts under the selected REST version. | V1 | Gated read-only public source and real-binary stdio run, compare identities/links to independent provider observation; absent gates explicitly skip rather than pass. | Independent gh/jq observation against the same authorized public PR | Substitute synthetic/base identity in facts.rs → deterministic identity fence red; live row also compares shape/identity when enabled. | github_live_smoke + stdio_live_smoke, backed by deterministic identity fences | Network/credentials | PENDING — Main, assembled separate live gate |
| C20: Gated fixture trust reaches GitHub without a production trust selector. | V1 trust branch | Through the existing helper, real stdio reads a GitHub projection from the TLS fixture; the same ordinary launch rejects the untrusted certificate and profile/env cannot select its CA. Authorized HTTPS/GitHub controls prove both mount paths run. | Real process outcomes and independent TLS listener transcript | In launch/profile mount code drop GitHub fixture root, or accept a test-root production profile/env field → fixture read fails or ordinary launch wrongly succeeds. | Existing architecture trust fence plus real stdio GitHub fixture contract | Seconds | PENDING — Main, MCP preparation checkpoint |

## Non-goals and future work

Permanent non-goals for ResourceFS here: capture/review verdicts, a generic provider framework, duplicate Cyril HTTP client, credential minting/sign-in, whole-PR orchestration, speculative fork fetches, synthetic merge substitution, mutation enablement, pretending numeric bounds are wire/peak-memory bounds, or claiming corporate/native acceptance from public fixtures. These belong to the settled ownership contract, not unfiled work.

Verified subsequent slices: rfs-bfwa conversation collections/individual comments/partial retention/cursors; rfs-r31i reviews/inline anchors; rfs-jrz7 exact commit/source; rfs-iktl PR commits/files; rfs-e5cv direct-tree comparison; rfs-3r0s provider merge-base comparison; rfs-nchb checks/statuses. Corporate/native consumer acceptance remains with cyril-9qn7 and its existing cyril-wcxv prerequisite; no parent/tracker reconciliation is performed by this ticket. No new scope is hidden behind an untracked deferral.

## Falsifier run log

C01 cheapest runnable implementation-independent placement invariant:

```text
cwd: /home/dwalleck/repos/resourcefs-wt-rfs-0n97-impl
cargo test -p resourcefs-mcp --test architecture_contract enforces_dependency_direction -- --exact
```

Observed: compiled the actual workspace crates; one test passed, zero failed/ignored, six filtered out; test execution 0.18 s, command wall time 10.82 s. This proves the baseline dependency invariant, not future facts behavior. P1's fresh independent provider comparison remains recorded separately in evidence.md. Every other falsifier awaits its real implementation; none is marked PASS from a model or proposed test.

Preapproval independent review found one cumulative-accounting falsifier gap; C12 now requires a real multi-body shared-ledger driver that distinguishes accumulation from an individual cap. C06 now mutates the production-emitted major, not the consumer oracle. Separate resolved-feature inspection corrected the initial hidden-NACK assumption and C09 now fences the actual feature premise. These are technical proof corrections before approval; no feature scope, protocol capability or mutation behavior was added.

Independent reviewer `PrFactsDesignGateReview` re-read the corrected scope and returned **PASS**, no remaining findings: C09's feature premise, C12's cumulative driver and C06's emitted-major mutation are now decisive design obligations. This was document review only, not implementation validation or requester approval.

Planning clarification: C17's already-approved fixture-trust subcheck is separated as C20 so the real GitHub stdio harness can be proved against an existing human projection before Facts exists. C17 retains published profile/tool/catalog exposure. No behavior, oracle meaning, ownership, interface or risk changed; the original approval applies.

## Approval

Requester approval, 2026-09-08: **"Approve and implement"**, selected in response to the explicit question approving this design, its module ledger and protected-parent fence transition. Approved risk acceptances: **None**. Proceed to budgeted-plan, then checkpointed implementation; this approval does not imply commits, pushes or PR creation.

Specific choices presented for approval: explicit optional read controls with honest unsupported-resource refusal; decimal-string native IDs/numbers and major/minor owned schema; validated public/ghe pairing with non-guessed custom-base compatibility; one shared fetch/cache extraction; the narrow successor shape-fence transition; and test-only GitHub CA propagation through the real stdio path.

Approved risk acceptances: **None.** No regression fence is waived. Numerical/transport/live proof obligations remain mandatory, and unavailable corporate/native inputs do not become a pass.

### Approved implementation placement revision

The formatted Facts implementation reached 952 physical lines and failed the approved 750-line tripwire. Responsibility review found no honest 202-line deletion: the remaining schema and presence declarations encode contract fields. The requester selected **"Extract private identity child"** after being offered that seam versus explicitly revising the single-file limit.

The approved revision moves identity/link helpers and validation orchestration into `github/facts/identity.rs`, with one private `validate` entrypoint returning `ValidatedIdentity` containing six borrowed operands. The parent retains native decoding, owned schema, acquisition orchestration, bounded serialization and publication checks. Limits remain 750 lines for the parent and 350 for the child. The placement fence requires the moved symbols in the child, rejects their return to the parent, restricts the child to one exposed function, and forbids HTTP/cache/clock/serialization dependencies there. No behavior, risk acceptance, or other ownership changes.
