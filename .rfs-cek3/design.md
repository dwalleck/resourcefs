# Falsifiable design: rfs-cek3

## Route and inputs

- **Route:** Structural, from `.rfs-cek3/route.md`.
- **Behavior source:** `.rfs-cek3/route.md` T4; `spec.md` is N/A because the ticket supplies the complete behavior.
- **Behavior set:** G1/W1/T1 requires a profile-launched MCP server to complete real TLS and return canonical HTTPS identity, reader-mode Markdown, a content-derived Version Tag, `mutable: false`, and bounded recovery metadata. G2/W2/T2 prohibits any production input that enables fixture trust and prohibits invalid-certificate or invalid-hostname bypasses. G3/W3/T3 strengthens the C14 healthy control from “TCP probe mounted” to “a real HTTPS document was served.” G4/W4/T4 requires an applicable named mutation at the production hop to turn the focused fence red and restoration to return it green.
- **Empirical inputs:** N/A — Structural route; current `HttpSubstrate` TLS contracts and fixture cover the premises, so no `evidence.md` or probe applies.

## Input shapes

| Input | Shapes | Status |
|---|---|---|
| Launch trust mode | system roots selected by the production `resourcefs` executable; one additional valid fixture root selected only by an integration-test harness | Covered by C1 and C3 |
| Fixture root bytes | valid DER CA; malformed DER | Covered by C1 and C2 |
| Production enablement attempts | profile field, CLI argument, environment variable, direct production-main call | Covered by C3; all remain absent |
| HTTPS source lifecycle | required + reachable; required + unreachable; optional + reachable; optional + unreachable | Required/reachable is covered by C4/C5; optional reachable/unreachable and required/unreachable are covered by C6 and the existing C14 fences |
| HTTPS source collection | no configured origin; one origin; several origins with distinct allowlist/credential/degraded state | Covered by C3: production construction remains `HttpSubstrate::new`; fixture trust is an explicit alternate launch input applied after the existing typed aggregation, without a second aggregation path |
| Reader-mode content | empty; small ASCII/Unicode Markdown; content at the inline boundary; content over the inline boundary but under the fetch ceiling | Covered by C4, C5, and C7 |
| Reader-mode projection | whole Resource; selected lines | Covered by C4 and C7 |
| Raw projection | whole Resource; selected lines | Covered by C7: the new Markdown constructor is selected only for reader mode, so raw remains on its existing projection path |
| Common read metadata | canonical reference; content type; Version Tag; mutability; bounded flag; displayed ranges/EOF; absent or present recovery/continuation references | Covered by C4 and C5 |
| TLS server identity | trusted matching hostname; trusted mismatching hostname | Covered by C1 and the existing hostname-binding fence |
| Named mutation lifecycle | mutation compiles and is permitted; red result; restored green result | Covered by C4 and the checkpointed-build oracle gate |

The core move is additive: it adds an explicit test-harness launch adapter and a typed Markdown projection constructor. It removes no production guard, ordering guarantee, validation, serialization point, or uniqueness invariant.

## Placement

### Test-only profile launch

- **Owner:** `resourcefs-mcp` launch/profile modules own turning a checked Server Profile into mounted Source Adapters; the integration-test-only entry point belongs in a `test_support` module exported only under the existing `test-support` feature.
- **New seam — chosen:** `resourcefs_mcp::test_support::serve_profile_with_https_root(profile, root)` accepts an already typed fixture root and runs the normal profile check/probe/mount/server path over stdio. The integration test re-executes its own test harness as the child MCP process; the shipped `resourcefs` main never calls this interface and reads no trust-related CLI argument, profile field, or environment variable. This keeps the module deep: one test interface exercises profile launch, mount, dispatch, bounding, rendering, and stdio.
- **Alternative A — rejected:** read `RESOURCEFS_TEST_HTTPS_ROOT` inside `run_cli`, `LaunchPlan::from_profile`, or `mount_https`. Even behind the Cargo feature, a non-test build can enable the feature; that makes fixture trust production-reachable and fails the ticket’s security criterion.
- **Alternative B — rejected:** add a hidden Server Profile field or CLI argument. Hidden syntax remains production syntax and expands the production trust interface.
- **Forbidden:** production CLI/profile/environment trust inputs; a second profile decoder; bypassing startup probes; a test that calls `mount_https` or `HttpsSource` directly instead of `rfs_read` through the profile-launched MCP server.

### Additional fixture root

- **Owner:** `resourcefs-sources::http::HttpSubstrate` remains the only module that constructs HTTP clients and therefore owns adding and validating a DER root for a test adapter.
- **New seam — chosen:** a `test-support`-gated, system-resolver constructor adds one validated fixture root while preserving the existing allowlist, ceilings, credentials, redirect policy, address policy, and both read/mutation clients. System resolution is deliberate: the end-to-end fixture uses a loopback hostname certificate, so the profile probe and mounted client follow the same ordinary host path.
- **Alternative A — rejected:** expose `reqwest::ClientBuilder` or a generic TLS configurator. It creates a shallow security interface and permits callers to disable verification or omit policy.
- **Alternative B — rejected:** reuse `with_host_lookup_and_roots` by injecting DNS from profile launch. The startup probe would still use system resolution, creating two host-resolution truths and weakening the end-to-end claim.
- **Forbidden:** `danger_accept_invalid_certs`, `danger_accept_invalid_hostnames`, custom certificate verifiers, a second HTTP client module, or trust-root parsing outside `resourcefs-sources::http`.

### Reader-mode media type

- **Owner:** `resourcefs-core::SourceResource` owns common Resource metadata; `resourcefs-sources::HttpsSource` owns choosing reader mode versus raw. MCP rendering only serializes the typed result.
- **New seam — chosen:** add a specific `SourceResource::markdown_projection` constructor backed by the constant `text/markdown; charset=utf-8`; `HttpsSource` chooses it only for extracted reader-mode content, including selected projections. The existing plain-text constructor remains the raw path.
- **Alternative A — rejected:** add a generic public `content_type: &'static str` constructor. It admits arbitrary and contradictory media types instead of making the supported state explicit.
- **Alternative B — rejected:** overwrite `contentType` in `resourcefs-mcp::render` when the reference begins with `https://`. That re-parses a typed Path Reference at the wrong layer and can disagree with the Source Resource.
- **Forbidden:** string-prefix inference in MCP, an HTTPS-specific field in `ReadResource`, or computing the Version Tag over rendered MCP text instead of authoritative Markdown.

### End-to-end TLS fixture

- **Owner:** `crates/resourcefs-mcp/tests/support` owns a minimal loopback TLS fixture and static non-secret CA/leaf/key bytes used only by MCP integration tests.
- **New seam:** no production seam. The fixture records accepted TLS sessions and request targets, providing an independent server-side oracle.
- **Forbidden:** external network dependence, system trust-store mutation, certificate-generation dependencies in the production graph, or assertions based only on client error text.

## Claims

1. **C1:** Adding the fixture CA preserves certificate-chain and requested-hostname verification.
2. **C2:** Malformed fixture-root DER is rejected before any HTTPS request can leave the process.
3. **C3:** The shipped `resourcefs` interface cannot select fixture trust; only the re-executed integration-test harness can call the explicit test-support launch interface.
4. **C4:** A profile-launched `rfs_read` of a small allowlisted HTTPS document returns the exact common reader-mode shape with canonical identity, `text/markdown; charset=utf-8`, a literal content-derived Version Tag, and `mutable: false`.
5. **C5:** A profile-launched `rfs_read` of over-inline-limit reader-mode Markdown spills losslessly and exposes correct bounded, recovery, continuation, range, and EOF metadata.
6. **C6:** The C14 healthy-origin control completes TLS, receives `/doc`, and returns the fixture document, while the otherwise-identical unreachable optional origin still degrades.
7. **C7:** Markdown media type is owned by `SourceResource` and survives empty, Unicode, and selected reader-mode projections without changing raw projections.

## Falsification

| # | Claim | Input shape | Falsifier | Oracle | Named mutation | Regression fence | Cost | Status |
|---|---|---|---|---|---|---|---|---|
| C1 | Fixture trust preserves chain and hostname verification | valid fixture CA; matching and mismatching hostname leaves | Trust the fixture CA, request both leaves at an authorized address, and fail if the matching handshake does not complete or the mismatching handshake completes/sends a request | TLS listener’s server-side handshake outcome and request log; neither is derived from the client result | `crates/resourcefs-sources/src/http/mod.rs`: set `danger_accept_invalid_certs(true)` on the client builder; reqwest accepts the equally trusted name-mismatched leaf, the listener receives `/doc`, and `tls_certificate_binds_hostname` turns red | `crates/resourcefs-sources/tests/http_tls_contract.rs::tls_certificate_binds_hostname` | <1 min | PASS |
| C2 | Malformed fixture DER is rejected before egress | malformed DER root | Construct the typed test root from invalid DER beside an accept-counting listener; any successful construction or accepted socket falsifies | Literal invalid DER bytes plus listener accept count zero | `crates/resourcefs-sources/src/http/mod.rs`: remove the temporary client-builder validation from `TestRootCertificate::from_der` and store the bytes directly; malformed construction succeeds and the malformed-root fence turns red | `crates/resourcefs-sources/tests/http_tls_contract.rs::malformed_additional_root_is_rejected_before_egress` | <1 min | PENDING — checkpointed-build, root-constructor slice |
| C3 | Fixture trust is not production-reachable | production CLI/profile/env attempts; explicit test-harness call | Mechanically inspect the package targets and source references: the production main/CLI/profile interfaces must expose no trust-root input or test environment token, while the test-only re-exec helper must call the gated interface | Cargo target/feature metadata plus a workspace source-token allowlist whose expected sites are literal test-support modules/tests | `crates/resourcefs-mcp/src/profile/mod.rs`: read `RESOURCEFS_TEST_HTTPS_ROOT` from the production mount path; the source-token allowlist names an unexpected production site and turns red | `crates/resourcefs-mcp/tests/architecture_contract.rs::profile_https_fixture_trust_is_test_harness_only` | <1 min | PENDING — checkpointed-build, launch-seam slice |
| C4 | Small profile HTTPS reads use the common Markdown shape | required reachable source; small Unicode reader-mode document; whole Resource | Launch the re-executed test harness from a real profile, call `rfs_read`, compare the literal structured key set and values, exact Markdown, precomputed SHA-256 Version Tag, rendered text, and server request target; any mismatch falsifies | Hand-authored expected key/value table, precomputed digest literal, and TLS server request log | `crates/resourcefs-sources/src/https.rs`: route reader-mode output through `SourceResource::text_projection` instead of `markdown_projection`; the response reports `text/plain` and `https_uses_common_read_shape` turns red. Checkpoint must apply this mutation, observe red, restore, and observe green | `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::https_uses_common_read_shape` | 1–2 min | PENDING — checkpointed-build, end-to-end read slice and oracle mutation gate |
| C5 | Over-limit Markdown spills losslessly | reader-mode Markdown at and one logical line over the inline limit | Read the over-limit fixture, compare the inline prefix/ranges/EOF with a separately built expected Markdown value, follow the artifact continuation, and reassemble exact expected bytes; missing recovery metadata, wrong bounds, or byte loss falsifies | Test-owned expected Markdown bytes and artifact-chain reassembly, independent of renderer metadata | `crates/resourcefs-core/src/read.rs`: mark the first bounded page complete and omit continuation publication; the reassembled bytes are short and the bounded-profile fence turns red | `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::https_profile_read_recovers_bounded_markdown` | 1–2 min | PENDING — checkpointed-build, end-to-end read slice |
| C6 | C14 healthy control serves a real document | optional reachable and optional unreachable profiles; required unreachable existing row | Replace the open TCP-only comparison with the TLS fixture, call `rfs_read('/doc')`, require the literal body and `/doc` request log; retain the closed-port degraded wording assertion | TLS server request log and literal response body, compared with the closed-port profile that differs only in reachability | `crates/resourcefs-mcp/src/profile/mod.rs`: omit healthy origins from the mounted allowlist while still returning `Some(HttpsSource)`; startup still succeeds but `/doc` is never served, so the healthy-content and request-log assertions turn red | `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::optional_https_degrades_while_other_sources_serve` | 1–2 min | PENDING — checkpointed-build, C14-control slice |
| C7 | Core owns Markdown media type across reader projections only | empty, Unicode, selected reader-mode content; raw branch | Construct Markdown Source Resources directly and drive complete/selected HTTPS adapter rows; require the Markdown literal for reader mode and the unchanged non-Markdown raw result | Literal media-type table keyed by typed projection mode, independent of MCP rendering | `crates/resourcefs-core/src/resource.rs`: set `markdown_projection` to `TEXT_CONTENT_TYPE`; core and adapter media-type rows turn red | `crates/resourcefs-core` resource tests plus `crates/resourcefs-sources/tests/http_policy_contract.rs::https_reader_mode_reports_markdown_for_complete_and_selected` | <1 min | PENDING — checkpointed-build, media-type slice |

## Non-goals and future work

### Permanent non-goals

- **Invalid-certificate or invalid-hostname acceptance:** never part of ResourceFS; it destroys the TLS identity property and is mechanically forbidden by C1/C3.
- **Production trust-store mutation by tests:** the fixture uses an explicit test adapter; changing OS trust, certificate files, or operator configuration would leak test authority outside the test process.
- **External live endpoint for the permanent fence:** upstream availability and content drift would make the regression fence nondeterministic; the existing live-smoke policy remains separate.

### Intended future work

N/A — this design defers no intended work.

## Falsifier run log

- 2026-08-29 — **C1 cheapest falsifier, PASS.** `cargo test -p resourcefs-sources --features test-support --test http_tls_contract tls_certificate_binds_hostname` → 1 passed, 0 failed. The matching trusted leaf completed; the equally trusted hostname-mismatched leaf remained rejected with no request.

## Self-review

1. Every enumerated input shape maps to C1–C7.
2. Every falsification cell is filled; no risk-acceptance row exists.
3. Oracles use server-side TLS/request observations, literal expected values/digests, artifact byte reassembly, or mechanical source/target metadata.
4. Every row names a file-level mutation and the fence that turns red.
5. Each fence localizes one claim.
6. C5 converts the measurement into deterministic byte/range assertions.
7. Every negative-space item is a permanent non-goal; no intended work is deferred.
8. Every capability records Owner, New seam, and Forbidden; C3 and C7 mechanically fence placement.
9. C1, the cheapest falsifier, is PASS; every remaining row is PENDING with checkpointed-build ownership.

## Approval

Revision 1 (2026-08-29): checkpointed-build proved the original C1 mutation inapplicable: `danger_accept_invalid_hostnames(true)` compiled but reqwest rejected that builder configuration before TLS, so it did not change the measured hostname property. The replacement `danger_accept_invalid_certs(true)` compiled, was permitted, made the mismatching fixture complete and serve `/doc`, and turned the same fence red. C2 now names the actual validation site: reqwest accepts arbitrary bytes into `Certificate`, while temporary client construction rejects malformed DER; skipping that construction makes the C2 fence red. Claims, placement, oracles, fences, scope, and risk acceptances are unchanged.

Requester approval (verbatim): "I approve Revision 1"
Date: 2026-08-29
Approved risk acceptances: None.
