# Evidence: rfs-34pz

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | `rmcp` 3.1.3 connection completion is a safe Path Session teardown boundary. | After peer input closes during an in-flight handler, does `RunningService::waiting()` wait for that handler, cancel it, or return while it can still execute? | PASS — probe and independent source oracle agree that `waiting()` can return after its five-second drain timeout while the handler still executes |
| P2 | A cross-process file lease can protect live session directories and become reclaimable after abnormal owner termination on supported desktop filesystems. | Do native Linux and Windows processes observe exclusive-lock contention while the owner lives and acquire after owner death, and does the same `flock` API compile for macOS? | PASS — runtime observations, macOS cross-check, and published/API-source oracle agree |
| N1 | Selector, limit, artifact, quota, identity, and TTL semantics in `spec.md`. | N/A — requester-approved future behavior, not an existing-system premise. | N/A — specification owns the behavior |
| N2 | 64 MiB object, 256 MiB session, and concurrent-admission stress targets. | N/A — future implementation scale/invariant gates, not existing-system behavior. | N/A — design/checkpoint territory |

## Data

- Source: production-shaped.
- Shape: P1 uses the repository's pinned `rmcp` 3.1.3 over an in-memory full-duplex MCP transport with one deliberately in-flight server request at peer EOF. P2 uses isolated temporary session/lease files and separate owner/contender processes through native Linux `flock`, Win32 `LockFileEx` under Wine, and an `x86_64-apple-darwin` cross-check of the `rustix::fs::flock` mapping.
- Safety: probes create only temporary Cargo projects and temporary lease files, contact only the three published oracle documentation URLs, and do not read, mutate, or delete repository or operator-owned cache state. Approval: N/A — production-shaped synthetic data is safe.

## Probe

- Files: `probe_rmcp_disconnect.py`, `probe_session_lease.py`
- Mechanism: P1 compiles a standalone Rust client/server against exactly `rmcp = 3.1.3`, closes the peer while a handler is gated, and timestamps handler, request-cancellation, and `waiting()` events. P2 runs a standalone Rust `flock` owner/contender on Linux, the equivalent C/Win32 `LockFileEx` process pair under Wine, kills each owner, retries acquisition, and cross-checks the Rust `flock` call for macOS.
- Run: `python .rfs-34pz/probe_rmcp_disconnect.py`; `python .rfs-34pz/probe_session_lease.py`

## Oracle

- P1 mechanism: statically inspect the pinned SDK's service loop, EOF branch, drain timeout, cancellation branch, and `RunningService::waiting()` implementation; source control flow cannot share the runtime probe's scheduler/event-observation failure mechanism.
- P1 run: `python .rfs-34pz/oracle.py`
- P2 mechanism: statically compare the exact `rustix`/`windows-sys` mappings with the published Linux `flock(2)`, Apple `flock(2)`, and Windows `LockFileEx` contention/process-termination contracts; documentation/source interpretation does not share the process probe's lock-observation failure mechanism.
- P2 run: `python .rfs-34pz/oracle.py`

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | `quitReason=Closed`; `waitingElapsedMs=5002`; handler not finished before `waiting()` returned; handler completed after return; request context then observed cancelled; request failed `TransportClosed`. | EOF breaks with `Closed`; closed drain is bounded to five seconds; handlers run in detached service tasks; `waiting()` joins only the service-loop handle; therefore it can return before a handler. | PASS |
| P2 | Linux and Windows/Wine both reported contention while the owner lived and acquisition after killed-owner exit; the `rustix::fs::flock` probe cross-compiled for `x86_64-apple-darwin`. | Linux/Apple document nonblocking `flock` contention and descriptor-associated locks; Windows documents immediate exclusive contention and OS unlock after process termination; pinned Rust bindings map the selected APIs. | PASS |

## Validated / learned

- P1: learned — `RunningService::waiting()` is a bounded transport-drain boundary, not a handler-join boundary. Teardown may invalidate the Path Session after `waiting()` returns, but every in-flight handler must observe cancellation/session liveness before resolving or committing state; the design cannot infer that no handler remains.
- P2: validated prior understanding — exclusive file leases reject a concurrent process while live and become acquirable after owner death on native Linux and Windows/Wine; the selected Unix API also compiles for macOS.

## Related issues

- Consulted: `rfs-jk0d` (accepted Path Session, recovery, limits, quotas, and testing contract); `rfs-cgbq` (completed selector/literal and workspace-authority prerequisite); `rfs-vl0u` (downstream artifact search/glob); `rfs-m739` (downstream structural selectors); `rfs-60g1` (downstream Session Scratch reuse); `rfs-ww6w` (downstream Resource Mirror); `rfs-5os7` (release Kiro recovery proof).
- Filed: none.
