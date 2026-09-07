# Route: rfs-nae2

Change: Expose enhanced native JQL as bounded, site-scoped, read-only Atlassian Query Resources.
Date: 2026-09-06

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | `docs/research/jira-cloud-contracts.md` documents enhanced POST search and opaque tokens but explicitly leaves terminal token shapes and tenant cursor behavior probe-backed. `.rfs-pm0y/evidence.md` covers direct issue reads, not enhanced-query response/error decoding. `.rfs-bcym/evidence.md` proves fixture search execution, not the complete native-query ordering/error/continuation contract. Current applicable evidence does not discharge every query premise. | yes |
| 2 | Structural boundary | Existing Jira address dispatch supports issue Aggregates, Fields, and key aliases. Query addresses, typed cursor identity, collection rendering, and source dispatch require cross-module placement. Open prerequisite `rfs-h212` owns the missing Jira fixed-collection/address/pagination work; it must not be silently duplicated or absorbed into this issue. | yes |
| 3 | Production-scale risk | Upstream pages, opaque continuation state, response bodies, and retained recovery content must remain bounded at 100 requested items, 1,000 records, and ten requests per logical page. Atomic failure, retry/deadline behavior, and artifact/source continuation coexistence require explicit stress and oracle checks. | yes |
| 4 | Explicit behavior | The issue and closed query/continuation decision `rfs-qsyk` specify the observable behavior below. The unresolved prerequisite work order is a scope/architecture decision, not permission to weaken these requirements. | yes |

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
| evidence.md, probe.* | prove-it-prototype | required — enhanced-query premises remain unverified |
| design.md | falsifiable-design | required — placement, authority, continuation, and scale decisions require approval |
| plan.md | budgeted-plan | required — independently green implementation slices |

Oracle checkpoint in `checkpointed-build`: required — Empirical route.

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build.

## Prerequisite and ownership boundary

- Claimed `rfs-nae2` for Daryl Walleck on branch `rfs-nae2`; discovered upstream is `origin/main` at routing time.
- `rfs-h212` remains open and remains an explicit blocking dependency. Existing HTTP transport, issue identity, regex search, and ADR-0006 recovery can be reused; Jira fixed-collection/address/page seams are not yet implemented.
- No dependency was removed, prerequisite implementation was absorbed, or production source code was changed.
- Before implementation, either the prerequisite lands or the requester explicitly approves a revised work order and ownership of shared seams. Design approval remains required afterward.

## Terminal criterion

Empirical — every empirical premise passes the prove-it-prototype gate, every downstream artifact satisfies its owning stage's completion criterion, and checkpointed-build records no FAIL. Not complete: prerequisite work-order decision and all downstream gates remain outstanding.
