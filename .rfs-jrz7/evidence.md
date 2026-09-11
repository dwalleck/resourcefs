# Evidence: rfs-jrz7

Date: 2026-09-09. Checked source and upstream revision: `635ab170ab57542c18272921298d575da2f8b08a`. Canonical directory: `.rfs-jrz7/` in `../resourcefs-wt-rfs-jrz7`. Route: `route.md`. spec.md: N/A — approved ticket behavior is fully recorded in route T4.

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|---|---|---|---|
| P1 | Selected-version GitHub immutable acquisition seam | Do REST 2022-11-28 commit metadata, nonrecursive tree entries and base64 blob bytes for this pinned nested regular file match the local Git object database? | PASS |
| P2 | Proposed github:// segment grammar and selector handling | N/A — feature not yet built: current ResourceAddress has no github:// variant. The proposed once-only decoding/refusal behavior must be demonstrated at design/build, not inherited from PR parsing. | N/A — feature claim |
| P3 | Hard numerical limits, cancellation, forbidden egress, retries, cache generations and exact binary/special-object acceptance | N/A — requirements on the implementation to be built. Existing probes cannot establish its future enforcement. | N/A — design/checkpoint obligations |

## Data

- Source: safe production-shaped existing repository snapshot, `dwalleck/resourcefs` pinned to `635ab170ab57542c18272921298d575da2f8b08a`, already available in the local Git object database.
- Shape: merge commit with two parents, three non-recursive directory levels and regular blob `crates/resourcefs-core/Cargo.toml`; complete sibling tree entries compared at every level.
- Safety: only explicit GET calls through existing gh credential handling to the repository's configured GitHub API; local Git plumbing is read-only. No upstream changes, fork expansion, URL following or tenant access. The probe never prints credentials. This is not a production forbidden-egress observer or adapter acceptance run.
- Approval: request to implement the issue includes applicable read-only public-upstream proof; no new corporate deployment/principal operations.

## Probe

- File: `probe.py`.
- Mechanism: gh REST GET responses decoded in Python; select the pinned tree chain by exact path component and decode the terminal blob's base64.
- Run: `python .rfs-jrz7/probe.py` from `../resourcefs-wt-rfs-jrz7`.
- Result: exit 0; exact retained output in `probe-result.json`.
- Five explicit GETs: commit, root tree, crates tree, resourcefs-core tree, blob. REST pin: `2022-11-28`. This is probe request count, not implementation budget evidence.

## Oracle

- Mechanism: local `git cat-file commit`, `git ls-tree -z` and `git cat-file blob` read the Git object database, independent of provider REST payload construction, provider paths, JSON/base64 decoding and future production HTTP acquisition.
- Run: invoked by `probe.py`, pinned object IDs only. Comparison output records the requested SHA, selected trees/blob and decoded SHA-256. Message comparison ignores final newlines solely to compare native provider text with Git storage; both observed trailing-newline counts are separately recorded (zero in this sample). Commit message is provider metadata, not a claim of raw commit-object bytes.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|---|---|---|---|
| P1 commit | SHA `635ab170...`, tree `c440246484e340dfe1d43fda5dcc31ea6b03a89a`, two parents recorded in JSON, native message | Local commit object tree/parents/message agree; both message trailing-newline counts 0 | PASS |
| P1 root tree | `c440246484e340dfe1d43fda5dcc31ea6b03a89a`, 43 entries, not truncated | Every path/mode/type/SHA agrees with ls-tree, independent of ordering | PASS |
| P1 crates tree | `3970e21fe17460acbbb7e51fbe7ebbce621cd2a2`, 3 entries, not truncated | Every path/mode/type/SHA agrees | PASS |
| P1 core tree | `560584718bd3ad724703d9612a49692dab774646`, 3 entries, not truncated | Every path/mode/type/SHA agrees | PASS |
| P1 blob | `8b57d630ab71fbc6daa17d949b311029362ff5aa`, 1,063 decoded bytes, SHA-256 `dbef110105ca17fba63d1785fce00321b083c30310062abd2218c0154619698b` | Exact byte equality with cat-file; provider size equals decoded length | PASS |

## Validated / learned

- P1: validated prior understanding — the selected-version commit/non-recursive-tree/blob seam provides the pinned nested regular-file identities and exact bytes, independently corroborated by Git plumbing. No Contents/raw-download endpoint is required for this observed chain.
- P2 classification correction: code inspection shows no existing github:// parser to reuse. Core must gain an explicit family arm and segment-aware grammar. PR's decode-whole-body-before-splitting is not the requested source grammar.
- Limits: this probe does not establish binary/symlink/submodule/LFS provider shapes, pathological Unicode trees, decoded boundary enforcement, actual ResourceFS request/deadline/allocation bounds, live adapter or MCP acceptance, cache/integrity overrides or corporate/native acceptance. Those remain design/build falsifiers and live smoke obligations, not inherited PASS results.

## Related issues

- Consulted: rfs-jrz7; rfs-0n97 prerequisite is present on main via merged PR #9 despite stale tracker status. One bounded tracker search (`rivets list --json --limit 200`, filtering immutable commit/exact source/Git tree/commit-path terms) additionally found rfs-e5cv (immutable tree comparisons) and rfs-iktl (PR commit/native-file facts); both describe separate future families, not prerequisites to this singular source path.
- Filed: none — no underlying-system defect found.

## Hand-off

P1 PASS with retained runnable probe and independent comparison; P2/P3 are classified non-premises. No production code or feature tests changed. Hand off to falsifiable-design; exact grammar/schema/placement still require requester approval.
