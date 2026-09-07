# Plan: rfs-nae2

## Inputs and partition

Design approved by requester: "Approve design", 2026-09-07; no risk acceptances. Route/evidence/design are the authoritative inputs. Main owns integration and checkpoint records. All writers use isolated worktrees and skip builds, tests, lint and formatting; Main supplies that proof after integration at each slice. No slice is complete merely because a writer returns.

Two atomic slices, dependency S1 → S2. S1 is a behavior-preserving request/header/replay extraction; S2 adds explicit read-only POST intent atomically with its real Jira consumer and the complete query feature/public-tool fences. Caller analysis found no existing read-only POST consumer: putting a crate-private constructor in S1 would leave dead code in non-test builds. No suppression, public visibility widening or invented consumer is permitted. This corrects partitioning, not approved architecture or behavior.

Diff projection: S1 1,600 lines including request extraction, tests and workflow artifacts; S2 2,900 lines including query implementation, deterministic/live/tool fences and docs. Churn margin 25% for fixture/verification corrections: total 5,625. Partition I1=S1 (2,000 with margin), I2=S2 (3,625 with margin). Open a draft PR at I1's checkpoint, then stack I2. Actual growth is checked at each checkpoint; an increment exceeding 4,000 requires repartition before advancing. No silent narrowing of acceptance.

## Shared proof conventions

- Formatter: `cargo +1.98.0 fmt --all`. Unique integration target: `/home/dwalleck/.cache/resourcefs-targets/rfs-nae2`; never share targets between writer worktrees. Use the persistent filesystem rather than `/tmp` tmpfs (29 GiB free at planning), avoiding RAM-backed build-storage pressure.
- Commands below run in `/tmp/rfs-nae2-query`, with `CARGO_TARGET_DIR=/home/dwalleck/.cache/resourcefs-targets/rfs-nae2`; exact test target/name corrections may be recorded without changing the approved oracle.
- Main records caller analysis using LSP references when available, with regex safety net; writers supply reuse and symmetry notes. Existing JSON, percent codec, cursor, row decoder, renderer and retry machinery must be reused.
- Every new/changed fence needs compiling mutation red and restored green. Shape mutations run against disposable source copies; no behavioral production test reads source text.
- Main runs relevant deterministic contracts at each checkpoint; final assembled workspace quality gates and native Linux/macOS/Windows CI precede delivery. Live query adapter and real-stdio smoke use reader-only credentials, with separate marker-owned setup/cleanup recorded in evidence.md.
- Stable categories follow design/rfs-lbps. Error bodies, query strings, credentials and upstream tokens never enter diagnostics. Tests use explicit canaries with positive observer controls.

## Module growth ledger

C=`crates/resourcefs-core/src/`; S=`crates/resourcefs-sources/src/`. Counts use the same physical-line census as the approved inherited fence; they are conservative tripwires, not depth measurements.

| Module | Baseline | Projected final | Responsibility/interface change | Protected-parent rule |
|---|---:|---:|---|---|
| C reference/jira.rs | 376 | 470–600 | Query newtype, sum variant and canonical parsing | N/A — existing grammar owner |
| C reference/source_page.rs | 97 | 97–120 | Necessary selector wiring only | N/A — existing syntactic owner |
| C reference.rs | 1941 | 1945–1965 | Export/exhaustive composition | No query bodies; max 1989 |
| C discovery.rs | 1341 | 1341–1350 | Existing-interface wiring only if needed | No native query knowledge; max 1367 |
| S atlassian/jira.rs | 285 | 290–310 | Child and dispatch/capability wiring | No query loop/retry; max 317 |
| S atlassian/jira/query.rs | 0 | 220–380 | Atomic native-order query page | N/A — new approved owner |
| S atlassian/jira/cursor.rs | 151 | 170–230 | Exact Query owner validation | N/A — existing cursor owner |
| S atlassian/jira/transport.rs | 279 | 320–390 | Uncached constant-endpoint query fetch | N/A — existing transport owner |
| S atlassian/jira/browse.rs | 330 | 330–350 | Shared visibility only if needed | No native query loop |
| S atlassian/wire/query.rs | 0 | 90–180 | Bounded payload and rejection structure | N/A — new approved wire owner |
| S atlassian/wire/collections.rs | 529 | 529–560 | Shared decoder reuse | No duplicate codec |
| S atlassian/render/collections.rs | 173 | 173–200 | Shared order-preserving rendering | No sorting or fetch |
| S atlassian/wire.rs | 584 | 587–595 | Child declarations/reusable visibility | No query codec; max 601 |
| S atlassian/render.rs | 482 | 482–486 | Visibility only if needed | No query body; max 488 |
| S http/request.rs | 0 | 180–260 | Move construction/header/replay cluster; explicit read intent | N/A — new approved owner |
| S http/read.rs | 192 | 210–340 | Intent-based replay and original-deadline wrapper | No source types |
| S http/mod.rs | 1666 | 1450–1550 | Remove request cluster; re-export/wire | Net shrink; no new policy body |
| Cargo.toml, sources/Cargo.toml, Cargo.lock | Existing manifests | Direct bytes entry and lock dependency edge | Immutable retry body ownership | No core infrastructure dependency |

## Slice S1: Extract request ownership without changing behavior

**Claim IDs:** C1. C12's staged request-owner checks apply here; its complete query-owner discharge belongs to S2. C7 lands wholly in S2.

**Expected behavior:** Existing GET replay, ordinary POST/PATCH single-attempt behavior, headers, credential/redirect policy and mutation body ceilings remain unchanged after request construction/header/replay ownership moves to its child. Body storage becomes immutable shared bytes without changing transmitted bytes. Read-only POST intent and the deadline-wrapper change remain together in S2.

**Oracle:** Existing HTTP contract expectations and independently recorded server method/body/attempt sequences; C1 inherited declaration census.

**Stress fixture:** Existing ceiling-sized mutation bodies, exact request bytes, ordinary POST/PATCH retryable-status response and existing GET retry controls. Expected: same accepted/refused bounds and transmitted bytes; only existing GET can replay.

**Regression fence:** Existing HTTP contracts plus owning-module equivalence tests, with cfg-only `http/read_tests.rs` if needed to preserve the production owner's tripwire. `.rfs-nae2/oracles/module_shape.py --stage http` extends inherited placement rules.

**Named mutation:** C1 move existing encode_cursor to the wrong owner in a disposable source copy; staged C12 move retry_copy back into HTTP parent. Observe localized rejection and restored green. Existing behavioral fences retain applicable mutation evidence or receive refreshed proof if changed.

**Complexity/production scale:** Body Vec-to-Bytes ownership transfer avoids a payload copy; existing GET replay has no body. Header validation retains 16 headers/16 KiB. No new runtime loop, request or serialization pass. Exercise existing ceiling-sized HTTP mutation fixture and compare transmitted bytes.

**Wall budget/phase:** Always-on construction retains the existing HTTP mutation production-budget fence and ceiling fixture; no new timed phase is introduced. New POST replay/deadline budget belongs S2. One-off module census: N/A — no runtime wall budget.

**Module shape:** Extract request construction/header/replay ownership to http/request.rs. Parent http/mod.rs net -116 to -216 lines; only re-export/client wiring allowed. http/read.rs retains retry ownership. `python .rfs-nae2/oracles/module_shape.py --stage http` must report C1/C12 staged PASS; required query owners are not checked until S2.

**Files:** `Cargo.toml`, `Cargo.lock`, `crates/resourcefs-sources/Cargo.toml`, `crates/resourcefs-sources/src/http/mod.rs`, `http/request.rs`, `http/read.rs`, `http/read_tests.rs`, necessary existing HTTP contracts, `.rfs-nae2/oracles/module_shape.py`, `scripts/ci-gates.py` staged oracle invocation, issue workflow artifacts.

**Estimate:** One focused transport/extraction implementation and checkpoint; estimate is a planning signal only.

**Diff estimate:** 1,600 lines, including moved lines, tests and artifacts.

**PR increment:** I1; mergeable transport behavior with real bounded-read tests, independent of JQL implementation.

**Commands and expected results:**
- `cargo test -p resourcefs-sources --lib http::` → controlled body/attempt/deadline oracle agrees, mutation does not replay, relevant existing unit behavior unchanged.
- Existing affected HTTP integration targets selected from the recorded caller analysis → TLS/egress/headers/mutation behavior unchanged.
- `python .rfs-nae2/oracles/module_shape.py --stage http` → staged ownership and inherited constraints PASS.
- Run C1 and staged C12 misplaced-owner source-copy mutations → exact path/symbol failure; restore → green.
- Existing release HTTP mutation budget target → unchanged content/bounds and accepted cost.

## Slice S2: Deliver exact site-scoped JQL through existing tools

**Claim IDs:** C2, C3, C4, C5, C6, C7, C8, C9, C10, C11, C12; rerun affected C1 proof without treating unchanged empirical premises as new feature proof.

**Expected behavior:** All six route acceptance clauses work end-to-end. Introduce explicit read-only POST intent/deadline handling, query grammar, execution, cursor ownership and wire error mapping atomically with real consumers, all exhaustive callers and source/tool contracts. No unused constructor or parser-only public state with an unimplemented source path.

**Oracle:** Hand-specified canonical pairs; independent request/row/attempt ledger; fixture-provisioned identities and direct reads for live order; published Error Collection; full-content and next-page ledgers for public recovery/search. See design C2–C12 for decisive controls.

**Stress fixture:** 1,000 distinct descending stable IDs across native pages; ten-attempt boundary including retry; short/empty nonterminal pages; later malformed/duplicate/foreign rows; ceiling-length encoded query and cursor; exact-terminal and clipped/nonterminal recovery; native validation canaries. Expected: exact original row order, no eleventh request, representable next owner, or atomic categorized failure with no partial Resource; recover every omitted current-page byte independently of next-page navigation.

**Regression fence:** `crates/resourcefs-core/tests/jira_query_reference_contract.rs`; `crates/resourcefs-sources/tests/jira_query_contract.rs`; owning HTTP tests for C7; cfg-only `crates/resourcefs-mcp/src/server/jira_query_tests.rs` for real MCP stdio; ignored live adapter and test-host stdio rows; `.rfs-nae2/oracles/module_shape.py --stage query`. Reuse existing fixture utilities; do not duplicate servers.

**Named mutation:** Design rows C2–C12: key restrictions, omitted query-owner comparison, numeric sorting, bypassed authority, removed query attempt stop, ordinary POST made replayable, forwarded native error prose, cached query, dropped public continuation, unsupported query search and misplaced assembler. Every behavioral mutation must compile and cause its intended observation, not an earlier unrelated error.

**Complexity/production scale:** Parsing/encoding O(Q), Q within existing reference ceiling. JSON request encoding O(Q+T), preflight length guard before allocation. Native response decoding O(B) under existing HTTP body ceiling. Assembly O(R) expected using existing duplicate-detection structures, R≤1,000 and ≤10 attempts; no sorting and no complete traversal beyond one logical page. Rendering O(output bytes), with existing lossless artifact ceilings; accumulated native input bounded by ten response ceilings. Maximum accepted cost: one decode/render pass per row and bounded owner encoding per native page, never quadratic repeated whole-page reconstruction.

Read-only POST construction is O(B), retry copies share immutable B-byte storage in O(1); at most two attempts and one retry permit across the logical operation. A 384 KiB payload with 10,000 replay copies is the stress input; no repeated payload serialization/copy is permitted.

**Wall budget/phase:** Always-on parse/owner/render CPU for 1,000 rows with 4 KiB summaries, long query and representable Unicode token: ≤2s release / ≤10s debug excluding compilation and network, with deterministic content/count assertion; repeated parsing must not scale quadratically. Network operation uses unchanged configured deadline. One-off fixture bootstrap/cleanup and shape census: N/A — no runtime wall budget.

The added HTTP always-on construction/replay stress has a ≤1s release / ≤5s debug wall bound excluding compilation/network. Original network deadline, cancellation and shared attempts have controlled request-ledger proof; timing alone is not a replay-allocation oracle.

**Module shape:** Add request intent in existing extracted request.rs and deadline/retry handling in read.rs; new owners query.rs and wire/query.rs; deepen grammar/cursor/transport, reuse compact decoder/renderer. Parent deltas: reference +4–24, discovery 0–9, Jira facade +5–25, wire +3–11, render 0–4, HTTP parent 0 beyond S1. MCP server gains only a cfg(test) module declaration, no production interface/body. `python .rfs-nae2/oracles/module_shape.py --stage query` → ownership/visibility/directions/tripwires PASS. Independent reviewer reconstructs assembled ownership before seeing design, then compares.

**Files:** All S2 production rows in the growth ledger as needed; named core/source/MCP contracts and existing fixture/live helpers; owning API/domain docs after smoke proof; shape oracle and CI invocation; issue evidence/design/plan/tracker records. No unrelated parent-checkout changes.

**Estimate:** One complete adapter-to-tool implementation plus deterministic/live/mutation/native-CI checkpoints; estimate is a signal only.

**Diff estimate:** 2,900 lines.

**PR increment:** I2 stacked on I1; mergeable complete JQL behavior, independently verified against controlled upstream and owned live fixtures.

**Commands and expected results:**
- `cargo test -p resourcefs-core --test jira_query_reference_contract` → canonical corpus matches independent expected values, malformed inputs fail locally.
- `cargo test -p resourcefs-sources --test jira_query_contract` → exact wire/order/ownership/authority/budget/error/cache oracle agreement.
- `cargo test -p resourcefs-mcp --lib server::jira_query_tests` → real MCP service over reexecuted stdio test-host read/recovery/search/glob agrees with content/navigation ledger.
- `python .rfs-nae2/oracles/module_shape.py --stage query` → C1/C12 PASS; injected wrong-owner body → localized red; restored → green.
- Focus each design mutation on the corresponding named contract → intended semantic red and restored green, recorded per claim.
- `cargo test --release -p resourcefs-sources --test jira_query_contract` → scale fixture matches content and measured budget.
- Gated real-adapter and stdio live commands, with reader-only execution environment → order/identity/terminal/validation shapes agree with independent fixture receipt/direct reads; cleanup succeeds.
- Repository formatter, clippy and workspace suite plus exact-head native CI → assembled quality and supported platforms pass before delivery.

### Technical proof-location correction

Source inspection found Atlassian is not yet mountable through the shipped Server Profile: profile integration belongs to verified ticket rfs-ue44. Existing ResourceFsServer::new is private. C10/C11 therefore use a cfg(test) server child and real stdio reexecution, following the existing HTTPS test-host/banner-handling pattern. Production constructor visibility, profiles and tool interfaces do not change. This proves the real MCP tool protocol/service, not a production `rfs --profile` Atlassian launch. The independent content/navigation oracle and approved feature scope are unchanged. Production profile integration remains with rfs-ue44.

## Plan critique and self-review

- Two independent increments; public Query grammar and implementation are atomic in S2. No fence waits for a later feature slice.
- All claims assigned exactly once for primary discharge; staged structural checks run in both slices. Existing C1 proof applies only until changed ownership invalidates it.
- All fourteen slice fields present. No approved-risk gaps, anonymous future work or new language/platform assumptions. TLS/process fixtures must reuse the cross-platform helpers fixed by h212.
- Request retry guard is subtractive: ordinary mutations and timeout behavior require explicit symmetry checks. Fixed browse sorting must not bleed into query assembly.
- Main supplies validation prohibited to writers at each slice checkpoint. At initial planning no implementation had started; completion receipts follow below rather than being presumed by the plan.

## S1 checkpoint

Integration owner: Main. S1 is behavior-preserving extraction; C7 and query execution remain unimplemented until S2.

Caller analysis: LSP returned no references, so text fallback covered HTTPS `https.rs:51,176`, GitHub `github/mod.rs:248`, mutation `github/mutation.rs:328,503,668`, Jira `atlassian/jira/transport.rs:70`, and existing HttpRequest re-exports. The moved shared header limit also serves parent `http/mod.rs:107,616,1050,1054`; its pub(super) visibility/import was repaired after the first compile exposed those consumers.

Reuse/symmetry: unchanged constructors, GET-only replay, non-GET path, mutation ceilings and TLS/header/credential dispatch. Vec ownership transfers to Bytes without serialization. New test code reuses the real TLS fixture. No protocol, category or public interface changes.

| Gate | Result | Evidence |
|---|---|---|
| 1 Affected tests | PASS | HTTP unit 9 passed (artifact://1104); HTTP/TLS/credential targets 39 passed (artifact://1106); GitHub adapter/mutation targets 36 passed (artifact://1109) |
| 2 Assigned falsifiers | PASS | C1 and staged C12 census reports exact owner placement; http parent 1491 lines versus baseline 1666 |
| 3 Stress | PASS | Release 64 MiB mutation sent completely, HTTP 201, within existing 500ms assertion (artifact://1114; test total 0.06s, not a separately printed transfer measurement) |
| 4 Implementation/oracle | PASS | Independent TLS request ledger agrees on exact encoded mutation body, GET headers, attempt counts and categories; the 64 MiB fixture observes complete length. Native JQL empirical comparisons remain valid premises, not S1 feature claims |
| 5 Module shape | PASS | `--stage http` passes; only request ownership moved; C12 full query-stage discharge remains assigned S2 |
| 6 Production budget | PASS | Existing always-on 64 MiB HTTP budget passes; no new loop/phase. New POST replay scale belongs S2 |
| 7 Regression fences | PASS | New owning-module and retained consumer contracts green; proof receipts in s1-mutation-results.json |
| 8 Named mutations | PASS | Compiling source-copy wrong-owner mutations red; four compiling behavioral mutations red for GET replay, body loss, attempt off-by-one and cancellation category |
| 9 Restored fences | PASS | Source-copy shape restored green; restored HTTP unit suite 9 passed (artifact://1124); integration tree never received mutations |

Technical mutation correction: C1 relocates the self-contained encode_jira_segment into reference.rs and imports it from the child, rather than moving cursor serialization with its private dependencies. This preserves the qualified-owner oracle while allowing `cargo check -p resourcefs-core` to pass (artifact://1113). C12 relocates retry_copy into the parent with the minimum field visibility needed to remain compilable (`cargo check -p resourcefs-sources`, artifact://1119); it fails on exact wrong/missing owner, not compilation. Temporary mutation sources were removed after restored proof.

Independent conformance: JqlHttpReviewer reconstructed request/read/client ownership before seeing design, then compared the approved ledger and reported no findings (agent://JqlHttpReviewer). It inspected the shared-header repair. No new public surface, second client, duplicated implementation or parent responsibility was found.

Assembled quality: workspace all-target/all-feature clippy passes with warnings denied (artifact://1125); pinned 1.98.0 formatting passes. Native-platform execution is delegated to the draft PR's exact-head CI, not claimed from local Linux results. CI now invokes the staged new oracle explicitly; S2 changes the stage to query.

Cleanup/sweep: no TODO/deferred/failure-suppression residue in touched request/read code. No user-facing behavior changed, so Behavior Contract/domain/changelog content intentionally remains unchanged. Issue workflow/proof artifacts retain the evidence. No live tenant fixtures remain from empirical probes.
