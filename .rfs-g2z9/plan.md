# Plan: the bounded HTTP substrate and the `https://` Source Adapter

## Inputs and partition arithmetic

- Route: Empirical (`.rfs-g2z9/route.md`). `spec.md` signed and re-signed 2026-08-23; `evidence.md` P1–P4 PASS; `design.md` **approved** ("I approve", 2026-08-23), claims **C1–C19**, no risk acceptances. Design gate verified: Falsification table complete with no empty cells, cheapest falsifier (C15 baseline) **PASS**, no row `FAIL`, every other row `PENDING — checkpointed-build` naming slice assignment here.
- Slice diff estimates: S1 320 + S2 480 + S3 960 + S4 300 + S5 300 + S6 380 + S7 520 + S8 780 + S9 320 = **4,360 changed lines**.
- Churn margin: **25% = 1,090 lines**. Rationale: this change introduces the workspace's first HTTP/TLS dependency (transitive-tree fallout on `Cargo.lock` and the architecture fence), adds a variant to the closed `ResourceAddress` sum (arm cascades across every exhaustive match, which in rfs-60g1 ran 25% above the estimate — 10 sites found against 8 projected), and every network fence needs loopback listener scaffolding whose first-use cost is routinely underestimated.
- Projected total: **5,450 changed lines** — this **exceeds** the 4,000-line review-size gate, so the plan is partitioned into **three independently mergeable PR increments** in dependency order.

### PR increment A — Policy foundation (S1, S2 — 800 lines)

Mergeable definition against the repository default branch (`main`): `https://` parses as a first-class address family, the encoded-separator guard is narrowed to filesystem-backed families, and the source-neutral HTTP policy types (address classification, `base_url` allowlisting, ceilings) exist in `resourcefs-core` with **zero new dependencies**. Verifies entirely through parser golden tables and pure policy unit tests — **no network, no HTTP client, no sockets**. Contains no transport, so it is reviewable as a self-contained security-decision layer before any egress exists.

### Sequencing gate between increments A and B (requester decision, 2026-08-23)

**Increment B must not start until `rfs-q12y` lands.** The requester directed that the dependency-vetting gate — `deny.toml` plus a CI workflow — be completed between increments A and B, so that reqwest's transitive tree (the largest single dependency addition this project has made, and security-critical code on the network egress path) enters the workspace under a vetting gate rather than into a vacuum. Increment A introduces no dependency and is unaffected; increment B's first slice (S3) is where reqwest appears. This is a sequencing constraint on the increments, not a change to any slice's contents: S3 still records the introduced tree (`cargo tree -i reqwest`, `cargo tree --duplicates`) in its commit message, and now also runs the `rfs-q12y` gate against it.

### Plan revision 1 — increment B slice order corrected (2026-08-23)

**Execution order within increment B is now S3 → S6 → S4 → S5 → S7.** The original order inverted a dependency: Slices 4, 5, and 7 all need a fixture that completes a real HTTPS exchange, and **only Slice 6 was scoped to build one**.

The constraint is structural, not incidental: `AllowedOrigin::new` rejects any non-`https` `base_url` (`http_policy.rs:210`) and `authorizes()` requires scheme equality (`:247`), so **every** request the substrate makes is `https`. Slice 3's plain loopback listeners were sufficient there only because its oracle is the TCP accept that *precedes* the TLS handshake — its own comment records this. For Slice 4 the same handshake failure is fatal: with no TLS session there is no `302`, so no redirect is attempted and the off-allowlist path log would be empty **because nothing worked**, not because policy refused. That fixture would have passed for the wrong reason — the fourth such case in this project, and the most convincing.

**Slice 6 therefore also owns the shared TLS fixture helper** that Slices 4, 5, and 7 consume: a loopback TLS listener, a static self-signed PEM embedded in test support (no `rcgen`), and a `#[cfg(feature = "test-support")]` constructor mirroring the existing `with_host_lookup` that adds the fixture root via `ClientBuilder::add_root_certificate`. `danger_accept_invalid_certs` is forbidden — it would disable the very verification C6 exists to prove.

**No new packages.** `tokio-rustls` v0.26.4 and `rustls` v0.23.43 are already in the resolved tree via `reqwest → hyper-rustls` (verified); declaring them dev-dependencies of `resourcefs-sources` adds dependency *edges* only, so `cargo deny check` is unaffected and nothing new needs vetting.

No claim, oracle, fence, or named mutation changes — every approved artifact stands. This is ordering and fixture ownership only, within `budgeted-plan`'s own artifact. Housekeeping: `http_policy_contract.rs`'s header comment lists the claims it covers and must be updated as each slice extends it.

### PR increment B — Bounded substrate (S3–S7 — 2,460 lines)

Mergeable definition: the single bounded HTTP substrate exists in `resourcefs-sources/src/http/`, constructs the workspace's only HTTP client, enforces address policy inside the resolver, authorizes every redirect hop, bounds and cancels body reads, works identically over TLS with hostname-bound certificate verification, and extracts deterministic reader-mode Markdown. Verifies against loopback listeners and local TLS fixtures without any Source Adapter. Depends on increment A; does not depend on C.

### Plan revision 2 — increment C gains Slice 10 and runs S8 → S10 → S9 (2026-08-23)

Design Revisions 2 and 3 added claims **C20** (credential transmission plus its leak-freedom) and **C21** (read-path cancellation) after this plan was written. They get **Slice 10**, and it runs **before Slice 9**, because Slice 9's helper search proved C14's second half cannot be fenced honestly without it.

The coupling, verified: `server.rs:1340` passes `None` for https, and an unmounted reference returns `SourceUnavailable` / *"no HTTPS origins are configured in the Server Profile"* (`compiled.rs:173-179`). So an optional source that is **degraded** and one that is **healthy** are observationally identical — both fail with `source_unavailable` because neither is ever mounted. A fence asserting "optional degrades → only its references fail" would pass with the entire degradation mechanism deleted. That is a second cause producing the same observation, which `falsifiable-design` step 6's Falsifier criterion now forbids, and the positive control that criterion requires — optional + reachable actually serving — is unreachable until an `HttpsSource` can be constructed at launch.

Slice 10 lands that wiring once: `configured_sources` → `HttpsSource` at launch, the `CredentialHeader` accessor, request-header support on `HttpRequest`, and the `OperationGuard` on the read path. Slice 9 then fences both halves of C14 with optional+reachable as its positive control.

No claim, oracle, fence, or named mutation changes; C20 and C21 were approved in design Revisions 2 and 3. This is ordering and slice creation inside `budgeted-plan`'s own artifact. Note also that **C15 is already fully discharged by Slice 3** (`architecture_contract.rs::single_http_client_module` and `http_policy_contract.rs::probe_egress_is_policed` both exist and pass); Slice 9 owes it only a re-run in the final integration check.

### PR increment C — HTTPS source and lifecycle (S8, S10, S9 — 1,100 lines projected, revised)

Mergeable definition: `HttpsSource` fills the read/discovery/catalog seams, HTTPS is read-only through the public tools, no credential or resolved address leaks on any channel, the encoded-URL wire fidelity is proven end to end, and the required-fails/optional-degrades lifecycle plus the policed probe behave per the rfs-r9m6 exit/channel table. Verifies through stdio MCP and compiled-CLI contracts. Depends on A and B.

---

## Slice 1: Admit `https://` as an address family and narrow the encoded-separator guard

**Claim IDs:** C1

**Expected behavior:** `https://<url>` parses to `ResourceAddress::Https(HttpsAddress)` for well-formed HTTPS URLs — including query, fragment, explicit port, internationalized host, and **percent-encoded path or query containing `%2F`/`%5C`** — and is parsed **before** the generic `input.contains("://")` rejection at `reference.rs:840`. `http://` (plain) is still rejected, and every existing family (`rfs://`, `rfs://workspace/…`, `artifact://`, `local://`, relative, absolute, `file://`) retains its exact current parse result. The global `validate_percent_encoding` call is narrowed to filesystem-backed families (workspace and `local://` scratch names) as C1-enabling implementation; every exhaustive `ResourceAddress` match gains a real `Https` arm (never a catch-all) returning a stable `source_unavailable` "https source not mounted" until increment C mounts the adapter.

**Oracle:** Hand-authored accept/reject table mapping input string → expected `ResourceAddress` variant or `ErrorCategory`, written from `spec.md`'s grammar and never computed by calling the parser.

**Stress fixture:** `https://example.com/doc`; `https://example.com/search?q=a%2Fb` (encoded separator in query); `https://example.com/a%2Fb/doc` (in path); `https://例え.jp/doc` (IDN); `https://example.com:8443/doc`; `https://example.com/doc#frag`; `https://example.com` (bare origin); `http://example.com/doc` (must reject); plus every existing family shape and the two existing encoded-separator rows (`a%2Fb`, `local://a%2Fb`) which **must still reject**. Expected: HTTPS rows yield `Https`; `http://` yields `invalid_reference`; existing families unchanged; filesystem encoded-separator rows still `invalid_reference`.

**Regression fence:** `crates/resourcefs-core/tests/path_reference_contract.rs::https_grammar` (created in this slice), plus the existing golden corpus — `tests/fixtures/workspace_references.json:33-34` and `path_reference_contract.rs:345,349` already fence the filesystem side of the narrowed guard, so over-narrowing turns existing tests red immediately.

**Named mutation:** From C1 — in `reference.rs`, move the `https://` arm *after* the generic `contains("://")` rejection; `https_grammar` turns red because a valid HTTPS URL returns `invalid_reference`. Restore → green.

**Complexity/production scale:** N/A — reason: URL validation is a constant-bounded parse of a reference already capped at 64 KiB by `validate_reference_input`; no new loop over unbounded input.

**Wall budget/phase:** N/A — reason: folded into the existing per-request `PathReference::parse` path; no new runtime phase.

**Files:** `crates/resourcefs-core/src/reference.rs` (add `ResourceAddress::Https(HttpsAddress)`, `HttpsAddress` wrapping a validated `url::Url`, parse ahead of the generic scheme rejection, narrow the `validate_percent_encoding` call site in `validate_reference_input`); `crates/resourcefs-core/src/lib.rs` (export); real `Https` arms in every exhaustive `ResourceAddress` match — known sites `crates/resourcefs-core/src/{discovery.rs,mutation.rs,read.rs,resource.rs,session.rs}` and `crates/resourcefs-sources/src/{artifact.rs,compiled.rs,filesystem_mutation.rs,local.rs}` — **verify the true set by compiling; rfs-60g1's equivalent slice found 10 sites against 8 projected**; `crates/resourcefs-core/tests/path_reference_contract.rs`.

**Estimate:** 0.75 day. **Diff estimate:** 320 (170 impl, 150 tests). **PR increment:** A — Policy foundation.

**Commands and expected results:**
- **Mutation applicability check, immediately after implementing and before the gate:** confirm C1's named mutation compiles and turns `https_grammar` red. The last change had five of fourteen mutations that could not fail as written; if this one cannot, correct the **fixture** so it can (never the assertion) and record the correction.
- `cargo test -p resourcefs-core --all-features --test path_reference_contract` → every accept/reject row agrees item-by-item with the literal table; existing family rows unchanged; the two filesystem encoded-separator rows still reject. Under the named mutation `https_grammar` fails on a valid HTTPS row; restored → green.
- `cargo test --workspace --all-features` → the existing golden corpus stays green, proving the guard was narrowed and not removed.
- `cargo build --workspace --all-targets --all-features` → compiles with a real `Https` arm at every match site and no catch-all.

---

## Slice 2: Source-neutral HTTP policy types in core

**Claim IDs:** C10, C11

**Expected behavior:** `resourcefs-core/src/http_policy.rs` provides pure decision logic with **no network and no client dependency**: `AddressClass` classification over `std::net::IpAddr` (loopback, RFC1918 private, link-local, CGNAT 100.64/10, multicast, unspecified, IPv4-mapped-IPv6 embedding a private v4, public); `AddressPolicy::authorize(IpAddr) -> Result<(), ResourceError>` parameterized by an origin's `allow_private_network`; `OriginAllowlist::authorize(&Url) -> Result<&AllowedOrigin, ResourceError>` implementing `base_url` **prefix** scoping; and the `HttpCeilings` triple (fetch 8 MiB, redirect depth 5, timeout 30 s), each lowerable and never raisable. Core gains **zero new dependencies** — `url` is already present and `std::net` is free.

**Oracle:** Two hand-authored literal tables — an address-class table derived from the RFCs (not from the classifier's branching) and a URL→verdict table for `base_url` scoping — both written from `spec.md` and the RFCs rather than by running the code.

**Stress fixture:** Addresses `127.0.0.1`, `::1`, `10.0.0.1`, `172.16.0.1`, `192.168.1.1`, `169.254.1.1`, `fe80::1`, `100.64.0.1`, `224.0.0.1`, `0.0.0.0`, `::`, `::ffff:10.0.0.1` (IPv4-mapped private), `8.8.8.8`, `2001:4860:4860::8888` — each under `allow_private_network` false **and** true. URLs: exactly the `base_url`; a path beneath it; a sibling path outside it; the same host with a different port; a prefix-collision case (`base_url` `https://ex.com/docs` versus URL `https://ex.com/docsecret` — **must deny**, proving path-segment-aware prefixing rather than naive string prefix); an undeclared host. Expected: every row matches its literal table under both grant values.

**Regression fence:** `crates/resourcefs-core/tests/http_policy_contract.rs::{base_url_scoping, private_network_grant}` (created in this slice).

**Named mutation:** C10 — in `http_policy.rs`, match on host equality instead of `base_url` prefix; `base_url_scoping` turns red on the same-host-outside-prefix row. C11 — omit the IPv4-mapped-IPv6 case from classification; `private_network_grant` turns red on the `::ffff:10.0.0.1` row. Restore each → green.

**Complexity/production scale:** Classification is O(1) per address over a fixed set of range checks; allowlist authorization is O(origins) with origins bounded by the profile's declared list (≤256 by the existing profile cardinality limits). Maximum accepted cost: under 1 ms for a full classification plus allowlist pass at 256 origins — rationale: pure integer/range comparisons with no allocation, called once per request.

**Wall budget/phase:** Always-on (per request, once per resolution and once per redirect hop): ≤1 ms at 256 declared origins. Rationale: this is pure comparison work on the request path; it must not become a measurable fraction of a network round trip.

**Files:** new `crates/resourcefs-core/src/http_policy.rs`; `crates/resourcefs-core/src/lib.rs` (`mod http_policy` + exports); new `crates/resourcefs-core/tests/http_policy_contract.rs`.

**Estimate:** 1.25 days. **Diff estimate:** 480 (230 impl, 250 tests). **PR increment:** A — Policy foundation.

**Commands and expected results:**
- **Mutation applicability check before the gate:** confirm both C10 and C11 mutations compile and turn their named fences red; correct the fixture (never the assertion) if either cannot.
- `cargo test -p resourcefs-core --all-features --test http_policy_contract` → every address-class row and every URL-scoping row agrees with its literal table under both grant values; the prefix-collision row denies. C10 and C11 mutations turn their named tests red; restored → green.
- `cargo tree -p resourcefs-core --depth 1` → no new dependency appears (the policy module adds none).
- `cargo test -p resourcefs-mcp --all-features --test architecture_contract` → core's forbidden-dependency assertions still pass.

---

## Slice 3: The bounded HTTP substrate, the single egress point, and the policed probe

**Claim IDs:** C2, C3, C7, C15, C18

**Expected behavior:** `resourcefs-sources/src/http/mod.rs` becomes **the only module in the workspace that constructs an HTTP client**. It builds one `reqwest::Client` whose `dns_resolver` is a `PolicyResolver` calling core's `AddressPolicy` — returning only policy-passing `SocketAddr`s and `Err` to deny, so a denied address **opens no socket at all** — and exposes `HttpSubstrate::fetch(HttpRequest, &OperationGuard) -> Result<BoundedHttpResponse, ResourceError>` whose request/response types are source-neutral (URL, method, headers, body bytes, content type, final URL) and name no HTTPS-source type. Connection pooling is retained, and a pooled connection is reused **only** to an address already authorized for that origin: pooling never yields a connection to an address policy did not authorize, and a policy that flips to deny cannot be bypassed by opening a *new* connection. `NetworkProbe` (`sources/probe.rs:58`) stops calling `TcpStream::connect` on an unvalidated system resolution and routes through the same `AddressPolicy`. The architecture fence is extended to assert exactly one module names the client. **This slice introduces `reqwest` 0.13.2 to the workspace.**

**Oracle:** **Server-side observation throughout, never the client's own report** — per-listener TCP accept counters bound on distinct loopback addresses (the evidence P1 oracle pattern), a **resolver invocation counter** compared against the connection count observed server-side for the pooling half, plus a **filesystem token scan** for the single-egress half (independent of the crate graph and module visibility), plus a compilation check for the source-neutral interface.

**Stress fixture:** A resolver returning a policy-denied address (assert zero accepts on that listener); a resolver returning a policy-passing address (assert exactly one accept); a **rebinding** resolver returning an authorized address on the first resolution and a private address on the second, with two listeners bound on distinct loopback IPs (assert each connection landed on the address its own resolution authorized, and the denied listener accepted zero); **two sequential reads of one origin over a live pooled connection** (count resolver invocations against server-observed connections; assert reuse contacted no unauthorized address) and a follow-up read after the policy flips to deny (assert a *new* connection cannot reach the now-denied address); a probe against an origin resolving private with `allow_private_network: false` (assert zero accepts); a neutral test consumer driving `HttpSubstrate::fetch` while referencing no `Https*` type (must compile). Expected: accept counters exactly as stated; the token scan returns exactly `["crates/resourcefs-sources/src/http/mod.rs"]`.

**Regression fence:** `crates/resourcefs-sources/tests/http_policy_contract.rs::{denied_address_opens_no_socket, rebinding_never_reaches_denied_address, pooled_reuse_stays_authorized, probe_egress_is_policed}`; `crates/resourcefs-sources/tests/http_substrate_contract.rs::substrate_is_source_neutral`; `crates/resourcefs-mcp/tests/architecture_contract.rs::single_http_client_module` — all created in this slice.

**Named mutation:** C2 — in `http/mod.rs`, have `PolicyResolver::resolve` return the address instead of `Err` when policy denies; `denied_address_opens_no_socket` turns red on a non-zero accept count. C3 — cache the first resolution and reuse it without re-authorizing; `rebinding_never_reaches_denied_address` turns red when the denied listener accepts. C7 — set `pool_max_idle_per_host(0)` and additionally skip policy on the reconnect path; `pooled_reuse_stays_authorized` turns red when a new connection reaches a now-denied address. C15 (two mutations) — restore the direct `TcpStream::connect` in `probe.rs`, turning `probe_egress_is_policed` red on a non-zero accept count; and construct a client in `https.rs` (or any second module), turning `single_http_client_module` red naming that file. C18 — take an `HttpsSource` parameter in `fetch`; `substrate_is_source_neutral` fails to compile. Restore each → green.

**Complexity/production scale:** One resolver invocation per new connection (O(addresses returned), bounded by DNS answer size); one policy pass per address. The token scan is O(workspace `.rs` bytes) and runs only in tests. Maximum accepted cost: resolver + policy under 1 ms per connection excluding DNS itself — rationale: the policy work is core's ≤1 ms pass from S2; DNS latency belongs to the network, not to this budget.

**Wall budget/phase:** Always-on (per new connection): policy evaluation ≤1 ms excluding DNS resolution and TCP handshake. Rationale: the substrate must not add measurable latency beyond the network cost it wraps. The token-scan fence is one-off test work: `N/A — reason: test-only phase; no production wall budget`.

**Files:** `Cargo.toml` (add `reqwest = { version = "0.13.2", default-features = false, features = [...] }` to `[workspace.dependencies]` with explicit feature selection per the repository's `default-features = false` convention); `crates/resourcefs-sources/Cargo.toml`; new `crates/resourcefs-sources/src/http/mod.rs`; `crates/resourcefs-sources/src/lib.rs` (`mod http`); `crates/resourcefs-sources/src/probe.rs` (route `NetworkProbe` through `AddressPolicy`); `crates/resourcefs-mcp/tests/architecture_contract.rs` (extend: `single_http_client_module`, add `reqwest` to core's and mcp's forbidden sets, pin `TcpStream::connect` to the policed module); new `crates/resourcefs-sources/tests/http_policy_contract.rs`; new `crates/resourcefs-sources/tests/http_substrate_contract.rs`; `crates/resourcefs-sources/tests/probe_runner_contract.rs` (existing probe expectations updated for the policy path).

**Estimate:** 2.75 days. **Diff estimate:** 960 (450 impl, 510 tests — 900 plus 60 for C7's pooling fence and its resolver/connection counters). **PR increment:** B — Bounded substrate.

**Commands and expected results:**
- **Mutation applicability check before the gate:** confirm all six mutations (C2, C3, C7, C15×2, C18) compile and turn their named fences red — C18's is a compile failure, which counts as red. Correct any fixture that cannot fail; never weaken an assertion.
- `cargo test -p resourcefs-sources --all-features --test http_policy_contract` → denied-address and rebinding rows show the exact accept counts stated in the fixture; the pooling rows show resolver invocations matching server-observed connections and no contact with a denied address; the probe row shows zero accepts. C2, C3, C7, and C15's probe mutation turn their named tests red; restored → green.
- `cargo test -p resourcefs-mcp --all-features --test architecture_contract` → `single_http_client_module` reports exactly one offender; core and mcp still reject `reqwest`. The second-client mutation turns it red naming the offending file.
- `cargo test -p resourcefs-sources --all-features --test http_substrate_contract` → the neutral consumer compiles and drives `fetch` without naming an HTTPS type; the C18 mutation breaks that compilation.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` → clean with the new dependency present.
- `cargo tree -p resourcefs-sources -i reqwest` and `cargo tree --duplicates` → record the introduced transitive tree and any duplicate versions in the commit message. **Note:** this repository has **no `deny.toml` and no CI workflow**, so no mechanized license/source vetting exists; the design's Placement names `deny.toml` as an owner but creating that machinery is new work covered by **no** design claim, so this slice records the tree rather than inventing an unfenced gate. The missing machinery is tracked at **rfs-q12y**; cite it in the commit message alongside the recorded tree.

---

## Slice 4: Authorize every redirect hop and bound chain depth

**Claim IDs:** C4, C5

**Expected behavior:** The substrate's client uses `redirect::Policy::custom`, calling core's `OriginAllowlist` for each hop and `attempt.stop()` to veto — so an off-allowlist redirect target is **never transmitted**. A chain deeper than the configured depth (5) returns `limit_exceeded`; a chain at exactly the depth succeeds. A redirect to a private address is denied by the resolver on the new connection.

**Oracle:** **Server-side request-path log** on the off-allowlist listener — direct evidence the URL was never sent (the evidence P2 oracle pattern) — plus a server-side hop counter on the redirecting listener, independent of the client's own redirect counter.

**Stress fixture:** No redirect; one on-allowlist hop (followed); a hop leaving the allowlist to a listener that logs every path it receives (assert `permission_denied` and an **empty** path log); a hop to a host resolving private under grant false; a chain at exactly depth 5 (succeeds); a chain at depth 6 (`limit_exceeded`); a redirect loop (terminates at the depth bound, not by hanging). Expected: the off-allowlist log contains nothing; depth boundary rows behave as stated; the loop terminates.

**Regression fence:** `crates/resourcefs-sources/tests/http_policy_contract.rs::{offsite_redirect_never_requested, redirect_depth_boundary}` (created in this slice).

**Named mutation:** C4 — in `http/mod.rs`, replace the custom redirect policy with `Policy::limited(5)`; `offsite_redirect_never_requested` turns red because `/secret` appears in the off-allowlist server's log. C5 — raise the depth passed to the policy by one; `redirect_depth_boundary` turns red on the depth+1 row. Restore each → green.

**Complexity/production scale:** One allowlist pass per hop, bounded at 5 hops; O(1) per hop over S2's ≤1 ms policy pass. Maximum accepted cost: ≤5 ms of policy work per request chain — rationale: five hops at the S2 per-pass budget.

**Wall budget/phase:** Always-on (per redirected request): ≤5 ms of authorization work across the whole chain, excluding network time. Rationale: bounded multiple of the S2 per-pass budget.

**Files:** `crates/resourcefs-sources/src/http/mod.rs` (custom redirect policy wiring); `crates/resourcefs-sources/tests/http_policy_contract.rs` (extended with the two new fences).

**Estimate:** 0.75 day. **Diff estimate:** 300 (110 impl, 190 tests). **PR increment:** B — Bounded substrate.

**Commands and expected results:**
- **Mutation applicability check before the gate:** confirm C4's and C5's mutations compile and turn their fences red; correct the fixture if not.
- `cargo test -p resourcefs-sources --all-features --test http_policy_contract offsite_redirect_never_requested` → `permission_denied` returned **and** the off-allowlist listener's received-path log is empty. The `Policy::limited` mutation turns it red; restored → green.
- `cargo test -p resourcefs-sources --all-features --test http_policy_contract redirect_depth_boundary` → depth 5 succeeds, depth 6 returns `limit_exceeded`, the loop terminates at the bound. The depth+1 mutation turns it red.

---

## Slice 5: Bound, cancel, and time out body reads

**Claim IDs:** C16

**Expected behavior:** Bodies stream through `bytes_stream()` and stop at the fetch ceiling **without buffering the whole body first**. A cancelled read returns `cancelled` and a timed-out read returns `source_unavailable`; both abandon the connection promptly and publish **no** Path Session state for the abandoned call. The ceiling bounds bytes ResourceFS accepts and retains — **not** bytes the peer transmits, which the fixture records explicitly so the weaker guarantee is the documented one.

**Oracle:** **Server-side flushed-byte counter and connection-close observation** (the evidence P3 oracle pattern), plus a Path Session artifact inventory taken before and after the abandoned call.

**Stress fixture:** A 64 MiB body against a 1 MiB ceiling — assert the client stops and record what the server actually flushed (evidence measured ~3 MiB; the fixture asserts the **retained** bound, and records transferred bytes as an observation, not an assertion of the stronger claim); cancellation before connect; cancellation mid-body (assert the server stops well short of the body); a deliberately slow trickling server exceeding the 30 s timeout; a body declaring a larger `Content-Length` than it sends; a chunked body with no declared length. Expected: retained bytes never exceed the ceiling; `cancelled` / `source_unavailable` as stated; the artifact inventory is byte-identical before and after every abandoned call.

**Regression fence:** `crates/resourcefs-sources/tests/http_bounds_contract.rs::{cancelled_read_abandons_connection, timeout_returns_source_unavailable}` (created in this slice).

**Named mutation:** From C16 — in `http/mod.rs`, await the full body before checking the guard; `cancelled_read_abandons_connection` turns red because the server writes the whole body. Restore → green.

**Complexity/production scale:** Streaming is O(retained bytes ≤ fetch ceiling); memory holds at most one ceiling-sized buffer (8 MiB) plus chunk overhead. Maximum accepted cost: peak retained allocation ≤ the fetch ceiling + 1 MiB headroom, and a cancelled read releases within one chunk boundary — rationale: the ceiling is the contract; headroom covers a single in-flight chunk.

**Wall budget/phase:** Always-on (per read): request wall time bounded by the 30 s timeout ceiling; cancellation observed within one chunk boundary. Rationale: the timeout is the requester-fixed contract value and cancellation must not wait on a full body.

**Files:** `crates/resourcefs-sources/src/http/mod.rs` (bounded streaming, guard checks, timeout); new `crates/resourcefs-sources/tests/http_bounds_contract.rs`.

**Estimate:** 0.75 day. **Diff estimate:** 300 (120 impl, 180 tests). **PR increment:** B — Bounded substrate.

**Commands and expected results:**
- **Mutation applicability check before the gate:** confirm C16's mutation compiles and turns its fence red; correct the fixture if not.
- `cargo test -p resourcefs-sources --all-features --test http_bounds_contract` → retained bytes never exceed the ceiling; the server's flushed-byte counter is recorded (transfer is **not** asserted to be bounded); cancellation and timeout return their stated categories; the artifact inventory is unchanged across abandoned calls. The full-body-await mutation turns `cancelled_read_abandons_connection` red; restored → green.

---

## Slice 6: TLS parity and hostname-bound certificate verification

**Runs SECOND in increment B, immediately after Slice 3** (see Plan revision 1). In addition to claim C6, this slice **owns the shared TLS fixture helper** that Slices 4, 5, and 7 consume: a loopback TLS listener over `tokio-rustls`, a static self-signed PEM for the fixture hostname embedded in test support, and a `test-support`-gated substrate constructor that trusts that root via `ClientBuilder::add_root_certificate` while leaving the production path untouched. Place the helper where the later slices can reuse it rather than inside a single test file. `danger_accept_invalid_certs` must not be used anywhere — it would disable the hostname-bound verification this slice exists to prove.

**Claim IDs:** C6

**Expected behavior:** Over TLS, address authorization behaves **identically** to the plain-HTTP path — the evidence residual, executed rather than inferred, since the probe exercised only plain HTTP. Certificate verification remains bound to the **requested hostname** even though the connection is pinned to a policy-validated address: a certificate whose name does not match the host is rejected **even when the address was authorized**.

**Oracle:** **Server-side accept counters plus the TLS handshake outcome recorded by the listener** — independent of the client's error text, which could report a mismatch for the wrong reason.

**Stress fixture:** A loopback TLS listener with a locally generated certificate: a TLS request to an authorized address (succeeds, one accept); a TLS request to a policy-denied address (zero accepts — the C2/C3 scenarios re-run over TLS); a certificate whose subject name does not match the requested host, served from an **authorized** address (must be rejected, and the listener records a failed handshake rather than a successful session); a rebinding resolver over TLS. Expected: accept-counter outcomes identical to the plain-HTTP rows, and the name-mismatch row rejected at the TLS layer.

**Regression fence:** `crates/resourcefs-sources/tests/http_tls_contract.rs::{tls_address_policy_matches_plain, tls_certificate_binds_hostname}` (created in this slice).

**Named mutation:** From C6 — in `http/mod.rs`, pin the TLS server-name to the resolved address instead of the URL host; `tls_certificate_binds_hostname` turns red because the name-mismatched certificate is accepted. Restore → green.

**Complexity/production scale:** N/A — reason: TLS adds no new loop of ours; the handshake cost belongs to the TLS implementation and is bounded by the existing request timeout.

**Wall budget/phase:** Always-on (per new TLS connection): handshake completes within the 30 s request timeout ceiling. Rationale: TLS setup is part of the request whose total is already bounded; no separate budget is meaningful for a local fixture.

**Files:** `crates/resourcefs-sources/src/http/mod.rs` (TLS configuration; server-name binding); `Cargo.toml` / `crates/resourcefs-sources/Cargo.toml` (TLS feature selection on the client if not already enabled in S3); new `crates/resourcefs-sources/tests/http_tls_contract.rs`; test-only certificate generation support in `crates/resourcefs-sources/tests/support/`.

**Estimate:** 1 day. **Diff estimate:** 380 (120 impl, 260 tests incl. certificate fixtures). **PR increment:** B — Bounded substrate.

**Commands and expected results:**
- **Mutation applicability check before the gate:** confirm C6's mutation compiles and turns `tls_certificate_binds_hostname` red. This is the highest-risk applicability check in the change — the mutation must not be pre-empted by the TLS library rejecting the configuration outright; if it is, correct the fixture so the mutation is observable.
- `cargo test -p resourcefs-sources --all-features --test http_tls_contract` → TLS accept-counter outcomes match the plain-HTTP rows item by item; the name-mismatched certificate is rejected from an authorized address, with the listener recording a failed handshake. The server-name mutation turns the hostname fence red; restored → green.

---

## Slice 7: Deterministic reader-mode extraction and the fetch ceiling

**Claim IDs:** C8, C9

**Expected behavior:** `resourcefs-sources/src/http/extract.rs` performs an in-house bounded HTML→Markdown pass over the `html5ever` tokenizer — dropping `script`/`style`/`svg`, keeping the defined tag subset (headings, paragraphs, lists, links, code, tables), normalizing whitespace. Output is **byte-deterministic** for identical input across runs and across separate processes, and **no canonical output derives from a `Debug` rendering** (the evidence P4 defect). A body exceeding the fetch ceiling returns `limit_exceeded` with the extractor **never invoked**; a body at exactly the ceiling succeeds.

**Oracle:** **Cross-process digest comparison plus a hand-authored expected Markdown** written from the markup rather than from running the extractor (the evidence P4 pattern, including the tendril storage-boundary case that caught the probe defect), plus an **extractor invocation counter** for the "no extraction over-ceiling" half and the server-side flushed-byte counter for the transfer observation.

**Stress fixture:** An 8-document corpus — well-formed HTML; malformed/unclosed tags; script/style/svg-bearing; astral-plane Unicode; attribute-heavy; deeply nested; entity-bearing; and **a doctype name crossing tendril's inline/heap storage boundary** (the case that exposed the probe's `{:?}` defect). Plus bodies at exactly the ceiling, ceiling+1, an over-declared `Content-Length`, and a chunked body. Expected: digests identical in-process and across a separate process for all 8 documents; the small document matches its hand-authored Markdown exactly; the ceiling+1 row returns `limit_exceeded` with the extractor invocation counter at **zero**.

**Regression fence:** `crates/resourcefs-sources/tests/http_extract_contract.rs::{extraction_is_deterministic, extraction_matches_golden}` and `crates/resourcefs-sources/tests/http_bounds_contract.rs::{fetch_ceiling_boundary, over_ceiling_never_extracts}` (created in this slice; the bounds file was created in S5 and is extended here).

**Named mutation:** C9 — in `extract.rs`, render one element's name via `{:?}` on its tendril instead of as a plain string; `extraction_is_deterministic` turns red on the `long_doctype` row across processes. C8 — in `http/mod.rs`, run extraction before the ceiling check; `over_ceiling_never_extracts` turns red because the invocation counter is non-zero. Restore each → green.

**Complexity/production scale:** Extraction is O(document bytes ≤ fetch ceiling) with a single pass over the token stream; output is bounded by input. Maximum accepted cost: an 8 MiB document extracts in ≤5 seconds and peak allocation stays ≤2× the document size — rationale: matches the repository's existing maximum-resource read budget and permits one input plus one output buffer.

**Wall budget/phase:** Always-on (per reader-mode read): ≤5 seconds at the 8 MiB fetch ceiling. Rationale: aligns with the existing bounded-read wall budget for maximum-size local resources.

**Files:** new `crates/resourcefs-sources/src/http/extract.rs`; `crates/resourcefs-sources/src/http/mod.rs` (ceiling check strictly before extraction); `Cargo.toml` / `crates/resourcefs-sources/Cargo.toml` (`html5ever`, version-pinned per the repository's `=` convention for parser crates, matching `regex`/`globset`/`ignore`); new `crates/resourcefs-sources/tests/http_extract_contract.rs`; `crates/resourcefs-sources/tests/http_bounds_contract.rs` (extended); extraction golden fixtures under `crates/resourcefs-sources/tests/fixtures/`.

**Estimate:** 1.5 days. **Diff estimate:** 520 (260 impl, 260 tests and fixtures). **PR increment:** B — Bounded substrate.

**Commands and expected results:**
- **Mutation applicability check before the gate:** confirm C8's and C9's mutations compile and turn their fences red. C9's mutation must be placed where the `Debug` rendering actually reaches canonical output — verify by observing the `long_doctype` row change, not merely that the code compiles.
- `cargo test -p resourcefs-sources --all-features --test http_extract_contract` → all 8 corpus digests identical in-process and across a separate process; the small document matches its hand-authored Markdown byte for byte. The `{:?}` mutation turns `extraction_is_deterministic` red on the storage-boundary row; restored → green.
- `cargo test -p resourcefs-sources --all-features --test http_bounds_contract` → ceiling row succeeds, ceiling+1 returns `limit_exceeded` with the extractor invocation counter at zero. The extract-before-check mutation turns `over_ceiling_never_extracts` red.

---

## Slice 8: The HTTPS Source Adapter, catalog mount, common contracts, and leak-freedom

**Carried finding from Slice 1 (`c4adc88`) — C19's fences must assert the real precondition.** The encoded-separator narrowing is not actually about "filesystem families"; the binding constraint is `percent_decode`'s documented precondition. That function indexes `bytes[index + 1]` under an `expect("percent escapes were validated")`, so it **panics on a trailing `%`** if reached without prior validation. Slice 1 placed validation at exactly the paths that reach it (verified: all four call sites — `reference.rs:584` for `local://`, and `:886`/`:978`/`:1003` under `parse_workspace_address` — are filesystem-family). C19's fences must therefore assert *the precondition*, not the family list: any reference form that reaches `percent_decode` validates first. A future family that calls it without validating is a panic, not a rejection, and a family-list assertion would not catch that.

**Claim IDs:** C12, C13, C17, C19

**Expected behavior:** `resourcefs-sources/src/https.rs` provides `HttpsSource` implementing `SourceAdapter` (reader-mode read and `:raw`), `DiscoveryAdapter` (search over the extracted Markdown), and `SourceCatalogMetadata`, holding `Arc<HttpSubstrate>` plus per-origin configuration derived from the already-built `HttpsSourceProfile`. `CompiledSources` registers it, adds `Https` arms for read/search/catalog, and routes every `Https` mutation to `unsupported_mutation`. Reads report `mutable: false` and use the common bounded-read/discovery contracts with **no HTTPS-specific result fields**. No credential material and no resolved IP address appears on any observable channel. `https://` self-lists in the `rfs://` catalog with a grammar line and an example that re-parses. C19's narrowing is formally fenced from **both** directions now that a wire exists.

**Oracle:** A literal expected structured-key set and a literal reference→category table (independent of the adapter); **raw byte substring scans for test-owned unique sentinels** across every observable channel (the rfs-r9m6 secret-sink pattern); the literal expected catalog line plus `PathReference::parse` over the rendered example; and the **server-side received request-line log** proving the percent-encoded form was transmitted unchanged — neither re-encoded nor decoded.

**Stress fixture:** Reader-mode read; `:raw`; `:raw` plus a line selector; a bare line selector; a search; `rfs_write` and `rfs_edit` attempts (both `unsupported_mutation`); an origin with a credential and an origin whose credential reference cannot resolve; denial by address, by allowlist, and by credential failure, each with **unique sentinels** for the credential value and the resolved IP scanned across tool results, error text, logs, and probe reports; `rfs://` catalog read with HTTPS mounted and unmounted; workspace and `local://` references carrying `%2F`/`%5C` (still `invalid_reference`); `https://ex.com/search?q=a%2Fb` and `https://ex.com/a%2Fb/doc` (parse **and** reach the wire with the encoding preserved). Expected: only common structured keys; `mutable:false`; zero sentinel occurrences anywhere; the catalog example re-parses; the encoded request line matches byte for byte.

**Regression fence:** `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::{https_uses_common_read_shape, https_is_read_only}`; `crates/resourcefs-sources/tests/http_policy_contract.rs::{no_credential_or_address_leak, encoded_url_reaches_wire_unchanged}`; `crates/resourcefs-core/tests/path_reference_contract.rs::encoded_separator_guard_scope`; `crates/resourcefs-sources/src/catalog.rs` tests and the `compiled.rs::tests` registry-length assertion (extended) — all created or extended in this slice.

**Named mutation:** C12 — in `http/mod.rs`, include the resolved `SocketAddr` in the denial message; `no_credential_or_address_leak` turns red on the IP sentinel. C13 — in `compiled.rs`, route `Https` mutation to the filesystem mutation adapter instead of `unsupported_mutation`; `https_is_read_only` turns red. C17 — omit the entry from `catalog_entries`; the catalog row and the registry-length assertion turn red. C19 (two mutations) — restore the global `validate_percent_encoding` call for every family, turning `encoded_separator_guard_scope` red on the HTTPS query row; and skip the guard for workspace references, turning the same test red on the workspace row. Restore each → green.

**Complexity/production scale:** Search scans the extracted Markdown, bounded by the fetch ceiling and the existing discovery result caps; reads reuse the common bounded-read path. Maximum accepted cost: search over an 8 MiB extraction within the existing 5-second discovery wall budget — rationale: same engine and ceilings as workspace discovery, one bounded document rather than a tree.

**Wall budget/phase:** Always-on (per request): reads and searches reuse the existing bounded-read and 5-second discovery budgets; each adds one network round trip bounded by the 30 s timeout. Rationale: no new engine, one new adapter behind proven budgets.

**Files:** new `crates/resourcefs-sources/src/https.rs`; `crates/resourcefs-sources/src/lib.rs`; `crates/resourcefs-sources/src/compiled.rs` (register, `Https` arms, mutation → `unsupported_mutation`, registry-length test); `crates/resourcefs-sources/src/catalog.rs` (entry + tests); `crates/resourcefs-core/src/reference.rs` (replace S1's placeholder `source_unavailable` arms with real routing where applicable); `crates/resourcefs-core/tests/path_reference_contract.rs`; `crates/resourcefs-sources/tests/http_policy_contract.rs` (extended); `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`.

**Estimate:** 2 days. **Diff estimate:** 780 (350 impl, 430 tests). **PR increment:** C — HTTPS source and lifecycle.

**Commands and expected results:**
- **Mutation applicability check before the gate:** confirm all five mutations (C12, C13, C17, C19×2) compile and turn their named fences red; correct any fixture that cannot fail, never the assertion.
- `cargo test -p resourcefs-mcp --all-features --test stdio_mcp_contract` → HTTPS reads expose only the common structured keys with `mutable:false` and non-empty equivalent text; write and edit both return `unsupported_mutation`. The C13 mutation turns `https_is_read_only` red.
- `cargo test -p resourcefs-sources --all-features --test http_policy_contract no_credential_or_address_leak` → zero occurrences of either sentinel across tool results, error text, logs, and probe reports, on success and every failure path. The address-in-message mutation turns it red.
- `cargo test -p resourcefs-core --all-features --test path_reference_contract encoded_separator_guard_scope && cargo test -p resourcefs-sources --all-features --test http_policy_contract encoded_url_reaches_wire_unchanged` → filesystem families still reject `%2F`/`%5C`; HTTPS shapes parse and the server's received request line preserves the encoding byte for byte. Both C19 mutations turn the scope fence red on their respective rows.
- `cargo test -p resourcefs-sources --all-features` → the catalog lists `https://` with an example that re-parses; the registry-length assertion reflects the new source.

---

## Slice 10: Credential transmission, launch wiring, and read cancellation

**Runs BEFORE Slice 9** (see Plan revision 2). Claims **C20** and **C21**.

**Expected behavior:** a configured HTTPS origin is constructed as a live `HttpsSource` at launch from the profile, so an allowlisted reference to a reachable origin actually serves. A configured credential is transmitted on that origin's requests, and its value appears in no tool result, error message, log line, probe report, or CLI diagnostic. An `rfs_read` of an `https://` reference honors cancellation rather than running to the 30-second timeout.

**Prerequisites this slice must build (all established as absent by Slice 8):** request-header support on `HttpRequest` (currently `{ url }`); an accessor on `CredentialHeader` (currently constructible but unreadable); consumption of `profile/model.rs`'s `configured_sources` (currently assigned and never read); and an `OperationGuard` reaching `HttpsSource::read` (`SourceAdapter::read` takes none, so the adapter builds an always-active guard).

**Fences, written to the amended criteria:**
- **C20 transmission** — the fixture's `requests()` log shows the credential header on the wire. Positive control: the same origin without a credential shows no such header, so the assertion is not satisfied by a request that never happened.
- **C20 leak-freedom** — a unique credential sentinel is absent from every observable channel, with each channel asserted **non-empty first** so the scan cannot be vacuous. This is the fence that was impossible before transmission existed.
- **C21 cancellation** — a cancelled `rfs_read` of a trickling fixture returns `cancelled` and the server stops short. Positive control: the same uncancelled read completes and retains the whole body, so a failure cannot be mistaken for the fixture never working. Assert the refusal **message** names cancellation, since a timeout occupies the same call position.

**Named mutations (verify applicability immediately after implementing):** C20 — drop the header from the request builder; the wire-log fence goes red because the header is absent while the control row still shows the no-credential case unchanged. C21 — remove the guard from the read path; the cancelled read runs to timeout while the uncancelled control still completes.

**Files:** `crates/resourcefs-sources/src/http/mod.rs`, `crates/resourcefs-sources/src/https.rs`, `crates/resourcefs-sources/src/configuration/https.rs`, `crates/resourcefs-core/src/source.rs` (the `OperationGuard` on `SourceAdapter::read` — a core trait change; enumerate every implementor and caller), `crates/resourcefs-mcp/src/profile/model.rs`, `crates/resourcefs-mcp/src/launch.rs`, `crates/resourcefs-mcp/src/server.rs`, plus their contract tests.

**PR increment:** C — HTTPS source and lifecycle.

## Slice 9: Required-fails-startup and optional-degrades lifecycle

**Runs AFTER Slice 10** (see Plan revision 2), which supplies the mounting that makes C14's second half fencible and provides its positive control (optional + reachable actually serving).

**Claim IDs:** C14

**Expected behavior:** A `required: true` HTTPS source whose connectivity probe fails prevents startup with a non-zero exit and a stderr diagnostic naming the source id; a `required: false` source whose probe fails becomes visibly **Degraded**, the server starts, every other source keeps serving, and only that source's references fail with `source_unavailable`. The explicit `resourcefs check` probe reports reachability without mutating anything and without running during ordinary reads.

**Oracle:** **Compiled-binary exit code and channel bytes** compared against the literal rfs-r9m6 exit/channel table (0/2/3/1), independent of in-process state.

**Stress fixture:** Four profiles — required+reachable, required+unreachable, optional+reachable, optional+unreachable — each launched as the **compiled binary** against a loopback fixture that refuses connections; plus a mixed profile where an unreachable optional HTTPS source coexists with a healthy workspace root, asserting the workspace root still serves while the HTTPS references fail. Expected: exit codes and stderr contents match the literal table; the Degraded state is visible in the check report; no probe runs during an ordinary read.

**Regression fence:** `crates/resourcefs-mcp/tests/cli_contract.rs::{required_https_fails_startup, optional_https_degrades}` (created in this slice).

**Named mutation:** From C14 — in the check path, treat a failed required probe as Degraded; `required_https_fails_startup` turns red because the process exits 0. Restore → green.

**Complexity/production scale:** Probing is O(configured network sources), run once per process at startup and once per explicit check. Maximum accepted cost: startup probing completes within the existing 5-second acquisition deadline at the profile's origin cardinality — rationale: matches the rfs-r9m6 startup budget.

**Wall budget/phase:** One-off per process (startup) and per explicit `check` invocation: `N/A — reason: one-off phase; the existing rfs-r9m6 five-second startup deadline is the gate`. No always-on phase is introduced — probes must not run on the read path, which the fixture asserts.

**Files:** `crates/resourcefs-mcp/src/profile/check.rs` (required/degraded handling for the HTTPS probe); `crates/resourcefs-sources/src/probe.rs` (report shape if needed); `crates/resourcefs-mcp/tests/cli_contract.rs`; `crates/resourcefs-mcp/tests/probe_contract.rs` (extended).

**Estimate:** 1 day. **Diff estimate:** 320 (110 impl, 210 tests). **PR increment:** C — HTTPS source and lifecycle.

**Commands and expected results:**
- **Mutation applicability check before the gate:** confirm C14's mutation compiles and turns `required_https_fails_startup` red; correct the fixture if not.
- `cargo test -p resourcefs-mcp --all-features --test cli_contract` → each of the four profile rows produces the exit code and stderr contents in the literal table; the mixed profile serves the workspace root while HTTPS references return `source_unavailable`. The Degraded-required mutation turns `required_https_fails_startup` red; restored → green.
- `cargo test -p resourcefs-mcp --all-features --test probe_contract` → the explicit probe reports reachability and mutates nothing; no probe fires during an ordinary read.

---

## Tracker taxonomy

- **Permanent non-goals** (rationale recorded in `design.md`; no tracker issue): HTTPS mutation, plain `http://` origins, JavaScript execution or headless rendering, boilerplate/navigation stripping heuristics, response caching, cookie jars, session affinity, and proxy configuration.
- **Intended future work** (verified tracker IDs, all confirmed to exist and cover the deferred work): **rfs-45ww** and **rfs-by2z** (GitHub over native HTTP APIs consumes this substrate — C18 exists so neither needs a second client or policy layer); **rfs-azd3** (downstream MCP over allowlisted Streamable HTTP likewise consumes it); **rfs-ww6w** (the MCP Resources mirror will expose `https://` references once mounted).
- **One gap surfaced by this plan, now classified as intended future work with a verified tracker ID:** `design.md`'s Placement names `deny.toml` as an owner for vetting the new transitive dependency tree, but **this repository has no `deny.toml` and no CI workflow** (verified 2026-08-23), and **no design claim (C1–C19) fences dependency vetting**. Creating that machinery is new work with no covering claim, so S3 records the introduced tree (`cargo tree -i reqwest`, `cargo tree --duplicates`) in its commit message rather than inventing a gate this plan cannot verify. The missing gate is tracked at **rfs-q12y** — "Vet the dependency tree with cargo-deny and a CI gate" — filed 2026-08-23 and citing this change's reqwest introduction as its motivating case. Per the contract's tracker taxonomy this deferral is intended future work, not a permanent non-goal.
- No other deferral phrase was introduced by this plan.

## Self-review

- [x] **Every design row is assigned to exactly one slice**: S1 C1; S2 C10/C11; S3 C2/C3/C7/C15/C18; S4 C4/C5; S5 C16; S6 C6; S7 C8/C9; S8 C12/C13/C17/C19; S9 C14. That is 1+2+5+2+1+1+2+4+1 = **19 claims, each exactly once**. Every slice's Claim IDs exist in the design table, and every `PENDING` falsifier is discharged by the slice implementing its claim.
- [x] **Every slice records all thirteen mandatory fields**, with `N/A — reason` in conditional cells (S1 complexity and wall budget; S6 complexity; S9 wall budget).
- [x] **Every claim's fence is created in the slice implementing it**; no fence creation is deferred. One decomposition note: C19's *implementation* (narrowing the `validate_percent_encoding` call site) lands in S1 as C1-enabling work, because C1's fence requires percent-encoded HTTPS URLs to parse. C19's own claim and **both** its fences land in S8, where a live wire makes the transmission half verifiable. The narrowing is never unfenced in between — `https_grammar` (S1, new) covers the HTTPS side and the pre-existing corpus (`workspace_references.json:33-34`, `path_reference_contract.rs:345,349`) covers the filesystem side, so over-narrowing turns existing tests red at S1.
- [x] **Every new fence carries its design-approved named mutation**; no fence-less claim exists (the design approved zero risk acceptances, so no `N/A — approved risk` value appears anywhere).
- [x] **Every new loop states asymptotic cost, production-scale input sizes, the resulting bound, and an explicit maximum accepted cost with rationale**; every always-on phase carries a wall budget with rationale, and the two one-off/absent phases carry `N/A — reason`.
- [x] **The partition rule was applied**: 4,360 + 25% churn (1,090) = **5,450 > 4,000**, so the slices are partitioned into three independently mergeable PR increments in dependency order — A (S1–S2, 800 lines, no network), B (S3–S7, 2,460 lines, loopback fixtures only), C (S8–S9, 1,100 lines, end-to-end). Every slice names its increment; each increment has a mergeable definition and verifies without the increments after it.
- [x] **The tracker taxonomy is applied**: permanent non-goals carry rationale, intended future work cites verified IDs, and the one unfenceable gap (`deny.toml`) is surfaced and classified as intended future work citing verified tracker ID **rfs-q12y**.
- [x] **Mutation applicability is treated as a known hazard**: `design.md` verified only 5 of 19 mutations against existing code, and in the previous change five of fourteen could not turn their fence red. Every slice's Commands field opens with an explicit confirmation step performed immediately after implementing and before the gate, correcting the fixture — never the assertion — when a mutation cannot fail.
- [x] **The plan declares no slice complete**; `checkpointed-build` exclusively judges completion.
