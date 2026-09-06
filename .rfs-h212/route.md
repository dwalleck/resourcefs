# Route: rfs-h212

Change: Add bounded read-only Jira project and issue browsing as the prerequisite for enhanced native JQL Query Resources.
Date: 2026-09-06

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | Direct issue reads are covered by `.rfs-pm0y/evidence.md`; fixture project search is covered by `.rfs-bcym/evidence.md`. Neither establishes every production project-browse response field, effective offset maximum, continuation authority, or supported site/project issue enumeration contract. `docs/research/jira-cloud-contracts.md` identifies response-dependent pagination and unverified enhanced-search cursor behavior. These premises require focused current evidence. | yes |
| 2 | Structural boundary | `crates/resourcefs-core/src/reference.rs` lacks project identifiers, project aliases, fixed Jira collection variants, and Jira page selectors. Adapter work touches `atlassian/jira.rs`, native wire decoding, compact rendering, and exhaustive identity/discovery dispatch. Existing source-neutral transport/recovery engines should be reused. | yes |
| 3 | Production-scale risk | One logical page may hold 1,000 records over ten bounded requests, with response-size, sorting, retry/deadline, caching, and artifact-recovery constraints. Stable decimal IDs must not narrow into machine integers. | yes |
| 4 | Explicit behavior | `rfs-h212` plus closed decisions `rfs-lcuh`, `rfs-qsyk`, `rfs-lbps`, and `rfs-8jnk` specify the behavior triples below. Requester explicitly selected “Implement h212 first”; this authorizes prerequisite scope, not implementation-design approval. | yes |

Unknown tests: none.

### Given/when/then behavior contract

1. Given a configured Jira Site Mount, when reading project collections, stable project Resources, or project-key aliases, then validate project identity and tenant authority and publish only stable-ID canonical references. Aliases remain read-only and do not become cache, search, or continuation identity.
2. Given site/project browse collections, when rendering a logical page, then publish compact deterministic Markdown rows linking canonical projects/issues. Project rows carry display key, name, and optional type/status; issue rows carry display key, summary, and optional status. Missing optional metadata remains absent.
3. Given a fixed browse page, when ordering its records, then compare stable decimal IDs numerically without narrowing to machine integers; do not substitute mutable keys or names for identity.
4. Given a collection larger than one upstream response, when reading a logical page, then request 100 items and stop at 1,000 records, ten requests, or authoritative termination; smaller authoritative maxima win.
5. Given a continuation, when following it, then validate Site Mount, endpoint family, parent collection, and returned membership. Use the settled typed offset/cursor vocabulary, never raw upstream URLs or session-hidden paging state. Preserve artifact Recovery References and source continuation according to ADR-0006.
6. Given empty, exact-boundary, smaller-maximum, malformed, or cross-parent pages, when reading or searching the collection, then return explicit empty output or the stable appropriate error, fail corrupt logical pages atomically, and never silently discard bytes or records.
7. Given `rfs_search` over a fixed browse Resource, then reuse regex/PCRE2 over rendered content. Atlassian glob and mutation remain unsupported. Public native JQL Query Resources remain owned by `rfs-nae2`; Server Profile/catalog publication remains owned by `rfs-ue44`.

## Selected route

Empirical — current native browsing and continuation premises need independent proof before cross-module design.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior is settled by the issue and closed domain decisions |
| evidence.md, probe.* | prove-it-prototype | required — endpoint and continuation premises remain unverified |
| design.md | falsifiable-design | required — typed addresses, placement, authority, bounds, and page ownership |
| plan.md | budgeted-plan | required — independently green atomic slices |

Oracle checkpoint in `checkpointed-build`: required — Empirical route.

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build.

## Scope and ownership

Requester selected “Implement h212 first” on 2026-09-06. `rfs-h212` is claimed on branch `rfs-h212`; its prerequisite `rfs-pm0y` is closed. `rfs-nae2` remains claimed and depends on completion of this issue. Shared browsing foundations are owned here; native caller-authored JQL remains the next issue. No production code changed during routing. The discovered upstream is `origin/main`.

## Terminal criterion

Empirical — every empirical premise is PASS, every downstream artifact satisfies its owning stage's completion criterion, and checkpointed-build records no FAIL. Not complete: empirical proof, design approval, implementation, and verification remain outstanding.
