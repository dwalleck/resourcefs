# Design: rfs-nae2

## Route and inputs

- Route: Empirical, with structural placement and bounded-scale obligations (`route.md`, T1–T3).
- Complete behavior set: `route.md`, “Given/when/then behavior contract”, clauses 1–6: exact site-scoped canonical JQL; native POST ordering and opaque continuation; stable-ID issue navigation and authority validation; native 400 versus malformed-reference categories; atomic 100/1,000/ten bounded pages with lossless recovery; existing rendered-content regex semantics and unsupported Atlassian glob.
- `spec.md`: N/A — those six clauses and the closed rfs-qsyk/rfs-lbps decisions specify behavior.
- Empirical inputs: `evidence.md` P1–P3 and `probe_post_pages.py`, `probe_post_error.py`, `probe-results.json`. P1 retains independently checked endpoint/schema evidence. P2 compares ASC/DESC POST traversal against independently provisioned key order and direct issue reads; both pass, including exact-boundary and empty termination. P3 compares native HTTP 400 diagnostic structure with the published Error Collection; PASS. No response `maxResults` was observed; no effective clamp or snapshot guarantee is inferred.
- Starting production state: merged prerequisite at `fad4cf2c11ec27f4272355c9e741badc9be0920e`; isolated `/tmp/rfs-nae2-query`. The dirty parent checkout is not an implementation workspace.

## Input shapes

Each row is covered by the listed claims. Cartesian field-presence combinations are exercised where independent fields can coexist; impossible combinations carry a rationale rather than a fixture.

| Shape | Reachable cells | Coverage |
|---|---|---|
| JQL segment | Empty; ASCII; Unicode; spaces; reserved slash, backslash, colon, percent and plus; escaped controls; dot-like strings; canonical/noncanonical escapes; invalid UTF-8; truncated escapes; over-encoded unreserved characters; reference byte ceiling and ceiling+1 | C2 |
| Query meaning | Valid native expression; native rejection; whitespace-only nonempty expression; explicit ascending/descending order; no ORDER BY | C2, C4, C8; ResourceFS never parses meaning |
| Address sum | Existing Jira root/projects/project/issues/issue variants; new Query variant; query versus fixed collection selectors; relative/workspace/HTTPS references remain their existing variants | C2, C12 |
| Site/authority | Configured owning site; unconfigured site; same origin under another Site ID; changed mount origin; returned valid/foreign/malformed issue and project authority | C3, C5 |
| Source cursor | Absent; valid; malformed envelope/base64/UTF-8; wrong site/family/exact query/origin; empty token; Unicode/spaces; repeated token; representable maximum and oversized token/reference | C3, C6 |
| Native rows | Empty, singleton, distinct multiple, duplicate IDs; IDs beyond machine integers; valid/mismatched key, ID and self URL; selected fields missing/null/value; invalid field types | C4, C5, C6 |
| Page metadata | isLast true/false/missing/null/nonboolean crossed with token absent/empty/nonempty/wrong type; requested/effective maximum absent, smaller valid value, zero, negative, over-request and wrong type; short nonterminal page | C4, C6 |
| Limits | Request maximum 100; logical records 999/1,000/over-return; attempts 9/10/exhausted; native/serialized/reference byte ceilings and ceiling+1; lower-only caller limits | C6, C9 |
| HTTP method/policy | GET; explicit read-only JSON POST; ordinary POST; PATCH; replay available/spent; response 200/304/400/401/403/404/410/413/429/503/other 5xx; redirect; transport failure | C7, C8, C9 |
| Retry/deadline | Retry-After absent/malformed/zero/valid/exceeds remaining time; jitter; original deadline before/after wait; cancellation before/during request or wait; physical attempts include retry | C7, C9 |
| Error document | Valid Error Collection with global array and/or field map and optional integer status; absent/empty members; malformed types; invalid JSON/media; oversized body; diagnostic credential/query/token canaries | C8 |
| Content tools | Fits displayed view; clipped view with artifact recovery; terminal/nonterminal source page crossed with artifact present/absent; valid/invalid regex; regex matches/no matches; glob invocation | C10, C11 |
| Cache observations | Repeated identical query with changed upstream rows; validator-bearing response; native error followed by success | C9 |
| Native snapshots and unknown provider languages | Snapshot isolation, translating JQL, and cross-site query fan-out | N/A — permanent non-goals below |

### Removed-invariant sweep

The feature is additive except for relaxing the HTTP read loop's GET-only replay guard. That guard currently implies no POST body is replayed and non-GET requests bypass the logical-read timeout wrapper. Replace method inference with explicit request intent: only GET and the dedicated read-only POST constructor can obtain a replay copy. Ordinary POST/PATCH retain one attempt and redirect refusal. All requests entering BoundedRead receive its original deadline. The retry permit remains one per logical operation, not one per native page; budget and deadline cannot be reset by query assembly. C7/C9 fence these invariants. Fixed browsing's numeric ordering remains unchanged; the new query assembler never calls its sorter (C4).

## Placement

1. **Query identity:** existing core `reference/jira.rs` owns validated `JiraQuery` and a Query address variant. Reuse canonical percent encoding, not issue-key validation: decoded JQL is nonempty UTF-8, preserved exactly, subject to the existing reference ceiling. A redacted Debug implementation avoids accidental raw-query diagnostics. Existing reference parsing/canonical identity is the caller interface; no provider JSON or HTTP enters core.
2. **Query execution:** private `atlassian/jira/query.rs` owns one logical page and native order, behind the existing source read interface. It reuses strict issue decoding/rendering and existing page/operation limits. It does not reuse the fixed collection sorter, build a client, interpret JQL or cache query results.
3. **Cursor ownership:** existing `jira/cursor.rs` adds a Query owner containing exact canonical query identity, site and configured-origin fingerprint. Validate before egress and before following/returning native continuation. Existing fixed collection ownership stays intact. No raw upstream token becomes a public unbound selector.
4. **Native wire:** private `atlassian/wire/query.rs` owns the enhanced POST payload and structured rejection decoding. It accepts the validated query and native token, emits selected fields and bounded JSON, and returns structural diagnostic counts/status, never provider prose or arbitrary field names. Preflight query-plus-token lengths before allocation; preserve the existing total reference bound and use a derived JSON escaping ceiling. Existing `wire/collections.rs` remains the sole issue-row/page decoder.
5. **Transport:** existing `jira/transport.rs` adds an uncached query fetch whose endpoint is the constant enhanced-search path under the configured site. It builds the payload through wire/query and maps query-specific 400 without relabeling all Jira GET 400 responses. No caller supplies an arbitrary endpoint or retry flag.
6. **HTTP request intent:** extract the existing request construction/header/replay cluster into private `http/request.rs`, re-exporting the existing HttpRequest interface. Add a crate-private explicit read-only JSON POST constructor and typed internal intent. Use shared immutable body bytes for cheap retry copies; the source-neutral HTTP layer never knows Jira types. Ordinary mutation constructors remain nonreplayable. A direct `bytes` dependency, already used transitively by reqwest, is permitted for body ownership rather than copying Vec payloads per attempt.
7. **Bounded read:** existing `http/read.rs` alone owns attempts, one retry permit, waits and immutable deadline. Request intent controls replay eligibility. Existing substrate remains the only client/credential/egress owner.
8. **Tools/recovery:** source dispatch and existing engine interfaces carry the Query Resource; no new MCP tool, query-language layer, recovery format or search engine. Only exhaustive-match and capability/catalog wiring may enter facades.

### Diagnostics decision

HTTP 400 with a valid native Error Collection becomes `invalid_pattern`; malformed native envelopes remain `source_unavailable`. Diagnostics expose bounded global/field error counts and permitted status/context only. They do not expose `errorMessages` strings or arbitrary `errors` keys/values. This intentionally prioritizes the existing rfs-lbps privacy contract over verbatim Jira explanations. Local encoding/ownership failures remain `invalid_reference`. Other statuses retain the accepted category policy. No English-message matching classifies an error.

## Module shape

### Current cluster and alternatives

The merged cluster already separates grammar, cursor ownership, fixed collection assembly, wire, rendering and bounded HTTP. Current sizes below are physical-line census signals, including comments/tests, not claims about depth. Core callers are PathReference/canonical identity and engine discovery; source callers are CompiledSources and Jira facade dispatch; transport callers are Jira reads; HTTP callers include HTTPS, GitHub, Atlassian and remote-source adapters. Contract tests cross those same public interfaces. Concrete network implementation remains HttpSubstrate, with controlled TLS upstreams used as test servers, not a second production client.

Three materially different choices for the new responsibilities:

| Alternative | Interface and caller example | Hidden implementation / dependencies | Trade-off |
|---|---|---|---|
| A — selected: private query assembler plus explicit HTTP read intent | Facade calls query read; query calls JiraRead query fetch; BoundedRead consumes HttpRequest carrying validated intent | Query owns order/page state; wire owns JSON; HTTP request owns replayable body; shared read owns deadline/retry | Small caller interface, one owner per policy, no hypothetical source-wide query abstraction; requires a cohesive request-cluster extraction |
| B — inline common path in browse/facade | Existing browse function accepts a mode and optional JQL; facade chooses it | One loop branches between fixed sorting and native order; transport accepts method/retry flags | Fewer files but callers must understand mutually exclusive modes and retry safety; invites illegal combinations and violates protected-facade ownership |
| C — generic native-query adapter framework | Source implements a QueryExecutor trait consumed by generic pagination | Generic layer owns request creation/order/continuation hooks; Jira supplies callbacks | Flexible extension but only one native-query adapter exists here; exposes policy hooks and splits atomic-page reasoning across callbacks; hypothetical seam rejected |

Existing grammar, cursor, compact decoder/renderer and engine interfaces are unambiguous owners: N/A — no new generic seam is needed there.

Deletion test: removing query/request/wire-query modules redistributes real page-order, replay-safety and serialization/error complexity into callers. Interface test: adapter/HTTP behavior is tested through the same interfaces callers use. Adapter test: new modules are concrete private ownership seams, not one-adapter traits. Locality test: each responsibility below has one owner and primary fence.

### Approved module ledger

Path prefixes: C=`crates/resourcefs-core/src/`; S=`crates/resourcefs-sources/src/`.

| Module/path; baseline → tripwire | Interface / owns | Hides or reuses | Must not own | Adapters / tests through | Change |
|---|---|---|---|---|---|
| C reference/jira.rs; 376 → 650 | Typed Jira grammar, exact JQL value and canonical spelling | Existing segment codec | JSON, HTTP, native language parsing | Core reference contracts / PathReference | Deepen |
| C reference/source_page.rs; 97 → 160 | Syntactic typed source selector | Existing cursor type | Jira token semantics | Core reference contracts | Retain; only necessary sum wiring |
| C reference.rs; 1941 → 1989 | Generic reference composition/identity | Jira child | Query parser bodies | Existing source reference callers/tests | Retain protected parent |
| C discovery.rs; 1341 → 1367 | Engine dispatch, projection and recovery | Source and artifact interfaces | Native query loops or JSON | Engine/MCP tool contracts | Retain protected parent |
| S atlassian/jira.rs; 285 → 317 | Source facade/direct issue/capability dispatch | Query, browse and transport children | Query loops, sorting, retries | SourceAdapter contracts | Retain protected parent |
| S atlassian/jira/query.rs; 0 → 400 | One atomic logical query page | Cursor, wire, render, JiraRead | HTTP client, JSON internals, fixed sorting | Concrete Jira adapter / query contracts | Create |
| S atlassian/jira/cursor.rs; 151 → 250 | Cursor ownership and representability | Existing envelope/codec | Transport or language parsing | Source reads with selectors | Deepen |
| S atlassian/jira/transport.rs; 279 → 400 | Site-owned fetch and status mapping | Wire payload; BoundedRead | Own retry loop, query result cache | Adapter and HTTP contracts | Deepen |
| S atlassian/jira/browse.rs; 330 → 600 | Existing fixed collection assembly/order | Shared issue decoder/renderer | Native query assembly | Existing browsing contracts | Retain |
| S atlassian/wire/query.rs; 0 → 200 | Bounded request JSON and structured rejection | Existing strict JSON machinery | HTTP, credentials, arbitrary diagnostic prose | Query adapter contract | Create |
| S atlassian/wire/collections.rs; 529 → 600 | Strict compact rows and page decoding | Existing typed wire model | Query execution or sorting | Existing wire plus query contracts | Retain; reusable visibility only |
| S atlassian/render/collections.rs; 173 → 350 | Compact Markdown preserving supplied row order | Existing escaping and stable references | Fetching or sorting | Source content contracts | Retain |
| S atlassian/wire.rs; 584 → 601 | Shared wire declarations | Child modules | New query codec body | Existing adapter contracts | Retain protected parent |
| S atlassian/render.rs; 482 → 488 | Shared rendering declarations | Collections renderer | Query assembly | Existing adapter contracts | Retain protected parent |
| S http/request.rs; 0 → 280 | Request construction, headers, intent and replay copy | Shared immutable bytes | Clients, retries, source-specific types | Existing HttpRequest/HTTP contracts | Create by extraction |
| S http/read.rs; 192 → 350 | Attempts, retry permit, deadline and cancellation | Request replay intent and substrate | Jira grammar or payload | Bounded HTTP contracts | Deepen |
| S http/mod.rs; 1666 → 1679, with net shrink expected | Existing sole client, egress and credentials | Request/read children | New request/retry responsibility bodies | HTTP policy/TLS contracts | Split request cluster; protected parent |
| Workspace/source Cargo manifests | Declare shared byte-buffer dependency | Existing dependency conventions | Core HTTP/JSON dependencies | Cargo/architecture checks | Update |

### Protected parents and mechanical enforcement

| Parent | Allowed change | Forbidden change | Exit condition |
|---|---|---|---|
| reference.rs | Export and exhaustive-match wiring | Query parsing/validation body | Existing ceiling; parser remains in Jira child |
| discovery.rs | Necessary existing-interface wiring | Query paging/transport | Existing ceiling; no native body |
| jira.rs | Module declarations, dispatch and capability/catalog wiring | Query assembly, sorting, retry | Existing ceiling; query implementation only in child |
| wire.rs, render.rs | Child declarations/reusable visibility | New codec/render loop bodies | Existing ceilings; single decoder/renderer owner |
| http/mod.rs | Request extraction, re-exports and construction wiring | Added replay/request policy implementation | Net shrink; one HTTP client owner remains |

Retain `.rfs-h212/oracles/module_shape.py --stage issues` as the browsing-placement fence (C1). Add `.rfs-nae2/oracles/module_shape.py` for C12: invoke the inherited fence and check new required paths, owned symbols, visibility, forbidden imports/client construction, parent diff bodies and growth tripwires against the recorded starting revision. Check the request extraction as a move, not duplicated implementation. CI runs the assembled fence. No production test reads source text. Numeric overruns with unchanged ownership require plan inspection; changed ownership requires design approval.

## Claims

- C1: The inherited browsing ownership and protected-parent constraints remain satisfied.
- C2: Query references preserve exactly one canonical decoding of nonempty UTF-8 JQL without applying issue-key restrictions or interpreting JQL.
- C3: Query continuation is bound to exact query, site, family and configured origin before any egress.
- C4: Enhanced POST execution preserves native row order and authoritative continuation rather than fixed-collection sorting or short-page inference.
- C5: Every published query issue link names the validated stable-ID Resource in the owning site.
- C6: One query page is atomic and bounded by requested page size, 1,000 rows, ten physical attempts and representable continuation.
- C7: Only explicitly read-only requests can replay, at most once within the original logical deadline and shared attempt budget.
- C8: Native structured query rejection and local malformed references remain distinct without exposing upstream diagnostic prose.
- C9: Query reads are uncached and retain shared egress, response-size, cancellation and deadline controls.
- C10: Public tool reads preserve both clipped-content recovery and independent source continuation without lost content.
- C11: Public search applies existing regex/PCRE2 to rendered query content and Atlassian glob stays unsupported.
- C12: The new responsibility placement conforms to the module ledger without a second client, duplicated codec/sorter or hypothetical generic query seam.

## Falsification

All PENDING rows are discharged by checkpointed-build at the implementing slice assigned in plan.md. Fences below name intended deterministic contracts, not claims that tests already exist. Positive controls use the same configured source/server and otherwise-valid input; an unrelated parse, auth or connection failure cannot count as the expected refusal.

| ID | Claim | Input shape | Falsifier / decisive control | Independent oracle | Named mutation and expected red | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Retain browsing ownership constraints | Existing owners and protected parents | Run inherited placement census; any named ownership/ceiling failure falsifies preservation | Approved h212 ledger and declaration census | Move existing encode_cursor into jira.rs; census names incorrect owner | Existing h212 module_shape.py | Subsecond census | PASS — baseline below; rerun assembled |
| C2 | Preserve one exact canonical JQL decoding | JQL segment, meaning, address sum | Round-trip Unicode/reserved/long values; reject malformed encoding; valid control reaches native execution | Hand-specified pairs and standard percent codec | Apply JiraKey validation in reference/jira.rs; slash/long values fail C2 | Core query reference contract | Focused contract | PENDING — checkpointed-build, grammar slice |
| C3 | Bind continuation before egress | Site/authority and cursor owner cells | Change one owner dimension; invalid_reference and zero new requests; unchanged owner positively advances | Independent owner fixture and server request ledger | Omit query comparison in jira/cursor.rs; changed-query request causes C3 red | Jira query cursor ownership contract | TLS fixture | PENDING — checkpointed-build, query slice |
| C4 | Preserve native order and terminal authority | Query meaning, rows, page metadata | Descending multi-page sequence, short/empty nonterminal progress and exact terminal; compare ordered output and navigation | Frozen ordered sequence and evidence P2 direct reads | Numerically sort rows in jira/query.rs; C4 order mismatch | Jira query native-order/terminal contract; ignored live query smoke | TLS fixture plus live tenant | PENDING — checkpointed-build, query slice |
| C5 | Publish only validated stable-ID links | Row identity and authority cells | Foreign/mismatched authority after valid rows fails atomically; otherwise-identical valid data publishes expected links including IDs beyond u64 | Configured origin and manually specified stable-ID links | Bypass row authority validation in query assembly; C5 foreign case incorrectly succeeds | Jira query authority contract | TLS fixture | PENDING — checkpointed-build, query slice |
| C6 | Bound atomic logical pages | Rows, metadata, limits, token progress/size | Verify 1,000-row and ten-attempt partition, retry consumption, malformed/duplicate later rows and unrepresentable/repeated tokens; all-valid control succeeds | Fixture ledger independently partitions explicit row/attempt budgets | Remove physical-attempt stop in jira/query.rs; page completion/continuation fails C6 when shared transport refuses the extra fetch | Jira query atomic-bound contract | Bounded synthetic workload | PENDING — checkpointed-build, query slice |
| C7 | Replay only explicit reads within shared budget/deadline | Method/intent, retry and deadline cells | 503 then success repeats read-only POST body; ordinary POST/PATCH remain single attempt; later page cannot regain retry; wait/cancel controls observe no extra request and successful control proves ledger | Server body/attempt ledger and controlled retry clock/jitter | Make ordinary POST replayable in http/request.rs; second mutation POST makes C7 red | Bounded HTTP read-only POST and retained mutation contracts | Local HTTP/TLS fixture | PENDING — checkpointed-build, HTTP slice |
| C8 | Distinguish safe native rejection from bad references | Error documents and local malformed references | Valid native 400 gives invalid_pattern; malformed envelope gives source_unavailable; local failure has no egress; server positively receives canaries which errors must omit | Published Error Collection, category table and explicit canary bytes | Forward errorMessages prose in wire/query.rs; C8 exposes canary | Jira native-query error/privacy contract | TLS fixture | PENDING — checkpointed-build, query slice |
| C9 | Keep queries uncached under shared transport controls | Cache, status, egress, bytes and cancellation cells | Repeat query with changed validator-bearing response; second content changes; denied/oversized/cancelled paths fail while corresponding granted/in-bound/completed controls succeed | Server ledger/bytes and configured grants/ceilings | Use fetch_cached for query in jira/transport.rs; stale second result makes C9 red | Jira query no-cache and shared HTTP control contracts | TLS fixture | PENDING — checkpointed-build, query slice |
| C10 | Preserve both public recovery channels | Clipped/unclipped crossed with terminal/nonterminal | Drive real MCP read, recover omitted current-page content, then navigate next source page; compare complete content and next identities | Full fixture content ledger and separate next-page identities | Drop continuation at query SourceResource construction; public navigation missing makes C10 red | MCP query recovery contract | Real stdio binary / controlled upstream | PENDING — checkpointed-build, tool slice |
| C11 | Retain regex semantics and unsupported glob | Search pattern/match and glob cells | Real MCP search with non-JQL regex and PCRE2 case finds expected regions; invalid regex/glob retain categories; successful read proves mount | Hand-computed match regions and existing category contract | Route Query search to unsupported in Jira facade; missing matches make C11 red | MCP query search/glob contract | Real stdio binary / controlled upstream | PENDING — checkpointed-build, tool slice |
| C12 | Enforce new module ledger | New owners, interfaces, dependencies and parent deltas | Source/diff census requires owners and forbids wrong bodies/imports; injected forbidden body must fail and restoration pass | Approved ledger and starting-tree diff | Move query assembler into jira.rs; C12 names forbidden owner | .rfs-nae2/oracles/module_shape.py | Census once new owners exist | PENDING — checkpointed-build, each structural slice |

The full Claims and Input shapes sections define each row's keyed claim and shape set. Every failure must report its C-number and observed mismatch. Mutation sites may be technically corrected to actual symbol names without changing their meaning; changed oracle meaning or architecture requires renewed approval.

## Non-goals and future work

Permanent non-goals for this interface:

- No ResourceFS JQL parser, normalization, query rewriting or cross-site fan-out: Jira owns query meaning and the Site Mount owns execution authority.
- No query-response caching, fabricated snapshot consistency, inferred effective page maximum or immortal token guarantee: they contradict accepted native-query policy or lack evidence.
- No query-result writes: Query Resources are read-only navigation/projection, not a mutation target.
- No arbitrary provider diagnostic prose: structured counts/status preserve the accepted privacy contract.
- No new public MCP tool or generic native-query framework: existing resource/search interfaces suffice and there is only one relevant native-query adapter.

Intended future work introduced by this design: none. Existing dependent tickets are not absorbed or removed.

## Falsifier run log

- C1 / inherited C16, 2026-09-07: `python .rfs-h212/oracles/module_shape.py --stage issues`, cwd `/tmp/rfs-nae2-query`, merged baseline production tree, exit 0 in 0.12s. Reported `C16 PASS stage=issues baseline=0b81d3118fde18fb4df5679ebb5830d67a124a64`; current parent counts: reference 1941/1989, discovery 1341/1367, Jira facade 285/317, wire 584/601, render 482/488, HTTP 1666/1679. This proves retained placement only, not the unimplemented query owners. C12 remains PENDING and must extend the fence at implementation.
- Empirical P2/P3 are retained from evidence.md with exact inputs/results; they support design premises, not completed feature claims. Owned live fixtures were cleaned after these runs.

## Approval

Requester: "Approve design" (Ask selection), 2026-09-07.

Approved: behavior interpretation, private module ownership, explicit read-only POST replay, structured-only diagnostics, module ledger and falsification plan.

Risk acceptances: None.

### Technical verification corrections

- Caller analysis found no existing read-only POST consumer. Plan partition now keeps S1 behavior-preserving request extraction and lands C7 in S2 with its real query consumer; no dead-code suppression or public constructor widening. The table's HTTP discharge names ownership, not a requirement to publish an unused interface.
- MCP Atlassian profile launch is not implemented in the baseline and belongs to verified rfs-ue44. C10/C11 retain real MCP stdio tool proof through an existing private ResourceFsServer constructor hosted by a cfg(test) child/reexecuted test binary, following the existing HTTPS fixture pattern. Tests live in `crates/resourcefs-mcp/src/server/jira_query_tests.rs`; server.rs gains only a test-gated declaration, with no production interface/body change. These results do not claim shipped Atlassian profile launch.
- The existing public-tool protocol and independent content/navigation oracle are unchanged; this is a test location/hosting correction under approval semantics. The original approval and no-risk-waiver decision remain applicable.
- C1's compiling wrong-owner mutation relocates the self-contained `encode_jira_segment` to reference.rs instead of cursor serialization with private dependencies. The inherited owner rule and expected wrong-owner failure are unchanged. C12's retry-copy relocation minimally exposes its bookkeeping field to keep the source-copy mutation compilable. Both fail their intended owner census and pass compilation; restoration is green (plan.md S1 checkpoint).
