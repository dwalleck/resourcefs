# Route: rfs-1dpd-rfs-sbeh

Change: Repair profile TLS disconnect handling and reuse the shared CLI stdio test harness.
Date: 2026-09-13
Source baseline: 11a6bf2 (main); requested tickets rfs-1dpd and rfs-sbeh claimed and started for omp in the primary tracker.

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The reported Windows ConnectionReset failure is accepted evidence. Baseline profile_tls.rs::serve propagates request-read errors and tolerates response disconnects only for BlockedNative. The fixture's private serve_request now permits deterministic fault injection after both real TLS handshakes complete. The locked tokio-rustls 0.26.4 common/mod.rs:100-107,198-208 propagates transport read errors unchanged; rustls 0.23.43 common_state.rs:674-684 requests transport data when no plaintext/close notification is pending. Main read these exact dependency sources. ErrorKind preservation is this verified dependency premise, not a claim made by the post-handshake injection tests. No new platform premise is needed. The ticket's keep-alive explanation is stale: responses send Connection: close. | no |
| 2 | Structural module shape | Only test support and cli_contract.rs change. Existing profile TLS fixture retains request parsing and failure logging; existing support/stdio.rs retains framing, process admission and teardown. No production interface, schema, seam, module ownership or dependency changes. CLI's duplicated test-only framing/lifecycle implementation is removed in favor of its existing shared owner. | no |
| 3 | Production-scale risk | No production code or workload changes. The existing test-only 16-process admission limit and 10-second EOF shutdown deadline are reused without altering their bounds. | no |
| 4 | Explicit behavior | Given an established TLS client with no request-head bytes, when reading returns ConnectionReset or ConnectionAborted, then the fixture succeeds as on clean EOF. Given any partial head, when those errors occur, then the fixture still fails. Given a response write/shutdown, when the client disconnects with BrokenPipe, ConnectionReset or ConnectionAborted, then every response variant tolerates it; unrelated failures remain errors and the failure log/assertion remains intact. Given a profile CLI process, when spawned, then it acquires and retains the existing process permit until teardown. Given protocol output, when read, then shared strict JSON-RPC 2.0 framing applies. Given EOF, when finishing, then the existing bounded shutdown and stdout/stderr/status checks apply. Existing profile authority and immutability behavior is unchanged. | yes |

Unknown tests: none.

## Selected route

Local — explicit test-harness corrections within existing responsibilities; no production changes or unverified premise.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior fully explicit |
| evidence.md, probe.* | prove-it-prototype | N/A — no unverified empirical premise |
| design.md | falsifiable-design | N/A — Local route: no design gate |
| plan.md | budgeted-plan | N/A — Local route: no plan gate |

Oracle checkpoint in checkpointed-build: N/A — Local route; checkpointed-build does not run.

## Downstream sequence

None — normal repository fix/TDD. Independent writers own profile_tls.rs and cli_contract.rs; Main owns routing, assembled validation and tracker updates. Writers do not run builds, formatters, linters or tests concurrently. Main runs deterministic red/restored-green checks after integration, then focused CLI/stdio contracts and the repository gate on the stable tree.

## Terminal criterion

The deterministic profile TLS disconnect regression tests demonstrate red on the pre-fix branches and green on the fix; cli_contract exercises real profile-launched processes under the shared harness, with a bounded regression/smoke check for restored safeguards. Run the originally affected github_source_stdio_reconstructs_exact_binary_and_enforces_decoded_caps and complete MCP cli_contract/stdio_mcp_contract targets. Record commands, source state, date and PASS/FAIL here. The full repository gate is required by AGENTS.md; no failed mandatory gate is waived.

Result: 2026-09-13 | PASS — focused behavioral proof, seven independent red/restored-green TLS mutations, three red/restored-green CLI safeguard smokes, final-source native Windows contracts, independent review, and the full repository gate all passed. No unresolved FAIL remains.

### CLI safeguard proof — 2026-09-13

- Main ran a throwaway integration target including the actual cli_contract.rs, with initialized real ResourceFS subprocesses. Two tests replaced the child after initialization with a controlled Rust helper: valid JSON carrying jsonrpc=1.0, and a child sleeping 12 seconds after EOF. The admission test held 16 initialized processes and attempted a 17th, then released one and observed admission.
- Negative command: `cargo test --locked -p resourcefs-mcp --test rfs_sbeh_smoke sbeh_smoke_ -- --test-threads=1` against baseline CLI SHA256 `f480c1aa3fc67aa8c139cf93d5c3d1dfff60d1bd1694da3ae6ec5a3a15dd1d03`. All three failed on their intended missing safeguard: accepted JSON-RPC 1.0; waited through 12-second EOF stall; 17th process bypassed admission. Receipt: `artifact://73`. Earlier exploratory runs are superseded by this exclusive-writer run.
- Restored command: the same smoke invocation against fixed CLI SHA256 `122ef7997f8c12f6b95673ec95af6427aaf848e68d86a3d508ec531ec2bb2342` — PASS, 3/3, 10.61 seconds. `cargo nextest run --locked -p resourcefs-mcp --test cli_contract` — PASS, 10/10. Receipt: `artifact://76`. Environment: Linux, isolated worktree target directory; no other build/test job on that resource. TLS edits do not affect this target or its dependencies.
- The temporary source/helper are retained outside the repository under `/tmp/rfs-1dpd-sbeh-proof` for this session, not permanent tests. Existing profile authority and immutability contracts provide ongoing real-CLI coverage.

### TLS boundary proof — 2026-09-13

Final TLS fixture SHA256: `6a41548ecaca039de6f05995ef3444c97ba73fbf120d56a99e1db02a75ce70fe`.

`cargo test --locked -p resourcefs-mcp --test stdio_mcp_contract profile_tls::tests:: -- --test-threads=1` — PASS, 7/7, 0.03 seconds. Every scenario completes both real TLS handshakes, injects a typed fault at the same private handler used by serve, asserts the fault executed, and has a ten-second overall timeout.

Each row below was applied alone to that snapshot; the named test failed by assertion (not compilation), then the exact snapshot was restored and all seven TLS tests passed. Command form: `cargo test --locked -p resourcefs-mcp --test stdio_mcp_contract profile_tls::tests::<test> -- --exact --nocapture`. Raw red/green logs are under `/tmp/rfs-1dpd-sbeh-proof/<mutation>-{red,green}.log`.

| Mutation | Named test | Red | Restored all-seven |
|---|---|---|---|
| read-propagates-reset — restore request-read `?` | serve_treats_empty_head_disconnects_as_eof | PASS | PASS |
| read-swallows-partial — remove empty-head guard | serve_propagates_partial_head_disconnects | PASS | PASS |
| read-swallows-other — accept Other at read | serve_propagates_unrelated_request_read_errors | PASS | PASS |
| response-blocked-only — restore old variant guard | serve_tolerates_response_disconnects_for_each_variant | PASS | PASS |
| response-rejects-reset — remove reset tolerance | serve_tolerates_response_body_disconnect | PASS | PASS |
| response-rejects-aborted — remove aborted tolerance | serve_tolerates_response_shutdown_disconnect | PASS | PASS |
| response-swallows-other — accept Other at write | serve_propagates_unrelated_response_write_errors | PASS | PASS |

### Review and integration

- Independent reviewer `HarnessReview` accepted the CLI repair and precise TLS tolerance. Two test-harness findings were verified and repaired: unbounded waits and a missing explicit fired-fault receipt. The first TLS regression design was replaced with simpler straight-line post-handshake fault injection; the reviewer re-read final TLS SHA256 above and found no remaining defects. Responsibility remains in the existing test-only fixture; the extraction is private and no production seam changes.
- A handshake-vacuity advisory did not reproduce: on the original fixed TLS snapshot `88403cfabf35f5121eb4a69d7a03e7eca871364d570bfe2980be95ead453a5f0`, replacing the read match with `?` made `serve_treats_empty_head_disconnects_as_eof` fail with `empty-head ConnectionReset: Err(Custom { kind: ConnectionReset, error: "injected read disconnect" })`. Its subtle readiness machinery was nevertheless removed. Later move-out-of-Pin/result-consumption advisories were rejected: successful compilation and seven passing tests disprove them; the patterns bind Copy fields without moving their enclosing values.
- `cargo nextest run --locked -p resourcefs-mcp --test cli_contract --test stdio_mcp_contract` — PASS, 67 tests passed, one intentional child-server entry skipped (`profile_https_test_server`, exercised by parent contracts), including the originally reported github_source_stdio_reconstructs_exact_binary_and_enforces_decoded_caps. Receipt: `artifact://106`.
- Final CLI SHA256 after cargo fmt: `f17b6eea7924b1902490e13f7116ebef08bdfdba7206a008d488e7e71d7bfc92`. CLI safeguard smoke evidence above remains applicable: only import formatting changed.
- Product docs, schema documentation, changelog and scaffold are intentionally unchanged: these fixes affect test fixtures only. No new dependency or production API was introduced. No live upstream smoke is required; the network tests use their real local TLS fixture.

### Native Windows verification — 2026-09-13

- Host: resourcefs-win11, native x86_64 MSVC, cargo 1.98.1, imported VS BuildTools environment. Source staged at `C:\rfs-1dpd-sbeh`, isolated target `C:\rfs-target-1dpd-sbeh`, debug profile with debug-info disabled; no release-mode claim is made for this native run.
- The runner printed SHA256 receipts matching the final TLS and CLI files above before compiling. Command: `cargo test --locked -p resourcefs-mcp --test cli_contract --test stdio_mcp_contract -- --test-threads=1`.
- Result: PASS. cli_contract 10/10 (8.04 seconds); stdio_mcp_contract 57 passed, one intentional child-server entry ignored (76.42 seconds). All seven new TLS regression tests and github_source_stdio_reconstructs_exact_binary_and_enforces_decoded_caps executed and passed. The final-source run took 92.53 seconds including transfer and build. The earlier native run against the superseded larger test harness is historical, not the final receipt.
- Driver: `/tmp/rfs-1dpd-sbeh-native/verify.ps1` via the configured WinRM helper. Script execution permission was scoped to that PowerShell process only. No machine-wide policy or repository build configuration changed.

### Complete repository gate — 2026-09-13

- Command: `CARGO_TARGET_DIR=/home/dwalleck/repos/resourcefs-1dpd-sbeh/target python scripts/ci-gates.py`, source baseline 11a6bf2 plus the final TLS/CLI file hashes above. Linux, isolated worktree build resource; no concurrent local compiler, test suite or source mutation. `RFS_SKIP_TEST_TARGETS` and `RFS_SKIP_RELEASE_TEST_TARGETS` were unset.
- Result: PASS, exit 0, 724.90 seconds. Terminal output: `All repository gates passed.` Receipt: `artifact://112`.
- Module placement, formatting, all-feature/all-target Clippy with warnings denied, functional nextest (816 passed; 37 intentional ignored rows handled by the runner's separate classifications), doctests, release workspace, enumerated ignored production budgets, fuzz formatting/type-check and cargo-deny all passed.
- The fuzz-only feature configuration emitted four warnings in unchanged Atlassian source files; its configured check succeeded. The workspace Clippy gate passed with warnings denied. No warning suppression, skipped target or gate waiver was added.
- Cleanup at implementation completion: temporary CLI smoke target removed before this gate; native staging server stopped; writing/review workers released. Session proof sources/logs remain outside the repository. Tracker records remained claimed/in progress pending publication; implementation and evidence were committed together. Publication authorization was supplied afterward, below.

### Main integration and PR authorization — 2026-09-13

- Requester authorization: "Pull main, merge it in, and open a PR". Scope: publish this branch as a PR targeting main for rfs-1dpd and rfs-sbeh; no PR merge or tracker closure authorized.
- Fetched main `263a69fc0de49c0e973592240e223612cb42096b` and merged it cleanly into the fix branch. The incoming delta from the original baseline contains only four research documents and closures of rfs-0n97, rfs-jrz7 and rfs-poau; no code, dependency, fixture, oracle, build configuration or gate script changed.
- Tested integration revision: `097d9ba4657a56ce07f27777961963d779515c35`; this merge commit is amended only to add this evidence record before publication. Both changed test-file SHA256s still match the final-source proofs above. The PR tracker delta contains exactly rfs-1dpd and rfs-sbeh.
- Fresh integration command: `CARGO_TARGET_DIR=/home/dwalleck/repos/resourcefs-1dpd-sbeh/target cargo nextest run --locked -p resourcefs-mcp --test cli_contract --test stdio_mcp_contract` — PASS, 67 passed, one intentional child-server entry skipped, 5.288 seconds. Receipt: `artifact://130`.
- Full repository gate, native Windows run, mutation checks, CLI safeguard smokes and independent review above remain valid PASS results: their complete source/configuration/dependency inputs are unchanged by this documentation/tracker-only merge. They were not rerun or represented as fresh integration runs.
- Primary main was also fast-forwarded to the fetched revision. Byte comparisons proved all four untracked document collisions identical to incoming main before replacing them with tracked copies. The primary tracker and every unrelated dirty file were restored byte-identically; the scoped temporary tracker stash was removed after verification. Primary-checkout dirty work is not included in this PR.
