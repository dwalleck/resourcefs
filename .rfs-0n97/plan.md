# Plan: rfs-0n97

## Inputs, ownership and approval

Approved design: design.md, requester **"Approve and implement"**, 2026-09-08; no risk waivers. Route is Empirical; evidence.md owns P1/P2 comparisons and the corrected transport-feature premise. C01 baseline dependency fence passed; all new implementation obligations remain pending. This plan declares no slice complete.

Main owns integration/checkpoints/evidence. Writers use separate verified Git worktrees and skip all validation while concurrent; Main applies isolated patches in the order below, then runs each owning focused checkpoint. No shared target directory or direct edits to another writer's checkout. On 2026-09-08 the requester selected **"Local checkpoint commits only"**: Main may commit verified slices locally; no pushes, PR creation, merges or tracker closure. Writers make no commits. The unavailable OMP `task.isolation.mode` setting is not isolation proof: explicit Git worktrees are the fallback, verified by their own top level/common directory.

Observed remote symbolic default: `refs/remotes/origin/HEAD` targets `refs/remotes/origin/main`, discovered with `git for-each-ref --format='%(refname:short) %(symref)' refs/remotes`. Source baseline remains pinned at 7de8d1477af6be825bada9015a6996b3f2917c7d. Increments below are an implementation/review partition, not publication authorization.

Formal claim ownership: S0 C02; S1 C01; S2 C04; S3 C09; S4 C18; S5 C20; S6 C03,C05,C06,C07,C08,C10,C11,C12,C13,C14,C15,C16,C17,C19. Earlier slices create and exercise supporting controls/fences immediately; composite source/MCP claims complete only when their real consumer exists in S6. Their final checkpoints retain applicable earlier evidence and exercise the remaining consumer path, never postpone a newly implemented mechanism's own fence.

## Cross-writer contracts

These are incidental Rust spellings implementing the approved interfaces, not new ownership decisions.

- `SourceAdapter::read(&self, reference: &PathReference, operation: &OperationGuard, acquisition: Option<&ReadAcquisitionLimits>)`. `ReadRequest.acquisition: Option<ReadAcquisitionLimits>`. Existing read-engine method shape stays unchanged. All current concrete resources reject explicit overrides until the Facts implementation exists; CompiledSources forwards to the chosen source and refuses catalog overrides itself. No new Facts variant or advertised placeholder in preparatory slices.
- `ReadAcquisitionLimits` is Copy/Clone/Eq with private positive fields, hard-default Default, `new(max_attempts: Option<usize>, timeout: Option<Duration>, max_response_bytes: Option<usize>, max_accepted_body_bytes: Option<usize>, max_representation_bytes: Option<usize>) -> Result<Self, ResourceError>`, matching getters and `intersect(self, other: Self) -> Self`. Hard limits are design.md's five values. No new serde/schema/provider dependencies in core.
- `ErrorReason` carries the design vocabulary and `as_str()`. `ResourceErrorDetails::new(reason)` has private optional fields and typed consuming builders/accessors. `HttpStatus::new(u16)` validates the HTTP three-digit range 100..=999. `AccessAmbiguity::MissingOrAccessHidden` is sufficient unless another approved ambiguity actually needs a distinct value. `RetryGuidance::{DelaySeconds(u64), AtUnixSeconds(u64)}` retains parsed safe guidance, not raw headers. `LimitDetail::new(kind, positive_bound, observed: Option<u64>)` validates its positive bound. `AcquisitionLimitKind` distinguishes attempts, elapsed nanoseconds, response-body bytes, admitted-body bytes and representation bytes, with explicit unit-bearing as_str values. ResourceError keeps private category/message, gains `with_details` and `details`; existing new() remains detail-less. All metadata are bounded enums/fixed numbers, not arbitrary maps/strings.
- `GithubDeployment::new(api_base_url: Option<String>, web_origin: Option<String>) -> Result<Self, ConfigurationError>` in configuration/github.rs; public default constructor or Default yields public GitHub. API-base/web-origin accessors preserve validated state. `GithubConfig::new` replaces its fourth raw API-base parameter with GithubDeployment and adds validated ReadAcquisitionLimits as its last argument. Existing `api_base_url()` delegates. New `deployment()` and `acquisition_limits()` accessors. No new HTTP stack or principal lifecycle.
- Private HTTP limit-bearing method names may follow existing BoundedRead patterns; it must expose effective limit/deadline/attempt/admitted-body state to the existing GitHub fetch caller without exporting it publicly. Core controls remain values, not runtime ledger state. Keep old no-override HTTP consumers unchanged.
- `github/fetch.rs` extracts current private cache/fetch types/functions and impl methods without changing their signatures or behavior in S4. Rust descendant access avoids public expansion. S6 extends this same owner with fact observation metadata and controlled acquisition.
- `mcp/acquisition.rs` owns one closed `AcquisitionInput` DTO and `into_limits`; profile and read use it. `mcp/server/read.rs` owns moved ReadInput/display parsing/execute_read/dispatch_read; parent registration delegates. `mcp/render/error.rs` owns one category/message/optional-details output DTO/conversion shared by existing three tool error families. Protocol validation errors remain invalid_params.

## Module growth ledger

Use design.md's exact baseline inventory and P/L metric definition. Figures below are physical-line projections because that is the enforced existing tripwire; lexical counts remain the separately recorded design signal. Unchanged adjacent candidate owners retain their baseline. Projection overruns trigger ownership inspection; only unchanged ownership permits a documented plan correction.

| Module | Baseline P | Projected final P | Responsibility/interface change | Protected-parent rule |
|---|---:|---:|---|---|
| core/source.rs |20|25–45|One explicit optional controls parameter|Pure trait only|
| core/read.rs |365|370–390|Request value/forwarding|No acquisition body|
| core/error.rs |78|200–300|Bounded typed details|No provider/JSON decoding|
| core/acquisition.rs |0|120–300|Validating values/intersection|New pure owner|
| core/reference.rs |1942|1945–1989|Facts variant/parser/canonical|Inherited cap/old unrelated bodies|
| core/lib.rs |78|80–95|Exports|Wiring only|
| sources/github/mod.rs |1498|1000–1350|Extract fetch; add facts dispatch/catalog|Must net shrink|
| sources/github/fetch.rs |0|300–650|One conditional fetch/cache owner|Private; no fact schema|
| sources/github/facts.rs |0|450–750|Native presence/owned facts and acquisition orchestration|No HTTP duplication/public provider trait|
| sources/github/facts/identity.rs |0|200–350|One private identity/link validator returning borrowed operands|No HTTP/cache/clock/serialization; approved implementation placement revision|
| sources/github/mutation.rs |1317|1315–1335|Moved-helper references/Facts refusal|No new write behavior|
| sources/http/mod.rs |1491|1490–1650|Effective body cap/safe metadata|Inherited cap/net-shrink; named bodies only|
| sources/http/read.rs |194|220–350|Shared controls/ledger|No provider state or second clock|
| sources/http/request.rs |228|228–240|At most mechanical control plumbing|Keep request ownership|
| sources/configuration/github.rs |127|240–360|Validated deployment/operator controls|Same owner, no network|
| sources/configuration/mod.rs |91|91–100|Exports|Wiring only|
| sources/compiled.rs |471|475–495|Controls forwarding/catalog refusal|No provider-specific implementation|
| sources/artifact.rs |319|323–335|Signature/override refusal|Existing behavior otherwise|
| sources/filesystem.rs |2963|2967–2979|Signature/override refusal|No filesystem refactor|
| sources/https.rs |221|225–237|Signature/override refusal|No new generic HTTP controls contract|
| sources/local.rs |400|404–416|Signature/override refusal|No scratch behavior changes|
| sources/atlassian/jira.rs |292|298–317|Signature/refusal/internal call|Inherited cap/children unchanged|
| sources/lib.rs |78|80–95|Config exports|Wiring only|
| mcp/acquisition.rs |0|70–200|One read/profile syntax bridge|No network/provider|
| mcp/server.rs |1431|1150–1350|Read extraction/registration delegation|Must net shrink; unrelated tokens frozen|
| mcp/server/read.rs |0|180–400|Concrete read processing|No source/HTTP implementation|
| mcp/render.rs |1577|1510–1570|Common error extraction|No duplicate error DTOs|
| mcp/render/error.rs |0|90–300|Safe error representation/schema|Not an operational error taxonomy|
| mcp/profile/model.rs |1815|1820–1860|GitHub fields/into_parts|No new policy bodies|
| mcp/profile/convert.rs |269|275–295|Construct validated config|No duplicate pairing policy|
| mcp/profile/mod.rs |266|270–330|Gated HTTP-root handoff to GitHub|Existing substrate only|
| mcp/launch.rs |242|245–280|Shared internal trust routing|No production selector|
| mcp/test_support.rs |27|27–40|Existing helper remains actual serve path|Feature gated only|
| mcp/lib.rs |21|23–30|Private module declarations|Gated support unchanged|

Prefixes are crates/resourcefs-{core,sources,mcp}/src/. Existing github/wire.rs, github/render.rs, http/extract.rs, configuration/https.rs, catalog.rs and profile/check.rs/schema.rs have no planned responsibility change; schema generator output changes through DTOs. If incidental wiring requires a change, retain their design inventory and explain its exact delta rather than creating another owner.

## Partition arithmetic

| Slice | Changed-line estimate (implementation/tests/fixtures/workflow included) | Increment |
|---|---:|---|
|S0 fence transition|1600|A — contract preparation|
|S1 core read/details cutover|1200|A — contract preparation|
|S2 deployment/operator configuration|650|B — source preparation|
|S3 bounded HTTP mechanism|900|B — source preparation|
|S4 shared fetch extraction/native diff fence|800|B — source preparation|
|S5 real MCP read/trust preparation|1100|C — MCP preparation|
|S6 complete PR facts|2700|D — PR facts|

Sum 8,950; churn margin 20% = 1,790 for fixture/schema/caller migration and structural-oracle repairs; total **10,740**, above 4,000, so partition is mandatory. Per-increment totals with margin: A 3,360; B 2,820; C 1,320; D 3,240. Each increment depends on preceding increments but not subsequent ones. Actual diff accounting may require another independently-green partition before any publication; it may not silently raise the threshold.

A is independently reviewable with validated source-neutral controls/errors and complete caller migration, while all currently existing resources truthfully reject explicit unsupported controls. It does not expose a Facts placeholder. B validates deployment configuration, real shared transport bounds and unchanged native diff independently of Facts. C proves actual legacy GitHub reads through the gated stdio fixture and exposes validated acquisition syntax/error output without pretending old resources support it. D adds the complete advertised facts behavior through those already-proved seams. Per the requester's explicit local-only selection, retain these partitions as local checkpoint commits; do not open the workflow's usual boundary draft PRs. Remote CI/publication is not authorized.

## Slice S0: Enforce the approved structural transition

**Claim IDs:** C02.
**Expected behavior:** A successor placement census permits only this approved staged owner/dispatch migration while retaining unrelated h212/nae2 obligations.
**Oracle:** Independent Git baseline/diff/symbol census, design ledger and existing inherited assertions.
**Stress fixture:** Disposable copies with an unlisted module, new forbidden parent body, wrong dependency and changed unrelated frozen server body; each must name its exact C02 violation while baseline succeeds.
**Regression fence:** `.rfs-0n97/oracles/module_shape.py`, `.rfs-0n97/oracles/module-ledger.json`, inherited oracle checks.
**Named mutation:** Add a serializer responsibility body to github/mod.rs; add an unlisted crates/*/src Rust file; introduce forbidden transport/provider dependency; change a non-read frozen server function. Each distinct mutation must fail and restoration pass.
**Complexity/production scale:** One-off source census with bounded lexical scans and inherited symbol comparisons; conservative upper bound O(total scanned source + sum of file bytes × declaration count), rather than assuming all repeated declaration scans are linear. Current three-crate repository; maximum accepted cost 10 s on the local checkout, rationale: non-runtime gate should remain cheaper than compilation. No production loop. This is a plan-only complexity correction; ownership and the measured ceiling are unchanged.
**Wall budget/phase:** N/A — one-off verification phase; no runtime wall budget.
**Module shape:** No production ownership move yet. Add narrow explicit policy input to nae2, stage-aware successor inventory/approved-node checks and runner handoff. `python .rfs-0n97/oracles/module_shape.py --stage baseline` → C02 PASS; later stages `core`, `configuration`, `transport`, `fetch`, `mcp`, `facts` progressively require completed owners without accepting placeholders. Every stage retains unrelated freezes.
**Files:** `.rfs-nae2/oracles/module_shape.py`, `.rfs-0n97/oracles/module_shape.py`, `.rfs-0n97/oracles/module-ledger.json`, `scripts/ci-gates.py`, retained .rfs-0n97 route/evidence/probe/design/plan artifacts.
**Estimate:** One bounded oracle implementation/review pass; timing is a planning signal only.
**Diff estimate:** 1600 changed lines.
**PR increment:** A — contract preparation.
**Commands and expected results:**
- `python .rfs-0n97/oracles/module_shape.py --stage baseline` → old owners and unchanged source accepted.
- The same command against each disposable `--root` mutation tree → C02 plus exact offending path/symbol; restored tree passes.

## Slice S1: Migrate the complete source-neutral read contract

**Claim IDs:** C01; immediate supporting value/refusal fences for C03/C08/C15, whose actual Facts consumer completes in S6.
**Expected behavior:** Validated five-dimensional limits intersect without raising ceilings; bounded typed details survive Rust error values; every adapter/read-engine caller uses the new signature; explicit controls on existing unsupported resources fail rather than disappear. Existing absent-control reads still work.
**Oracle:** Explicit per-dimension min/boundary corpus and existing public source/read-engine results, plus dependency census.
**Stress fixture:** Unequal limits across all five dimensions; zero/above-hard/Duration boundary; direct adapter and CompiledSources unsupported requests paired with successful absent-control reads. Missing detail differs from present bounded detail; invalid HTTP status/limit construction fails.
**Regression fence:** New `crates/resourcefs-core/tests/acquisition_contract.rs`, existing read_engine_contract and source adapter refusal contracts; existing architecture_contract.
**Named mutation:** Replace min with max in acquisition intersection; accept zero; drop an adapter's explicit override rejection; introduce actual serde_json decoding in core error.rs. Relevant focused fence fails; restore passes.
**Complexity/production scale:** New value operations O(1), exactly five dimensions and finite detail fields, no heap allocation for limits/detail payloads; no new data-size loop. Maximum accepted cost: 100,000 constructor/intersection operations within 100 ms in release; rationale: scalar policy must be negligible versus I/O.
**Wall budget/phase:** Always-on constant read dispatch/policy validation; production-scale scalar operation target above. No added background phase.
**Module shape:** core owns controls/details; all seven source implementations migrate atomically. Parent deltas follow growth ledger; no Facts grammar/advertisement yet. `python .rfs-0n97/oracles/module_shape.py --stage core` → C02 PASS.
**Files:** core src/{acquisition.rs,error.rs,source.rs,read.rs,lib.rs}; sources src/{compiled.rs,artifact.rs,filesystem.rs,https.rs,local.rs,atlassian/jira.rs,github/mod.rs}; MCP src/server.rs ReadRequest construction and render.rs inline source fixture only; core tests/acquisition_contract.rs and read_engine_contract.rs. Mechanical test migration list below is part of this slice.
**Estimate:** One implementation and complete caller-integration pass.
**Diff estimate:** 1200 changed lines.
**PR increment:** A — contract preparation.
**Commands and expected results:**
- `cargo test -p resourcefs-core --test acquisition_contract --test read_engine_contract` → invalid controls rejected, componentwise lowering and existing recovery/cancellation correct.
- `cargo test -p resourcefs-sources --test compiled_sources_contract --test local_dispatch_contract --test artifact_adapter_contract` → no-override behavior retained; explicit unsupported controls fail at direct and compiled seams.
- `cargo test -p resourcefs-mcp --test architecture_contract enforces_dependency_direction -- --exact` → C01 survives actual source graph.
- `cargo check --workspace --all-targets --all-features` → no omitted trait/ReadRequest callers; run after integration, not in agents.

Mechanical caller test files under sources/tests: artifact_adapter_contract.rs, atlassian_jira_adapter_contract.rs, compiled_sources_contract.rs, filesystem_adapter_contract.rs, github_adapter_contract.rs, github_live_smoke.rs, github_mutation_contract.rs, http_policy_contract.rs, https_credential_contract.rs, https_live_smoke.rs, https_status_contract.rs, jira_browse_budget_contract.rs, jira_browse_discovery_contract.rs, jira_live_smoke.rs, jira_query_contract.rs, local_dispatch_contract.rs, local_isolation_contract.rs, local_mutation_contract.rs, local_root_contract.rs, local_selector_contract.rs and support/jira.rs. Inspect support/mod.rs's trait-object uses but do not change unchanged constructor wiring. Core's three test adapters and MCP render's inline adapter also migrate. AST candidates include unrelated ReadEngine/harness two-argument reads: do not blindly rewrite every `.read(a,b)` call. Retry LSP references; if the linked-worktree lookup remains incomplete, use scoped syntax inspection and compiler completeness, recording the tooling limitation.

## Slice S2: Validate deployment and operator acquisition configuration

**Claim IDs:** C04.
**Expected behavior:** Public defaults and matched ghe pairs configure successfully; contradictory/unsafe origins fail before credentialed acquisition; custom legacy reads remain configured without guessed web identity; facts eligibility requires explicit custom web origin. Valid operator limits are stored without raising hard caps.
**Oracle:** Explicit hostname/config corpus plus actual profile/CLI schema/deserialization behavior.
**Stress fixture:** Public mismatch, api.a.ghe.com with b.ghe.com, invalid tenant/path/port/userinfo, custom API with absent/present web origin, null/wrong acquisition values; valid paired neighbors succeed.
**Regression fence:** configuration_contract.rs, profile_contract.rs and schema_contract.rs with updated published schema fixture.
**Named mutation:** Remove pair equality/class-path validation; invalid pair then configures. Coerce omitted custom web origin from API hostname; unsupported identity then becomes fabricated. Restore returns green.
**Complexity/production scale:** Origin validation O(input URL bytes), existing repository normalization loop unchanged (up to 4096 entries); accepted origin host bounded by DNS/tenant syntax. No new per-request loop. Maximum accepted work is a single validation pass, no DNS/network/credential subprocess during pairing.
**Wall budget/phase:** N/A — one-off validated configuration construction; no runtime wall budget.
**Module shape:** Existing configuration/github owner deepens; MCP acquisition syntax module is shared concrete input owner. Profile model gains only fields/conversion wiring. `python .rfs-0n97/oracles/module_shape.py --stage configuration` → C02 PASS.
**Files:** sources src/configuration/{github.rs,mod.rs}, src/lib.rs; MCP src/acquisition.rs, src/lib.rs, src/profile/{model.rs,convert.rs}; sources tests/configuration_contract.rs and every GithubConfig constructor in github_adapter_contract.rs, github_live_smoke.rs, github_mutation_contract.rs; MCP tests/{profile_contract.rs,schema_contract.rs}, tests/fixtures/server-profile.schema.json if that existing fixture name is confirmed before editing. Generated schema comes from existing CLI generator, not hand-written duplication.
**Estimate:** One configuration/profile pass.
**Diff estimate:** 650 changed lines.
**PR increment:** B — source preparation.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test configuration_contract` → matching pairs accepted, unsafe/conflicting pairs rejected.
- `cargo test -p resourcefs-mcp --test profile_contract --test schema_contract` → emitted schema and actual deserializer agree on new operator inputs; public profiles remain valid.
- Actual `rfs schema`/`rfs check` commands from existing CLI contract tests → generated contract and concrete invalid-profile exit behavior, not TCP-as-API proof.

## Slice S3: Enforce physical-request and body-admission mechanics

**Claim IDs:** C09; immediate transport supporting fences for C10/C12/C15 complete before their Facts consumer in S6.
**Expected behavior:** One shared bounded read honors effective deadline/response/attempt/admitted-total controls and safe metadata. Enabled-transport retries charge physical attempts; redirects remain refused; current protocol feature premise is enforced. Legacy no-override consumers retain behavior.
**Oracle:** Independent loopback HTTP byte/method/timestamp log and explicit aggregate byte arithmetic; Cargo resolved graph for protocol premise.
**Stress fixture:** Lower-one attempt with a retryable response and allowed-retry control; shared hard attempt boundary; delayed headers/body/wait; two separate 8 MiB admitted bodies followed by one byte, plus unequal smaller bodies; competing caps relaxed so each guard is decisive. No fabricated extra PR requests.
**Regression fence:** Existing http/read_tests.rs, http_bounds_contract.rs and http_substrate_contract.rs; new issue-local `oracles/transport_features.py`.
**Named mutation:** Skip attempt charge before retry; reset accumulated admitted bytes; use default response cap; restart deadline; enable reqwest http2 in a disposable manifest. Each relevant request/count/time/feature assertion must turn red, restore green.
**Complexity/production scale:** Streaming O(body bytes), each response <=8 MiB, accepted total <=16 MiB, at most10 attempts and one existing retry permit. Ledger updates O(1). Maximum accepted extra bookkeeping allocation 4 KiB per logical read, excluding retained headers/bodies; no body clone for counters. Local processing of 16 MiB through the controlled loopback driver <=1 s release, rationale: ample headroom below30 s logical limit without a wire-speed guarantee.
**Wall budget/phase:** Always-on; one effective logical deadline <=30 s covers send/body/wait. Local processing target above; no independent provider retry clock.
**Module shape:** Existing HTTP mod/read deepen; no provider symbols or new request client. Keep inherited parent cap/net-shrink and named-body exceptions. `python .rfs-0n97/oracles/module_shape.py --stage transport` → C02 PASS.
**Files:** sources src/http/{mod.rs,read.rs,read_tests.rs}, tests/{http_bounds_contract.rs,http_substrate_contract.rs}; `.rfs-0n97/oracles/transport_features.py`; runner budget inventory only for a genuinely retained production-budget row.
**Estimate:** One bounded-transport implementation and adversarial fixture pass.
**Diff estimate:** 900 changed lines.
**PR increment:** B — source preparation.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test http_bounds_contract --test http_substrate_contract` → exact body/admitted boundaries and independent physical requests obey limits.
- `cargo test -p resourcefs-sources --lib http::read_tests` → existing retry/cancel semantics plus new shared-ledger cases hold.
- `python .rfs-0n97/oracles/transport_features.py` → resolved all-feature graph has no http2/http3; enabled-protocol mutation fails C09 explicitly.

## Slice S4: Extract one GitHub fetch owner and fence native diff

**Claim IDs:** C18.
**Expected behavior:** The shared private fetch/cache cluster moves without changing human/conditional/mutation consumers; native unpaged diff remains exact native media and bytes and refuses partial/invalid text.
**Oracle:** Independent upstream server's original diff bytes and captured Accept; existing cache/generation behavior.
**Stress fixture:** Complete diff with no final newline; truncated body; invalid UTF-8; valid textual control; JSON and diff media cache entries remain distinct.
**Regression fence:** Existing github_adapter_contract.rs with dedicated native-diff behavior cases; existing mutation/wire/cache tests.
**Named mutation:** Native diff Accept changed to JSON; reconstruction from file patches; remove truncated refusal. Each fails native media/bytes/refusal rather than incidental wording; restore green.
**Complexity/production scale:** N/A — extraction retains existing loops/limits; new test fixture does not add production work. No new cache, decode or rendering pass.
**Wall budget/phase:** Always-on behavior retained, no added runtime phase or wall allowance.
**Module shape:** Move cache/fetch types/impl methods into private github/fetch.rs; unique ownership and net-shrinking parent. No facts schema yet. `python .rfs-0n97/oracles/module_shape.py --stage fetch` → C02 PASS.
**Files:** sources src/github/{mod.rs,fetch.rs,mutation.rs if moved helpers require imports}; tests/github_adapter_contract.rs, existing support TLS fixture only if needed for native byte response.
**Estimate:** One extraction/compatibility pass.
**Diff estimate:** 800 changed lines.
**PR increment:** B — source preparation.
**Commands and expected results:**
- `cargo test -p resourcefs-sources --test github_adapter_contract --test github_wire_contract --test github_mutation_contract` → legacy public behavior remains intact; new native-diff oracle cases decisive.
- Native-diff named mutations against the focused cases → expected media/byte/refusal failures, then restored success.

## Slice S5: Prepare the actual MCP acquisition/error/trust path

**Claim IDs:** C20; immediate MCP protocol/error supporting fences for C08/C15/C17 retained into S6.
**Expected behavior:** Real tools/call validates acquisition syntax before work, uses the new ReadRequest field and reports typed unsupported-control details on existing resources. Gated test root reaches actual GitHub mount; normal production launch cannot select it. Other tools preserve absent-detail output.
**Oracle:** Reexecuted real stdio server, independent TLS listener and actual CLI/schema responses, not mocked dispatch.
**Stress fixture:** Valid legacy GitHub title/body from a path-aware TLS fixture; same launch without root refuses certificate; explicit test-root profile/env cannot enable trust; valid/no-override controls prove requests are observable. Null/unknown/zero controls fail protocol validation; valid controls on local resource fail as unsupported tool result.
**Regression fence:** stdio_mcp_contract.rs, architecture_contract.rs trust invariant, profile/schema tests; existing HTTP fixture support deepens to independently record method/path/headers and serve named response sequences.
**Named mutation:** Drop GitHub root propagation; allow a production test-root selector; omit real dispatch acquisition validation or error details. The appropriate actual-process behavior fails; restore green.
**Complexity/production scale:** Parse/convert fixed five-field controls and finite details O(1); no arbitrary map/error-body retention. Existing output recovery remains linear in output bytes. Maximum accepted new metadata <=4 KiB per error output and one conversion per tool error, rationale: bounded enums/numbers need no large envelope. Fixture process is test-only.
**Wall budget/phase:** Always-on fixed protocol conversion <=1 ms release per input/error; one-off launch trust selection has no runtime wall budget. No new network request for syntax validation.
**Module shape:** Extract read processing to server/read.rs and shared error representation to render/error.rs; existing acquisition DTO reused. Parent server/render shrink; profile/launch only trust forwarding. `python .rfs-0n97/oracles/module_shape.py --stage mcp` → C02 PASS.
**Files:** MCP src/{server.rs,server/read.rs,render.rs,render/error.rs,lib.rs,launch.rs,test_support.rs,profile/mod.rs}; tests/{stdio_mcp_contract.rs,architecture_contract.rs}, tests/support/profile_tls.rs and stdio support only as required for bounded real process proof. Reuse existing ignored test host rather than adding unclassified ignored fixtures; if impossible, update runner classification in the same slice.
**Estimate:** One MCP extraction/real-fixture pass.
**Diff estimate:** 1100 changed lines.
**PR increment:** C — MCP preparation.
**Commands and expected results:**
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract` with named new preparation filters → valid GitHub fixture read through actual launch, ordinary trust refusal, concrete invalid_params versus unsupported tool error.
- `cargo test -p resourcefs-mcp --test architecture_contract --test schema_contract --test profile_contract` → no production trust selector or contract drift.
- Focused root-propagation and dispatch mutations → actual-process red; restore green.

## Slice S6: Deliver complete deployment-qualified PR facts

**Claim IDs:** C03,C05,C06,C07,C08,C10,C11,C12,C13,C14,C15,C16,C17,C19.
**Expected behavior:** Complete bounded versioned PR JSON through direct source and real MCP, actual identity/presence/provenance and conditional semantics, lower-only limits, safe machine errors, read-only authority, exact recovery and live public proof. No preparatory placeholder is exposed before this slice.
**Oracle:** Design's independent native fixture/presence matrix, server method/credential/byte/timestamp logs, external Python/jq consumer, independent SHA256/length and live gh observation. Retain only applicable earlier transport/fence results.
**Stress fixture:** Distinct head/base/fork/merge IDs; >2^53 IDs; missing/null/empty/Unicode/unknown values; contradictory parent and missing required identity;200/304/error/generation sequence with distinct provider/HTTP clocks; large escaping representation exact/plus one; cancellation barriers; safe/secret-bearing error responses; denied target positive controls; multipage real recovery.
**Regression fence:** New sources tests/github_facts_contract.rs through public SourceAdapter, focused existing github_reference_contract/mutation contracts, real stdio facts cases, existing live source/stdio files; retained earlier transport ledger tests. Permanent tests each defend a named plausible bug, not private DTO structure or wording.
**Named mutation:** Apply C03/C05–C08/C10–C17/C19 design mutations to actual implementation: wrong route/identity; body default/ID precision/state coercion/schema major2; stale304 metadata/fallback; max-for-min; missing final deadline/cancellation; cache admission skip; envelope exclusion; authority-following; prose-based errors/raw leak; recovery reread/drop; missing schema/catalog. Each fence reports its own observable mismatch; restore green. C19 uses deterministic identity mutation plus actual gated live run, not unstable upstream counts.
**Complexity/production scale:** Decode/validate/serialize O(n) for <=8 MiB response and <=16 MiB final document, finite field vocabulary, no extra provider collection traversal. Bounded writer stops before exceeding representation cap. Maximum accepted local processing <=1 s release at production-size fixture; incremental heap <=96 MiB measured separately (raw/decoded/cache/output ownership plus conservative allocator overhead), rationale: detect accidental duplicate trees/copies without claiming a public process-memory bound. Admission bookkeeping remains O(1); no unrestricted intermediate Value tree. Cache metadata is fixed-field, not a growing acquisition log.
**Wall budget/phase:** Always-on per facts read; one effective deadline <=30 s includes network/body/wait and final acceptance. Local decode/project/serialize budget above. Live smoke is a separate one-off credential-gated phase; absent gate is a skip, never PASS.
**Module shape:** Add private facts.rs and the requester-approved private facts/identity.rs child (design.md's implementation placement revision); extend shared fetch observation/control seam, add typed Facts parser/canonical/refusal/catalog wiring. Identity validation has one private function and no HTTP/cache/clock/serialization. No provider JSON in core/MCP, no new client or capture coordinator. `python .rfs-0n97/oracles/module_shape.py --stage facts` → all final required owners and protected rules pass.
**Files:** core src/reference.rs and tests/github_reference_contract.rs; sources src/github/{facts.rs,fetch.rs,mod.rs,mutation.rs}, tests/{github_facts_contract.rs,github_mutation_contract.rs,github_live_smoke.rs}; MCP tests/{stdio_mcp_contract.rs,stdio_live_smoke.rs,profile_contract.rs,schema_contract.rs} as behavior requires; scripts/ci-gates.py for exact retained ignored-budget/live classification. Existing DESIGN.md, README and applicable domain/schema documentation are updated to the implemented contract without importing unrelated sibling draft work. Production docs and caller examples follow the same cutover.
**Estimate:** One full feature implementation plus adversarial integration/evidence pass.
**Diff estimate:** 2700 changed lines.
**PR increment:** D — PR facts.
**Commands and expected results:**
- `cargo test -p resourcefs-core --test github_reference_contract` → Facts canonical grammar without legacy reinterpretation.
- `cargo test -p resourcefs-sources --test github_facts_contract --test github_adapter_contract --test github_mutation_contract` → exact identity/presence/cache/limits/error/cancellation/readonly oracle outcomes.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract` with new facts filters → real full recovery equals acquired bytes; no additional upstream acquisition on recovery.
- Retained production-budget row in release → <=1 s local processing and <=96 MiB incremental heap at production-size fixture, with exact runner inventory updated.
- `scripts/live-smoke.sh` → required enabled public source and real-binary rows actually run; record exact L/S observations in evidence.md, not skipped-as-pass.
- `python scripts/ci-gates.py` → assembled complete repository gate; do not use --help, which runs the gate. One final formatter/clippy/full-suite run after writers are integrated; focused checkpoints precede it.
- Fresh-context nonimplementing reviewer reconstructs production owners/interfaces before seeing design, then compares ledger → no unresolved mismatch.

## Self-review and boundaries

- Every C01–C20 row has exactly one completion owner; preparatory mechanism tests are exercised when introduced and valid evidence reused at the composite consumer checkpoint.
- Every slice has fourteen fields and an independent existing/current interface check, with no dependence on later Facts behavior for preparation acceptance.
- Every retained fence has a named production or structural mutation; no risk waiver or test-only oracle mutation substitutes for a production defect.
- New loops/phases, production sizes, explicit accepted costs and physical growth tripwires are recorded. These measurements are implementation obligations, not asserted results.
- The >4,000 partition arithmetic includes fixtures/workflow artifacts and churn. All new modules remain within approved owners; no pass-through generic framework is planned.
- Rust platform checklist: no new OS-specific path semantics, no claim that Linux establishes native Windows behavior. Corporate/native consumer acceptance remains cyril-9qn7/cyril-wcxv. Every error/control/identity branch follows design's shape matrix, preserves missing versus corrupt, and uses stable categories, not error-message matching.
- Subsequent collection/source/comparison/CI work remains in the seven verified successor tickets named in design.md; no additional untracked deferral or parent-tracker modification.
- This document plans work only. checkpointed-build exclusively records completion and assembled evidence; no slice is declared done here.
