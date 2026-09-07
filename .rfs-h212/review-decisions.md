# PR #1 review decisions — 2026-09-06

Source: requester-supplied `pr1-code-review-2026-09-06.md`, findings F1–F22. The platform prerequisite is separated below Jira extraction so platform repairs are independently reviewable. Jira cache changes (F2/F21) remain in the extraction slice, not the platform prerequisite.

## Decisions

| Finding | Premise | Decision | Result / verification boundary |
|---|---|---|---|
| F1 — dangling target followed by `..` | Verified | Accept | Existing-resource traversal uses successful kernel canonicalization, not the creation-target ancestor fallback or lexical parent collapse. `dangling_absolute_symlinks_preserve_kernel_resolution_errors` covers missing and dangling parent components against kernel behavior. |
| F2 — zero-byte ETag poisoning | Verified | Accept | Drop only a zero-byte validator before caching. Repeated reads refetch successfully; the distinct valid quoted-empty validator remains usable. Both new Jira adapter rows pass in the integrated candidate; this delta belongs to S1. |
| F3 — resolution error category and identity | Verified | Modify | Map canonicalization, link reads and fallback metadata failures through `resource_io_error(identity, "resolve", error)`. A non-directory traversal retains the requested Resource identity and the existing operational category rather than becoming a blanket permission denial. |
| F4 — dangling target through an alias | Verified | Modify | Require existing-target canonicalization equally for canonical and aliased targets. Both missing targets produce `not_found`; no reconstruction of an absent target is used. Covered with F1. |
| F5 — default-policy false positives | Verified | Modify | Require the exact policy exit mask and one intended crate-attributed error, reject fallback-configuration diagnostics, and retain a positive MIT-only control. Missing/renamed policy probes fail rather than passing under defaults. |
| F6 — partial production-budget gate | Verified | Modify | Run the complete release workspace and inventory all compiled ignored rows. Execute all 12 approved production rows; missing and unclassified rows fail closed. Live rows and the child-server helper are explicitly excluded. |
| F7 — shared-runner timing | Verified | Modify | Keep functional assertions on every platform, use scaled debug timing budgets, and retain strict production thresholds in the explicit release gate. No Windows exclusion or silent timing return remains in the affected rows. Native matrix evidence is still required. |
| F8 — enforcement and independent execution | Verified | Modify | CI invokes the executable gate even after a non-cancellation setup failure; its independent gates continue after failures and return final failure. Main protection requires all three platform contexts, strict up-to-date checks and administrator enforcement, as the requester selected. |
| F9 — deleted workflow-text fence | Verified | Accept | Replace the source-text mirror with the shared executable gate and behavioral failure/inventory probes. `AGENTS.md`, q12y spec/design/plan and the native tracker explicitly supersede the historical mirror claim. Hosted runs, not source comparison, establish matrix execution. |
| F10 — invisible profile override | Verified | Modify | Force release debug assertions off through both Cargo configuration and the runner environment. Actual Cargo metadata reports all 65 test harnesses with `debug_assertions=false` despite an incoming true override. Full functional/release execution replaces silently skipped timing bodies. |
| F11 — cargo-deny diagnostics | Verified | Modify | Preserve human/proxy output without treating it as policy evidence; JSON-shaped malformed output fails closed. Missing tool and malformed JSON probes retain actionable diagnostics and fail. |
| F12 — symlink to the workspace root | Verified | Accept | Represent exact-root resolution with `WorkspacePath::root()` and use directory-handle operations. Root-alias list/glob/search and directory-read classification are covered without manufacturing an empty ordinary path. |
| F13 — absolute spelling assertion | Verified | Accept | Absolute-path and File-URI representations compare exact strings; filesystem component comparison remains only where component equivalence is the contract. Core reference/tag contracts pass. |
| F14 — secondary panic during cleanup | Verified | Modify | Cache the unwind state and use fallible stderr writes while retaining child status and stderr. Real stdio fault injection exits 101 with stderr directed to `/dev/full`; restoring infallible `eprintln!` aborts with SIGABRT under the same conditions. |
| F15 — missing structured tool error | Verified | Modify | A local expected-success assertion includes the complete tool result; expected-error checks remain unchanged. A real stdio unsupported-scheme injection reports `invalid_reference`, its message, requested path and full structured envelope. |
| F16 — macOS interpreter | Verified | Modify | Install modern Homebrew Bash in the macOS lane and use that interpreter in the operator fixture. Keep the bootstrap's explicit interpreter requirement; native macOS verification remains required. |
| F17 — uninterruptible target resolver | Verified | Reject scope expansion; track separately | Requester selected separate resolver-isolation work in rfs-a7wm. Preserve legitimate absolute aliases and precise errors here; do not substitute lexical-prefix rejection. Filesystem canonicalization can still block a worker on an unresponsive mount. No cancellation/latency bound is claimed for that call. |
| F18 — ambient temporary directory | Verified | Accept | Share a feature-gated, synthetic native-absolute URI fixture helper. It validates contained relative names and performs no filesystem or environment lookup. Both relative and empty `TMPDIR` runs pass all 24 selected reference/tag tests. |
| F19 — duplicate rewrite work | Verified | Modify | Metadata resolution returns its borrowed-or-owned resolved Workspace Path for open to reuse. Ordinary paths remain borrowed; the previous second rewrite is removed. Existing alias behavior is covered; no measured syscall or latency improvement is claimed. |
| F20 — 64 MiB assertion copy | Verified | Modify | Compare committed text by borrow under its mutex, preserving the independent state oracle and receipt tag check. Same-length corruption fails at byte 0 with 820 bytes of complete diagnostics; the restored exact-limit control passes. |
| F21 — repeated cache eviction | Verified | Modify | Document the generation/body eviction invariant in the existing branches. Avoid introducing an abstraction solely to remove the small, authority-sensitive repetition. This comment belongs to S1. |
| F22 — generation mismatch | Verified existing behavior | Reject retry change | Preserve the existing fail-closed policy, consistent with the GitHub adapter. A retry is a separate concurrency-policy change, not necessary for extraction or the zero-byte-validator fix. |

## Executed local evidence

The integrated candidate at `/tmp/rfs-ci-platform` was exercised with Rust 1.98.0 and an aliased temporary directory. These results describe that candidate; the standalone prerequisite and hosted platforms have separate gates below.

- `python scripts/ci-gates.py`: passed formatting, all-target/all-feature Clippy, complete functional and release workspace tests, all 12 inventoried ignored production budgets, dependency vetting, and the S1 module-placement oracle. The process completed with `All repository gates passed.` The terminal capture retains the final verdict but elides middle suite output; no total test count is inferred from that truncated capture.
- Real Cargo `--no-run --message-format=json` through the runner's release command/environment: 65 test harness profiles, all with debug assertions disabled despite the incoming true override.
- Throwaway runner control-flow probe with injected subprocess outcomes: accepted the classified inventory, rejected a newly unclassified row and a missing production row, and returned 1 for formatting failure after subsequent dependency vetting ran. This is control-flow evidence; the full gate above is the real-tool evidence.
- Supply-chain smoke: both ordinary fixtures pass; renamed staged policy and missing committed policy fail; benign multi-line human/proxy warnings pass; missing cargo-deny and malformed JSON fail with retained diagnostics. All temporary source/policy changes were restored.
- Core `path_reference_contract` and `version_tag_contract`, separately with relative and empty `TMPDIR`: 24 passed in each run.
- Real stdio healthy fixture, unsupported-scheme injection, broken-stderr run and infallible-write mutation: outcomes as F14/F15 above. Healthy control passes after restoration. The abort probe disables core dumps and terminates its remaining process group.
- Exact-limit mutation state corruption: mismatch detected at byte 0 without dumping the 64 MiB operands; restored control passes. All injected source changes were restored.

## Standalone and hosted acceptance

The standalone prerequisite's lints, full functional/release workspaces, all 12 ignored production budgets and dependency vetting passed. Its aggregate correctly returned 1 for a single TLS import-order formatting error while continuing all later gates. `cargo +1.98.0 fmt --all` corrected that ordering and the subsequent formatting check passed; this is not represented as an exit-zero rerun of the whole entry point. Native Linux/macOS/Windows matrix results must be recorded before closing the platform blockers. Local Linux success is not evidence for native Windows directory classification, native trust-store behavior or Darwin process-group cleanup.

Main branch protection was applied through GitHub's API and its response verified: required contexts `gates (ubuntu-latest)`, `gates (macos-latest)`, `gates (windows-latest)` from the GitHub Actions app; strict mode and administrator enforcement enabled. This configuration evidence is distinct from successful check execution.

## Follow-up two-axis review

### Standards

No hard violation remains after the reviewer corrected its initial report: the fallible diagnostic writes are explicitly justified test-only behavior, and the alleged new advisory-policy contradiction was not present in the changed text. Keep the small debug/release timing branches beside their individual thresholds rather than adding a cross-crate policy helper. Document the runner's Python 3.9+ requirement.

F10 additionally fails closed when Cargo reports debug assertions enabled for a compiled release test target, covering overrides that defeat the global setting. A controlled-metadata probe exercises that rejection alongside the existing missing/unknown-row and continued-gate checks.

### Spec

Native aarch64 macOS evidence resolves the static concern: `process_admission`, `stream_boundaries` and `unix_tree_cleanup` pass in both functional and release suites in runs 34070623469 and 34070750919. The reviewer withdrew the finding. Persistent permission denial intentionally remains fail-closed; the observed transient zombie window no longer fails cleanup.

## Requester-approved performance follow-ups — 2026-09-07

The first native matrices exposed only previously unenforced production timing limits; all platform functional/release suites, formatting, lint and dependency checks passed. Ubuntu's push matrix passed completely. Failed measurements:

| Boundary | Native measurement | Previous limit | Revised enforced limit | Investigation |
|---|---|---|---|---|
| 64 MiB journal fingerprint and exact comparison | macOS 196.034167 / 206.22575 ms; Ubuntu PR 50.374804 ms | 50 ms | 500 ms | rfs-o4am |
| Fresh TLS 64 MiB mutation request, including fixture capture | Windows 119.3156 / 191.5212 ms | 100 ms | 500 ms | rfs-q5l8 |

Requester direction: when there is no immediate solution, increase the budget and file performance investigations. No backend change or fixture-copy optimization has yet been proven portable and sufficient. Keep those investigations separate rather than extending this prerequisite. The journal's count/yield limits and debug comparison limit are unchanged; all exact identity, full-body and timing assertions remain active.

Evidence: [push matrix](https://github.com/dwalleck/resourcefs/actions/runs/34070623469), [PR matrix](https://github.com/dwalleck/resourcefs/actions/runs/34070750919). The two revised ignored tests pass locally with release debug assertions explicitly disabled. In [the revised PR matrix](https://github.com/dwalleck/resourcefs/actions/runs/34073108972), Windows passes the complete gate and Ubuntu passes every production budget. macOS is still running at this checkpoint.

Ubuntu's sole remaining failure is an incidental wording assertion in `timeout_returns_source_unavailable`: the expected `source_unavailable` category and explicit configured-deadline diagnostic are correct, but the assertion requires the substring `timeout`. Remove that wording assertion rather than pinning another spelling. The successful full-transfer control and failing-transfer category assertions remain. The targeted debug test and formatting check pass after this removal; production timeout behavior is unchanged.
