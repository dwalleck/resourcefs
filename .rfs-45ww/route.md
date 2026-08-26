# Route: rfs-45ww

Change: Browse allowlisted GitHub issues and pull requests as bounded read-only Resources.
Date: 2026-08-25

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The adapter depends on current GitHub REST behavior for issue/PR response shapes, issue-style comments versus review submissions versus inline review comments, diff media, pagination, conditional caching, rate-limit headers, credential handling, and upstream error responses. Current repository evidence proves the source-neutral bounded HTTP transport in `crates/resourcefs-sources/src/http/mod.rs`, but `HttpRequest` currently carries only a URL and no existing `evidence.md` covers GitHub API behavior. These premises require a deterministic probe and independent oracle before design. | yes |
| 2 | Structural boundary | The change adds `issue://` and `pr://` typed address families to `resourcefs-core`, a GitHub Source Adapter and compiled dispatch in `resourcefs-sources`, request-header/status handling in the shared HTTP seam, and live GitHub source construction from the existing Server Profile schema. It therefore changes public parser/configuration APIs and crosses core, source, and MCP launch boundaries. | yes |
| 3 | Production-scale risk | Repository issue/PR listings, conversations, reviews, inline comments, and diffs are paginated and potentially large. Network latency, response volume, pagination bounds, cancellation, caching, and GitHub rate limits are production-scale concerns that require explicit budgets and stress fixtures. | yes |
| 4 | Explicit behavior | Given an explicitly readable repository, when a caller lists, reads, searches, or selects an issue/PR Aggregate or Field Resource, then ResourceFS must use the common bounded read/search/selector/recovery contracts and keep all Resources read-only. Given a repository absent from the readable allowlist, when any `issue://` or `pr://` reference resolves, then access is denied without upstream disclosure. Given a PR, when its conversation comments, review submissions, inline review comments, or diff are addressed, then each remains a distinct projection and stable comment identities are separate Field Resources. Given credential, network, upstream, rate-limit, limit, or cancellation failure, when the adapter returns, then the result is bounded and carries a stable `ResourceError` category. Given deterministic fake-upstream fixtures, when the adapter is exercised directly, then behavior is proven without live mutations. Unresolved decisions: canonical collection/item path spellings for review submissions, inline comments, and diff files; list/search scope and ordering; pagination/cache policy; aggregate rendering; and the exact existing `ErrorCategory` mapping for rate-limit and upstream HTTP statuses. | no |

Unknown tests: none

## Selected route

Empirical — external GitHub API semantics are unverified, and the change also crosses public/module boundaries with network-scale risk and unresolved observable details.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — T4 records unresolved observable behavior |
| evidence.md, probe.* | prove-it-prototype | required — T1 records unverified GitHub API premises |
| design.md | falsifiable-design | required — Empirical route |
| plan.md | budgeted-plan | required — Empirical route |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

interrogated-spec → prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records PASS for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no FAIL.
