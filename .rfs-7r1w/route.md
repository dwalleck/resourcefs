# Route: rfs-7r1w

Change: Resolve a credential command's relative `PATH` entries against the configuration base, so `rfs check` and `rfs serve` agree from any working directory
Date: 2026-08-28

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | **No unverified premise.** Both halves of the mismatch and the behavior the fix depends on are covered by current repository evidence, read today and reproduced today at the product surface with `target/debug/resourcefs` against a temporary profile whose credential helper touches a marker file (`command-bin/credential-helper`, `PATH` literal `command-bin`). (a) The checker is cwd-independent: `profile/check.rs::check_executable` splits `PATH` and joins each non-absolute entry onto `self.base` before `is_executable_in`. Run A (cwd = profile dir) and run B (cwd = repo root) both returned `{"ok":true,…,"state":"notProbed"}`. (b) The executor is cwd-dependent: `process/mod.rs::resolve_environment` inserts a declared `PATH` literal verbatim into the child environment and `build_command` never calls `current_dir`, so the OS resolves a relative entry against the child's inherited cwd. Run C (`check --probe`, cwd = profile dir) ran the helper (marker present, no credential diagnostic); run D (identical command, cwd = repo root) did not (marker absent, `"diagnostic":"source credential could not be resolved"`). (c) The fix's premise — an absolute `PATH` entry resolves from any cwd, and the *child's* declared `PATH` is what the lookup consults — is verified by run E (absolute `PATH`, cwd = repo root): marker present, no credential diagnostic; the helper is in a temporary directory absent from the parent's `PATH`, so the child's value was used. The in-tree unit test `profile::check::tests::an_unmountable_kind_is_refused_before_its_credential_resolves` independently exercises (c) on every `cargo test` run. | no |
| 2 | Structural boundary | **No API, schema, or boundary change, and no open placement decision.** The change is confined to two private free functions in `resourcefs-sources::process` — `resolve_environment` (gains the base parameter it needs) and its `build_command` caller — both `fn` items private to the module. No public type changes: `CommandSpec`, `ChildEnvironment`, and `EnvironmentValue` (exported from `lib.rs:20-22`) are consumed unchanged, and `CommandExecutor::new` already accepts and canonicalizes the base (`process/mod.rs:255-277`). The Server Profile JSON Schema is unchanged — a relative `PATH` literal is already accepted and already documented as resolvable by the checker. Placement is determined by existing structure rather than open: `resolve_program` (`process/mod.rs:597`) already resolves a relative `argv[0]` against `command_base` inside the executor, so the executor is already the owner of "relative means relative to the configuration base"; the checker cannot be the owner because it computes its joined `path` as a local for validation only and never rewrites the value that reaches `CommandSpec`. No existing test records the current behavior — `process_contract.rs:98` asserts only that a `PATH` entry is present, not its value. | no |
| 3 | Production-scale risk | **No scale dimension.** The change adds one bounded string rewrite (split `PATH`, join non-absolute entries onto an already-canonicalized base, rejoin) on the credential-helper spawn path, which runs once per configured source at startup or explicit `check`, under the existing `MAX_LIVE_COMMAND_TREES` admission semaphore. No latency, throughput, memory, concurrency, or data-volume dimension changes; command timeouts and output ceilings are untouched. | no |
| 4 | Explicit behavior | **Fully explicit.** Complete observable behavior contract (the behavior source for implementation, since `spec.md` is `N/A`): **(1)** *Given* a Server Profile whose credential command declares a `PATH` literal containing a non-absolute directory that names an executable helper relative to the profile directory, *when* `rfs check`, `rfs check --probe`, or `rfs serve` resolves that credential, *then* the same helper executable resolves and runs, from any process working directory. **(2)** *Given* that profile, *when* the helper runs, *then* its argv, its declared environment, and the secret taken from its stdout are unchanged from today, and the child's working directory is unchanged (it is not repointed). **(3)** *Given* a credential command whose `PATH` literal contains only absolute directories, *when* it resolves, *then* behavior is byte-identical to today (run E). **(4)** *Given* a credential command that declares no `PATH` entry, *when* it resolves, *then* the parent's `PATH` is inherited by `inherit_automatic` and its non-absolute entries are resolved against the configuration base exactly as a declared value's are — an ambient `PATH` holding only absolute entries, which is the normal case, is therefore unchanged. This matches `check_executable`, which applies its join to whichever `PATH` it validates: the declared value when there is one, otherwise the ambient value it falls back to (`if path.is_none() { path = self.environment_value("PATH") }`). Exempting the inherited value would leave the checker stricter than the executor for exactly the inputs this bug is about. **(5)** *Given* a `PATH` literal whose non-absolute entry names no executable under the profile directory, *when* `rfs check` validates it, *then* it fails with the existing not-executable diagnostic, unchanged. **(6)** *Given* a `PATH` value sourced by `{"kind":"inherit"}`, *when* it resolves, *then* its non-absolute entries are resolved against the configuration base exactly as a literal's are, matching `check_executable`, which applies one join rule to whichever value it validates. Two candidate edge readings are settled by existing repository behavior rather than left open: a non-absolute entry means "relative to the configuration base" (the rule `check_executable` and `resolve_program` already implement), and that reading covers `.` and the POSIX empty entry, which are non-absolute and therefore joined like any other. | yes |

Unknown tests: none

Revision, 2026-08-27: the requester decided that an inherited ambient `PATH` must be resolved against the base like a declared one, rather than forwarded unchanged. That is a scope decision, so all four tests were rerun. T1, T2, and T3 are unaffected and keep their verdicts and evidence: the mechanism and its premises are identical, the change stays inside the same private function, and one additional bounded rewrite of an already-in-memory string carries no scale dimension. T4 keeps its `yes` verdict with behavior point (4) rewritten above; precedence is unchanged, so the route remains Local.

## Selected route

Local — T1 no, T2 no, T3 no, T4 yes. Every premise is covered by current repository evidence reproduced today; the change is confined to private functions in one module whose ownership of relative-path resolution is already established by `resolve_program`; there is no scale dimension; and the observable behavior contract is complete above.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior fully explicit (T4 yes): the complete given/when/then contract is recorded in the T4 evidence above |
| evidence.md, probe.* | prove-it-prototype | N/A — no unverified premise (T1 no): premises (a), (b), and (c) are covered by current repository code paths plus runs A–E recorded in the T1 evidence |
| design.md | falsifiable-design | N/A — Local route: no design gate |
| plan.md | budgeted-plan | N/A — Local route: no plan gate |

Oracle checkpoint in `checkpointed-build`: N/A — Local route: checkpointed-build does not run

## Downstream sequence

none — implement with normal repository fix/TDD

## Terminal criterion

Local — the focused behavioral verification named here records PASS. The verification is the marker-file oracle run from a working directory that is **not** the profile directory, which is what runs C/D distinguished: `cargo test -p resourcefs-sources --test process_contract` and `cargo test -p resourcefs-mcp --lib profile::check` must pass with a **relative** `PATH` literal (the absolute-`PATH` workaround removed per the ticket's third acceptance criterion), plus `cargo test --workspace --all-features` for regression and `cargo clippy --workspace --all-targets --all-features -- -D warnings`.

Result: 2026-08-27 | `cargo test -p resourcefs-sources --test process_contract` (7 passed) + `cargo test -p resourcefs-mcp --lib profile::check` (1 passed) + `cargo test --workspace --all-features` (59 suites, 0 failed) + `cargo clippy --workspace --all-targets --all-features -- -D warnings` (clean) + `cargo fmt --all -- --check` (clean) | PASS

Result after the 2026-08-27 revision: 2026-08-27 | the same commands, with `process_contract` now at 8 passed (the inherited-`PATH` fence added) | PASS

Fence integrity, checked before each result was recorded — a fence that only ever passes proves nothing, so this is part of the criterion rather than an extra. Two named mutations, each reverted afterwards:

- *Forward `PATH` verbatim* (applied to `resolve_search_path`): all four fences fail — `declared_relative_path_resolves_against_the_command_base`, `declared_relative_path_resolves_a_bare_program`, `inherited_relative_path_resolves_against_the_command_base`, and `profile::check::tests::an_unmountable_kind_is_refused_before_its_credential_resolves`.
- *Exempt the inherited `PATH`* (restrict the rewrite to declared entries, the behavior superseded by the 2026-08-27 revision): only `inherited_relative_path_resolves_against_the_command_base` fails, and the declared fences still pass. The mutation discriminates the revised behavior specifically rather than the fix as a whole.

Product-surface confirmation, same profile and same commands as runs C/D/E in the T1 evidence, against the rebuilt binary: run D (relative `PATH`, cwd = repo root) went from marker absent with `"diagnostic":"source credential could not be resolved"` to marker present with no credential diagnostic; runs C and E are unchanged. `scripts/live-smoke.sh` also passes all three rows (S1–S5, L1–L6, H1–H5).
