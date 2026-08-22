# Route: rfs-r9m6

Change: Operate ResourceFS from one strict Server Profile
Date: 2026-08-20

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The change depends on subprocess behavior for direct argv execution, a minimal environment, bounded stdout/stderr, timeout, and cancellation. `crates/resourcefs-mcp/src/cli.rs` currently constructs only CLI roots and contains no profile or subprocess implementation; repository search finds no `std::process::Command`, `tokio::process::Command`, `env_clear`, or profile probe covering these premises. The required cross-platform cleanup and bounded-I/O behavior is therefore unverified. | yes |
| 2 | Structural boundary | `crates/resourcefs-mcp/src/cli.rs` exposes only `serve --root`; adding `--config`, `check`, and `schema` changes the public CLI. A strict versioned JSON schema and profile-to-source composition add a new configuration boundary between `resourcefs-mcp` and `resourcefs-sources`. `LaunchRootSource::Profile` exists in `crates/resourcefs-sources/src/filesystem.rs`, but has no caller. | yes |
| 3 | Production-scale risk | Helper and configured-command execution introduces concurrent child processes, potentially unbounded output, and cancellation/timeout cleanup. The request explicitly requires hard time and output caps, so memory, process-lifetime, and concurrency budgets require stress fixtures and checkpoint evidence. | yes |
| 4 | Explicit behavior | The issue and `DESIGN.md:225-251` specify strict JSON, one profile, the three CLI commands, root precedence, distinct grants, referenced secrets, direct argv, minimal environments, caps, redaction, and no telemetry. They do not define the exact version-1 JSON shape, supported source-definition fields, secret-reference grammar, CLI stdout/stderr/exit contracts, probe-mode syntax, schema output format, or cancellation result. Those observable decisions require interrogation. | no |

Unknown tests: none

## Selected route

Empirical — direct-argv helper isolation, bounded I/O, timeout, and cancellation depend on unverified operating-system/runtime behavior; the public profile and CLI contract is also unresolved and structural.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — exact profile and CLI behavior is unresolved (T4 no) |
| evidence.md, probe.* | prove-it-prototype | required — Empirical route (T1 yes) |
| design.md | falsifiable-design | required — Empirical route |
| plan.md | budgeted-plan | required — Empirical route |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

interrogated-spec → prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records PASS for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no FAIL.
