# Evidence: rfs-h212

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | Native project read/search responses carry stable identity and compact metadata for canonical browsing. | Which identity, metadata, request-limit, and ordering facts hold? | PASS — schema, independent rendered documentation, and populated live reads agree; native numeric-ID sorting is unavailable. |
| P2 | Project offset pagination can advance from authoritative response metadata rather than assumed request size or immutable totals. | Which offset/max/terminal/next-link facts hold? | PASS — published pagination rules agree with three observed native pages. |
| P3 | Site/project issue browsing has a supported read endpoint with compact identity fields and bounded continuation. | Which non-deprecated endpoint, bounded predicate, selected fields, and continuation state work? | PASS — enhanced GET search, documented Project `IS NOT` support, populated token traversal, and direct-read comparison agree after correcting a false numeric-ID predicate. |
| N1 | ResourceFS grammar, 100/1,000/ten bounds, numeric ordering, error categories, and optional metadata. | N/A — requested behavior fixed by rfs-h212 and closed rfs-lcuh/rfs-qsyk/rfs-lbps. | N/A — specified behavior, not an external premise. |
| N2 | HTTP retry/deadline, existing direct issue identity, regex matching, and artifact/source continuation coexistence. | N/A — shared HTTP/read/discovery contracts, rfs-pm0y/rfs-i0c9, and ADR-0006 cover the existing substrate. | N/A — applicable repository evidence. |

## Data

- Run date: 2026-09-06 UTC. Public Jira REST v3 OpenAPI, independently rendered Atlassian documentation, and the existing disposable `https://kiro-tethys.atlassian.net` tenant.
- Shape: real project identity/summary objects; real enhanced-search issue identity/selected-field objects; native offset links and opaque tokens. Small pages exercise actual continuation without assuming a stable upstream count.
- Safety: probe programs issue bounded reader-authenticated GETs only, reject redirects, and print shapes/booleans rather than response prose or credentials. Reader credentials came from the ignored local environment file and were passed only through the child environment.
- Fixture setup was separate from probing: the existing manifest-driven operator provisioned its marker-owned disposable fixtures using the provisioner account. Unrelated visible resources were only read. This is the previously established disposable-fixture workflow, not a mutation experiment against production data.
- Setup: `scripts/atlassian-fixture-bootstrap.sh bootstrap --site https://kiro-tethys.atlassian.net` exited 0 (`bootstrapped`). Cleanup with the same operator/site exited 0 (`cleaned`); both `.resourcefs/atlassian-fixture-state.json` and its `.pending` receipt were absent afterward.

## Probe

- `python .rfs-h212/probe_openapi.py`: standard-library schema extraction; no production Rust imports. Exit 0. Source: <https://dac-static.atlassian.com/cloud/jira/platform/swagger-v3.v3.json>; SHA-256 `5a549e4a1aec18210fe9bceb065c9bf13f7cd53105ac947d65958ac947be1d49`; version `1001.0.0-SNAPSHOT-82b018affa468e58f284fbe4df33536469d757df`.
- `python .rfs-h212/probe_live.py`, with `RFS_LIVE=1` and execution-only reader environment: bounded project/search requests, supported bounded issue query, and rejected/incorrect candidate controls. Final exit 0.
- `python .rfs-h212/probe_pages.py`, with the same reader gate/environment and ignored fixture state: native `maxResults=1` traversal, project ID/key equivalence, expected fixture membership, selected fields versus separate direct issue GETs, and project-parent membership. Final exit 0; all membership/identity/comparison booleans true. Reconciliation parameters are not used in the final run.

## Oracle

Independent scouts inspected rendered operation prose/examples and pagination/JQL guides rather than traversing the OpenAPI object graph. Main additionally inspected the actual browser DOM when reader extraction lost the Project operator table. Real search results were compared with fixture identities established by the separate setup operator and with direct resource GETs, not with a fake search fixture or future production decoder.

Primary sources:

- [Project operations](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/): ID/key lookup, permission-visible project listing, and offset response examples.
- [Enhanced issue search](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/#api-rest-api-3-search-jql-get): current GET/POST operations, visibility, selected fields, token response; old `/search` operations are being removed.
- [REST pagination](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/): operation-specific/changeable maxima, authoritative returned maximum, changing totals, and permitted empty pages.
- [Search and reconcile](https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/): opaque enhanced-search continuation example.
- [Bounded JQL](https://support.atlassian.com/jira-service-management-cloud/docs/what-is-advanced-search-in-jira-cloud/): a field/operator condition is required; ORDER BY alone is unbounded.
- [JQL operators](https://support.atlassian.com/jira-software-cloud/docs/jql-operators/): `IS NOT EMPTY` selects a field with a value, subject to field compatibility.
- [JQL fields](https://support.atlassian.com/jira-software-cloud/docs/jql-fields/#Space): browser DOM for the Space/Project field explicitly lists supported operators `=, !=, IS, IS NOT, IN, NOT IN`. The page documents the terminology transition without changing existing JQL queries.
- [Legacy field reference](https://confluence.atlassian.com/jirasoftware/advanced-searching-fields-reference-1528533189.html): `project` accepts project IDs/keys/names; `id` is an alias of `issueKey`, not a separately documented numeric range field.

## Comparisons

| ID | Probe output | Independent oracle output | Verdict |
|----|--------------|---------------------------|---------|
| P1 | Project search requested 100 and returned `maxResults:100`; rows carried `id,key,name,self,projectTypeKey`. Optional archived/deleted metadata was absent. Direct ID and key lookups agreed. `orderBy=id` returned 400. Schema ordering enum excludes ID. | Project docs describe ID/key lookup and permission-visible rows. Published model makes metadata optional. No numeric-ID order is promised by the rendered operation. | PASS — stable identity and compact data exist; native sorting and optional metadata assumptions corrected. |
| P2 | Three populated project pages had `(startAt,maxResults,rows)` of `(0,1,1)`, `(1,1,1)`, `(2,1,1)`; first two had `isLast:false` and `nextPage`, final had `isLast:true` and no next link. All fixture projects appeared. | Rendered project example has the same offset/max/next-link/terminal shape. Pagination guide warns that actual maxima and totals can change and empty pages are possible. Schema documents a project-search maximum of 100. | PASS — actual offset traversal agrees; changing totals/smaller server maxima remain implementation boundary cases, not claims observed in this small tenant. |
| P3 | `project IS NOT EMPTY ORDER BY key ASC` returned populated issue rows and all fixture IDs. Requesting one row traversed eight pages: seven nonterminal pages with tokens, then terminal `isLast:true` without a token. Explicit fields returned `id,key,self,fields`; fields were `project,status,summary`. Default fields returned only `id`. Fixture summaries/identity agreed with direct GETs, and project-scoped rows matched the requested project ID. | Enhanced-search schema/docs specify token pagination, bounded JQL and default ID-only selection. Browser field table confirms Project supports `IS NOT`; operator semantics select a populated project field. Project field docs establish issue membership. Direct reads independently establish fixture identity and metadata. | PASS — supported bounded enumeration and native continuation are demonstrated, not inferred from an accepted empty response. |

Observed page/row counts describe this run only. They are not a future live-test assertion or a promise about total tenant state. This stage does not claim production adapter behavior, thousand-row behavior, or corrupt-response handling has been implemented or tested.

## Validated / learned

- P1 — Learning: requesting native numeric project ordering is invalid (400), and real compact project rows can omit archived/deleted status metadata. Stable project identity survives key lookup; optional metadata cannot be fabricated from absence.
- P2 — Validated prior understanding: real project paging uses offset metadata and validated native next links, unlike enhanced-search opaque tokens. Neither immutable totals nor requested page size alone supplies the pagination contract.
- P3 — Learning: `id > 0 ORDER BY id ASC` returned 200 with zero issues even after known fixtures were visible through project-scoped search. `id` aliases the issue-key field; acceptance does not establish numeric-range completeness. The corrected populated-project predicate traversed real tokens and matched direct reads. Reader-mode documentation initially hid the field-operator rows; browser DOM resolved that oracle defect without changing the source claim. A probe-only `reconcileIssues` comma-list also returned 400; repeated GET parameters worked, and reconciliation was removed from the final normal-pagination probe. Default issue-search selection is ID-only, so compact metadata is not implicit.

Additional published facts: project search defaults to live status and exposes live/archived/deleted filters; enhanced GET and POST expose `includeArchivedProjects`, default false. These are visibility-selection facts, not a claim that archived fixture behavior was exercised.

## Related issues

- Bounded prior-art search: native Rivets list filtered once for Jira, Atlassian, continuation, and pagination titles; not repeated by downstream evidence work.
- Consulted: rfs-h212, rfs-nae2, rfs-pm0y, rfs-bcym, rfs-qsyk, rfs-lcuh, rfs-lbps, rfs-8jnk; source-cited Jira research from rfs-4df1 and existing direct-read evidence.
- Filed: none. Observed disagreements were incorrect assumptions or probe/reader defects, not an established Jira defect.

## Hand-off

PASS: all empirical premises discharged; standalone probes and independent comparisons recorded; safe data/cleanup recorded; no production code or feature tests changed. Hand off this evidence to falsifiable-design. No design approval is implied.

## Implementation live record — S1

- Date: 2026-09-06 UTC, after the behavior-preserving core/Jira/HTTP extraction and before the S1 commit.
- Setup: the existing marker-owned fixture operator `bootstrap` exited 0. The reader-only test environment was derived from the ignored state receipt; provisioner credentials were not passed to the Rust test.
- Command: `cargo test -p resourcefs-sources --all-features --test jira_live_smoke live_jira_issue_read -- --ignored --nocapture`, with `RFS_LIVE=1`, reader `ATLASSIAN_*` credentials and fixture IDs, JSON field `summary`, ADF field `description`.
- Result: PASS — one real ignored row ran (not a skipped gate), 1.96 seconds test execution. Stable/alias identity, Field index navigation, canonical JSON/ADF representation, exact-content Version Tags, repeat-read content and secret redaction passed against Jira.
- This verifies the moved direct-read implementation only; project/issue browsing is not yet implemented. Owned fixtures remain available for the upcoming adapter live checks and must be cleaned after their final use.
