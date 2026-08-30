# Jira Cloud API contracts for ResourceFS

## Question and scope

This note records the current, documented contracts that matter for a Jira Cloud source adapter: direct issue reads; project and issue browsing; JQL search; field and create/edit metadata; Atlassian Document Format (ADF); comments; issue and comment mutations; pagination; validation and concurrency signals; authentication and permissions; errors and rate limits; API versions and deprecations; and bounded bulk/asynchronous operations.

This is an evidence note, not an implementation plan. It deliberately separates what Atlassian documents from behavior that must be checked against the target tenant. Confluence, Jira Data Center, and the Jira UI are out of scope.

## Executive answer

- **Use explicit Jira Cloud REST API v3.** Atlassian calls v3 the latest version. v2 and v3 expose the same operation set, but v3 adds ADF support for comment bodies, worklog comments, issue `description`/`environment`, and multiline custom fields. Prefer `/rest/api/3/...`, not an implicit `latest` alias. [REST API v3 introduction](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#version)
- **Use IDs as canonical joins and retain keys for display.** An issue accepts an `issueIdOrKey` and returns `id`, `key`, and `self`; a project accepts `projectIdOrKey`; a comment has its own `id`. Jira may resolve an old/moved or differently-cased issue key and returns the key of the issue it found. Do not assume the input key is the returned canonical key. [Get issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-get), [Get project](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/#api-rest-api-3-project-projectidorkey-get), [Get comment](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/#api-rest-api-3-issue-issueidorkey-comment-id-get)
- **Use cursor pagination for enhanced JQL and offset/link pagination elsewhere.** `GET`/`POST /rest/api/3/search/jql` expose `nextPageToken` and `isLast`; the older `/rest/api/3/search` GET/POST pair is marked “currently being removed.” Project search and comment lists use `startAt`/`maxResults` and paging metadata, while the generic API contract warns that the returned `maxResults` and `total` are not immutable. [Issue search](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/), [Search and reconcile](https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/), [Pagination introduction](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#pagination)
- **Search is eventually consistent by default.** After a write, enhanced JQL search can be stale for seconds or minutes. `reconcileIssues` strengthens read-after-write only for the listed issue IDs. An edit must request `returnIssue=true` if the caller needs the updated issue ID to reconcile. [Search and reconcile](https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/)
- **Issue mutation is field/update based, not a transactional document replacement.** Create and edit requests use `fields` and/or `update`; issue edit does not transition an issue (a requested transition is ignored). Create/edit metadata is the source for fields visible or editable to the calling principal, but the edit endpoint itself does not check screen configuration. [Create issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-post), [Edit issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-put), [Edit metadata](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-editmeta-get)
- **No documented HTTP compare-and-swap validator was found for issue/comment edits.** The v3 operation documentation exposes no `If-Match`, `ETag`, `If-Unmodified-Since`, or expected-version request contract. Issue edit documents a possible `409 Conflict`, but does not define its cause. Treat `updated` timestamps and a successful response as observations, not atomic preconditions; verify tenant behavior before calling a `409` a version conflict. [Edit issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-put), [Update comment](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/#api-rest-api-3-issue-issueidorkey-comment-id-put)
- **Bulk operations are bounded but may be partial.** Bulk create accepts up to 50 issues and can return both created issues and per-request errors in a `201` response. Bulk fetch accepts 100 issues by default and up to 1000 only when the request uses explicitly selected, single-value fields and restricted expands are absent; it can return both issues and issue errors. [Bulk create](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-bulk-post), [Bulk fetch](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-bulkfetch-post), [Jira Cloud changelog](https://developer.atlassian.com/cloud/jira/platform/changelog/)
- **ADF is structured JSON, not a string.** ADF has a root `doc` node with `version` and `content`; nodes use `type`, optional `content`, `marks`, and `attrs`. Jira v3 uses it for comments and issue rich-text fields. The official schema pointer is `http://go.atlassian.com/adf-json-schema`; the list of schema-valid nodes is not necessarily the list accepted by each product field. [ADF structure](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/), [ADF schema](http://go.atlassian.com/adf-json-schema)
- **Authentication and visibility are both required.** Basic authentication uses an Atlassian account email and API token (password authentication is deprecated); Atlassian recommends OAuth 2.0 for integrations. OAuth 3LO calls use the cloud-ID URL form. The effective permissions of the user/app still gate every operation and issue-level security can filter results. [Authentication](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#authentication), [Basic auth](https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/), [Permissions](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#permissions)
- **Rate limits have three independent dimensions in the current Cloud contract:** hourly points quota, per-endpoint burst rate, and per-issue write rate. A `429` carries retry guidance (`Retry-After`) and a `RateLimit-Reason`; clients should use bounded exponential backoff with jitter and preserve the reason. [Rate limiting](https://developer.atlassian.com/cloud/jira/platform/rate-limiting/)

## Evidence by decision criterion

### 1. Canonical resources and identifiers

| Resource | Accepted identifier | Documented response identity | Contract consequence |
| --- | --- | --- | --- |
| Issue | `issueIdOrKey` string: numeric issue ID or issue key | `id` (string in examples), `key`, `self`; fields under `fields` | Persist Jira issue `id` for joins/reconciliation and retain the returned `key` for display. Jira performs a case-insensitive search and checks moved issues; it returns the found issue without a redirect. |
| Project | `projectIdOrKey` string | `id`, `key`, `self`, `name`, `style`, `simplified` and other details | Use project ID for durable joins and key for user-facing JQL/display. |
| Issue type | `issueTypeId` string in create metadata | `id`, `name`, `self`, `subtask` | Create field metadata is project + issue-type specific; do not use only the display name. |
| Field | System field ID/key (for example `summary`) or custom field ID/key (for example `customfield_10101`) | `id`, often `key`, `name`, `schema`, `clauseNames`, capability flags | Resolve metadata to the field ID/key and preserve the schema; field names are not unique enough to be a machine key. |
| Comment | `id` string under an issue | `id`, `self`, `author`, `updateAuthor`, `created`, `updated`, `body`, optional `visibility` | A comment is a child resource; an issue ID/key alone is not enough to address an edit. |
| ADF document | Root `type: "doc"`, `version: 1`; node types and attributes | Nested JSON object | Treat the value as typed JSON and preserve unknown data unless the target operation rejects it. |

The direct issue response example shows the identity tuple (`id`, `key`, `self`) and a project object with its own `id`, `key`, and `self`. The same issue operation explicitly says that a moved/case-insensitive lookup returns the found issue's key. [Get issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-get)

### 2. Direct reads and bounded issue fetches

**Single issue.** `GET /rest/api/3/issue/{issueIdOrKey}` returns an `IssueBean`. The caller can select `fields`, `properties`, `expand`, `fieldsByKeys`, `updateHistory`, and `failFast`. It is documented as anonymously accessible, but authenticated results still require *Browse projects* and issue-level security permission when configured. [Get issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-get)

**Bulk issue read.** `POST /rest/api/3/issue/bulkfetch` requires `issueIdsOrKeys` and accepts `fields`, `fieldsByKeys`, `properties`, and `expand`. The current contract is:

- default maximum: 100 issues per request;
- maximum 1000 only if at least one field is explicitly included, no more than 100 fields are included, no included field is multi-valued (`comment`, `worklog`, and `attachment` are examples), and restricted expands (`changelog`, `editmeta`, `operations`, `renderedFields`, `transitions`, and `versionedRepresentations`) are absent;
- a request exceeding the applicable bound is rejected with `400`;
- returned issues are in ascending `id` order;
- a successful response may contain both `issues` and `issueErrors`, so `200` is not an all-items-success assertion.

The 1000-item shape was also announced in the Jira Cloud changelog; the changelog says explicit fields, at most 100 fields, single-value fields, and restricted expands are required. [Bulk fetch issues](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-bulkfetch-post), [Jira Cloud changelog](https://developer.atlassian.com/cloud/jira/platform/changelog/)

### 3. Project browsing

`GET /rest/api/3/project/search` is the paginated project browser. It supports `startAt`, `maxResults`, `orderBy`, `query`, `keys`, `id`, `typeKey`, `categoryId`, and `action`. The documented response is a page object with:

```text
isLast, maxResults, nextPage, self, startAt, total, values[]
```

The `values` entries contain project IDs and keys plus project details. Returned projects are limited by *Browse Projects*, *Administer Projects*, or *Administer Jira* as described by the endpoint. [Get projects paginated](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/#api-rest-api-3-project-search-get)

`GET /rest/api/3/project/{projectIdOrKey}` returns project details and requires *Browse projects* (subject to issue/project visibility). [Get project](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/#api-rest-api-3-project-projectidorkey-get)

The older `GET /rest/api/3/project` endpoint returns all visible projects without pagination and is explicitly deprecated; Atlassian directs clients to project search for search and pagination. [Deprecated get all projects](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/#api-rest-api-3-project-get)

### 4. JQL search, pagination, and consistency

Use `GET` or `POST /rest/api/3/search/jql` (enhanced JQL search). The POST request body documents `jql`, `maxResults`, `nextPageToken`, `fields`, `fieldsByKeys`, `expand`, `properties`, and `reconcileIssues`. The GET form has the same query concepts, but POST is the safe choice for a large JQL expression. [Enhanced JQL GET](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/#api-rest-api-3-search-jql-get), [Enhanced JQL POST](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/#api-rest-api-3-search-jql-post)

The enhanced response is `SearchAndReconcileResults` with `issues[]` and `isLast`; the search-and-reconcile guide shows `nextPageToken` in a response when another page exists. The token is a string and has no documented arithmetic/offset semantics, so ResourceFS should treat it as an opaque continuation value and stop on `isLast` (or the absence of a continuation token). [Issue search](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/), [Search and reconcile example](https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/)

The older `GET` and `POST /rest/api/3/search` operations are each marked **Deprecated — Currently being removed**, with a link to Atlassian change `CHANGE-2046`. Their old response shape uses `startAt`, `maxResults`, `total`, `issues`, and `warningMessages`; this is not the target contract for new code. [Deprecated JQL GET](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/#api-rest-api-3-search-get), [Deprecated JQL POST](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/#api-rest-api-3-search-post)

The search-and-reconcile guide is explicit that enhanced JQL is not read-after-write consistent by default. The delay can be seconds to minutes and can be longer for changes affecting many issues. Supplying `reconcileIssues` causes only the named issue IDs to receive stronger consistency. For an edit, request `returnIssue=true` to obtain the ID to put into that array. This is a consistency aid, not a general transaction or snapshot guarantee. [Search and reconcile](https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/)

### 5. Field metadata and create/edit metadata

**Global field catalog.** `GET /rest/api/3/field` returns an array of `FieldDetails`. The documented shape includes `id`, optional `key`, `name`, `untranslatedName`, `clauseNames`, `custom`, `navigable`, `orderable`, `searchable`, and a `schema` object. The endpoint applies visibility rules, including configuration/screen association and *Browse Projects* for ordinary fields. [Get fields](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-fields/#api-rest-api-3-field-get)

**Paginated field catalog.** `GET /rest/api/3/field/search` is the paginated field browser for Classic Jira projects. It can filter by `type`, field `id`, and a name/description `query`, and supports `startAt`, `maxResults`, `orderBy`, and `expand`. It is not a direct per-issue editability answer. [Get fields paginated](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-fields/#api-rest-api-3-field-search-get)

**Create metadata.** The global `GET /rest/api/3/issue/createmeta` operation is deprecated. It returns projects, issue types, and optionally create-screen fields, and invalid project/type combinations do not produce errors. Atlassian's replacement is project/type scoped:

- `GET /rest/api/3/issue/createmeta/{projectIdOrKey}/issuetypes` returns a page with `issueTypes`, `maxResults`, `startAt`, and `total`;
- `GET /rest/api/3/issue/createmeta/{projectIdOrKey}/issuetypes/{issueTypeId}` returns a page with `fields`, `maxResults`, `startAt`, and `total`;
- field metadata contains at least `fieldId`, `key`, `name`, `operations`, `required`, and `hasDefaultValue`, with allowed values where applicable.

These endpoints require *Create issues* in the requested project, even though the reference pages state that the operations can be accessed anonymously when the corresponding permission is granted publicly. [Deprecated create metadata](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-createmeta-get), [Create metadata issue types](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-createmeta-projectidorkey-issuetypes-get), [Create metadata fields](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-createmeta-projectidorkey-issuetypes-issuetypeid-get), [Create metadata deprecation notice](https://developer.atlassian.com/cloud/jira/platform/changelog/#CHANGE-1304)

**Edit metadata.** `GET /rest/api/3/issue/{issueIdOrKey}/editmeta` returns the fields visible and editable to the caller. The reference lists screen availability, field configuration visibility, field-specific display conditions, custom-field context, project/type/status presence, valid workflow assignment, editable workflow step, *Edit issues*, and workflow field permissions. The endpoint has two privileged overrides: `overrideScreenSecurity` bypasses screen/field-configuration checks, and `overrideEditableFlag` bypasses workflow-presence/editability checks. The edit operation itself does not check screen configurations, so metadata is a capability discovery aid rather than a complete write authorization proof. [Get edit metadata](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-editmeta), [Edit issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-put)

### 6. ADF rich-text contract

The official ADF document describes ADF as JSON for rich text stored in Atlassian products. In Jira Cloud, comments and multiline text custom fields are ADF; v3 also uses ADF for issue `description` and `environment`. The root is a `doc` node with `version` and `content`. Nodes have a required `type`; block nodes have `content`; the root has `version`; optional `marks` and `attrs` carry formatting and node-specific data. [ADF structure](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/), [v3 ADF coverage](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#version)

The docs link the canonical JSON Schema at `http://go.atlassian.com/adf-json-schema`. The ADF page warns that nodes/marks present in the schema might not be valid in a particular implementation; therefore schema validation is necessary but not sufficient for a given Jira field. The implementation must preserve the distinction between a plain string field (for example a single-line custom field) and an ADF field. [ADF structure and schema pointer](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/)

Jira exposes an experimental `GET /rest/api/3/issue/limit/adf/report` that reports ADF byte-size breaches for comment, worklog, custom-field, description, and environment ADF. The current response example exposes a `limits` value of `1048576` for those field types and reports the number of breaching entities per issue. Treat this as a current reported limit, not an eternal constant: the endpoint is explicitly experimental and the v3 introduction says experimental features may change without notice. Add/comment mutation documentation also advertises `413 Request Entity Too Large`. [ADF limit report](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-limit-adf-report-get), [Add comment responses](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/#api-rest-api-3-issue-issueidorkey-comment-post), [Experimental features](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#experimental)

### 7. Comments: reads, visibility, and mutations

**List comments.** `GET /rest/api/3/issue/{issueIdOrKey}/comment` returns `PageOfComments`. Request parameters are `startAt`, `maxResults`, `orderBy`, and `expand`; the documented response has `comments[]`, `maxResults`, `startAt`, and `total`. The example does not include `isLast`, so use the endpoint's actual paging metadata and the generic pagination warning rather than requiring an `isLast` field. Comments are filtered by Browse Projects, issue security, and comment visibility. [Get comments](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/#api-rest-api-3-issue-issueidorkey-comment-get)

**Fetch comments by IDs.** `POST /rest/api/3/comment/list` accepts an ID list and returns a paginated `values[]` response with `isLast`, `maxResults`, `startAt`, and `total`. This is useful for bounded fan-in when the caller already has comment IDs. [Get comments by IDs](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/#api-rest-api-3-comment-list-post)

**Add.** `POST /rest/api/3/issue/{issueIdOrKey}/comment` accepts an ADF-capable `body`, optional `visibility`, and optional `properties`; successful creation returns `201` and a `Comment` object with ID, authors, timestamps, body, and visibility. Required project permissions are *Browse projects* and *Add comments*, plus issue-level security visibility where configured. [Add comment](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/#api-rest-api-3-issue-issueidorkey-comment-post)

**Update.** `PUT /rest/api/3/issue/{issueIdOrKey}/comment/{id}` accepts `body`, `visibility`, and `properties`, returning `200` with the updated comment. The caller needs *Browse projects* and either *Edit all comments* or *Edit own comments*. Child comments inherit visibility; attempting to update a child comment's visibility is documented to return `400`. [Update comment](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/#api-rest-api-3-issue-issueidorkey-comment-id-put)

No comment endpoint documents an ETag/conditional header, comment revision number, or compare-and-swap token. `created`/`updated` and `updateAuthor` are audit observations, not a write precondition. This absence is based on the current v3 operation reference and must be live-probed before relying on a conflict classification. [Issue comments reference](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/)

### 8. Issue creation and editing

**Create one.** `POST /rest/api/3/issue` creates an issue or permitted subtask. Request fields include `fields`, `update`, `properties`, optional `transition`, and `historyMetadata`; `updateHistory` is a query parameter. Fields are constrained by create metadata. A subtask requires a subtask issue type and a parent ID/key; in a next-gen project, a child must be in the same project as its parent. `description`, `environment`, and multiline custom fields use ADF; single-line text fields use strings. Success is `201 Created` with a created issue identity. Required permissions are *Browse projects* and *Create issues*. [Create issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-post)

**Bulk create.** `POST /rest/api/3/issue/bulk` accepts `issueUpdates[]` for up to 50 issues/subtasks. The reference explicitly says a request can partially succeed: `201` is returned if any creation succeeds, and the `CreatedIssues` result contains details of created issues and failures. Failures include missing required fields, invalid values, fields unavailable for the type, insufficient permissions, invalid parent/project relationships, disabled subtasks, and other invalid conditions. A caller must not interpret `201` as all input items having been created. [Bulk create issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-bulk-post)

**Edit one.** `PUT /rest/api/3/issue/{issueIdOrKey}` accepts `fields`, `update`, `properties`, and optional query flags `notifyUsers`, `overrideScreenSecurity`, `overrideEditableFlag`, `returnIssue`, and `expand`. A transition is not supported and is ignored; use the transition operation separately. Without `returnIssue`, success is `204 No Content`; with it, success is `200` and the response body is an issue. The reference lists `400`, `401`, `403`, `404`, `409`, and `422` outcomes. It specifically documents `409 Conflict` but does not define a stale-version or concurrency algorithm. [Edit issue](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-put)

There is no documented standard bulk-edit endpoint in the bounded issue CRUD contract above. Bulk create and bulk fetch have explicit item bounds; an integration that needs many edits must either use the separately documented Jira bulk operation it actually needs or make individually authorized edits and surface per-item outcomes. Do not infer atomicity from HTTP method or from a batch-like request shape.

### 9. Pagination and ordering guarantees

The Jira REST introduction defines the common page pattern as `startAt`, `maxResults`, `total`, `isLast`, and `values`. It defines `startAt` as the first item index and `maxResults` as the maximum page size. Limits differ by operation and may change without notice; Atlassian recommends requesting a large value and using the returned `maxResults` to discover the effective maximum. `total` may change while the client requests later pages, and a requested page can therefore be empty. `isLast` is not returned by every operation. Ordering is operation-specific; when supported, `orderBy` uses `+`/`-` direction. [Expansion, pagination, ordering](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#expansion)

This yields three distinct adapter rules:

1. For project and comments, carry the returned offset metadata and stop according to the fields that operation actually returns; do not synthesize a stable total.
2. For enhanced JQL, carry the opaque `nextPageToken` and use `isLast`; do not send `startAt` as a substitute.
3. For every page, permit an empty page and duplicate/missing records caused by a moving result set unless the operation's own contract provides stronger reconciliation.

### 10. Authentication, scopes, permissions, and anonymity

The v3 introduction documents these base URL forms:

- Forge calls: `/rest/api/3/<resource>` through the Forge bridge/runtime;
- Connect calls: `https://<site-url>/rest/api/3/<resource>`;
- OAuth 2.0 3LO calls: `https://api.atlassian.com/ex/jira/<cloudId>/rest/api/3/<resource>`;
- Basic-auth ad-hoc calls: `https://<site-url>/rest/api/3/<resource>`.

The basic-auth guide requires an Atlassian account email and API token, says password authentication is deprecated, and recommends Basic auth only for simple scripts/manual calls. Atlassian recommends OAuth 2.0 for integrations and warns against collecting customers' individual tokens or asking customers to create custom 3LO apps. [REST authentication](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#authentication), [Basic auth](https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/)

The exact OAuth scopes vary by operation. Current reference examples include:

- enhanced JQL read: classic `read:jira-work`; granular scopes such as `read:issue-details:jira` and related metadata scopes;
- create issue: classic `write:jira-work`; granular `write:issue:jira`, `write:comment:jira`, `write:attachment:jira`, and `read:issue:jira` as applicable;
- edit issue: classic `write:jira-work`; granular `write:issue:jira`;
- comments: classic `read:jira-work`/`write:jira-work`; granular comment, project, group, and project-role scopes according to the operation.

The endpoint's own scope list is authoritative for a chosen auth product and tenant. A token's OAuth scopes do not grant Jira project permissions by themselves. [Jira OAuth scopes](https://developer.atlassian.com/cloud/jira/platform/scopes-for-oauth-2-3LO-and-forge-apps/), [Create issue scopes](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-post), [Edit issue scopes](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-put), [Comment scopes](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/)

Operation permissions are separate from scopes:

| Operation | Documented effective permissions |
| --- | --- |
| Direct issue read / JQL result visibility | *Browse projects* and, if configured, issue-level security permission |
| Project browse | *Browse Projects*, or the documented administrative alternatives for project search |
| Create issue | *Browse projects* and *Create issues* |
| Edit issue | *Browse projects* and *Edit issues*; issue-level security must allow access |
| List comments | Browse project, issue-level security, and comment visibility |
| Add comment | *Browse projects* and *Add comments* |
| Update comment | *Browse projects* and *Edit all comments* or *Edit own comments* |

“Can be accessed anonymously” on a reference page means that the operation supports anonymous requests when the required Jira permission is granted to the Public group; it does not bypass project, issue-security, or comment-visibility checks. [Permissions and anonymous access](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#permissions), [Anonymous operations](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#anonymous-operations)

### 11. Errors and validation signals

The common Jira error body is an `Error Collection` object:

```text
errorMessages: string[]
errors: object mapping field/identifier to string
status: integer (optional in the schema)
```

The introduction says operations can return standard HTTP status codes with a body containing details of one or more errors. ResourceFS should preserve both the HTTP status and structured field/global messages; a plain status is insufficient to explain invalid fields or permission configuration. [Status codes and error schema](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#status-codes)

Relevant documented outcomes include:

- direct issue read: `200`, `401`, `404`;
- JQL search: `200`, `400`, `401` (with JQL validation and permission failures represented by the error response);
- project search: `200`, `400`, `401`, `403`;
- issue create: `201`, `400`, `401`, `403`, `422`;
- issue edit: `200` or `204` on success depending on `returnIssue`, plus `400`, `401`, `403`, `404`, `409`, `422`;
- list comments: `200`, `400`, `401`, `404`;
- add comment: `201`, `400`, `401`, `404`, `413`;
- update comment: `200`, `400`, `401`, `404`.

These are documented possibilities, not a promise that every tenant or every failure mode emits exactly one status. Preserve unknown statuses as transport/source errors and record the server's error body. [Issue operations](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/), [Issue search](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/), [Project operations](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/), [Comment operations](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/)

### 12. Rate limits and retry signals

The current Jira Cloud rate-limit documentation describes three independent systems:

1. an hourly points quota based on request work/object complexity;
2. a per-tenant, per-endpoint burst rate limit;
3. a per-issue write limit.

The page says the points-based and tiered app quotas apply to Forge, Connect, and OAuth 2.0 (3LO) apps from the announced enforcement date; API-token traffic is not affected by that new points quota and continues under existing burst limits. It describes a default global pool of 65,000 points/hour, with reviewed per-tenant pools varying by edition and user count. Writes are charged the base point in the published model; exact object costs and policies can change. [Rate-limiting overview](https://developer.atlassian.com/cloud/jira/platform/rate-limiting/)

All three systems return `429 Too Many Requests`. The documented reason values are:

- `jira-quota-global-based` or `jira-quota-tenant-based` for hourly quota;
- `jira-burst-based` for the per-endpoint burst limit;
- `jira-per-issue-on-write` for per-issue writes.

The relevant headers are `Retry-After` (seconds), `RateLimit-Reason`, `X-RateLimit-Limit`, `X-RateLimit-Remaining`, and, on `429`, `X-RateLimit-Reset`. The docs publish per-issue thresholds of 20 writes per two seconds and 100 writes per 30 seconds. They recommend respecting `Retry-After`, exponential backoff with jitter, a bounded retry count, and lower write frequency for a hot issue. A transient `503` may also carry `Retry-After` even though it is not a rate-limit response. [Rate-limit headers and per-issue limits](https://developer.atlassian.com/cloud/jira/platform/rate-limiting/#rate-limit-related-headers), [Retry practices](https://developer.atlassian.com/cloud/jira/platform/rate-limiting/#best-practices-for-handling-rate-limit-responses)

No universal per-second number should be hard-coded: burst behavior is endpoint-specific and the docs describe steady-state examples rather than a single Jira-wide constant. [Burst API rate limit](https://developer.atlassian.com/cloud/jira/platform/rate-limiting/#burst-api-rate-limit)

### 13. API versions, deprecations, and asynchronous behavior

The v3 introduction identifies v3 as latest and says v2/v3 have the same operations, with v3 adding ADF. The current reference marks these legacy surfaces relevant here as deprecated:

- `/rest/api/3/search` GET and POST: currently being removed; use enhanced `/search/jql`;
- `/rest/api/3/project`: unpaginated all-project list; use `/project/search`;
- `/rest/api/3/issue/createmeta`: global create metadata; use project/type-scoped create metadata.

Use explicit v3 paths in durable source identities. [v3 version](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#version), [Search deprecation](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/#api-rest-api-3-search-get), [Project deprecation](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/#api-rest-api-3-project-get), [Create metadata deprecation](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-createmeta-get)

The general REST contract says an operation that schedules a long-running task returns `303 See Other` with a task URL in `Location`; `GET /rest/api/3/task/{taskId}` reports status/progress and includes a JSON `result` when complete. Tasks are not guaranteed to run in order, and task details are retained only for a limited period. [Asynchronous operations](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/#async-operations), [Tasks](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-tasks/)

The direct issue read, issue create/edit, enhanced JQL search, project search, and comment operations documented above show normal `200`, `201`, or `204` success responses rather than a task handoff. Other Jira operations (for example archive or project deletion) explicitly expose asynchronous task behavior. Therefore a bounded ResourceFS operation should treat `303` as non-final and follow it only when the selected endpoint's contract documents the task; this must be verified with a live tenant probe rather than assumed for all bulk-looking APIs.

## Implications for ResourceFS

These are constrained implications of the evidence above, not an implementation design:

1. **Identity model:** keep Jira numeric issue/project/comment IDs as source identities; keep the returned issue/project keys as mutable display attributes. A moved-key lookup means the canonical returned key can differ from the requested key.
2. **Continuation model:** represent JQL continuation as an opaque token and project/comment continuation as operation-specific offset/link metadata. A single universal page decoder would misrepresent the API.
3. **Consistency model:** a JQL-backed browse should be labeled eventually consistent unless the caller uses `reconcileIssues` for named IDs. A successful edit followed immediately by ordinary JQL is not proof that the write is visible.
4. **Mutation authority:** Jira supplies no documented general CAS token for issue/comment edits. ResourceFS must not claim external optimistic concurrency from `updated`, `ETag` assumptions, or a `204`; a `409` must retain its raw meaning until a tenant probe establishes why it occurs.
5. **ADF values:** descriptions, environments, comments, and multiline custom fields need lossless structured JSON handling. ADF schema validation cannot guarantee field acceptance, and ADF limits are both field-specific in practice and documented through an experimental report.
6. **Partial bulk outcomes:** bulk create/fetch are item-bearing result envelopes. A transport success can contain item errors; callers need a per-item result rather than an all-or-none boolean.
7. **Permissions as source state:** Browse Projects, issue-level security, comment visibility, and mutation permissions can make two callers observe different resource graphs. Missing data must not automatically be interpreted as deletion.
8. **Rate-limit taxonomy:** retain `Retry-After`, reset, remaining-capacity, and `RateLimit-Reason` so the caller can distinguish a hot issue from a tenant/app quota or a single bursty endpoint.
9. **Endpoint lifecycle:** new code should avoid deprecated search, project-list, and global create-metadata endpoints. A source identity should record explicit v3 rather than relying on a moving alias.
10. **Async boundary:** a `303` task handoff is not an issue result. A bounded operation needs a separate task state/result observation or a clear unsupported outcome.

## Unresolved questions and required live probes

The following are intentionally unresolved because the public reference does not guarantee tenant-specific behavior:

1. **Auth installation:** Which OAuth product (Forge, 3LO, Connect) and cloud-ID/site URL will ResourceFS use? Probe token expiry/revocation, granted scopes, and the exact `401`/`403` bodies for a valid token lacking Jira project permission.
2. **Issue key resolution:** Against a tenant with a moved issue, probe old key, new key, numeric ID, and mixed-case key. Record returned `id`, `key`, status, and whether `self` remains v3.
3. **Visibility filtering:** Use a principal with and without Browse Projects, issue-level security, and comment visibility. Distinguish `404` concealment from a true missing issue where possible; do not infer from one tenant.
4. **JQL cursor behavior:** Probe first and later `/search/jql` pages with a stable deterministic `ORDER BY`, token reuse, empty pages, and a changed dataset. Verify whether `nextPageToken` is absent or null on the terminal page and whether any server-side maximum is lower than requested `maxResults`.
5. **Read-after-write:** Create/edit a disposable issue, search immediately without reconciliation, then search with `reconcileIssues`; record visibility delay and whether reconciliation is limited to the named ID.
6. **Edit conflict semantics:** Run two controlled edits against one issue and inspect `409`, `updated`, changelog, and final fields. Separately try `If-Match`, `If-Unmodified-Since`, and fabricated version fields only as non-destructive probes; no conditional-write support is documented.
7. **Metadata drift:** Probe both Classic and team-managed projects. Compare `/field`, `/field/search`, project/type create metadata, and `editmeta` with fields hidden by screen/configuration/workflow. Verify whether invalid metadata combinations are silently omitted as documented.
8. **ADF acceptance:** Probe minimal ADF (`doc` version 1), schema-valid but field-unsupported nodes, plain strings in ADF fields, ADF in single-line fields, and near-limit payloads. Record exact `400`, `413`, or `422` error bodies and byte accounting.
9. **Comment authorization:** Probe add/update own/update-other comment, restricted visibility, child-comment visibility, and comment list pagination. Verify whether comments omitted by visibility are indistinguishable from missing IDs.
10. **Bulk partials and bounds:** Probe bulk create with one invalid item among valid items; verify whether valid items persist and how `errors` map to input order. Probe bulk fetch at 100, 101, 1000, and 1001 IDs with and without the optimized field/expand shape; assert the returned ordering and `issueErrors` mapping.
11. **Rate limits:** In a controlled tenant/app, record all `429` headers and `RateLimit-Reason` values for burst, per-issue write, and quota conditions. Do not assume the published hourly pool or per-issue thresholds are unchanged at runtime.
12. **Async boundary:** Probe only endpoint-documented async operations and confirm `303`/`Location`/task result behavior. Confirm that direct issue CRUD, JQL, project search, comments, and bulk fetch/create remain synchronous for the tenant and API product in use.

## Ambiguities and newly discovered bug candidate

- **Ambiguity:** Jira documents issue edit `409 Conflict` but does not say whether it means stale data, workflow conflict, a per-issue serialization conflict, or another condition. It must not be presented as an optimistic-concurrency guarantee.
- **Ambiguity:** The enhanced-search reference documents `nextPageToken` as a string request field while the terminal example omits it; the search-and-reconcile guide shows it in a non-terminal response. The continuation loop must be probe-backed and token-opaque.
- **Ambiguity:** ADF's official schema includes more nodes/marks than every Jira field accepts. Field capability and tenant probes remain necessary.
- **Potential documentation bug (to verify):** the `POST /rest/api/3/issue/bulkfetch` reference is a read-only operation and documents read-oriented OAuth scopes, but its page currently labels the required Connect app scope as `WRITE`. This conflicts with the operation's read semantics and should be checked against a Connect installation or Atlassian's scope metadata before treating it as an authorization requirement. [Bulk fetch reference](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-bulkfetch-post)

## Primary-source references

- [Jira Cloud REST API v3 introduction](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/)
- [Jira Cloud REST API v3 OpenAPI link](https://dac-static.atlassian.com/cloud/jira/platform/swagger-v3.v3.json)
- [Jira issue operations](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/)
- [Jira issue search](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/)
- [Jira projects](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-projects/)
- [Jira issue fields](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-fields/)
- [Jira issue comments](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/)
- [Jira asynchronous tasks](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-tasks/)
- [Jira search and reconcile guide](https://developer.atlassian.com/cloud/jira/platform/search-and-reconcile/)
- [Jira rate limiting](https://developer.atlassian.com/cloud/jira/platform/rate-limiting/)
- [Jira basic authentication](https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/)
- [Jira OAuth 2.0 scopes](https://developer.atlassian.com/cloud/jira/platform/scopes-for-oauth-2-3LO-and-forge-apps/)
- [Atlassian Document Format structure](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/)
- [Atlassian canonical ADF schema pointer](http://go.atlassian.com/adf-json-schema)
- [Jira Cloud platform changelog](https://developer.atlassian.com/cloud/jira/platform/changelog/)
