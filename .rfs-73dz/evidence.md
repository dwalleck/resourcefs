# Evidence: rfs-73dz

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | Supported desktop filesystems expose a same-directory replacement primitive with atomic name visibility suitable for workspace replacement. | While 128 complete 1 MiB versions replace one path, do native Linux and Windows readers observe only complete old/new bytes without missing/error windows under the selected primitive, and does the same Unix path compile for macOS against a published atomic-rename contract? | PASS |
| P2 | Supported platforms expose an atomic no-clobber same-filesystem rename suitable for `MV`. | Does the native primitive reject an existing destination without changing source or destination and move successfully when the destination is absent on Linux and Windows, while the selected Unix API compiles for macOS against its `RENAME_EXCL` contract? | PASS |
| N1 | Tool schemas, patch grammar, Version Tag prefixes, seen-region union, receipt coverage, grants, error precedence, cancellation, and output behavior. | N/A — requester-approved future behavior in `spec.md`, not existing-system behavior. | N/A — specification owns the behavior |
| N2 | 64 MiB ceilings, allocation budgets, lock contention, and parser fuzz duration. | N/A — performance and scale gates for code not yet built. | N/A — design/checkpoint territory |
| N3 | Permission/newline preservation, linked-path rejection, and failure injection. | N/A — required implementation behavior rather than a claim about an existing subsystem. | N/A — design/checkpoint territory |

## Data

- Source: production-shaped.
- Shape: isolated same-directory regular UTF-8 workspace files with two complete 1 MiB versions, 128 replacements, a continuously opening/reading peer, conflict and absent-destination move cases, copied Unix mode bits, the repository-pinned `rustix = 1.1.4`/`windows-sys = 0.61.2`, native Linux, the project Windows VM, and an `x86_64-apple-darwin` cross-check.
- Safety: the probe creates only temporary Cargo projects and OS temporary files, uploads one throwaway statically linked executable to `C:\Windows\Temp`, removes it after execution, and never reads or mutates repository, profile, workspace, or operator-owned Resource data. Approval: N/A — generated production-shaped temporary data is safe.

## Probe

- File: `probe_atomic_replace.py`
- Mechanism: compiles one standalone Rust executable using `rename`/`rustix::RenameFlags::NOREPLACE` on Unix and `SetFileInformationByHandle(FileRenameInfoEx)` with POSIX semantics, with and without `REPLACE_IF_EXISTS`, on Windows; native reader stress classifies complete versions, partial versions, missing paths, access errors, and sharing errors; conflict fixtures compare both directory entries; the same Unix code is checked for `x86_64-apple-darwin`.
- Run: `./.rfs-73dz/probe_atomic_replace.py`

## Oracle

- File: `oracle_atomic_contracts.py`
- Mechanism: independently downloads and normalizes the Linux and Apple rename manuals plus Microsoft `FILE_RENAME_INFORMATION`, `ReplaceFileW`, and `MoveFileExW` reference contracts, then checks the published atomic replacement, old-handle/new-open, no-clobber, ACL, and explicit cross-volume-copy statements. It shares neither the Rust probe wrappers nor the intended production engine.
- Run: `./.rfs-73dz/oracle_atomic_contracts.py`

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | Linux `rename`: 128 × 1 MiB replacements; 537 old and 556 new complete observations; 0 invalid, missing, access-denied, sharing, or other errors; Unix mode preserved. Windows `FileRenameInfoEx`: 128 × 1 MiB replacements; 22 old and 9 new complete observations; 0 invalid, missing, access-denied, sharing, or other errors. `x86_64-apple-darwin` check passed. | Linux: `replaceKeepsNamePresent=true`. Apple: `replaceKeepsNamePresent=true`. Windows: `replaceOpensNewIdentity=true`, `replaceExistingFlag=true`; published semantics say existing handles remain valid and later opens resolve to the renamed file. | PASS |
| P2 | Linux `RenameFlags::NOREPLACE` and Windows `FileRenameInfoEx` without `REPLACE_IF_EXISTS` both reported `existingDestinationRejected=true`, `sourceUnchangedOnConflict=true`, `destinationUnchangedOnConflict=true`, and `absentDestinationMoved=true`; `rustix::RenameFlags::NOREPLACE` compiled for `x86_64-apple-darwin`. | Linux `RENAME_NOREPLACE=true`; Apple `RENAME_EXCL=true`; Microsoft's `FILE_RENAME_INFORMATION` contract says replacement is opt-in and otherwise an existing destination fails. | PASS |

## Validated / learned

- P1: learned — a first Windows trial using `ReplaceFileW` completed replacements but exposed 136 `not_found` and 28,865 sharing-violation read windows in one 128-iteration run; `MoveFileExW(REPLACE_EXISTING)` then failed with access denied under the same reader. `FileRenameInfoEx` with replace and POSIX flags produced only complete old/new observations with zero reader errors, and Microsoft's independent contract says existing handles remain valid while subsequent opens resolve to the renamed file. The design must use handle-based POSIX rename for Windows atomic visibility; `ReplaceFileW` remains relevant only as evidence that ACL/attribute preservation must be performed separately rather than by choosing a visibility-weaker replacement primitive.
- P2: validated prior understanding — `rustix::RenameFlags::NOREPLACE` on Unix and `FileRenameInfoEx` without `REPLACE_IF_EXISTS` on Windows provide the required conflict/absent-destination behavior without copy/delete fallback; the Apple mapping compiles and its published `RENAME_EXCL` contract agrees.

## Related issues

- Consulted: rfs-jk0d (parent mutation contract); rfs-r9m6 (independent grants); rfs-cgbq (contained canonical workspace identity); rfs-34pz (Path Session and exact Version Tags); rfs-ewh2 (catalog immutability); rfs-5os7 (Kiro public-tool release gate); rfs-60g1 (Session Scratch consumer); rfs-trdn (archive consumer); rfs-c85i (SQLite consumer); rfs-by2z (GitHub consumer); rfs-nb4s (Vault consumer); rfs-cdlp (skill/rule consumer); rfs-xiwg (notebook consumer); rfs-6le8 (binary mutation exclusion). This is the upstream search outcome copied from `spec.md`; no search was repeated.
- Filed: none — both empirical premises passed and no underlying-system defect or deferred work was discovered.
