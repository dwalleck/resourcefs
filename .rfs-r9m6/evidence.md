# Evidence: rfs-r9m6

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|---|---|---|---|
| P1 | serde 1.0.219 and schemars 1.0.4 can express the strict tagged profile boundary | For representative closed profile/source types, do deserialization and draft-2020-12 validation agree on valid objects, unknown fields/kinds, wrong-kind fields, missing required fields, and schemaVersion 1? | PASS |
| P2 | Ordinary `Option<T>` distinguishes a missing optional field from explicit JSON null | Do serde derive and schemars reject `"workspace": null` while accepting a missing `workspace`? | PASS — learning recorded |
| P3 | Direct argv plus an explicitly cleared environment preserves literal arguments and excludes ambient variables | On a native POSIX launch, does a bare executable resolve through the supplied PATH while the child sees only explicit variables and literal metacharacters? | PASS |
| P4 | tokio 1.47.1 can drain stdout and stderr concurrently to one byte beyond a ceiling without deadlock or unreaped children | Can both streams independently reach 65,537 observed bytes, stop, and reap within five seconds? | PASS |
| P5 | An owned POSIX process group can terminate a resistant direct child and grandchild within the specified grace/force sequence | After SIGTERM, a one-second grace, and group SIGKILL, are both descendants non-running and their independent heartbeats stopped? | PASS |
| P6 | Native Windows supplies the selected PATH/SystemRoot environment behavior and Job Objects terminate resistant descendants | On Windows 11, does direct execution preserve literal argv, omit an ambient secret, and does Job termination leave zero live child/grandchild processes with stopped heartbeats? | PASS |
| N1 | Profile/source field names, numeric ceilings, reports, and exit codes | These are requester-approved behavior in `spec.md`, not claims about an existing external system. | N/A — specification decision |
| N2 | The 32-child ceiling and profile cardinalities meet production load | These are design/checkpoint stress targets, not existing-system facts. | N/A — checkpoint evidence |

## Data

- Source: production-shaped generated fixtures.
- Shape: closed JSON profiles with the same tagged/optional/required patterns; direct child commands with literal argv and environment sentinels; concurrent stdout/stderr streams crossing the 64 KiB helper boundary; resistant two-generation child trees on native Linux and Windows.
- Safety: every fixture lives under a temporary directory, starts only probe-owned processes, uses no network or repository mutation, and removes its files/processes before returning. The Windows fixture runs inside the dedicated local `resourcefs-win11` libvirt VM.

## Probes

- File: `probe_schema.py`
  - Mechanism: compiles pinned serde/schemars representative Rust types, emits their schema, and records semantic deserialization outcomes.
  - Run: `./.rfs-r9m6/probe_schema.py | jq '{versions, observations}'`
- File: `probe_environment.py`
  - Mechanism: compiles a native Rust parent/helper, resolves the helper by the command-supplied PATH, calls `env_clear`, and records child argv/environment.
  - Run: `./.rfs-r9m6/probe_environment.py`
- File: `probe_bounded_pipes.py`
  - Mechanism: compiles a pinned tokio Rust parent that concurrently reads an endlessly writing child's stdout/stderr to limit+1, then kills and reaps it.
  - Run: `./.rfs-r9m6/probe_bounded_pipes.py`
- File: `probe_posix_tree.py`
  - Mechanism: creates a resistant two-generation process group, applies grace/force group termination, and reads Linux process states.
  - Run: `./.rfs-r9m6/probe_posix_tree.py`
- Files: `probe_windows.ps1`, run through `windows_evidence_runner.py`
  - Mechanism: on native Windows 11 build 26200, starts a PowerShell child with a cleared, explicit environment and literal argv; assigns it to a Win32 Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; permits a resistant grandchild; calls `TerminateJobObject`; and reconciles PIDs plus heartbeat files.
  - Run: `uv run ./.rfs-r9m6/windows_evidence_runner.py`

## Oracles

- `oracle_schema.py`: Python jsonschema 4.25.1 independently evaluates the emitted draft-2020-12 schema against a hand-authored expected matrix; this differs from Rust deserialization and schemars generation. Run: `uv run ./.rfs-r9m6/oracle_schema.py`.
- `oracle_environment.py`: Python subprocess launches native `env` and `printf` directly with a hand-built environment; this differs from Rust `Command::env_clear`. Run: `./.rfs-r9m6/oracle_environment.py`.
- `oracle_bounded_pipes.py`: Python subprocess plus independent reader threads enforce the same stream boundary; this differs from tokio async I/O. Run: `./.rfs-r9m6/oracle_bounded_pipes.py`.
- `oracle_posix_tree.py`: heartbeat-file growth, rather than `/proc` process state, determines whether either descendant continued executing after group termination. Run: `./.rfs-r9m6/oracle_posix_tree.py`.
- `oracle_windows.ps1`: on the same native guest, creates the same resistant child tree without using a Job Object, then independently terminates and reconciles the known child/grandchild PIDs and observes heartbeat stability. `windows_evidence_runner.py` uploads and invokes both scripts over local WinRM without embedding VM credentials.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|---|---|---|---|
| P1 | Minimal and valid HTTPS objects accepted; unknown top-level/source fields, missing `required`, unknown kind, wrong-kind field, extra field, and schemaVersion 2 rejected. | Draft202012Validator produced the same result for all eight rows. | PASS |
| P2 | serde accepted missing `workspace` and explicit `workspace: null`; emitted schema also admitted null. | Draft202012Validator admitted null while the hand table expected rejection. | PASS — both mechanisms agree on actual library behavior; prior model was wrong |
| P3 | Argument was exactly `literal;$HOME&\|<>()`; environment keys were exactly PATH/RFS_LITERAL; parent sentinel absent. | Argument was identical; environment keys were exactly PATH/RFS_LITERAL; parent sentinel absent. | PASS |
| P4 | stdout/stderr each observed 65,537 bytes; over-limit true for both; child reaped in 2 ms. | stdout/stderr each observed 65,537 bytes; no deadlock; child reaped in 34 ms. | PASS |
| P5 | Resistant child/grandchild remained sleeping after grace, then both were absent after group SIGKILL at 1,004 ms. | Both heartbeat files stopped at sizes 54/53 and remained unchanged during the 300 ms independent observation. | PASS |
| P6 | Native Windows `Microsoft Windows NT 10.0.26200.0`: literal argument round-tripped; parent secret absent; configured PATH/SystemRoot/RFS_LITERAL present; child/grandchild absent after Job termination; heartbeat sizes remained 60/7 over 300 ms; 502 ms elapsed. | Same platform and environment observations; explicit PID termination left child/grandchild absent; heartbeat sizes remained 54/7 over 300 ms; 529 ms elapsed. | PASS |

## Validated / learned

- P1: validated prior understanding — pinned serde/schemars tagged closed objects and an independent draft-2020-12 engine agree on the representative strictness matrix outside null handling.
- P2: learning — a derived `Option<T>` deliberately treats missing and explicit null as `None`, and schemars emits that same allowance. The approved null-rejection contract therefore cannot rely on ordinary Option derive.
- P3: validated prior understanding — native Rust direct argv preserves shell metacharacters and `env_clear` excludes ambient secrets while using the supplied PATH for bare executable lookup.
- P4: validated prior understanding — pinned tokio drains both pipes concurrently to limit+1 without deadlock and supports bounded kill/reap.
- P5: validated prior understanding — Linux process groups contain the direct child and grandchild for the selected one-second grace/force cleanup sequence.
- P6: validated prior understanding — native Windows direct argv preserves the configured metacharacters, cleared child environments omit the ambient secret, and a kill-on-close Win32 Job can force-terminate the full resistant descendant tree within the bounded cleanup window.

## Related issues

- Consulted upstream from `spec.md`: rfs-jk0d, rfs-cgbq, rfs-87cv, rfs-34pz, rfs-vl0u, rfs-73dz, rfs-60g1, rfs-9o5v, rfs-vl7z, rfs-g2z9, rfs-btji, rfs-uo2w, rfs-azd3, rfs-45ww, rfs-by2z, rfs-cdlp, rfs-nb4s, rfs-5os7, and rfs-58r1.
- Filed: none.
