# Route: rfs-by2z

Change: Add grant-controlled GitHub Field Resource mutation and issue, pull-request, and conversation-comment Creation Targets.
Date: 2026-08-27

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | Production correctness depends on GitHub REST mutation behavior for issue/PR PATCH, issue/comment POST, request/response shapes, status handling, and on distinguishing failures before transmission from outcomes that become unknown after transmission. The current `GithubSource` and `.rfs-45ww/evidence.md` cover GET, pagination, validators, redirects, and real read-only upstream shapes only; `.rfs-45ww/design.md` explicitly defers every POST/PATCH/PUT/DELETE and live mutation to rfs-by2z. No current evidence covers these mutation premises. | yes |
| 2 | Structural boundary | The change makes `operationId` operational in the public `rfs_write` input schema, extends core mutation request/receipt/session state, routes GitHub mutation through `CompiledSources`, and adds mutation implementation to the GitHub Source Adapter. This crosses the MCP, core, and source-adapter module boundaries and changes public mutation behavior. | yes |
| 3 | Production-scale risk | Each requested mutation targets one repository Resource or Creation Target, performs bounded document parsing and a bounded number of HTTP requests, and reuses existing per-Resource serialization and HTTP ceilings. No bulk fan-out, unbounded data volume, or new throughput/memory budget is requested. | no |
| 4 | Explicit behavior | The issue and accepted `DESIGN.md` provide these observable triples: GIVEN an existing GitHub title, body, or stable conversation comment and repository update authority, WHEN replacement supplies its current content-derived Version Tag, THEN ResourceFS updates only that Field Resource; GIVEN a stale/missing Version Tag or missing grant, WHEN replacement is requested, THEN it fails without mutation; GIVEN `comments/new` Markdown, repository create authority, and an operation ID, WHEN creation succeeds, THEN the result names the canonical created `comments/<id>` reference; GIVEN an issue creation document, WHEN it has required title frontmatter and repository create authority, THEN it creates an issue using the remaining Markdown as body; GIVEN a PR creation document, WHEN it has title/head/base, optional draft, and repository create authority, THEN it creates a PR using the remaining Markdown as body; GIVEN one live Path Session and a completed operation ID/content pair, WHEN the same pair repeats, THEN it returns the prior canonical reference without another upstream mutation; GIVEN the same operation ID with different content, WHEN it repeats in that session, THEN it is rejected; GIVEN an upstream outcome becomes unknown after transmission, WHEN handling or considering a retry, THEN ResourceFS does not retry automatically and requires explicit upstream reconciliation; GIVEN another Path Session, WHEN an operation ID repeats, THEN no cross-session deduplication is promised; GIVEN labels, assignees, status, reviews, or inline review comments, WHEN mutation is attempted, THEN it remains unsupported. Unresolved decisions remain: the strict frontmatter grammar and rejection rules; operation-ID syntax/limits and error category; the observable journal state and repeat behavior after an unknown outcome; exact success receipt/version semantics for Creation Targets; and precise error mapping for GitHub mutation responses. | no |

Unknown tests: none

## Selected route

Empirical — required GitHub mutation and transmission-outcome premises are unverified; the public and cross-module behavior also has unresolved observable details.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — T4 has unresolved frontmatter, operation-journal, receipt, and error behavior |
| evidence.md, probe.* | prove-it-prototype | required — Empirical route and T1 verdict |
| design.md | falsifiable-design | required |
| plan.md | budgeted-plan | required |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

interrogated-spec → prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records `PASS` for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no `FAIL`.
