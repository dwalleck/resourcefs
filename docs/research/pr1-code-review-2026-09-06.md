# PR #1 code review (rfs-h212 → main, head f50c0b3)

Automated review run on 2026-09-06 at effort `xhigh`, with the instruction not to cap
functional or structural findings at fifteen. Twenty-two findings survived verification;
the reviewer reproduced several by running the code. Findings are numbered in the
reviewer's severity order (F1 is most severe) and grouped by theme below.

**Scope caveat.** The review covered the six commits on the PR head `f50c0b3`. It did
not see the uncommitted working-tree changes (Jira project browsing, `browse.rs`,
`wire/`, `render/`, the new contract tests, or the Linux-only module-placement CI step).

## Index by theme

| # | Theme | Location | One line |
|---|---|---|---|
| F1 | Filesystem symlink containment | `crates/resourcefs-sources/src/filesystem.rs:2542` | rewrite_absolute_symlink resolves `..` lexically when the absolute target does not exist |
| F3 | Filesystem symlink containment | `crates/resourcefs-sources/src/filesystem.rs:2535` | Routing symlink targets through resolve_absolute_target maps every non-NotFound canonicalize failure to Per... |
| F4 | Filesystem symlink containment | `crates/resourcefs-sources/src/filesystem.rs:2627` | The alias fix (f50c0b3) only works when the target exists |
| F12 | Filesystem symlink containment | `crates/resourcefs-sources/src/filesystem.rs:2546` | An absolute symlink whose canonical target IS the workspace root makes strip_beneath return Some("") (paths... |
| F17 | Filesystem symlink containment | `crates/resourcefs-sources/src/filesystem.rs:2535` | rewrite_absolute_symlink now calls std::fs::canonicalize (line 2625) on the raw, workspace-controlled absol... |
| F19 | Filesystem symlink containment | `crates/resourcefs-sources/src/filesystem.rs:1398` | open_workspace_entry runs the absolute-symlink rewrite chain twice per absolute-symlinked entry |
| F2 | Jira transport | `crates/resourcefs-sources/src/atlassian/jira/transport.rs:57` | fetch_cached stores CacheMetadata with an empty ETag (BoundedHttpResponse::etag() returns Some("") for an e... |
| F21 | Jira transport | `crates/resourcefs-sources/src/atlassian/jira/transport.rs:159` | AtlassianSource::cache (lines 159-179) is byte-identical to GithubSource::cache (github/mod.rs:361-381 |
| F22 | Jira transport | `crates/resourcefs-sources/src/atlassian/jira/transport.rs:90` | On a 304, a generation mismatch for atlassian.jira.<site> returns SourceUnavailable ("Jira read was invalid... |
| F6 | CI workflow and budgets | `.github/workflows/ci.yml:49` | The new 'Production-scale release budgets' step builds only three resourcefs-core test binaries, so nine ot... |
| F7 | CI workflow and budgets | `.github/workflows/ci.yml:49` | The 1 ms `scratch_name_enumeration_fits_its_budget` assertion the release step now carries onto macos-lates... |
| F8 | CI workflow and budgets | `.github/workflows/ci.yml:46` | `--no-fail-fast` (added in f50c0b3) exposes pre-existing macOS/Windows failures for the first time — 92 mac... |
| F9 | CI workflow and budgets | `crates/resourcefs-mcp/tests/supply_chain_contract.rs:213` | The PR deletes ci_workflow_mirrors_local_gates and GATE_COMMANDS (base lines 213-256), the only in-repo fen... |
| F10 | CI workflow and budgets | `crates/resourcefs-core/tests/local_quota_contract.rs:270` | Three previously unconditional wall-clock budgets (1 ms here; 5 s at read_engine_contract.rs:812 and mutati... |
| F5 | Supply-chain gate tests | `crates/resourcefs-mcp/tests/supply_chain_contract.rs:230` | With `--config` removed, license_gate_rejects_copyleft passes even when no deny.toml is loaded |
| F11 | Supply-chain gate tests | `crates/resourcefs-mcp/tests/supply_chain_contract.rs:257` | assert_rejected_by JSON-parses every stderr line with `.expect("cargo-deny JSON output")`, so any non-JSON ... |
| F16 | macOS platform | `crates/resourcefs-sources/tests/atlassian_fixture_operator_contract.rs:102` | The PR's macOS-portability change to this constructor (canonical TMPDIR parent, 'Normal state parents must ... |
| F13 | Test quality | `crates/resourcefs-core/tests/path_reference_contract.rs:84` | The Absolute arm of the golden test switched from exact string equality to Path equality without need (Abso... |
| F14 | Test quality | `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs:740` | Drop for McpProcess now runs eprintln! unconditionally on the thread::panicking() path (line 740, plus the ... |
| F15 | Test quality | `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs:476` | The panic-time Drop diagnostics this PR added were blind to the very failure they were added for |
| F18 | Test quality | `crates/resourcefs-core/src/reference.rs:1877` | The Windows fix replaced hermetic `file:///...` literals with `Url::from_directory_path/from_file_path(std:... |
| F20 | Test quality | `crates/resourcefs-core/tests/mutation_engine_contract.rs:1244` | Test-only |

## Filesystem symlink containment

### F1. `crates/resourcefs-sources/src/filesystem.rs:2542`

rewrite_absolute_symlink resolves `..` lexically when the absolute target does not exist: resolve_absolute_target's NotFound fallback (line 2627) returns the un-canonicalized path, strip_beneath keeps `missing-dir/../secret.txt`, and normalize_relative_symlink_target (line 2556, `Component::ParentDir if normalized.pop()`) collapses it to `secret.txt`, so a dangling link is silently redirected to an existing sibling and read() returns that file's content and canonical reference where the kernel returns ENOENT (pre-existing on main; the PR's canonicalize-first change narrows it but explicitly routes the symlink path through the fallback).

**Failure scenario.** Root /real/root contains secret.txt ("SECRET") and link.txt -> /real/root/missing-dir/../secret.txt with missing-dir absent. std::fs::read(link.txt) -> Err(NotFound). resourcefs read("link.txt") -> Ok(content="SECRET", canonical_reference="rfs://workspace/workspace/secret.txt"). Reproduced on PR head and on main. Fix: in the symlink path treat canonicalize NotFound as NotFound (or reject any lexical fallback containing a ParentDir component); the fallback is only appropriate for resolve_absolute's root matching.

### F3. `crates/resourcefs-sources/src/filesystem.rs:2535`

Routing symlink targets through resolve_absolute_target maps every non-NotFound canonicalize failure to PermissionDenied "absolute Resource could not be safely resolved" (lines 2628-2633) with no resource identity, where main's lexical rewrite let the subsequent cap-std open fail through resource_io_error (io-kind-mapped category plus identity). Category regression for ENOTDIR/ENAMETOOLONG/EIO (SourceUnavailable -> PermissionDenied) and message/identity regression for EACCES and absolute-symlink loops; contradicts rfs-e2b7's acceptance text "Preserve ... errors" and f50c0b3's "no ... error category is broadened", yet .rivets/issues.jsonl closes rfs-e2b7 on Linux-only evidence. Affects file (2496), metadata (1454) and directory (1478) callers; no test covers a broken absolute-symlink target.

**Failure scenario.** Workspace root <root> with regular file <root>/notes.txt and symlink <root>/link -> <root>/notes.txt/child; workspace-relative read of 'link'. Main: category source_unavailable, message "failed to open Resource 'link' (NotADirectory)". Head: canonicalize fails ENOTDIR (not NotFound) -> category permission_denied, message "absolute Resource could not be safely resolved". Clients branching on ErrorCategory (render.rs emits category().as_str()) see a spurious permission denial with no resource name.

### F4. `crates/resourcefs-sources/src/filesystem.rs:2627`

The alias fix (f50c0b3) only works when the target exists: `Err(error) if error.kind() == io::ErrorKind::NotFound => lexical_target` keeps the un-aliased spelling, so a DANGLING absolute symlink spelled through a root alias fails strip_beneath and is classified PermissionDenied "Resource symlink resolves outside the selected Workspace Root" instead of NotFound; the identical dangling link spelled via the canonical root yields NotFound, and the aliased link with the target present succeeds. Error category depends on target existence plus spelling, not containment.

**Failure scenario.** Exact layout of the new test absolute_symlink_target_aliases_preserve_containment with workspace/target.txt deleted before the first read: read(link.txt) -> PermissionDenied (observed in a run); canonical-spelled dangling -> NotFound; aliased-present -> Ok. Same shape on macOS for a root canonicalized to /private/var/... and a link spelled /var/... with a missing target. Also reached via capability_metadata (1454) and open_capability_directory (1478). Fix: canonicalize the deepest existing ancestor of lexical_target and re-append the missing tail before the containment check.

### F12. `crates/resourcefs-sources/src/filesystem.rs:2546`

An absolute symlink whose canonical target IS the workspace root makes strip_beneath return Some("") (paths.rs:93-95); with no trailing components `rewritten` is empty and `WorkspacePath::new("")` fails with InvalidReference "Path Reference must address a Resource below a Workspace Root" (reference.rs:111-115). Pre-existing for the canonical spelling (dirlink -> /real/root), but the PR's switch to resolve_absolute_target newly routes alias spellings (dirlink -> /alias/root, previously PermissionDenied) into the same misleading error; the failure surfaces first in capability_metadata (line 1454) and the glob/list walker records it as a spurious InvalidReference diagnostic and skips the entry (verified at runtime).

**Failure scenario.** Root /real/root contains a.txt and dirlink -> /real/root (or -> /alias/root resolving to it). read('dirlink') or a walk visiting it -> cap-std PermissionDenied -> rewrite_absolute_symlink -> strip_beneath("/real/root", "/real/root") = Some("") -> WorkspacePath::new("") -> Err(InvalidReference). Observed: dirlink and aliaslink both return InvalidReference while dirlink/a.txt and aliaslink/a.txt succeed. Fix must also handle the empty relative path in contained_workspace_path (2686-2694) and route empty-path cap-std ops to dir_metadata()/try_clone() as the branch at 1370-1392 already does.

### F17. `crates/resourcefs-sources/src/filesystem.rs:2535`

rewrite_absolute_symlink now calls std::fs::canonicalize (line 2625) on the raw, workspace-controlled absolute target BEFORE the strip_beneath containment check, from every walked entry (walk_workspace -> open_workspace_entry -> capability_metadata:1398) as well as reads, inside spawn_blocking (744/844/873) with no timeout and unreachable by OperationGuard cancellation; it also yields two distinguishable messages for outside targets (EACCES/ELOOP -> 'could not be safely resolved' vs ENOENT/existing -> 'resolves outside the selected Workspace Root'). Not new in kind — main's resolve_absolute already ran the identical logic for caller-supplied absolute/file:// references to the same observer, and DESIGN.md:134 requires canonical-target containment — so low severity, but the PR adds a content-driven vector (symlinks planted in the workspace) that base rejected lexically with zero host I/O.

**Failure scenario.** Workspace content contains `link -> /mnt/stale/x` on a hung NFS/FUSE/autofs mount: rfs_read of link, or any rfs_glob/rfs_search whose walk reaches it, blocks in canonicalize on a blocking-pool thread; client cancel returns but the thread stays wedged and repeated requests exhaust the pool. Base rejected this with PermissionDenied immediately. Cheap mitigation: collapse resolve_absolute_target errors to the single containment message on the symlink path and only canonicalize targets that lexically start with a configured root's prefix.

### F19. `crates/resourcefs-sources/src/filesystem.rs:1398`

open_workspace_entry runs the absolute-symlink rewrite chain twice per absolute-symlinked entry: capability_metadata (line 1398) discards the rewritten WorkspacePath it computed, and open_capability_directory (1400) / open_capability_file (1415) re-derive it from the original path, so cap-std fails with PermissionDenied again and rewrite_absolute_symlink re-walks every component. The PR's change at 2535 (normalize_platform_path -> resolve_absolute_target) makes each redundant pass a std::fs::canonicalize rather than a pure string op. Affects discovery candidates (2038, once per absolute-symlinked entry in a glob/search walk) and the scope/.gitignore opens (1084/1321/1363/1748) once per operation; rfs_read calls open_capability_file directly and pays once.

**Failure scenario.** Per affected entry at depth d: 2 x [d symlink_metadata + 1 read_link + 1 canonicalize] + 2 failed + 2 successful cap-std ops, on the blocking pool. Fix: have capability_metadata return (Metadata, WorkspacePath) and open with the rewritten path (the post-open final-path containment checks at 1401-1406/1425-1429 still guard races), halving the walk cost for symlink-heavy trees.

## Jira transport

### F2. `crates/resourcefs-sources/src/atlassian/jira/transport.rs:57`

fetch_cached stores CacheMetadata with an empty ETag (BoundedHttpResponse::etag() returns Some("") for an empty `ETag:` header, http/mod.rs:1164-1171, and the store at line 120 only checks is_some()), then every later read hits the is_empty() guard at lines 57-62 and returns an error WITHOUT cache_remove_if_generation; nothing ever bumps the atlassian.jira.* generation (cache_remove_namespace is only called from github/mutation.rs), so the issue stays unreadable for the rest of the session. Moved verbatim from base; the GitHub transport has no such guard and self-heals by refetching.

**Failure scenario.** Jira returns 200 with an empty ETag header for GET /rest/api/3/issue/10001. Read 1 succeeds and caches {etag:""}; reads 2..n of jira://acme/issues/10001 fail `source_unavailable: Jira session cache ETag is empty` and make zero upstream requests (reproduced: 1 upstream request observed across 3 reads). Fix: filter empty ETags before storing at line 111/120, or evict with cache_remove_if_generation before returning at line 61.

### F21. `crates/resourcefs-sources/src/atlassian/jira/transport.rs:159`

AtlassianSource::cache (lines 159-179) is byte-identical to GithubSource::cache (github/mod.rs:361-381: cache_put_if_generation then evict on LimitExceeded), and the doc comment explaining the eviction invariant lives only on the GitHub copy. This PR moves the function verbatim from atlassian/jira.rs per the approved design ('move existing helpers', .rfs-h212/design.md:63), so the duplication is pre-existing debt relocated rather than introduced; the rest of transport.rs (request, CacheMetadata, fetch_cached, classify_status, sanitize_fetch_error) genuinely diverges from GitHub's.

**Failure scenario.** No runtime failure. If the LimitExceeded-eviction contract (a ceiling refusal drops any prior entry so a later 304 cannot revive stale content) is changed in one adapter and not the other, the two sources silently diverge, and an editor of the Jira copy has no in-file explanation of the invariant. Low-priority follow-up: extract a 21-line source-neutral put-or-evict helper beside BoundedRead in http/read.rs.

### F22. `crates/resourcefs-sources/src/atlassian/jira/transport.rs:90`

On a 304, a generation mismatch for atlassian.jira.<site> returns SourceUnavailable ("Jira read was invalidated during cache revalidation") instead of refetching without the validator. Latent only: nothing bumps that namespace today (the sole bumper cache_remove_namespace is called only from github/mutation.rs:762/1123 with GITHUB_CACHE_NAMESPACE), the code is moved verbatim from base, and it mirrors GitHub's identical fail-closed 'retry the read' handling at github/mod.rs:299-309. Not a defect of this PR; recorded for completeness.

**Failure scenario.** Requires a future Jira mutation path to call cache_remove_namespace("atlassian.jira.<site>") between fetch_cached's generation snapshot and the 304 response; the read would then fail and the caller must retry, exactly as GitHub reads do after a concurrent mutation. No such path exists in the PR head.

## CI workflow and budgets

### F6. `.github/workflows/ci.yml:49`

The new 'Production-scale release budgets' step builds only three resourcefs-core test binaries, so nine other `if !cfg!(debug_assertions)` timing assertions are skipped in the debug Tests step and never compiled in release: resourcefs-core/tests/discovery_engine_contract.rs:618 (catalog_target_validation_production_budget), selector_golden_contract.rs:208 (stress_selection_budget), resourcefs-sources/tests/session_storage_contract.rs:320 and :460, resourcefs-sources/tests/filesystem_adapter_contract.rs:543, resourcefs-sources/src/catalog.rs:555, resourcefs-mcp/src/render.rs:750/1298/1337. The `budget` filter also selects mutation_engine_contract::operation_journal_budget, which is #[ignore]d (line 585) and is reported ignored rather than run; and the same PR deletes ci_workflow_mirrors_local_gates, so nothing asserts the step's binary list.

**Failure scenario.** A commit regresses select_utf8 (256 MiB narrow selection takes 30 s), NamespaceCatalog::source_document (2 s at the 4 MiB ceiling), session heartbeat, or MCP page rendering: the debug step skips every timing assert and the release step never builds those binaries, so CI is green on all three OSes while the source asserts 10 s / 250 ms / 100 ms / 25 ms budgets that are violated. Minimal fix: add `--test discovery_engine_contract --test selector_golden_contract`; the sources/mcp gates need `-p` additions or a single mechanism (e.g. `#[cfg_attr(debug_assertions, ignore)]` + `cargo test --release --workspace`).

### F7. `.github/workflows/ci.yml:49`

The 1 ms `scratch_name_enumeration_fits_its_budget` assertion the release step now carries onto macos-latest/windows-latest already flaked within this PR: in debug on macOS it passed in run 34058197760 and FAILED at 1.500375 ms in run 34058944306 on identical test and library code (3c1634e touched only ci.yml and two other test files); the author's response was to gate it to release and add it to this step. Release-mode margin is only ~2.5-4x on macOS (local release 67-95 us with 2.8x spikes; debug 276 us), and the step has never executed on macOS/Windows (skipped behind Tests failures in all 6 runs). The 25 ms inline_artifact budget has >20x margin and is not a realistic flake source.

**Failure scenario.** Once the unrelated resourcefs-mcp failures are fixed and the step executes on macos-latest/windows-latest, a scheduler preemption or ~3x latency spike inside the ~0.3 ms enumeration window pushes elapsed past 1 ms and the platform job fails with no code change, exactly as already observed at 1.50 ms. Mitigation: run wall-clock budgets on one platform (`if: matrix.os == 'ubuntu-latest'`) or use the repo's scaled-threshold convention (mutation_engine_contract.rs:587) instead of a hard 1 ms bound on shared runners.

### F8. `.github/workflows/ci.yml:46`

`--no-fail-fast` (added in f50c0b3) exposes pre-existing macOS/Windows failures for the first time — 92 macOS tests (TLS fixture `https://tls.invalid` failures across 10 resourcefs-sources binaries incl. the moved Jira/BoundedRead contract tests, atlassian_fixture_operator_contract, 3 stdio https tests) and 2 Windows tests (CRLF line-ending byte comparisons in architecture_contract.rs:391 and schema_contract.rs:32; repo has no .gitattributes) — none caused by the diff (earlier runs stopped at the first failing binary and main died at Lints). Because 'Production-scale release budgets' and 'Dependency vetting' are sequenced after Tests with no `if: always()`, both are SKIPPED on the failing platforms, and no branch protection exists (branches/main/protection -> 404; PR mergeStateStatus UNSTABLE, mergeable MERGEABLE), so the new header claim 'run on each supported platform ... Hosted execution is the workflow oracle' is unenforced.

**Failure scenario.** PR #1 can merge green-on-Ubuntu with 2/3 platform jobs red; release budgets and `cargo deny check` are never observed on macOS/Windows; the rfs-h212 refactor's own Jira transport and BoundedRead contract tests have never passed on macOS, so a regression in the moved retry/deadline/cache code would be invisible there. Fix: `if: always()` (or `!cancelled()`) on the budget and vetting steps, required status checks on main, and a `.gitattributes` (`* text=auto eol=lf`) for the Windows byte-comparison tests.

### F9. `crates/resourcefs-mcp/tests/supply_chain_contract.rs:213`

The PR deletes ci_workflow_mirrors_local_gates and GATE_COMMANDS (base lines 213-256), the only in-repo fence that CI runs the documented gate commands, declares push/pull_request triggers and contains no tabs, with no replacement test, no doc substitute (AGENTS.md/DESIGN.md/docs never list the gate commands) and no spec/design/evidence record; .rfs-q12y/spec.md:51, design.md:61, plan.md:74-93 and the rfs-q12y closing note in .rivets/issues.jsonl:62 still cite it as the C5 fence. The stated rationale ('hosted runs are the oracle') covers job execution, not gate weakening. Separately, .rfs-h212/plan.md:65 assigns module_shape.py's CI hook to this slice and head ci.yml:24-52 has none.

**Failure scenario.** Edit ci.yml:43 to `cargo clippy --workspace --all-targets --features test-support -- -D warnings` (the spec's own named C5 mutation), drop `cargo deny check` (ci.yml:52), or remove the `pull_request:` trigger: `cargo test --workspace --all-features` stays green and hosted CI passes, now with a weaker gate than maintainers run locally. Deeper fix: one script/xtask that both CI and maintainers invoke, so there is nothing to mirror.

### F10. `crates/resourcefs-core/tests/local_quota_contract.rs:270`

Three previously unconditional wall-clock budgets (1 ms here; 5 s at read_engine_contract.rs:812 and mutation_engine_contract.rs:1249) are now wrapped in `if !cfg!(debug_assertions)`, which is false under the documented local gate `cargo test --workspace --all-features` (.rfs-h212/plan.md:122) and CI's Tests step, so they assert nothing there; sole enforcement is the ci.yml release step whose `budget` substring filter exits 0 on zero matches. The gate keys off the build profile rather than an explicit opt-in: no [profile.*] override exists in-repo, so CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true or a config.toml `[profile.release] debug-assertions = true` makes the release command skip every budget and stay green, while `[profile.dev] debug-assertions = false` enforces 1 ms / 5 s against opt-level-0 code. The repo's other convention (scaled thresholds under debug: mutation_engine_contract.rs:587, jira_wire_contract.rs:308, jira_render_contract.rs:229) does enforce under the documented gate, so two conventions now coexist.

**Failure scenario.** 1) A regression makes scratch_names O(n^2) or re-copies the 64 MiB buffer: the documented gate and CI Tests step are green because the assert bodies are dead code. 2) Rename exact_limit_edit_budget to exact_limit_edit_timing: the release filter matches 4 tests instead of 5, libtest exits 0, enforcement drops silently. 3) `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo test --release ... budget`: 5 passed, no timing checked. Fix: `#[cfg_attr(debug_assertions, ignore = "release-only budget")]` (visible as ignored, fails loudly if forgotten) or the scaled-threshold convention.

## Supply-chain gate tests

### F5. `crates/resourcefs-mcp/tests/supply_chain_contract.rs:230`

With `--config` removed, license_gate_rejects_copyleft passes even when no deny.toml is loaded: cargo-deny (0.19 verified) silently falls back to a built-in config whose empty allow-list emits error/`rejected` for EVERY crate (MIT root included) plus a `log` WARN "unable to find a config path, falling back to default config" that assert_rejected_by ignores, so the test no longer proves the committed allow-list is what rejects GPL-3.0 (the exact 'passes for the wrong reason' mode the old design guarded against). source_gate_rejects_git is NOT affected (default unknown-git is warn -> exit 0 + severity warning). The doc comment at lines 244-245 claiming a missing config exits non-zero is false.

**Failure scenario.** Experiment without deny.toml: exit 4, diagnostics `rejected` for both copyleft-dep and the MIT license-fixture-root -> assert_rejected_by("rejected") passes. Any future staging breakage (filename typo, cargo-deny lookup-dir change, a dropped write call) leaves the test green with the fallback warning in stderr. Fix: fail if any log entry contains "falling back to default config" and assert exactly one `rejected` diagnostic whose crate is copyleft-dep (i.e. the MIT root is accepted).

### F11. `crates/resourcefs-mcp/tests/supply_chain_contract.rs:257`

assert_rejected_by JSON-parses every stderr line with `.expect("cargo-deny JSON output")`, so any non-JSON line emitted by the cargo proxy (not cargo-deny) turns the diagnosable gate assertion (which prints stdout+stderr) into an opaque serde panic that discards them. Under a working cargo-deny stderr is entirely JSON, so CI is sound; the base substring check over combined output never panicked. The ci.yml install step `taiki-e/install-action@cargo-deny` is unpinned, so the diagnostic codes `rejected`/`source-not-allowed` the test now depends on can drift upstream (pre-existing exposure in a different form).

**Failure scenario.** Reproduced: a legacy ~/.cargo/config (no .toml) makes cargo print a 3-line `warning: ... deprecated in favor of config.toml` block before dispatching to cargo-deny, which then emits correct JSON and exits 4/8; `!success` passes, `serde_json::from_str` fails on line 1, and the test panics with `cargo-deny JSON output: Error("expected value", line: 1, column: 1)` on a machine where the gate works. cargo-deny not installed (`error: no such command: deny`, exit 101) panics the same way and hides the install hint. Fix: `.filter_map(|line| serde_json::from_str::<Value>(line).ok())` so the final assertion with full stderr remains the failure path.

## macOS platform

### F16. `crates/resourcefs-sources/tests/atlassian_fixture_operator_contract.rs:102`

The PR's macOS-portability change to this constructor (canonical TMPDIR parent, 'Normal state parents must not inherit a symlink from the host's TMPDIR') cannot make the file pass on macos-latest: the harness inherits the runner PATH (lines 144/158) and scripts/atlassian-fixture-bootstrap.sh:45 uses `declare -A`, which macOS's bash 3.2 rejects (`declare: -A: invalid option`), so 20/22 tests fail identically in both latest runs (panics at :328/:343) regardless of the TMPDIR fix, while the file's `#![cfg(unix)]` gate (line 1) admits macOS.

**Failure scenario.** Every macOS CI run stays red on this binary; the symlink-ancestor fix the PR made here is never exercised on the platform it targets. Fix: gate the file on target_os = "linux" or a bash>=4 precondition (skip with a message), add a `brew install bash`/PATH step for macOS, or rewrite the script without associative arrays.

## Test quality

### F13. `crates/resourcefs-core/tests/path_reference_contract.rs:84`

The Absolute arm of the golden test switched from exact string equality to Path equality without need (Absolute values are the literal decoded input, reference.rs:1485-1487, so to_string_lossy() already equals the corpus value on every host). On Windows Path::eq treats corpus rows :20 `\\server\share` and :47 `\\server\share\` as equal (non-verbatim UNC prefixes get an implicit RootDir, so both decompose to [Prefix(UNC), RootDir]) and rows :16/:17/:19 no longer pin `\` vs `/` body separators or drive-letter case; the `\\?\UNC` rows (:22/:48) are unaffected. Only Relative/Canonical genuinely needed Path comparison (WorkspacePath renders with backslashes on Windows). The two identical Relative|Canonical and Absolute arms also duplicate 7 lines and `address_observation`'s `value` String is now computed and discarded for three of four variants.

**Failure scenario.** A later cfg(windows)-gated change to the Absolute branch (e.g. rebuilding the PathBuf from components, which turns `\\server\share` into `\\server\share\`, or `/`<->`\` normalization) passes golden_workspace_references on windows-latest, and the POSIX legs never execute the gated code, so CI stays green while spelling preservation is broken on Windows. Fix: keep the base `value.as_str()` string assertion for Absolute (and FileUri) and use Path comparison only for Relative/Canonical, which also collapses the duplicate arms.

### F14. `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs:740`

Drop for McpProcess now runs eprintln! unconditionally on the thread::panicking() path (line 740, plus the stderr dump at 759-760); eprintln! panics on a failed stderr write (EPIPE/ENOSPC/EAGAIN, not EBADF), and a panic in a destructor during unwinding aborts the whole test binary — demonstrated with a mirror harness: exit 134/SIGABRT with later tests never reported, versus exit 101 with full results for the base-shaped Drop. Under libtest's default capture (what CI runs) the write goes to the in-memory buffer and cannot fail, and the repo's only --nocapture path (`cargo live`, filter `live_`) runs no test in this binary, so the trigger is narrow. The same Drop also evaluates thread::panicking() twice around the kill and adds the third copy of the stderr take()+read_to_string drain in this file (finish 579-585, inline 2942-2949) plus a fourth in cli_contract.rs:1031-1037 whose ProfileMcpProcess Drop (1046-1053) got none of the new diagnostics.

**Failure scenario.** A developer runs the binary with --nocapture (or RUST_TEST_NOCAPTURE=1) while fd 2 is a pipe whose reader exited, /dev/full, or an O_NONBLOCK pty, and any test fails: the Drop's eprintln! gets EPIPE/ENOSPC/EAGAIN, panics while already unwinding, and the process aborts, losing results for every test not yet run. Fix: `let _ = writeln!(io::stderr(), ...)` for diagnostics emitted from a panicking context, one panicking-gated block after the kill (`match &observed_status`), and a shared `fn take_stderr(&mut self) -> io::Result<String>` (or a Child-level helper in tests/support/) used by both Drop impls and both finish methods.

### F15. `crates/resourcefs-mcp/tests/stdio_mcp_contract.rs:476`

The panic-time Drop diagnostics this PR added were blind to the very failure they were added for: in the macOS log every https failure prints `resourcefs child status before cleanup: Ok(None)` and an empty `resourcefs child stderr after failure:` because the server reports the HTTPS fetch failure inside the tool result, not on stderr. call_read_arguments (476-486) only asserts the JSON-RPC-level `error` is absent, so a tool-level `ok:false` result is returned silently, and the assertions at :3448 (`structuredContent.content` -> `left: Null`), :3630 (keys `[contractVersion, error, ok, requestedPath]`) and :3707 (`bounded` -> Null) each index one field and never print the `error` object the log proves is present.

**Failure scenario.** A separate diagnostics branch (rfs-ci-diagnostics) had to be created to learn the cause of the macOS failures. Asserting `structuredContent["ok"] == true` with the full structuredContent in the panic message (or printing `error` in call_read when ok is false) would have shown the root cause in the same run, making the new Drop diagnostics unnecessary for this class of failure.

### F18. `crates/resourcefs-core/src/reference.rs:1877`

The Windows fix replaced hermetic `file:///...` literals with `Url::from_directory_path/from_file_path(std::env::temp_dir().join(..)).expect(..)` inlined at nine sites in four files (reference.rs:1877,1880; path_reference_contract.rs:239,247,266,439,561; version_tag_contract.rs:108; resourcefs-mcp/src/render.rs:712) in two spellings with two expect strings and no shared helper (a similar private helper already exists in resourcefs-sources/src/catalog.rs:245, and resourcefs-core's test-support feature is the natural home). The tests now depend on TMPDIR being absolute: a relative or empty TMPDIR makes Url::from_*_path return Err(()) and the .expect panics in setup before the real assertion (reproduced: `TMPDIR=.tmp cargo test -p resourcefs-core` -> duplicate_root_id_is_rejected panics `directory file URI: ()`). parse_file_uri's cfg-split (reference.rs:1550 vs 1555) means the next author's `file:///workspace/x` literal is green on ubuntu/macOS and fails only on the windows-latest leg.

**Failure scenario.** Developer or sandbox shell exports TMPDIR=.tmp (or empty) and runs the core tests: seven tests panic at setup with 'directory file URI: ()' / 'absolute local file path: ()', masking whether the parsing logic under test is correct; and the nine copies drift independently on the next change. Fix: one test-support helper (`temp_directory_uri(name)` / `temp_file_uri(name)`) using std::path::absolute (MSRV 1.88) or a cfg-gated hermetic literal (`file:///C:/workspace/{name}/` on Windows), called from all nine sites.

### F20. `crates/resourcefs-core/tests/mutation_engine_contract.rs:1244`

Test-only: `assert_eq!(adapter.text_content().await, expected)` deep-clones the ~64 MiB committed String out of the tokio Mutex (text_content, lines 132-138: `content.clone()`) and memcmps it; on mismatch assert_eq! Debug-formats both 64 MiB operands into a ~128 MiB panic message. The receipt-tag assertion on lines 1245-1248 already verifies content identity by hash (Replace commit derives the tag from content, 216-219). The comparison runs after `elapsed` is captured (line 1242), so the 5 s budget is not distorted; `adapter.clone()` at 1229 is an Arc clone.

**Failure scenario.** Any regression in the exact-limit edit path makes the test panic with two 64 MiB escaped strings, which libtest buffers in memory and CI log capture truncates or is swamped by, hiding the first differing byte; every passing run adds a transient 64 MiB allocation + memcpy + memcmp. Fix: compare length plus first-mismatch offset with a custom message, or a tag/length computed under the lock.
