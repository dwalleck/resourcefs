# Evidence: rfs-bcym

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | Jira Cloud exposes an administrator-controlled synchronous lifecycle for projects plus stable-ID issue and comment creation, lookup, pagination, and deletion. | Do current Jira REST v3 contracts expose project/issue/comment create, lookup/list, and delete operations with successful identity-bearing create responses and already-absent outcomes? | PASS |
| P2 | Confluence exposes a complete fixture lifecycle for spaces, pages, hierarchy-bearing parents, footer comments, storage bodies, and CQL pagination. | Do current Confluence REST contracts expose ordinary/private space creation, page and footer-comment creation with stable identity/parent fields, list/search cursors, and deletion operations? | PASS |
| P3 | One provisioner-only Confluence space can serve as the required reader-concealed object without relying on ResourceFS mutation code. | Does a current Confluence REST create contract explicitly create a private space, while product permissions filter reads to the authenticated principal? | PASS |
| P4 | Cleanup must model Confluence space deletion differently from synchronous object deletes. | Is Confluence space deletion asynchronous and identity-bearing, while Jira project/issue/comment and Confluence page/comment deletes have synchronous success statuses and all relevant deletes document not-found outcomes? | PASS |
| P5 | Pagination can be forced with a small stable fixture rather than hundreds of created objects. | Do Jira comment/JQL and Confluence space/page/comment/CQL list contracts accept explicit page-size plus offset/token/cursor parameters? | PASS |
| N1 | The exact checked-in fixture contents, manifest schema, ignored state schema, owner markers, output vocabulary, and idempotent reconciliation algorithm. | N/A — these are requested feature behavior and design choices, not claims about an existing external system. | N/A — specified behavior |
| N2 | API-token Basic authentication over account email and token. | N/A — current applicable evidence already covers this premise in `.rfs-pm0y/evidence.md` P5 against the same pinned Jira schema/auth guide; Confluence research records the same direct Basic-auth contract. | N/A — current repository evidence |
| N3 | A specific disposable tenant's installed product templates, RBAC rollout, principal grants, and observed propagation latency. | N/A — the operator must validate these at execution and the finished command's terminal smoke must run against the selected tenant; the design does not assume one template, rollout state, or latency without checking. | N/A — runtime input to verify |

## Data

- Source: production-shaped — Atlassian's current published Jira REST v3, Confluence REST v2, and Confluence REST v1 OpenAPI documents plus their separately rendered first-party operation pages.
- Shape: vendor schemas and operation prose used to publish the production create/read/list/delete paths, request models, response identities, pagination parameters, permission requirements, private-space behavior, and asynchronous deletion contract. Jira schema version `1001.0.0-SNAPSHOT-699dda19a3d49050afba1f0e24e0b62d363c1be4`, SHA-256 `7b92e6a64584be28d2222e38e8752db7b9f422aa91ed0d20995cfaf6168293d4`; Confluence v2 version `2.0.0`, SHA-256 `451377c5a598ee8155acc11b611404f309bed4a4292ea87f88ed3bfed38fa0a8`; Confluence v1 version `1.0.0`, SHA-256 `70088490f4069e0a5b082411ac5296b822041b2848256444f0dfca3f3d319e73`.
- Safety: read-only anonymous fetches of public vendor documentation; no tenant, credentials, production state, or mutation involved. | Approval: N/A — safe public production-shaped data.

## Probe

- File: `probe_openapi.py`
- Mechanism: Python's standard JSON decoder independently fetches and traverses the three published OpenAPI documents, resolves request/response references, and emits only the create/read/list/delete paths, statuses, pagination parameters, request members, stable identity/parent members, storage representation enum, private-space controls, and asynchronous deletion result named by P1–P5. It imports neither ResourceFS production code nor the future shell command and does not use `curl` or `jq`.
- Run: `python .rfs-bcym/probe_openapi.py`

## Oracle

- Mechanism: manual inspection of Atlassian's separately rendered REST operation pages and the source-cited Jira/Confluence research notes; this uses vendor prose, permission statements, response examples, and human comparison rather than the probe's schema traversal and differs from the production command's `curl`/`jq` request and validation path.
- Run: Read `https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/:619-760,2181-2305`, `https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-space/:409-560`, `https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-space/:6598-6705`, `docs/research/jira-cloud-contracts.md:121-188`, and `docs/research/confluence-cloud-contracts.md:22-122`.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | Jira v3 exposes `POST /rest/api/3/project` → 201 with required `id`/`key`/`self`, `DELETE /project/{projectIdOrKey}` → 204/401/404, issue create → 201 with `id`/`key`/`self`, issue delete → 204/400/401/403/404, comment create → 201 with `id` and body/visibility members, comment delete → 204/400/401/404/405, direct lookup, comment listing, and enhanced JQL. | Rendered project docs say an Administer Jira principal creates from a project template and receives `ProjectIdentifiers`; delete is synchronous 204 and documents 404. Source-cited issue/comment research records 201 identity-bearing creates, 204 deletes, stable-ID reads, ADF comment bodies, and permission-filtered lists. | PASS |
| P2 | Confluence exposes v1 ordinary/private space POSTs, v2 space POST, v2 page POST/GET/list/DELETE, v2 footer-comment POST/list/DELETE, and v1 CQL. Space results contain `id`/`key`; page results contain `id`/`spaceId`/`parentId`/`body`; comment results contain `id`/`pageId`/`parentCommentId`/`body`; page/comment writes accept `storage`. | Rendered and source-cited docs describe space creation, page creation under `spaceId` with optional `parentId`, footer comments under a page or parent comment, exact representation-tagged storage bodies, cursor-paginated collections, and v1 CQL. | PASS |
| P3 | Confluence v2 space create exposes `createPrivateSpace`; v1 separately exposes `POST /wiki/rest/api/space/_private`. | Rendered v1 docs define private-space creation as visible only to the creator; research records that Confluence permissions still filter reads even when the credential has an API scope. A provisioner-created private space therefore supplies a product-native reader-concealment boundary. | PASS |
| P4 | Confluence v1 space delete returns 202/401/404 and a required long-task `id` plus `links`; Jira project/issue/comment and Confluence page/footer-comment deletes expose 204 success and 404 among their documented outcomes. | Rendered Confluence docs say space deletion permanently deletes in a long-running task and require polling the returned status link; rendered Jira project docs and source-cited object docs describe synchronous 204 deletion. | PASS |
| P5 | Jira comment list exposes `startAt`/`maxResults`; enhanced JQL exposes `maxResults`/`nextPageToken`; Confluence v2 space/page/comment lists expose `limit`/`cursor`; v1 CQL exposes `limit`/`cursor`. | Source-cited research distinguishes Jira offset and opaque-token paging from Confluence cursor/Link paging and explicitly permits forcing progression with small request limits and at least two fixtures. | PASS |

## Validated / learned

- P1: validated prior understanding — Jira's administrator project lifecycle and issue/comment lifecycles expose enough stable authority for an external fixture operator; create response members other than `ProjectIdentifiers` are schema-optional, so the command must validate every consumed ID/key/parent at point of use.
- P2: validated with a compatibility boundary — Confluence has both v1 and RBAC-gated v2 space creation. The operator can use the current v1 ordinary/private creation paths for tenant-independent fixture setup while using v2 for page/comment objects consumed by the read adapter.
- P3: validated prior understanding — a provisioner-only private Confluence space is the smallest concealed fixture; no Jira permission-scheme synthesis is required merely to satisfy the cross-product concealment acceptance criterion.
- P4: learned — Confluence space deletion is not complete at HTTP 202. Cleanup must validate the returned long-task identity/status reference, poll with a fixed bound, and only report success after terminal completion or verified absence; every other selected object delete is synchronous.
- P5: validated prior understanding — two records plus an explicit small page size are sufficient to exercise real cursor/offset/token progression, avoiding a rate-heavy large fixture.

## Related issues

- Consulted: `rfs-pm0y` (current Jira OpenAPI/auth evidence), `rfs-lbps` (live tenant and evidence gates), `rfs-8jnk` (read-first milestone/operator script), `rfs-lzbn` (blocked live release), `rfs-0zbv` (destination contract).
- Filed: none — the comparisons agreed and revealed no underlying-system defect or separate future-work gap.

## Deterministic operator evidence

- `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract -- --nocapture` — PASS, 10/10.
- The fake upstream observed the shipped script through `curl --config -`, with canary credentials absent from argv, child environment, logs, state, stdout, and stderr.
- Lifecycle proof: first bootstrap created the complete Jira/Confluence graph; second bootstrap issued no creates; verify traversed provisioner and reader authority plus every paging family; first cleanup polled each Confluence space deletion to completion and emptied every owned collection; second cleanup issued no mutation.
- Named mutations C2–C11 each made the approved regression fence red. The C6 fence additionally denied the final directory rename after all upstream work and proved the prior state document remained byte-identical.
- `bash -n scripts/atlassian-fixture-bootstrap.sh`, `jq -e . fixtures/atlassian-live/manifest.json`, `cargo fmt --all -- --check`, and Rust LSP diagnostics all passed.
- Final focused review hardened four boundaries: exact owner-token shape rather than arbitrary substring, immediate post-create authority reads, pre-creation rejection of every existing symlinked state-path ancestor, and complete fake credential-pair authentication. Added independent foreign-object and mismatched-create regressions; reviewer recheck found no remaining high-confidence defect.
- Live disposable-tenant evidence (C12): pending tenant URL plus provisioner and reader credentials; no `ATLASSIAN_*` credentials were present in the implementation environment.
