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
- Live disposable-tenant evidence (C12): completed 2026-09-05 against `https://kiro-tethys.atlassian.net` (disposable tenant, provisioner + least-privileged reader principals, execution-only credentials).

## Live gate (C12) — 2026-09-05

- Mechanism: the shipped operator ran against the real disposable tenant with execution-only `ATLASSIAN_*` credentials; independent read-only `curl`/`jq` probes (provisioner and reader) provided the oracle for absence and visibility checks.
- First pass: bootstrap failed fast at the first page (`identity_mismatch logical=page`). Probing the tenant (read-only, plus one scratch page and one scratch comment, both deleted) exposed production behaviors the fake upstream could not imitate:
  1. Confluence **strips leading HTML comments** from stored page and footer-comment bodies, so the `<!--rfs-owner:...-->` ownership marker cannot survive in a body.
  2. Confluence **re-encodes raw `—`/`π` as `&mdash;`/`&pi;`** in storage values (pre-encoded entities round-trip byte-exact).
  3. Confluence **injects `ac:schema-version` and a random `ac:macro-id`** into `ac:structured-macro` elements at storage time.
  4. A page created without `parentId` reports the **space homepage id as `parentId`** on read-back.
  5. `GET /rest/api/3/project/search` **omits `description`** unless `expand=description` is passed (marker-based project adoption).
  6. Footer comments reject v2 properties (`GenericContentType` only); **v1 content properties** (`/wiki/rest/api/content/{id}/property`) round-trip exactly for pages and footer comments.
- Repairs: ownership markers moved to a `rfs-owner` v1 content property for pages and comments (adoption, verify, and cleanup discovery now read it); manifest storage values dropped body-comment markers and pre-encode entities; body comparison normalizes provider-injected macro attributes; root-page validation accepts `parentId == space homepageId` (id captured from space reads in every mode); project search passes `expand=description`; cleanup tolerates documented 403 on Jira issue/comment deletes and relies on the project-delete cascade with the final re-discovery still failing on any residue. The fake upstream now imitates all six live behaviors (comment stripping, entity re-encoding, macro-attribute injection, homepage parenting, description-expansion gating, `FINISH_SUCCESS` long-task terminal status, plus 403 issue-delete denial scenario), and the deterministic suite gained foreign-page-property, missing-property, and delete-denied fences.
- Second pass, all against the live tenant: bootstrap → success; bootstrap → success (idempotent convergence; the deterministic suite proves second-run write absence); verify → success (provisioner and reader authority, every paging family, reader-visible/concealed boundary); cleanup → first attempt surfaced one more live gap — long-task terminal status is `FINISH_SUCCESS`, not `COMPLETE` — fixed and rerun to success; cleanup → second run idempotent success.
- Final absence checks (independent of operator state): provisioner project search returned 0 fixture projects; provisioner and reader space lists returned 0 fixture spaces; reader `GET /rest/api/3/project/RFSFIX` returned 404.
- `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract` — PASS, 10/10 after the repairs. `bash -n`, `jq -e .`, and `cargo fmt --all -- --check` clean. The residue page left by the first (pre-repair) bootstrap was deleted manually before rerunning; every later mutation went through the operator.
- Credential canary checks: live bootstrap/verify/cleanup stdout, stderr, operator state, and this evidence contain no tenant credential values.

## Review decisions — 2026-09-06

The review baseline is `518b99e`. The historical C12 run above proved successful convergence, visibility, and absence; it did **not** independently prove that the second live bootstrap made zero writes. The review also demonstrated credential leaks the earlier curl-only canary observation missed. Those historical claims are not proof for the corrected implementation.

| finding-id | finding | reviewer | evidence-state | evidence | decision | fix | note |
|---|---|---|---|---|---|---|---|
| F1 | Credential pair enters jq argv; encode without secret arguments. | BoundaryReview, Main | Verified | Successful baseline bootstrap with temporary jq instrumentation observed the canary credential in argv. | Accept | Slice 3: in-memory secret encoding and non-exported shell credentials. | Observe every helper, not only curl. |
| F2 | Curl configuration persists credentials; pipe configuration directly. | BoundaryReview, Main | Verified | Temporary curl instrumentation found the canary in `config-1` during successful baseline bootstrap. | Accept | Slice 3: stream configuration directly into curl. | Restrictive permissions do not make disk persistence acceptable. |
| F3 | Separate Confluence create/property requests strand unowned objects. | ProvisionReview, Main | Verified | One property POST returning 503 made baseline bootstrap fail; repeated bootstrap and cleanup both failed `foreign_collision logical=page`. | Modify | Slice 4: atomic ownership publication if supported by the real API; otherwise a validated durable creation receipt. | Never adopt a foreign unmarked object merely because its title/body matches. |
| F4 | Scoped rediscovery drops moved stable identities and reports false cleanup success. | CleanupReview, Main | Verified | Moving a fixture issue to another fake project preserved its ID/marker; baseline cleanup exited zero, removed state, and left the issue alive. | Modify | Slice 4: re-fetch saved IDs, verify ownership, and prove direct absence before removing state. | Confluence space/title drift has the same identity-loss root cause. |
| F5 | Root footer-comment listing cannot rediscover replies. | ProvisionReview, Main | Verified | Root-only fake responses plus an accepted parented-comment manifest made two successful bootstraps grow comments from two to three. Published REST exposes replies through `/footer-comments/{id}/children`. | Modify | Slice 4: parent-scoped discovery and all caller migrations; production-shaped reply fake. | Known manifest parent IDs avoid unnecessary tenant-wide traversal. |
| F6 | Empty state path reaches upstream mutations before state commit fails. | BoundaryReview | Verified | Baseline parse_args accepts empty `$2`; prepare_state_and_lock derives `.lock` and a temporary directory before bootstrap network calls. | Accept | Slice 3: reject empty destination before side effects. | Regression covers every lifecycle mode. |
| F7 | Fake discards request origin, masking wrong-tenant routing. | HarnessReview | Verified | Baseline fake dispatches parsed.path without checking scheme/netloc. | Accept | Slice 3: validate exact fixture origin at the fake boundary. | Test contract defect, not a demonstrated production routing bug. |
| F8 | Raw failure assertions inspect stderr but not stdout. | HarnessReview | Verified | Baseline raw400 contract bounds/scans only output.stderr. | Accept | Slice 3: bound and redact both captured streams. | Test contract defect, not a demonstrated production stdout leak. |
| F9 | Recorded C12 second-run success lacks live no-write observation. | EvidenceReview, Main | Verified | Historical C12 row explicitly relies on deterministic tests for second-run write absence. | Accept | Slice 4: capture live request methods independently and compare stable IDs/versions before and after the second run. | Execution credentials are available locally; values are never recorded. |

Rejected review hypotheses: the embedded-marker fake page becomes visible when bootstrap creates its matching stable space ID, so the claimed invisible fixture was refuted; exact state-shape checks already cover credential-state injection; recording OpenAPI hashes is provenance and need not reject every later vendor schema revision.

### Slice 3 checkpoint — F1, F2, F6, F7, F8

- Affected contracts and restored fences: **PASS**, `cargo test -p resourcefs-sources --test atlassian_fixture_operator_contract` — 12 passed, 37.50 seconds.
- Pending falsifiers: **N/A — no additional external premise for transport/input repair**. Stress/oracle agreement: **PASS**; helper argv/environment and operator-generated files are canary-free, empty state arguments cause no request, wrong fake origins fail, and both failure streams remain bounded/redacted.
- Production-loop budget: **N/A — no new production loop**. Always-on wall budget: **N/A — one-off command**. Existing request deadlines remain unchanged.
- Named mutations: **PASS**. Restoring secret jq argv failed `secret reached helper argv`; restoring disk config failed `secret reached a temp file`; removing empty-path guards failed the zero-egress assertion; removing fake origin rejection accepted HTTP and failed its fence; echoing upstream prose to stdout failed the diagnostic bound. All mutations were restored before the 12-test green run.
- Caller/reuse audit: `api_request` keeps its signature and response globals for every lifecycle caller; parsing and credential setup remain entrypoint-only. JSON escaping reuses jq stdin rather than adding a new encoder/dependency. The direct fake helper has one test caller and executes Python explicitly to avoid transient executable-file-busy races when parallel tests create scripts.
- Verification-tool correction: sharing one Cargo target directory between worktrees reused a test executable with the isolated checkout's `CARGO_MANIFEST_DIR`. A direct binary-path check identified it; forcing the root test rebuild restored correct checkout coverage. The final green result above exercised the repaired root script.
