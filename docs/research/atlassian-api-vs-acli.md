# Native Atlassian Cloud REST APIs versus ACLI

**Question.** Which integration substrate can satisfy a ResourceFS Atlassian Portable Source: the native Jira and Confluence Cloud REST APIs, an ACLI-backed adapter, or a split choice?

**Scope.** This note compares the current official Jira Cloud REST API, Confluence Cloud REST APIs, and Atlassian Command Line Interface (ACLI) documentation against the ticket's operational criteria. It does not choose a ResourceFS resource grammar or settle the broader Atlassian domain model. Evidence is from Atlassian documentation and first-party API references retrieved 2026-08-29. “Not documented” is not treated as “not implemented”; it is a reason to require a live probe before depending on the behavior.

## Executive answer

Use a **native HTTP REST client as the production ResourceFS substrate for both products**. Jira REST v3 and Confluence REST v2/v1 expose the authoritative JSON/ADF/storage data, operation-specific permissions, pagination links, HTTP status and error bodies, and rate-limit headers that a source adapter needs. They also permit ResourceFS to own one explicit credential reference and one explicit site target per configured source.

Treat ACLI as an **optional human/CI convenience for Jira only**, not as the authoritative adapter boundary. Atlassian's published ACLI command tree documents Jira, organization administration, Rovo Dev, and feedback; it has no published `acli confluence` namespace. Jira ACLI does cover useful work-item search/view/create/edit/delete/comment commands, JSON output, ADF input, and convenience pagination, but its docs do not define a versioned JSON schema, continuation-token contract, response-header channel, or conditional-write contract. Its mutable login/switch profile, local executable dependency, six-month per-version support window, and manual update flow are a poor fit for a server-owned, multi-site source.

A native adapter still needs to normalize representations and implement ResourceFS policy: API JSON object key order is not a deterministic byte contract; ADF/storage/view are different representations; Atlassian versions and rate limits can change; and HTTP APIs do not automatically provide ResourceFS's content-derived Version Tag or commit-time revalidation. Those are adapter responsibilities, not reasons to insert ACLI.

| Criterion | Native Jira REST | Native Confluence REST | Official ACLI evidence | ResourceFS decision |
|---|---|---|---|---|
| Version and support signal | v3 is documented as latest; v2/v3 expose the same operations, with v3 ADF support. Changelog and deprecation notices are published. | v2 is the current documented API; v1 remains needed for the CQL search resource. Changelog/deprecation notices are published. | GA since 2025-05-27; official docs say each CLI version is supported only six months; changelog page currently records v1.3.15-stable (2026-03-25) and OAuth re-authorization. | Native APIs win for a long-lived adapter, with changelog/deprecation monitoring. Pin the documented operation versions and fail closed on unsupported representations. |
| Machine-readable output | Direct HTTP JSON with operation schemas, expansions, and response metadata. | Direct HTTP JSON with OpenAPI-backed v2 objects and Link/`_links`. | `--json` is documented for Jira commands, but no stable schema/versioning promise or raw-response envelope is documented. | Native. Canonicalize the selected native projection inside ResourceFS. |
| Direct read | Get issues by key/ID; fields and expansions are selectable. | Get pages by ID with body-format and metadata expansions. | Jira work-item `view --json`; no Confluence command in the published tree. | Native for both products. |
| Browse/search | Issue/project/field resources plus JQL issue search; permissions filter results. | v2 page/space/children resources plus v1 CQL search; permissions filter results. | Jira work-item search accepts JQL, fields, limit, `--paginate`; command reference has project/field/filter/board/sprint search. | Native. Keep native JQL/CQL as source semantics and bound/fail invalid queries. |
| Create/edit/delete | Issue create/edit/delete and comment operations are documented; custom fields and metadata can be requested. | Page create/update/delete; update requires the page version and body. Comment create/update/delete. | Jira create/edit/delete and bulk commands; create/edit body flags cover summary/description/common fields, not every native field. | Native. ACLI may be a separate operator tool, never the source of truth. |
| Comments | Separate paginated issue-comment list/get/add/update/delete; comment body is ADF in v3. | Footer and inline comments, children, and CRUD; comment bodies support native representations. | Jira comment create/list/update/delete; list has JSON and `--paginate`; no Confluence command. | Native. Model footer/inline/thread distinctions explicitly in projections. |
| Pagination | Offset `startAt`/`maxResults`; `total` and `isLast` are operation-dependent and can change while paging. | Cursor-based v2 pagination via `Link` and `_links.next`; v1 CQL search also returns cursor links in its current reference. | `--paginate` hides the loop; no documented ResourceFS continuation token or raw link output. | Native links are authoritative. Preserve/bound continuation in ResourceFS rather than trusting CLI aggregation. |
| Body fidelity | v3 explicitly supports ADF for issue descriptions/environment, comments, worklog comments, and textarea fields. | Page and comment APIs expose `storage`, `atlas_doc_format`, and in some reads `view`; writes select a representation. | Jira description/comment inputs accept plain text or ADF; default view fields omit comments and output representation is not a fidelity contract. | Native. Preserve an authoritative native representation and render a separate agent projection. |
| Errors and rate limits | HTTP status plus error collection; rate headers and `Retry-After` expose limit state/reason. | HTTP statuses are documented per operation; rate docs specify 429, headers, and retry behavior. | Standard statuses, human error messages, and trace IDs; `--ignore-errors` can continue bulk work. No response-header channel is documented. | Native. Map status/body/headers to stable ResourceFS errors; never use `--ignore-errors` for authoritative mutation. |
| Credential ownership | ResourceFS can resolve a token/secret or 3LO credential and send it directly to the selected site/cloud ID. | Same; basic auth and OAuth/app auth are documented. | CLI login accepts site/email/token or browser OAuth; `auth switch` mutates the selected account/site. Credential store and per-request isolation are not documented. | Native credential reference owned by ResourceFS. If ACLI is offered, run it as an explicitly isolated optional process. |
| Multiple-site targeting | Site URL (basic auth) or product/cloud ID URL (3LO); source configuration can bind one target. | Site URL for direct calls; source can bind one Confluence target. | Login/switch supports site and email, but work-item commands have no documented per-request site flag. | Native per-site client/configuration. ACLI needs a live parallel-profile/isolation probe before any use. |
| Install/update/licensing/support | No Atlassian CLI or local binary; only HTTP client and credentials. API product access/plan still applies. | Same. | OS packages/binaries for macOS/Linux/Windows and arm64/amd64; downloads use `latest`; manual update and six-month support. No ACLI-specific license text is surfaced in the cited docs. | Native minimizes deployment and update burden. Confirm legal licensing separately if distributing ACLI. |
| Deterministic testing | Local fake HTTP server can control JSON, pagination links, status, headers, and representations. | Same, including v1/v2 fixtures. | Requires a versioned external binary and CLI-owned transport/profile; no documented alternate base URL or injectable transport/schema. | Native tests can be deterministic and contract-focused. ACLI tests can only fence CLI invocation separately. |
| Live smoke | Read-only and controlled mutation smoke is feasible with a test site, account/token or OAuth app, project/space, and permissions. | Same, with a test page/space and safe cleanup. | Jira smoke requires installing a supported binary and authentication. No official Confluence smoke command is documented. | Make direct REST read-only smoke a release gate; keep ACLI Jira smoke optional and never claim Confluence ACLI coverage. |

## Evidence by decision criterion

### 1. Version, support, and change policy

Jira's REST v3 introduction calls v3 the latest version and says v2 and v3 offer the same collection of operations, while v3 adds ADF support for issue descriptions, environments, comments, worklog comments, and textarea custom fields ([Jira REST v3 introduction][jira-intro]). The introduction also says experimental features may change without notice. Jira maintains a first-party platform changelog with new features and deprecation notices ([Jira changelog][jira-changelog]). This is a usable versioned surface, not a promise that every optional field is immutable.

Confluence's current reference is REST API v2, described as an improvement over v1; the API index still exposes REST v1 and the v1 search resource is the documented CQL search operation ([Confluence REST v2 introduction][conf-v2-intro]; [Confluence REST v1 search][conf-v1-search]). Confluence also publishes a product changelog with current deprecation notices ([Confluence changelog][conf-changelog]). Therefore a Confluence adapter should not silently assume “v2 everywhere”; it should use v2 for page/space/comment operations and the documented v1 search surface where required, with separate fixtures and deprecation tracking.

ACLI's changelog records general availability in v1.1.0 on 2025-05-27 and, at the time of this research, v1.3.15-stable on 2026-03-25. That release changed OAuth scopes and required site administrators to re-authorize connected sites; the changelog explicitly says ACLI is not a Marketplace app and that administrators manually upgrade their machine ([ACLI changelog][acli-changelog]). The install guide says each CLI version is supported for only six months ([ACLI install][acli-install]). The published pages do not establish that v1.3.15 is the absolute newest binary on the research date: downloads use a `latest` URL, while the changelog page has a stated last-update date. A deployment must record the exact binary version and verify support at install time.

**Recommendation.** Prefer native API versions and monitor their product changelogs. Require a versioned ACLI binary only for an optional Jira CLI integration, not as the ResourceFS protocol boundary.

### 2. Stable machine-readable output

The native APIs are HTTP interfaces whose documented successful responses are JSON objects. Jira documents expansion and paginated response envelopes and supplies a downloadable OpenAPI document from the REST introduction ([Jira REST v3 introduction][jira-intro]). Confluence v2 supplies an OpenAPI document and documents `MultiEntityResult` objects, `_links`, and the `Link` response header ([Confluence REST v2 introduction][conf-v2-intro]; [Confluence page API][conf-page]). Native JSON is therefore machine-readable and inspectable without parsing terminal text. It is not by itself deterministic bytes: JSON member order, optional expansions, localized fields, and representation choice still need an adapter-owned canonical projection.

ACLI documents a `--json` flag for Jira search, view, create, edit, and comment commands. It also documents `--csv` and normal human-oriented output, and its output guide shows piping JSON to `jq` ([ACLI search][acli-search]; [ACLI view][acli-view]; [ACLI output guide][acli-output]). This establishes a useful success-output mode, but the official docs do not define a versioned JSON schema, an envelope that carries the original HTTP response, or a guarantee that field defaults/order remain stable across CLI updates. `--json` is evidence of serialization, not evidence of a stable ResourceFS wire contract.

**Recommendation.** Consume native response DTOs and explicitly choose a ResourceFS projection. Treat ACLI JSON as a convenience output requiring version-pinned snapshot tests, not as the authoritative schema.

### 3. Direct read, browse, and search

Jira's issue operation group documents create, get, edit, and delete issue operations, while the separate search group documents issue search. The REST introduction describes permission-filtered operations and JQL/field selection ([Jira issues][jira-issues]; [Jira issue search][jira-search]; [Jira REST v3 introduction][jira-intro]). This covers direct issue reads, project/field metadata, and JQL search without scraping Jira's web UI.

Confluence v2 documents page listing, page-by-ID reads, pages-in-space, label pages, spaces, children, descendants, and related metadata. The v1 search group documents CQL content search and cursor links ([Confluence page API][conf-page]; [Confluence children API][conf-children]; [Confluence v1 search][conf-v1-search]). The page API states that only pages visible to the authenticated user are returned, and operation scopes/permissions are shown alongside each operation.

ACLI's published Jira tree has work-item/project/field/filter/board/sprint commands. Work-item search accepts JQL, selected fields, a limit, and `--paginate`; work-item view accepts a key, selected fields, `--json`, or `--web` ([ACLI command index][acli-index]; [ACLI search][acli-search]; [ACLI view][acli-view]). The official tree has no Confluence command section. Search results from community or third-party “Confluence ACLI” tools are not evidence for official ACLI support.

**Recommendation.** Use native endpoints for both products, preserving native JQL/CQL as optional query semantics. Do not infer a Confluence ACLI adapter from an unrelated command or community project.

### 4. Create, edit, delete, and comments

Jira's issue group documents `POST` create, `GET` get, `PUT` edit, and `DELETE` issue operations ([Jira issues][jira-issues]). The issue-comments group separately documents paginated comment retrieval and add/get/update/delete comment operations ([Jira comments][jira-comments]). Native operations expose operation-specific permission requirements and OAuth scopes.

Confluence v2 documents page `POST` create, `PUT` update, and `DELETE` operations. Published pages are created by default unless status is set to draft; update requests include the page ID, status, title, body, and version object ([Confluence page API][conf-page]). The comment group documents footer and inline comment retrieval, children, create, update, and delete operations ([Confluence comments][conf-comments]). This is broader and more explicit than treating a page as one opaque string.

Jira ACLI documents create, edit, delete, and bulk work-item commands. Create/edit support summaries, projects, types, assignees, labels, descriptions, JSON input files, and `--json`; edit supports JQL/filter/key targeting and `--ignore-errors` ([ACLI create][acli-create]; [ACLI edit][acli-edit]). Comment create/list/update/delete commands are documented, including JSON output and bulk-oriented options ([ACLI comment create][acli-comment-create]; [ACLI comment list][acli-comment-list]; [ACLI command index][acli-index]). The documented flags are a useful subset, not evidence that every native Jira field, transition, property, attachment, or conditional-write feature is exposed.

There is no published official ACLI Confluence command surface. Consequently ACLI cannot satisfy the combined Jira-and-Confluence create/edit/comment destination, regardless of how capable its Jira commands are.

**Recommendation.** Implement mutations against native APIs only. Keep source grants and operation receipts above the native client; do not rely on CLI bulk “continue on error” semantics for ResourceFS writes.

### 5. Pagination and deterministic bounded reads

Jira documents offset pagination with `startAt`, `maxResults`, and often `total`/`isLast`. It warns that each operation can have a different maximum, limits may change, `total` may change between requests, and some operations omit fields. The client must therefore tolerate an empty later page and use the returned values rather than assuming a fixed maximum ([Jira REST v3 introduction][jira-intro]). This is sufficient for bounded reads, but ResourceFS must define its own result ceiling and continuation identity.

Confluence v2 uses cursor pagination. A collection request uses `limit` and optional `cursor`; the server returns a `Link` header with `rel="next"` and also places a relative next URL under `_links.next`. When there is no next page, neither is present ([Confluence REST v2 introduction][conf-v2-intro]). Page and comment operations repeat this contract. The current v1 search reference also returns `next`/`prev` cursor URLs and says to use the returned links rather than constructing them ([Confluence v1 search][conf-v1-search]).

ACLI's Jira search and comment-list references provide `--paginate`; the comment-list page says the requested limit is ignored when paginating ([ACLI search][acli-search]; [ACLI comment list][acli-comment-list]). That is a useful convenience loop but the docs do not say that the CLI returns the upstream cursor/link or a stable continuation token. It is therefore unsuitable for ResourceFS continuation and recovery identity.

**Recommendation.** Use native links/offset metadata as the source of continuation, enforce local ceilings, and test changing/empty pages. Never derive a continuation contract from ACLI's aggregated output.

### 6. Body fidelity and representation choice

Jira v3 explicitly adds ADF support for issue descriptions/environment, comments, worklog comments, and textarea custom fields; comment examples in the native operation reference show the ADF document shape (`type: doc`, `version: 1`, and content) ([Jira REST v3 introduction][jira-intro]; [Jira comments][jira-comments]). Direct REST access can request selected fields/expansions and retain the native ADF tree before rendering a text/Markdown projection.

Confluence v2 page reads accept `body-format` and expose `storage`, `atlas_doc_format`, and (for a single page) `view` bodies. Page create/update bodies select a representation, and page/comment responses carry a version object ([Confluence page API][conf-page]; [Confluence comments][conf-comments]). These representations are not interchangeable: storage XHTML, ADF, and rendered view can differ in macros, links, formatting, or unsupported constructs. A ResourceFS adapter must choose which representation is authoritative for a Version Tag and which is merely rendered output.

ACLI Jira create/edit/comment inputs accept either plain text or ADF, and work-item view can request description/comments with `--fields`; however, the documented default view fields are `key,issuetype,summary,status,assignee,description`, so comments are not included by default ([ACLI create][acli-create]; [ACLI edit][acli-edit]; [ACLI view][acli-view]). The docs do not promise byte-preservation or a stable way to request every native representation. No official Confluence ACLI body path is documented.

**Recommendation.** Preserve native ADF/storage in source-local state and expose a deterministic derived projection. Do not hash rendered Markdown or a CLI's human output as if it were authoritative content.

### 7. Errors, permissions, and rate-limit visibility

Jira's REST introduction says standard HTTP status codes apply and failed operations may return an error collection with `errorMessages`, field-keyed `errors`, and `status` ([Jira REST v3 introduction][jira-intro]). Operation references enumerate status responses and permission/scope requirements. Jira's current rate-limit guide documents HTTP 429, `Retry-After`, `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`, `X-RateLimit-NearLimit`, and `RateLimit-Reason`, including reasons for quota, burst, and per-issue write limits ([Jira rate limits][jira-rate]). A direct client can retain all of these values for stable error mapping and retry decisions.

Confluence operation references expose operation-specific HTTP statuses such as 400, 401, 404, and 413, and the v2 introduction describes permission and OAuth/Connect scope requirements ([Confluence page API][conf-page]; [Confluence REST v2 introduction][conf-v2-intro]). Confluence's rate guide documents 429 handling, `Retry-After`, `X-RateLimit-*`, `RateLimit-Reason`, and points/tier quotas ([Confluence rate limits][conf-rate]). Rate policies can depend on auth mode and product/customer tier, so the response is more authoritative than a hard-coded client estimate.

ACLI's troubleshooting guide says unexpected backend errors produce trace IDs, expected errors produce messages, standard HTTP status codes are used, and `--ignore-errors` lets a multi-entity command continue after a failure ([ACLI troubleshooting][acli-troubleshooting]). That is useful for a human, but no documented CLI flag returns upstream response headers, raw error JSON, or rate-limit reasons. A CLI command can also deliberately suppress individual failures, which conflicts with ResourceFS's need to know whether every requested mutation succeeded.

**Recommendation.** Make native status/body/header capture part of the adapter contract. For ACLI, treat nonzero exit, stderr, missing/invalid JSON, and any ignored error as failure; do not claim rate-aware retry or exact error classification without a version-specific probe.

### 8. Credential ownership and authorization

Jira documents basic auth with an email/API token for ad-hoc calls and OAuth 2.0 3LO URLs using a cloud ID; it also documents Forge/Connect app authentication ([Jira REST v3 introduction][jira-intro]; [Jira basic auth][jira-basic]). The basic-auth page says passwords are deprecated, API tokens can be individually revoked, and basic auth is recommended for simple scripts/manual calls rather than more secure app flows. It also warns that a distributed app collecting customer API tokens or asking each customer to create an individual 3LO app does not meet the cited cloud-app security requirements ([Jira basic auth][jira-basic]).

Confluence v2 documents basic auth for direct REST calls and OAuth/JWT options for apps, with authorization based on the authenticated user or app scopes ([Confluence REST v2 introduction][conf-v2-intro]). Both products expose per-operation scopes and product permissions; authentication does not bypass Jira project/issue or Confluence space/page visibility.

ACLI documents API-token login by reading a token from stdin, browser OAuth login, and (for admin commands) API-key authentication. Jira login requires a site and email; `auth switch` selects a site/email account ([ACLI getting started][acli-start]; [ACLI login][acli-login]; [ACLI switch][acli-switch]). These operations establish that credentials and target selection belong to the CLI's profile mechanism. The cited docs do not specify the profile storage location, encryption/keychain behavior, or a request-scoped credential object that ResourceFS could own and audit.

**Recommendation.** ResourceFS should own a secret reference and the selected Atlassian site/cloud ID, send credentials through a native client, and redact them from logs. ACLI may consume an explicitly provisioned bot token in CI, but its login profile must not become an implicit ResourceFS credential store.

### 9. Multiple-site targeting and isolation

Native direct requests identify a site in the host URL for basic-auth calls; Jira's 3LO form identifies the product and cloud ID in the `api.atlassian.com/ex/...` path ([Jira REST v3 introduction][jira-intro]). The analogous Confluence operation URLs are site-relative for direct calls and can be addressed through the authenticated app/OAuth context ([Confluence REST v2 introduction][conf-v2-intro]). This lets ResourceFS compile one client/configuration per named site and reject cross-site identity confusion before a request.

ACLI supports selecting a site at login and switching among saved site/email combinations ([ACLI login][acli-login]; [ACLI switch][acli-switch]). The documented work-item commands do not accept a site flag on each operation; they use the selected account. That is adequate for interactive use but creates a process/profile isolation question for a long-lived multi-site server. Whether separate config directories, environment variables, or concurrent invocations are safe is not documented here.

**Recommendation.** Use independent native clients and credential references per site. If ACLI is ever invoked, require an isolated process/profile per site and prove that behavior in a live probe before enabling concurrent sites.

### 10. Installation, update, licensing, and support burden

A native REST integration has no Atlassian CLI executable to install or update; its deployment burden is the HTTP/TLS client, ResourceFS binary, and an operator credential. Atlassian product/API access and customer plan restrictions still apply, but there is no separate local ACLI runtime lifecycle.

ACLI officially publishes macOS, Linux, and Windows packages for arm64 and amd64. The package page provides `latest` tarball/deb/rpm/executable links and says each version is supported for six months ([ACLI downloads][acli-downloads]). The CI guide downloads the latest Linux binary at runtime and authenticates a bot account with an API token ([ACLI CI][acli-ci]). This is feasible for a human/CI helper but undesirable as an unpinned production dependency; the exact version, checksum, and support date must be recorded by the deployment.

The ACLI changelog says it is not a Marketplace app, so updates and OAuth re-authorization are operator actions. The cited official docs do not state an ACLI-specific license or Marketplace entitlement. Do not infer an open-source or permissive license from binary availability; obtain legal terms from Atlassian if ResourceFS distributes or bundles it.

**Recommendation.** Avoid making ACLI a required runtime dependency. If retained as an optional helper, pin the binary, check its six-month support window, and document manual update/re-authorization and licensing ownership.

### 11. Deterministic testing

Native APIs provide the right seam for deterministic tests: a local fake HTTP server can return documented success shapes, ADF/storage bodies, pagination links, 400/401/403/404/409/413/429/5xx responses, and rate headers. Tests can verify request paths, selected fields/expansions, body representation, continuation handling, redaction, and ResourceFS error mapping without an Atlassian account. A separate authenticated smoke suite can prove that fixture permissions and product plan behavior match the fake contract.

ACLI deterministic tests would need to execute a particular external binary, establish CLI auth/profile state, and assert stdout/stderr/exit code. The official docs do not expose an alternate API base URL, injectable HTTP transport, stable response schema, or a test account fixture. A fake `acli` executable can test ResourceFS subprocess handling, but it cannot prove Atlassian API semantics. Version pinning reduces drift; it does not supply the missing transport contract.

**Recommendation.** Make native HTTP fake-upstream tests the primary contract suite. Fence ACLI with a small, separately versioned subprocess smoke only if the optional helper is shipped.

### 12. Live-smoke feasibility and missing evidence

A direct native read-only smoke is feasible with a provisioned Atlassian Cloud test site, a Jira project and Confluence space/page, an API token or one approved OAuth app, and permissions matching the intended source grants. The smoke should read an issue/page, list a bounded collection, run JQL/CQL search, request ADF/storage, follow one continuation link, and deliberately observe a permission denial and a rate/error response where safe. Mutating smoke should use disposable fixtures and verify version/body behavior before cleanup.

A Jira ACLI smoke is feasible only after installing a supported exact binary and authenticating through token stdin or browser OAuth. It should compare `view --json`, `search --json --paginate`, create/edit/comment, and error/trace output with direct REST results. There is no official Confluence ACLI command documented to smoke. This research environment had no `acli` executable and no authenticated Atlassian tenant, so no live command or API behavior was claimed.

Required live probes before any optional ACLI adapter are:

1. `acli --version` and every relevant `--help` page from the exact pinned binary, especially `acli confluence --help`.
2. Whether `--json` output includes stable fields, continuation metadata, generated IDs, and version values across two releases.
3. Whether CLI failures preserve exit status, stderr, trace IDs, HTTP 429 headers/retry delay, and partial-batch outcomes.
4. Whether an explicit site can be selected per process without `auth switch` races or cross-site credential leakage.
5. Whether Jira custom fields, ADF comments, page-like rich content, and optimistic/conditional edits round-trip without conversion.
6. Whether Atlassian support/legal terms permit distributing the selected CLI binary and relying on it after its six-month support window.

## Implications for ResourceFS

- Keep the source boundary at a native HTTP client for both Jira and Confluence. This preserves response status, headers, pagination links, native bodies, operation scopes, and source-specific error details.
- Treat Jira and Confluence as separate API surfaces even when one configured site hosts both. Jira v3 issue/comment data and Confluence v2 page/comment data have different IDs, versions, bodies, and pagination semantics.
- Store one explicit site/cloud target and one operator-owned secret reference per source. Do not let a mutable ACLI account-switch profile decide which site a ResourceFS request reaches.
- Separate authoritative native content from rendered agent text. ADF, Confluence storage, and view bodies should not be silently interchanged; deterministic serialization and content-derived Version Tags belong in the source adapter.
- Preserve native continuation metadata while enforcing ResourceFS result/output ceilings. Jira offset metadata and Confluence cursor URLs must not be replaced by ACLI's hidden aggregation loop.
- Map HTTP status, native error body, scopes/permissions, and rate headers into stable ResourceFS errors. Retry only according to captured source metadata and operation idempotence; never turn ACLI `--ignore-errors` into successful ResourceFS state.
- Build fake-upstream contract fixtures first, then add read-only authenticated smokes. ACLI can be tested as an optional subprocess helper but must not be required for either product or for deterministic tests.

## Unresolved/live-probe questions

- What exact native REST operation/version set will remain required as Jira v3 and Confluence v2/v1 changelogs deprecate fields or search surfaces?
- Which native body representation is authoritative for each ResourceFS Version Tag: Jira ADF versus selected rendered fields, and Confluence storage versus ADF? How will macros/unsupported nodes be rendered without losing the source body?
- Do the chosen Jira and Confluence operations expose ETags or other conditional headers consistently enough for adapter revalidation, and what exact stale-write behavior occurs on concurrent edits?
- What are the concrete per-operation maximums and rate-limit headers on the target tenant and auth mode? The docs state policies, but a live tenant is needed to confirm fixture behavior and plan-specific limits.
- Does the exact ACLI binary include any undocumented Confluence namespace or endpoint override? The published command index does not; only an exact-binary `--help` probe can answer the implementation question.
- If ACLI remains optional, where are its credentials stored, can profile selection be process-isolated, and can a pinned binary be redistributed under an acceptable license/support arrangement?
- What cleanup and unknown-outcome procedure is safe for live mutation smokes, especially when a request times out after the server may have committed?

No newly discovered implementation bug is asserted by this research. The material risk is a substrate/design candidate: using ACLI's hidden profile, aggregated pagination, or human error output as if it were a stable native source contract.

## Primary-source references

### Native Jira Cloud REST API

- [Jira Cloud REST API v3 introduction: version, ADF, auth, pagination, status codes](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/)
- [Jira issue operations: create/get/edit/delete](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/)
- [Jira issue search and JQL](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/)
- [Jira issue comments: get/add/update/delete](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/)
- [Jira Cloud rate limiting](https://developer.atlassian.com/cloud/jira/platform/rate-limiting/)
- [Jira Cloud platform changelog](https://developer.atlassian.com/cloud/jira/platform/changelog/)
- [Basic auth for Jira Cloud REST APIs](https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/)
- [Atlassian Document Format structure](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/)

### Native Confluence Cloud REST API

- [Confluence Cloud REST API v2 introduction](https://developer.atlassian.com/cloud/confluence/rest/v2/intro/)
- [Confluence v2 page operations: list/get/create/update/delete](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/)
- [Confluence v2 comment operations: footer/inline/children CRUD](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-comment/)
- [Confluence v2 children/descendants operations](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-children/)
- [Confluence v1 CQL search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/)
- [Confluence advanced searching using CQL](https://developer.atlassian.com/cloud/confluence/advanced-searching-using-cql/)
- [Confluence Cloud rate limiting](https://developer.atlassian.com/cloud/confluence/rate-limiting/)
- [Confluence Cloud changelog](https://developer.atlassian.com/cloud/confluence/changelog/)

### Official Atlassian CLI (ACLI)

- [ACLI introduction](https://developer.atlassian.com/cloud/acli/guides/introduction/)
- [Official ACLI command reference/index](https://developer.atlassian.com/cloud/acli/reference/commands/)
- [Jira work-item search](https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-search/)
- [Jira work-item view](https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-view/)
- [Jira work-item create](https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-create/)
- [Jira work-item edit](https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-edit/)
- [Jira work-item comment create](https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-comment-create/)
- [Jira work-item comment list](https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-comment-list/)
- [Jira auth login](https://developer.atlassian.com/cloud/acli/reference/commands/jira-auth-login/)
- [Jira auth switch](https://developer.atlassian.com/cloud/acli/reference/commands/jira-auth-switch/)
- [ACLI getting started/authentication](https://developer.atlassian.com/cloud/acli/guides/how-to-get-started/)
- [ACLI troubleshooting: JSON, errors, trace IDs, status codes](https://developer.atlassian.com/cloud/acli/guides/troubleshooting-guide/)
- [ACLI output redirection and `--json`](https://developer.atlassian.com/cloud/acli/guides/manage-command-chaining-and-output-redirection/)
- [ACLI installation/support window](https://developer.atlassian.com/cloud/acli/guides/install-acli/)
- [ACLI supported package downloads](https://developer.atlassian.com/cloud/acli/guides/download-supported-packages/)
- [ACLI use on CI](https://developer.atlassian.com/cloud/acli/guides/use-acli-on-ci/)
- [ACLI changelog](https://developer.atlassian.com/cloud/acli/changelog/)

[jira-intro]: https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/
[jira-changelog]: https://developer.atlassian.com/cloud/jira/platform/changelog/
[jira-issues]: https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/
[jira-search]: https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/
[jira-comments]: https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/
[jira-rate]: https://developer.atlassian.com/cloud/jira/platform/rate-limiting/
[jira-basic]: https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/
[conf-v2-intro]: https://developer.atlassian.com/cloud/confluence/rest/v2/intro/
[conf-page]: https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/
[conf-comments]: https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-comment/
[conf-children]: https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-children/
[conf-v1-search]: https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/
[conf-rate]: https://developer.atlassian.com/cloud/confluence/rate-limiting/
[conf-changelog]: https://developer.atlassian.com/cloud/confluence/changelog/
[acli-index]: https://developer.atlassian.com/cloud/acli/reference/commands/
[acli-search]: https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-search/
[acli-view]: https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-view/
[acli-create]: https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-create/
[acli-edit]: https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-edit/
[acli-comment-create]: https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-comment-create/
[acli-comment-list]: https://developer.atlassian.com/cloud/acli/reference/commands/jira-workitem-comment-list/
[acli-login]: https://developer.atlassian.com/cloud/acli/reference/commands/jira-auth-login/
[acli-switch]: https://developer.atlassian.com/cloud/acli/reference/commands/jira-auth-switch/
[acli-start]: https://developer.atlassian.com/cloud/acli/guides/how-to-get-started/
[acli-output]: https://developer.atlassian.com/cloud/acli/guides/manage-command-chaining-and-output-redirection/
[acli-troubleshooting]: https://developer.atlassian.com/cloud/acli/guides/troubleshooting-guide/
[acli-install]: https://developer.atlassian.com/cloud/acli/guides/install-acli/
[acli-downloads]: https://developer.atlassian.com/cloud/acli/guides/download-supported-packages/
[acli-ci]: https://developer.atlassian.com/cloud/acli/guides/use-acli-on-ci/
[acli-changelog]: https://developer.atlassian.com/cloud/acli/changelog/
