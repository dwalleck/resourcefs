# Budgeted plan: rfs-cek3

## Inputs and partition

Approved design: `.rfs-cek3/design.md`, requester approval "I approve" on 2026-08-29. Route: Structural. `spec.md` and empirical evidence are N/A per `route.md`.

Projected changed lines:

- Slice 1: 150
- Slice 2: 130
- Slice 3: 220
- Slice 4: 650, including test support and fixture-equivalent review weight for three small DER files
- **Slice sum:** 1,150
- **Churn margin:** 35% = 403 lines. The margin is deliberately larger than the usual small-change allowance because the stdio re-exec harness and TLS fixture may need platform-neutral shutdown/error handling after first compile.
- **Projected total:** 1,553 changed lines

The projected total is below the 4,000-line review-size gate. One PR increment is sufficient.

### PR increment: `rfs-cek3-profile-https-fence`

Slices 1–4 in dependency order. Mergeable definition: typed Markdown metadata, validated test-root construction, the production-unreachable test launch adapter, and its end-to-end fences land together; every slice is independently green and verifies without a later slice, while the increment as a whole closes the user-visible profile/MCP contract. No later increment exists.

## Slice 1: Give reader-mode HTTPS a typed Markdown Resource shape

**Claim IDs:** C7

**Expected behavior:** `SourceResource` can represent extracted Markdown without accepting an arbitrary media-type string; HTTPS reader-mode reads, including selected projections, report `text/markdown; charset=utf-8`, while raw projections retain their existing plain projection path.

**Oracle:** A literal projection-mode → media-type table plus direct `SourceResource` assertions; no MCP rendering participates.

**Stress fixture:** Empty Markdown, Unicode Markdown, a selected line, and the same response through `:raw`; reader rows remain Markdown and raw remains non-Markdown.

**Regression fence:** Core resource tests for `markdown_projection` plus `crates/resourcefs-sources/tests/http_policy_contract.rs::https_reader_mode_reports_markdown_for_complete_and_selected`, created in this slice.

**Named mutation:** From C7 — in `crates/resourcefs-core/src/resource.rs`, make `markdown_projection` use `TEXT_CONTENT_TYPE`; the core and Source Adapter media-type rows must turn red, then return green after restoration.

**Complexity/production scale:** N/A — no new loop or allocation; an existing constructor call selects one static media-type constant.

**Wall budget/phase:** N/A — no new runtime phase; the existing HTTPS projection construction changes constructor only.

**Files:** `crates/resourcefs-core/src/resource.rs`; the existing core resource test module/file; `crates/resourcefs-sources/src/https.rs`; `crates/resourcefs-sources/tests/http_policy_contract.rs`.

**Estimate:** 30–45 minutes.

**Diff estimate:** 150 changed lines.

**PR increment:** `rfs-cek3-profile-https-fence`.

**Commands and expected results:**

- `cargo test -p resourcefs-core markdown_projection` → empty and Unicode Markdown expose the literal Markdown media type and a content-derived Version Tag.
- `cargo test -p resourcefs-sources --features test-support --test http_policy_contract https_reader_mode_reports_markdown_for_complete_and_selected` → whole/selected reader rows report Markdown and the raw control does not.
- Apply the C7 named mutation, rerun both commands → at least the literal media-type assertions fail specifically for C7; restore and rerun → both return green.

## Slice 2: Validate one additional root inside the HTTP substrate

**Claim IDs:** C1, C2

**Expected behavior:** Test-support callers can construct the normal system-resolver substrate with one validated DER root; matching fixture identity succeeds, hostname mismatch remains rejected, and malformed DER fails before egress.

**Oracle:** TLS listener handshake/request records for valid roots; literal malformed DER plus listener accept count zero for rejection.

**Stress fixture:** A trusted leaf for the requested hostname, an equally trusted leaf for another hostname, and malformed root bytes. Expected: completed/requested, rejected/no request, constructor error/no accept.

**Regression fence:** `crates/resourcefs-sources/tests/http_tls_contract.rs::{tls_certificate_binds_hostname, malformed_additional_root_is_rejected_before_egress}`, with the malformed row created in this slice and the existing hostname row retained.

**Named mutation:** From C1 — enable invalid-hostname acceptance when an added root is present; the mismatching server records a completed session/request. From C2 — suppress the DER parsing error and continue with system roots; malformed construction succeeds. Each mutation turns its own row red, then restoration returns it green.

**Complexity/production scale:** N/A — no production loop changes. Test-support construction validates one bounded certificate once at launch; the existing client builder still owns root installation.

**Wall budget/phase:** N/A — one-off test-support launch construction; no production phase is introduced.

**Files:** `crates/resourcefs-sources/src/http/mod.rs`; `crates/resourcefs-sources/src/lib.rs`; `crates/resourcefs-sources/tests/http_tls_contract.rs`.

**Estimate:** 30–45 minutes.

**Diff estimate:** 130 changed lines.

**PR increment:** `rfs-cek3-profile-https-fence`.

**Commands and expected results:**

- `cargo test -p resourcefs-sources --features test-support --test http_tls_contract` → matching identity completes and sends, mismatching identity sends nothing, malformed DER is rejected with zero accepted sockets.
- Apply C1’s named mutation and rerun `tls_certificate_binds_hostname` → the mismatch row fails on the server-side oracle; restore → green.
- Apply C2’s named mutation and rerun `malformed_additional_root_is_rejected_before_egress` → construction/no-egress assertions fail; restore → green.

## Slice 3: Add a production-unreachable profile-launch test adapter

**Claim IDs:** C3

**Expected behavior:** A test harness can pass a typed fixture root directly into the normal profile check/probe/mount/server path, but `resourcefs` main, CLI syntax, Server Profile schema, and environment handling provide no route to select it.

**Oracle:** Cargo package target/feature metadata and a literal source-reference allowlist for the test interface/root token, plus successful default-feature compilation of the shipped binary.

**Stress fixture:** Inject a production-path environment lookup for the fixture-root token as the mutation. Expected: the architecture fence names the unexpected site even though compilation still succeeds.

**Regression fence:** `crates/resourcefs-mcp/tests/architecture_contract.rs::profile_https_fixture_trust_is_test_harness_only`, created in this slice.

**Named mutation:** From C3 — in `crates/resourcefs-mcp/src/profile/mod.rs`, read `RESOURCEFS_TEST_HTTPS_ROOT` from the production mount path. The source-site allowlist turns red; restoration returns it green.

**Complexity/production scale:** N/A — production execution remains unchanged; the alternate entry point is test-support-only and performs the existing one-off profile launch.

**Wall budget/phase:** N/A — one-off integration-test process launch; no production phase is introduced.

**Files:** `crates/resourcefs-mcp/src/lib.rs`; `crates/resourcefs-mcp/src/test_support.rs` (new); `crates/resourcefs-mcp/src/launch.rs`; `crates/resourcefs-mcp/src/profile/mod.rs`; `crates/resourcefs-mcp/tests/architecture_contract.rs`.

**Estimate:** 45–75 minutes.

**Diff estimate:** 220 changed lines.

**PR increment:** `rfs-cek3-profile-https-fence`.

**Commands and expected results:**

- `cargo check -p resourcefs-mcp --bin resourcefs --no-default-features` → shipped binary compiles without the test adapter in its input surface.
- `cargo test -p resourcefs-mcp --test architecture_contract profile_https_fixture_trust_is_test_harness_only` → only the gated test-support module and integration tests may name the fixture-root interface; CLI/profile/env production sites remain absent.
- Apply C3’s environment-hook mutation and rerun the architecture fence → it fails naming `profile/mod.rs`; restore and rerun → green.

## Slice 4: Drive profile HTTPS through stdio and strengthen C14

**Claim IDs:** C4, C5, C6

**Expected behavior:** The standard integration-test executable re-executes itself as a child MCP server, explicitly calls the test-support profile-launch adapter, completes TLS to a loopback fixture, and proves small common Markdown shape, over-limit lossless recovery, and a C14 healthy origin that served `/doc` rather than merely accepting its startup probe.

**Oracle:** Hand-authored structured-key/value tables, precomputed SHA-256 Version Tags, exact expected Markdown bytes and artifact-chain reassembly, plus the TLS fixture’s server-side handshake/request-target log.

**Stress fixture:** A small Unicode HTML document; a deterministic extracted Markdown document one logical line beyond the 48 KiB inline ceiling; an optional source at a closed port; and the same optional profile against a live TLS fixture. Expected: exact inline shape, exact lossless continuation, startup-specific degradation only for the closed port, and `/doc` recorded/returned for the live origin.

**Regression fence:** `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs::{https_uses_common_read_shape, https_profile_read_recovers_bounded_markdown, optional_https_degrades_while_other_sources_serve}`, created/strengthened in this slice.

**Named mutation:** From C4 — in `crates/resourcefs-sources/src/https.rs`, replace `markdown_projection` with `text_projection`; the end-to-end content-type row turns red. From C5 — in `crates/resourcefs-core/src/read.rs`, mark the first bounded page complete and omit continuation publication; byte reassembly turns red. From C6 — in `crates/resourcefs-mcp/src/profile/mod.rs`, omit healthy origins from the mounted allowlist while still returning `Some(HttpsSource)`; the healthy request/body assertions turn red. Restore each independently and require green.

**Complexity/production scale:** The only new loop is in test fixture code: O(c) accepted connections, with c ≤ 6 per test (one startup probe plus bounded read/continuation connections). Maximum accepted fixture work is 6 loopback connections and 8 MiB retained test body, matching the production fetch ceiling; exceeding either indicates a runaway test or request loop. Production algorithms are unchanged.

**Wall budget/phase:** N/A — one-off test execution. The fixture permits 10 seconds for child startup/shutdown and 5 seconds per expected request, test-only bounds chosen to avoid timing flakes while still detecting hangs.

**Files:** `crates/resourcefs-mcp/Cargo.toml`; `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs`; `crates/resourcefs-mcp/tests/support/profile_tls.rs` (new); `crates/resourcefs-mcp/tests/fixtures/profile_https/ca.der` (new); `crates/resourcefs-mcp/tests/fixtures/profile_https/server.crt.der` (new); `crates/resourcefs-mcp/tests/fixtures/profile_https/server.key.der` (new).

**Estimate:** 2–3 hours.

**Diff estimate:** 650 changed lines including fixture-equivalent review weight.

**PR increment:** `rfs-cek3-profile-https-fence`.

**Commands and expected results:**

- `cargo test -p resourcefs-mcp --test stdio_mcp_contract https_uses_common_read_shape` → child launches from the profile, TLS `/doc` is recorded, the exact Markdown/read metadata and precomputed Version Tag match.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract https_profile_read_recovers_bounded_markdown` → first page is bounded with artifact recovery/continuation; following the continuation reassembles exact expected Markdown bytes and reaches EOF.
- `cargo test -p resourcefs-mcp --test stdio_mcp_contract optional_https_degrades_while_other_sources_serve` → closed origin alone reports startup degradation; live origin completes TLS, records `/doc`, and returns the literal document.
- Apply C4, C5, and C6 mutations one at a time; rerun each owning command → its independent oracle fails for the named property; restore after each → owning command returns green.

## Tracker taxonomy

No intended work is deferred. Permanent negative space is inherited from the approved design: never disable TLS certificate/hostname verification, never mutate production trust for a test, and never make the permanent fence depend on an external endpoint.

## Self-review

1. C1–C7 are each assigned exactly once; every PENDING design row is assigned to its implementing slice.
2. Every slice carries all thirteen mandatory fields; conditional fields contain explicit N/A reasons.
3. Each slice creates or retains its own permanent fence and carries the approved named mutation.
4. New-loop complexity is stated only for the test TLS accept loop; production loop/scale changes are absent.
5. 1,150 + 403 churn margin = 1,553; one mergeable PR increment is below 4,000 lines.
6. No intended work is deferred; permanent negative space retains its rationale.
7. No slice is declared complete here; checkpointed-build owns every completion decision.
