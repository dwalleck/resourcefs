# CI platform failures on rfs-h212 (2026-09-06)

Assessment of the hosted CI runs on branch `rfs-h212` between 20:02 and 22:30 UTC
on 2026-09-06, plus the `rfs-ci-diagnostics` probe run. Ubuntu is green as of
commit `f50c0b3`. Every remaining failure is macOS- or Windows-specific.

## Why it took six pushes

The workflow ran `cargo test --workspace` without `--no-fail-fast` until
`f50c0b3`. Cargo stops at the first failing test binary, so each run exposed
one failure per platform and hid the rest. Each push cost ten to fifteen
minutes and revealed the next problem.

| Run | Trigger commit | What it exposed |
|---|---|---|
| 34056746167 | `661cbcf` | Clippy 1.98 `byte_char_slices` lint on all three platforms |
| 34058195481 | `5fc30bb` | Debug-build timing budget (Linux, macOS); `file:///one` fixture path (Windows) |
| 34058941625 | `3c1634e` | `/var` vs `/private/var` profile root (macOS); cargo-deny 0.20 `--config` (Linux); same Windows path fixture |
| 34060431140 | `f774e2d` | Same macOS root; cargo-deny colour output (Linux); native separators in path fixtures (Windows) |
| 34061222489 | `b95c2ad` | Session path canonicalisation (macOS); backing-file URL fixture (Windows) |
| 34062996140 | `f50c0b3` | First complete list: 92 macOS failures, 2+ Windows failures |

Run 34062996140 is the first run with the full picture. Everything below is
taken from it and from the probe run 34064080061.

## Open root causes

### 1. macOS: fixture TLS certificates exceed Apple's validity limit (confirmed)

About 70 of the 92 macOS failures are HTTPS-fixture tests in `resourcefs-sources`
(`http_*`, `https_*`, `github_mutation_contract`, `secret_contract`). The 19
failures in `atlassian_fixture_operator_contract` have a different cause; see
cause 6. The probe run shows the underlying TLS error:

```
InvalidCertificate(Other(OtherError("“tls.invalid” certificate is not standards compliant: -67901")))
```

`-67901` is `errSecCertificateValidityPeriodTooLong`. Apple's SSL policy
rejects TLS server certificates valid for more than 825 days (issued after
2019-07-01), and this applies to custom trust anchors as well as preinstalled
roots. The committed leaves are valid for 36,500 days:

| Fixture | Not before | Not after |
|---|---|---|
| `crates/resourcefs-sources/tests/fixtures/tls/match.crt.der` | 2026-08-23 | 2126-07-30 |
| `crates/resourcefs-sources/tests/fixtures/tls/wrong.crt.der` | 2026-08-23 | 2126-07-30 |
| `crates/resourcefs-mcp/tests/fixtures/profile_https/server.crt.der` | 2026-08-29 | 2126-08-05 |

The MCP crate's profile fixture has the same lifetime and will fail the same
way once its tests reach macOS. Key material is otherwise acceptable (P-256
with SHA-256 in sources, RSA-2048 with SHA-256 in MCP; both carry SANs).

Fix: reissue the leaf certificates with a validity of at most 825 days. The
CA may stay long-lived; the limit applies to server leaves. The module doc in
`crates/resourcefs-sources/tests/support/tls.rs` says the fixtures "were
minted for 100 years, so expiry is not a maintenance trap"; that claim must be
replaced with the re-mint date and the openssl invocation. Generating
certificates at test time with `rcgen` would remove the trap, but the same doc
deliberately keeps certificate generation out of the dependency tree.

### 2. macOS: process-group kill returns EPERM (mechanism confirmed, trigger not)

`process_contract::process_admission` fails at the final
"permit released after cleanup" assertion (`process_contract.rs:335`). The
probe run shows why:

```
[DEBUG-rfs-ci-process] terminate: Os { code: 1, kind: PermissionDenied }
[DEBUG-rfs-ci-process] liveness: Os { code: 1, kind: PermissionDenied }
[DEBUG-rfs-ci-process] force:     Os { code: 1, kind: PermissionDenied }
```

`crates/resourcefs-sources/src/process/unix.rs` treats only `ESRCH` as "no
live processes" and "nothing to signal". XNU's `killpg1` filters zombies out
of the group before permission-checking each member and returns `EPERM` when
no signalable member remains, where Linux signals zombies successfully. Any
`EPERM` therefore marks cleanup as failed on macOS and the permit is not
released cleanly.

The exact trigger (which member was in which state when cleanup ran) is not
pinned down; the full macOS job log is needed for that. A second site,
`process_contract.rs:203` (stream boundaries), failed only in the full run and
not in the probe, so it may be timing-related.

### 3. Windows: CRLF checkout breaks byte-for-byte comparisons (strong inference)

Two failures in `resourcefs-mcp`:

- `architecture_contract.rs:391` asserts that `lib.rs` contains a literal
  joined with `\n`.
- `schema_contract.rs:32` compares `resourcefs schema` stdout against
  `include_bytes!("fixtures/server-profile-v1.schema.json")`.

Both pass on Linux, so the content is right. The repository has no
`.gitattributes`, the index stores LF, and hosted Windows runners default to
`core.autocrlf=true`, so the working tree is CRLF and the comparisons fail.

Fix: add a `.gitattributes` with `* text=auto eol=lf` (and renormalise), or
set `core.autocrlf=false` before checkout in the workflow. Other tests that
read checked-in files should be reviewed for the same assumption:
`selector_golden_contract.rs`, `path_reference_contract.rs`,
`profile_contract.rs`, `stdio_mcp_contract.rs`.

### 4. Windows: TLS contract reports UnknownIssuer (unconfirmed)

On the probe branch only, `http_tls_contract` fails on Windows with:

```
InvalidCertificate(UnknownIssuer)
```

The substrate passes the fixture CA as an extra root, and
rustls-platform-verifier 0.6.2 on Windows builds an exclusive-root chain
engine and retries with it only when the first chain build reports a partial
chain. Whether that retry fires for this fixture, and whether the gate run
hits the same failure, could not be confirmed: the job log obtainable through
`gh run view --log-failed` was truncated before the sources crate's results.
Download the raw log for job 101566756311 to settle it.

### 5. Windows: directory reads map to `permission_denied` (tracked)

`filesystem_adapter_contract::maps_missing_directory_and_binary_resources` and
`stdio_mcp_contract::renders_complete_success_and_errors` are already filed as
`rfs-y9af` by the agent working the branch.

### 6. macOS: fixture bootstrap script needs bash 4 (from the code review)

The 19 failures in `atlassian_fixture_operator_contract` (panics at `:328` and
`:343`) are not TLS. The harness inherits the runner's PATH, and
`scripts/atlassian-fixture-bootstrap.sh` uses `declare -A`, which the bash 3.2
shipped with macOS rejects with `declare: -A: invalid option`. The file is
gated on `cfg(unix)`, so macOS runs it. The PR's TMPDIR canonicalisation change
in that file cannot pass on macOS until this is fixed.

Fix: rewrite the script without associative arrays, or gate the test on
`target_os = "linux"`, or install bash 4+ on the macOS runner and put it first
on PATH.

## Side notes

- `main` is red on the Clippy 1.98 `byte_char_slices` lint. The branch fixed
  it in `5fc30bb`; main has not been updated.
- The uncommitted `ci.yml` change adds a Linux-only "Jira module placement"
  step running `.rfs-h212/oracles/module_shape.py`. That is rfs-h212 work,
  not a CI repair.
- Every hosted job still warns that `actions/checkout@v4` targets Node 20.

## Suggested order

1. Add `.gitattributes` (cause 3). One file, fixes a class.
2. Reissue both fixture certificate sets (cause 1). Clears about 70 macOS failures.
   Fix the bootstrap script's bash 4 dependency (cause 6) for the other 19.
3. Handle `EPERM` in `process/unix.rs` (cause 2), ideally by reaping before
   signalling rather than by widening the error mapping.
4. Re-run and read the full raw logs to settle causes 4 and 5.
