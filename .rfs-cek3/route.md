# Route: rfs-cek3

Change: Fence the HTTPS common read shape through a profile-launched server using test-only fixture trust.
Date: 2026-08-29

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | Current repository evidence covers the relevant behavior: `crates/resourcefs-sources/src/http/mod.rs::HttpSubstrate::with_host_lookup_and_roots` already builds a certificate-validating client with injected DER roots under `test-support`; `crates/resourcefs-sources/tests/support/tls.rs` supplies a deterministic locally trusted CA, matching leaf, listener, and request log; `crates/resourcefs-mcp/src/profile/mod.rs::mount_https` currently calls `HttpSubstrate::new`, proving the launch-path gap without an external-system premise. | no |
| 2 | Structural boundary | The change must carry test-only trust from the profile-launched MCP process into the Source Adapter constructor. That crosses the `resourcefs-mcp::profile` → `resourcefs-sources::HttpSubstrate` module boundary and touches a security-sensitive trust boundary even though the production profile schema and default production API remain unchanged. | yes |
| 3 | Production-scale risk | The mechanism is test-only construction input used once at launch. It does not add production request-path work, state growth, concurrency, or data-volume exposure. The read itself continues through the existing bounded HTTPS adapter. | no |
| 4 | Explicit behavior | **G1** a Server Profile allowlists a reachable fixture HTTPS origin and the test-only launch gate supplies its CA and host resolution; **W1** the compiled profile-launched server receives `rfs_read` for that origin; **T1** it completes a real TLS exchange and returns the common read shape: canonical HTTPS reference, `text/markdown`, content-derived Version Tag, `mutable: false`, and bounded-read metadata, with `artifact://` recovery when inline output omits bytes. **G2** the production/default binary is built without test support; **W2** the test gate inputs are presented; **T2** no alternate trust anchor is accepted, and no invalid-certificate or invalid-hostname bypass exists anywhere. **G3** the C14 healthy-origin control uses the same reachable TLS fixture; **W3** the profile-launched server reads its document; **T3** the assertion proves response bytes were served rather than merely proving the TCP probe mounted the source. **G4** the named common-shape mutation is made at an applicable production hop; **W4** the focused fence is compiled and run; **T4** the mutation is permitted, changes the measured property, turns the fence red, and restoration returns it green. | yes |

Unknown tests: none

## Selected route

Structural — test-only trust must cross a module boundary and preserve a production security invariant; design approval is required before implementation.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior fully explicit (T4 verdict yes) |
| evidence.md, probe.* | prove-it-prototype | N/A — no unverified premise (T1 verdict no) |
| design.md | falsifiable-design | required — Structural route |
| plan.md | budgeted-plan | required — Structural route |

Oracle checkpoint in `checkpointed-build`: required — Structural route

## Downstream sequence

falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Structural — every downstream artifact satisfies its owning stage's completion criterion, ending with no FAIL in checkpointed-build's recorded gate.

Result: 2026-08-29 | PASS — approved `design.md` Revision 1 and `plan.md` satisfy their owning criteria; all four checkpointed slices record no FAIL; C1–C7 falsifiers, independent oracles, regression fences, named-mutation red checks, and restored-green checks passed; full formatting, Clippy, and workspace test gates passed.
