# Evidence: rfs-nae2

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|---|---|---|---|
| P1 | Enhanced search supports read-only POST with explicit JQL, selected fields and opaque continuation; direct issue reads provide stable identity. | Do existing independently checked endpoint/schema/fixture results still apply after browsing landed? | PASS — retained evidence below |
| P2 | Populated enhanced POST search honors explicit native ordering across opaque-token pages and returns the selected compact identity/metadata family. | Do ascending/descending POST traversals match independently provisioned fixture keys and separate direct issue reads, and what terminal/token shapes occur? | PASS |
| P3 | Invalid native JQL POST returns a structured validation rejection. | What status and allowlisted diagnostic member shapes does a deliberately invalid query return, compared with the documented Error Collection? | PASS |
| N1 | Canonical query spelling, Site Mount ownership, local 100/1,000/ten bounds, native order retention, query-bound cursors, regex/recovery and read-only POST retry/no-cache policy. | N/A — specified behavior in rfs-nae2, rfs-qsyk and rfs-lbps; implementation claims are not vendor premises. | N/A — specified behavior |
| N2 | A fixed effective upstream maximum, snapshot consistency, token immortality or tenant-wide completeness. | N/A — no such guarantee is assumed. Follow validated native continuation even after short pages; record explicit returned maximum metadata if available rather than infer a clamp from fixture count. | N/A — no assumed external guarantee |

## Preimplementation probe data

- Source: production-shaped marker-owned fixtures from the checked-in disposable Atlassian fixture manifest, using the previously established operator workflow at `https://kiro-tethys.atlassian.net`.
- Shape: multiple known issue keys and stable IDs in an owned project, compact summary/status/project fields, explicit one-item token pages, exact-boundary and empty terminal pages, and an intentionally invalid native query.
- Safety: fixture setup/cleanup is separate from the probes and uses the existing ownership-validating operator. Probes use only the least-privileged reader account, direct GET and the documented read-only enhanced POST-search endpoint. They reject redirects, bound bodies and request counts, and emit shapes/booleans/owned fixture identifiers rather than query text, native tokens, diagnostic prose or credentials. No production source is changed.
- Preimplementation P2/P3 setup and cleanup only: on 2026-09-07, `bash scripts/atlassian-fixture-bootstrap.sh bootstrap --site https://kiro-tethys.atlassian.net` exited 0 (`bootstrapped`). After both comparisons passed, the corresponding `cleanup` command exited 0 (`cleaned`); the ignored state and pending receipt were absent. Execution credentials were checked absent from all captured output. The separate postimplementation run below had an operator cleanup failure and audited reconciliation; this earlier success is not evidence that its cleanup command passed.

## Probe

- P1: retain `.rfs-h212/probe_openapi.py`, `.rfs-h212/probe_pages.py` and their recorded results; retain `.rfs-bcym/evidence.md` corrected C12. They establish POST availability/execution and the shared enhanced response family, not ordered POST traversal or native-query error bodies.
- P2: `RFS_LIVE=1 python .rfs-nae2/probe_post_pages.py`, with execution-only reader credentials and `RFS_ATLASSIAN_SITE`, exited 0. Eight total requests: two independent direct reads, two one-item pages in each ordering, one requested-100 page, and one logically empty query.
- P3: `RFS_LIVE=1 python .rfs-nae2/probe_post_error.py`, with the same reader environment, exited 0. One read-only enhanced POST; only status, media, sizes and diagnostic types/counts were recorded.
- Fresh non-secret outputs: `probe-results.json`. The first P2 attempt failed before network access because the probe used `state.origin` instead of the receipt's existing `state.site.origin`; the probe was corrected and rerun. This was a probe defect, not a production defect or changed oracle.

## Oracle

- P1: separately rendered first-party operation documentation and independently provisioned/read fixture identities; see retained results below.
- P2: provisioner receipt keys determine the expected issue sequence by their numeric suffix within one project; separate direct issue GETs determine exact stable identity and selected field values. Neither depends on the POST page assembler or its token traversal.
- P3: manual comparison with the native query endpoint's documented HTTP 400 and the common Error Collection (`errorMessages: string[]`, `errors: object<string,string>`, optional integer `status`) recorded in `docs/research/jira-cloud-contracts.md` section 11. This differs from the live probe's request/response decoding.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|---|---|---|---|
| P1 | h212 P3 (2026-09-06) extracted enhanced GET/POST schema; populated GET selected `id,key,self,fields`, advanced seven nonterminal token pages then observed `isLast:true` without a token. Corrected bcym C12 (2026-09-06) observed successful read-only POST search during independent fixture verification. | Separately rendered operation pages confirm enhanced search; independently provisioned identities and direct issue GETs agree with selected rows. Endpoint/versioned resource family and credential model are unchanged by the local browsing implementation. | PASS — retained |
| P2 | ASC returned `[10084,10085]`; DESC returned `[10085,10084]`. Each traversal had one nonterminal one-row page with `isLast:false` and a token, then a one-row exact-boundary terminal page with `isLast:true` and no token. All selected `id/key/self/fields` values matched direct reads. Requested 100 returned both fixture rows terminally; the response exposed no `maxResults`. The contradictory predicate returned zero rows, `isLast:true`, no token. | Independently provisioned keys in one project establish the numeric key-suffix order; direct issue GETs establish identity/metadata. Reversing the explicit order reverses the expected sequence. The contradictory predicate is empty by Boolean logic. No effective numeric clamp is inferred from two fixture rows; native continuation remains authoritative. | PASS |
| P3 | HTTP 400, `application/json`, 125 bytes, one global string diagnostic, zero field diagnostics, no structured `status` member; no raw diagnostic prose emitted. | The enhanced POST operation documents 400; the common Error Collection permits a string array, string-valued field map and optional integer status. Observed members/types agree. | PASS |

## Validated / learned

- P1: validated prior understanding — published POST execution and shared compact identity/token concepts remain applicable. This does not promote GET membership evidence into POST order proof.
- P2: validated prior understanding — selected POST rows share the existing compact response family and preserve explicit native ordering across opaque-token pages. Exact-boundary and empty termination use `isLast:true` without a token. No snapshot, unstated ordering or effective-clamp guarantee is inferred.
- P3: validated prior understanding — native malformed-JQL rejection is HTTP 400 with a structured global-error array; the optional status field is absent. This validates structure, not permission to return upstream prose.

## Related issues

- Consulted through the existing h212 prior-art search and native ticket reads: rfs-h212, rfs-pm0y, rfs-bcym, rfs-qsyk, rfs-lcuh, rfs-lbps, rfs-i0c9, rfs-4df1, rfs-4srt, rfs-8jnk, rfs-lzbn and rfs-0zbv. No broad tracker search repeated.
- Preimplementation probes filed no issue; the initial receipt lookup failure was repaired in the probe itself. Postimplementation live verification exposed the existing fixture operator recovery defect filed as rfs-br1u.

## Primary references

- https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/#api-rest-api-3-search-jql-post
- https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/
- https://dac-static.atlassian.com/cloud/jira/platform/swagger-v3.v3.json

## Postimplementation live verification

Date: 2026-09-07. Fresh owned fixtures were bootstrapped and verified (both commands exit 0). Test processes received only reader credentials, `RFS_LIVE=1`, `RFS_ATLASSIAN_SITE`, and the absolute owned receipt path; no provisioner credentials or general account fallback. Details: `s2-live-results.json`.

| Row | Real execution | Independent oracle and observed result | Verdict |
|---|---|---|---|
| L1 | `cargo test -p resourcefs-sources --test jira_live_smoke live_jira_jql -- --ignored --nocapture` | Receipt key-suffix ordering and separate stable-ID/key-alias reads agree with ASC/DESC query traversal. One-item logical/native pages exercise real opaque continuation through terminal state. One test passed, 2.12s. | PASS |
| L2 | `cargo test -p resourcefs-mcp --lib server::jira_query_tests::live_jira_query_stdio -- --ignored --nocapture` | Actual MCP service over reexecuted stdio agrees with receipt order and direct summary field reads; rendered-content search matches owned rows; malformed native JQL returns `invalid_pattern`. One test passed, 2.30s. This is not a shipped Atlassian Server Profile launch (rfs-ue44). | PASS |
| L3 | Existing fixture operator `cleanup` and two receipt-based resumptions | Failed respectively at `space_delete_poll` response validation, trashed-page ownership revalidation, and deleted-space `page_list` HTTP 404. The original async task payload was not retained; no exact native status/schema is inferred. | FAIL — rfs-br1u |
| L4 | Separate audited owned-container reconciliation and direct absence checks | Revalidated remaining exact space/project IDs, keys and manifest ownership markers before deletion. All 14 receipt resource endpoints returned 404; both Jira project trash searches were empty and terminal. Removed the receipt only afterward; pending receipt absent. | PASS — clean postcondition, not a successful operator invocation |
| L5 | Fresh complete repeat after live-harness cleanup | Removed unused empty issue-coordinate fields from query setup by passing only the four source-construction inputs; failure diagnostics no longer echo a query/cursor reference. Bootstrap and verify exited 0; adapter and real stdio rows each passed in 2.40s; the unchanged standard cleanup operator exited 0 (`cleaned`) for this later fixture set, with state/pending absent. This later success does not erase L3's interrupted-cleanup defect. | PASS |

The operator source and foreign-object refusal guards were not changed for JQL. The independent cleanup postcondition is complete; the operator's resumability defect remains explicitly tracked rather than hidden by a green smoke row.

## Assembled deterministic and mutation verification

- Focused contracts: core 5, owning HTTP 17, query adapter 13, real query stdio 2, existing stdio integration 39 passed. The effective-maximum contract also exercises a nonterminal clamp lowering the next request and rejects a later attempted increase.
- C1–C12 each received a compiling responsibility/semantic mutation, the intended localized red result, and restored green. All 12 passed; production byte restoration verified. `s2-mutation-results.json` records exact commands, mutation descriptions, source hashes and exit codes. Full captured logs are retained in the session's `local://rfs-nae2-C*-{compile,red,restored}.log` artifacts.
- Independent reviewer reconstructed ownership before opening the ledger, then found no actionable production defect and confirmed C1–C12 placement/consumer routing. Its next-request clamp coverage observation was addressed without changing the implementation or oracle.
- Final local `python scripts/ci-gates.py`: all gates passed. Functional workspace 612 passed/22 ignored; release workspace 612 passed/22 ignored; all 12 classified ignored production budgets passed; formatter, warnings-denied clippy, query-stage placement and dependency vetting passed. Lossless log: `local://rfs-nae2-final-gates.log`.
- Initial assembled gates caught the still-staged `http` CI oracle invocation and one needless test borrow; both were corrected. That attempt also reported a functional failure whose only detail was lost by the tool's truncated “full” artifact; it is not claimed diagnosed. The subsequent complete, losslessly captured workspace and release runs passed.
