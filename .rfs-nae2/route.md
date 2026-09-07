# Route: rfs-nae2

Change: Expose enhanced native JQL as bounded, site-scoped, read-only Atlassian Query Resources.
Date: 2026-09-06

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | Retain `.rfs-h212/evidence.md` P3 for enhanced GET identity/token/terminal observations and `.rfs-bcym/evidence.md` corrected C12 for successful read-only POST execution. Neither records an independent ordered POST traversal or malformed-native-JQL POST diagnostic envelope. Those two POST-specific comparisons remain unverified; published request/response and shared substrate facts are retained rather than reprobed. | yes |
| 2 | Structural boundary | Merged browsing provides Jira grammar, typed source cursors, compact issue decoding/rendering, fixed collection assembly, and bounded GET transport. Native query identity/admission, native-order assembly, uncached query-specific status handling, and explicit read-only POST replay/deadline semantics require cross-module placement. Existing browsing owners remain authoritative; do not duplicate them or widen mutation retry eligibility. | yes |
| 3 | Production-scale risk | Upstream pages, opaque continuation state, response bodies, and retained recovery content must remain bounded at 100 requested items, 1,000 records, and ten requests per logical page. Atomic failure, retry/deadline behavior, and artifact/source continuation coexistence require explicit stress and oracle checks. | yes |
| 4 | Explicit behavior | The issue and closed query/continuation decision `rfs-qsyk` specify the observable behavior below; `rfs-lbps` fixes read-only POST retry and no-cache policy. The requester chose browsing first, and that prerequisite is now merged and closed. | yes |

Unknown tests: none.

### Given/when/then behavior contract

1. Given a configured Jira Site Mount and a nonempty canonical percent-encoded UTF-8 JQL segment, when `jira://<site>/search/<segment>` resolves, then execute that exact native query only within the owning Site Mount; malformed references fail `invalid_reference` before query execution.
2. Given successful enhanced POST-search pages, when producing a Query Resource, then preserve upstream row order and opaque typed `nextPageToken` continuation, stopping only on validated authoritative terminal state.
3. Given returned query rows, when publishing navigation, then validate returned site/object authority and emit only canonical stable-ID issue Resource references.
4. Given a native query rejection with HTTP 400, when rendering an operational error, then return `invalid_pattern` with bounded structured diagnostics; keep reference-encoding errors in `invalid_reference`.
5. Given large or paginated results, when producing one logical page, then use 100-item requests and stop at 1,000 records, ten requests, or authoritative termination. Fail malformed pages atomically. Preserve Recovery References and source continuation according to ADR-0006 rather than discarding omitted content or exposing an unbound cursor.
6. Given a Query Resource used through `rfs_search`, when matching content, then apply existing regex/PCRE2 semantics to rendered query content, not a ResourceFS reinterpretation of JQL. Atlassian `rfs_glob` remains explicitly unsupported.

## Selected route

Empirical — query wire/terminal/error premises require independent evidence, and the change also crosses address, adapter, and pagination boundaries.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — requested behavior is explicit in the issue and existing domain decisions |
| evidence.md, probe.* | prove-it-prototype | PASS — P1 retained; fresh ordered POST and rejection comparisons P2/P3 passed; owned fixtures cleaned |
| design.md | falsifiable-design | approved — requester selected "Approve design" on 2026-09-07; no risk acceptances |
| plan.md | budgeted-plan | recorded — two independently verifiable increments with per-slice checkpoints |

Oracle checkpoint in `checkpointed-build`: required — Empirical route.

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build.

## Prerequisite and ownership boundary

- Claimed `rfs-nae2` for Daryl Walleck; resumed in isolated branch `rfs-nae2-query` from discovered upstream `origin/main` at `fad4cf2c11ec27f4272355c9e741badc9be0920e`.
- `rfs-h212` is closed on merged main. PR #3 merged as `47abb6f67f0e2e2b18346d21ace15ccab20830e6`; PR #4 merged as `fad4cf2c11ec27f4272355c9e741badc9be0920e`, after both exact-head native matrices passed.
- The blocking dependency remains recorded and is satisfied, not removed or absorbed. Reuse its grammar/cursor/wire/render/transport and HTTP/regex/recovery seams. The original dirty parent checkout remains untouched.
- Query execution is not implemented yet. Empirical evidence and design approval are complete; S1 HTTP and shape writers are working in separate isolated worktrees. Main supplies integration and every checkpoint gate.

## Terminal criterion

Empirical — every empirical premise passes the prove-it-prototype gate, every downstream artifact satisfies its owning stage's completion criterion, and checkpointed-build records no FAIL. Evidence and approval are complete; implementation, verification and delivery remain outstanding.
