# Evidence: rfs-by2z

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | GitHub's current REST mutation contract exposes the six required issue, pull-request, and issue-comment operations with response identities sufficient to build canonical Resource references and authoritative replacement tags. | For each required mutation, what method/path, request fields, success status, and response type/identity does the public contract expose? | PASS |
| P2 | GitHub documents machine-distinguishable non-success statuses for these operations and no endpoint-specific conditional `If-Match` write contract. | Which statuses do the six current operations document, and does either independent published contract attach `If-Match` to them? | PASS |
| P3 | ResourceFS must distinguish a definitely pre-transmission transport failure from an unknown transmitted outcome to implement retry behavior. | Does the approved behavior require that distinction? | N/A — the approved spec permits exactly one mutation transport attempt and conservatively treats any possibly transmitted unvalidated creation outcome as unknown. |
| P4 | Strict frontmatter, operation-ID syntax/journal transitions, receipt semantics, cache invalidation, grants, and stable error mapping are external facts. | Are these claims about existing-system or third-party behavior? | N/A — they are requester-approved behavior for the feature being built, not empirical premises. |
| P5 | The current ResourceFS code already supports generic mutation HTTP requests and GitHub Creation Target receipts. | Is this an unverified premise? | N/A — current repository evidence directly shows `HttpRequest::get`/GET-only credentialed transport, read-only GitHub dispatch, no Path Session operation journal, and `MutationAdapter::commit -> Result<(), ResourceError>`; the design must change those seams rather than assume they exist. |

## Data

- Source: production-shaped public contracts: GitHub's current read-only REST documentation pages for issues, issue comments, and pull requests; the published `github.com/google/go-github/v74@v74.0.0` module source; and published `@octokit/openapi-types@29.0.0` generated from GitHub's OpenAPI description.
- Shape: the exact six api.github.com operations, JSON request/response schemas, methods, paths, and documented status distributions the production adapter will use.
- Safety: all probes perform GETs against public documentation/package registries and inspect downloaded source/type archives in memory. They send no request to a mutating api.github.com endpoint and write no upstream or production state. Approval: N/A — production-shaped public read-only data needs no snapshot risk acceptance.

## Probe

- File: `probe_github_mutation_docs.py`
- Mechanism: independently fetches GitHub's Markdown REST documentation, isolates the six named operation sections, and extracts method/path, body parameters, documented statuses, success schema fields, and any documented `If-Match` contract.
- Run: `python3 .rfs-by2z/probe_github_mutation_docs.py`

## Oracle

- Files: `probe_go_github_oracle.py`, `probe_openapi_types_oracle.py`
- Mechanism: the first oracle downloads a fixed published Go client module and extracts the method implementations, request structs, response types, routes, and headers; the second downloads the latest published Octokit OpenAPI type package and extracts operation routes, status sets, request fields, success statuses, and response schemas. These fail through compiled-client/source-generation drift rather than the probe's human-document parser, and neither shares the planned Rust adapter implementation.
- Runs: `python3 .rfs-by2z/probe_go_github_oracle.py`; `python3 .rfs-by2z/probe_openapi_types_oracle.py`

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | `create_issue`: `POST /repos/{owner}/{repo}/issues`, required title, optional body, `201`, response documents `number`; `update_issue`: `PATCH /repos/{owner}/{repo}/issues/{issue_number}`, title/body fields, `200`; `create_comment`: `POST /repos/{owner}/{repo}/issues/{issue_number}/comments`, required body, `201`; `update_comment`: `PATCH /repos/{owner}/{repo}/issues/comments/{comment_id}`, required body, `200`; `create_pull`: `POST /repos/{owner}/{repo}/pulls`, head/base required with title/body/draft fields, `201`, response documents `number`; `update_pull`: `PATCH /repos/{owner}/{repo}/pulls/{pull_number}`, title/body fields, `200`. | Go v74 independently emits the same six verbs and route formats, request types containing the required fields, and `Issue`/`IssueComment`/`PullRequest` response types. Octokit 29 independently emits identical methods/paths/success statuses and `issue`/`issue-comment`/`pull-request` success schemas. | PASS |
| P2 | No operation section documents `If-Match`. Status sets: create issue `{201,400,403,404,410,422,503}`; update issue `{200,301,403,404,410,422,503}`; create comment `{201,403,404,410,422}`; update comment `{200,422}`; create pull `{201,403,422}`; update pull `{200,403,422}`. | Octokit 29 emits the identical six status sets. Go v74 sets no `If-Match` header in any of the six method implementations. | PASS |

## Validated / learned

- P1: validated prior understanding — the docs probe and both independent published-client/type oracles agree on all six method/path pairs, create/update success statuses, request field availability, and response types carrying issue/PR numbers or comment IDs plus authoritative field values.
- P2: new learning — the originally approved status table covered the common `401/403/404/409/422/429/5xx` classes but omitted three statuses explicitly documented for the issue-family endpoints: `400`, `410`, and `301`. Probe and OpenAPI oracle agree exactly on those additions. The spec must classify them before design hand-off. Both independent mechanisms also agree that these endpoint contracts expose no `If-Match`; ResourceFS can promise immediate compare-before-write, not an atomic remote content-hash condition.
- API-version note: GitHub's Markdown documentation currently advertises `X-GitHub-Api-Version: 2026-03-10` even when the page URL carries `apiVersion=2022-11-28`; the existing adapter's `2022-11-28` header remains live-GET-proven by rfs-45ww, while this evidence validates the current mutation shapes through two current published representations. No mutation request was sent to test version-specific behavior.

## Related issues

- Consulted: rfs-jk0d (accepted product/mutation/session contract); rfs-73dz (shared versioned mutation engine and explicit remote-journal deferral); rfs-45ww (typed GitHub read adapter, GET-only empirical evidence, and mutation hand-off); rfs-g2z9 (single bounded HTTP substrate); rfs-r9m6 (strict source/repository grants); rfs-34pz (Path Session lifecycle/isolation).
- Filed: none — the comparison found a specification classification gap, not an underlying-system defect or intended future-work item.
