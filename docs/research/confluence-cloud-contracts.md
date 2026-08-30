# Confluence Cloud page contracts

## Question and scope

This note records the authoritative, currently published contracts for **Confluence Cloud pages and the page-adjacent APIs** that a ResourceFS source would need: direct reads, spaces and page browsing, CQL search, metadata, body representations, versions, hierarchy, comments, mutations, pagination, concurrency signals, authentication, permissions, errors, rate limits, endpoint versions, and deprecations. It is intentionally about Confluence Cloud only; Jira and ResourceFS implementation design are out of scope.

Sources were checked on 2026-08-29. The Atlassian reference pages are generated from OpenAPI, so the linked OpenAPI documents are included where the page contains a compact description but the schema is the useful contract.

## Executive answer

* **Use REST API v2 for page/space/hierarchy/comment operations.** Its base paths are `/wiki/api/v2/...`, it models page and space IDs as strings in responses, and its collection endpoints use cursor pagination (`Link: ... rel="next"` and `_links.next`). The v2 introduction describes v2 as an improvement over v1 ([v2 introduction](https://developer.atlassian.com/cloud/confluence/rest/v2/intro/); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).
* **CQL search remains a v1 surface.** The v2 OpenAPI currently lists no search path; the v1 reference documents both `GET /wiki/rest/api/content/search?cql=...` (content-shaped results) and `GET /wiki/rest/api/search?cql=...` (search-result-shaped results). Both use cursor links in their documented examples ([v1 content search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-content/#api-wiki-rest-api-content-search-get); [v1 search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/#api-wiki-rest-api-search-get); [v1 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/swagger.v3.json)).
* **A page is addressed by its Confluence content ID, not its title or space key.** The v2 entity schemas describe `id`, `spaceId`, and `parentId` as strings. The same OpenAPI describes path ID parameters as `int64`, so clients should preserve IDs as opaque strings and not assume an application integer type ([page API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).
* **Bodies are representation-tagged strings.** v2 reads return objects such as `{representation, value}` under requested representation keys. v2 writes accept a flat `{representation, value}` or a nested body (`storage`, `atlas_doc_format`, or `wiki`). A direct page read can request primary formats including `storage`, `atlas_doc_format`, `view`, `export_view`, `anonymous_export_view`, `styled_view`, and `editor`; collection reads are narrower and expose storage/ADF. Do not silently treat rendered `view` HTML as a lossless write format ([page API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).
* **Page updates are whole-resource, version-numbered writes.** `PUT /wiki/api/v2/pages/{id}` requires `id`, `status`, `title`, `body`, and `version`; the documented version number is current version + 1, except a draft update requires version 1. The operation does not document `If-Match`, ETags as its write precondition, or a 409 response in its v2 response list. Treat `version.number` as the only documented concurrency input and verify actual conflict status/headers against a tenant ([page update](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/#api-pages-id-put); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).
* **Comments have separate footer and inline resources.** Top-level/reply creation uses `POST /wiki/api/v2/footer-comments` or `/inline-comments`; page-root listing uses `/pages/{id}/footer-comments` or `/pages/{id}/inline-comments`. Footer editing changes body plus a comment version; inline editing changes body and/or resolution, but a dangling inline comment cannot be updated. Comment writes require comment-space permissions and `write:comment:confluence` for OAuth 3LO/Forge ([comment API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-comment/); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).
* **No scope grants around user permissions.** Atlassian states that Confluence permissions still control access even when an app has a matching scope. Reads return only content the authenticated principal may view; create/update operations require the corresponding space/content permission ([v2 introduction](https://developer.atlassian.com/cloud/confluence/rest/v2/intro/); [scopes](https://developer.atlassian.com/cloud/confluence/scopes-for-oauth-2-3lo-and-forge-apps/)).
* **429 is part of the platform contract, not a page-specific error.** Current rate-limit guidance says any REST API can return 429, clients should honor `Retry-After`, and responses may carry `X-RateLimit-*`, `RateLimit-Reason`, and (during the points-model transition) beta or non-beta `RateLimit` headers. The same page says API-token traffic is not affected by the new points-based rollout, while app traffic is; this and the beta/enforcement wording need a live probe before choosing policy ([rate limiting](https://developer.atlassian.com/cloud/confluence/rate-limiting/)).

## Evidence by decision criterion

### 1. Endpoint version and identifier contract

The v2 reference uses `/wiki/api/v2` and links a versioned OpenAPI document. It exposes page, space, ancestor, child/descendant, version, and comment groups. The v1 reference uses `/wiki/rest/api` and continues to expose content and search operations. The v2 introduction says its definitions and performance are intended to improve v1; it does **not** state that all v1 endpoints are removed ([v2 introduction](https://developer.atlassian.com/cloud/confluence/rest/v2/intro/); [v1 introduction](https://developer.atlassian.com/cloud/confluence/rest/v1/intro/)).

The v2 `PageBulk`/`PageSingle` schemas define `id`, `spaceId`, `parentId`, `authorId`, `ownerId`, and `lastOwnerId` as strings. `SpaceBulk`/`SpaceSingle` define space `id`, `key`, `homepageId`, and account IDs as strings. In contrast, the OpenAPI path parameters for page and space IDs are rendered as `integer`/`int64`. This is a published schema inconsistency, not evidence that IDs are safely convertible to a local integer. Treat a Confluence ID as an opaque string at the ResourceFS boundary, and retain the exact value returned by the API ([v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

The `PageSingle.status` enum is `current`, `draft`, `archived`, `historical`, `trashed`, `deleted`, or `any`; page collection filters are narrower (`current`, `archived`, `deleted`, `trashed`). A page has `title`, `spaceId`, optional `parentId`, `parentType`, nullable tree `position`, original author/owner IDs, creation time, a `version`, body, and optional metadata collections ([v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

### 2. Direct page reads, metadata, spaces, and browsing

**Direct read:** `GET /wiki/api/v2/pages/{id}` returns a specific page. The documented query contract includes:

* `body-format` (single-resource formats include `storage`, `atlas_doc_format`, `view`, `export_view`, `anonymous_export_view`, `styled_view`, and `editor`);
* `get-draft`, `status[]`, and numeric `version` selectors; and
* `include-labels`, `include-properties`, `include-operations`, `include-likes`, `include-versions`, `include-version`, `include-favorited-by-current-user-status`, `include-webresources`, `include-collaborators`, and `include-direct-children` flags.

The response combines `PageSingle` with a base URL link. Labels, properties, operations, likes, and versions are optional fields with their own `results`, `meta`, and `_links` objects. The page’s `version` includes `number`, `createdAt`, `message`, `minorEdit`, and `authorId` ([page API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/#api-pages-id-get); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

**Page browsing:**

* `GET /wiki/api/v2/pages` lists pages and supports `id[]`, `space-id[]`, `sort`, `status[]`, `title`, `body-format`, and `subtype` filters. Its page result is `PageBulk` (metadata plus body), not the expanded `PageSingle` shape.
* `GET /wiki/api/v2/spaces/{id}/pages` lists pages in a space. `depth` is `all` (default) or `root`; it supports page status/title/sort/body-format filters. The result is `MultiEntityResult<Page>`.
* `GET /wiki/api/v2/spaces` lists spaces sorted by ID ascending by default, with filters for IDs, keys, type, status, labels, favorite state, description format, and icon. `GET /wiki/api/v2/spaces/{id}` returns one space and can include description, icon, operations, properties, permissions, role assignments, and labels. Space list and page list limits are documented as default 25, minimum 1, maximum 250 ([page API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/); [space API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-space/); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

**Hierarchy:**

* `GET /wiki/api/v2/pages/{id}/ancestors` returns minimal `{id, type}` ancestor objects in top-to-bottom order. The docs say if more are available, continue by calling with the first ancestor ID in the response; the endpoint takes `limit` (default 25, max 250).
* `GET /wiki/api/v2/pages/{id}/direct-children` returns minimal child content objects (`id`, status, title, type, spaceId, childPosition), including database, embed, folder, page, and whiteboard content. It uses `cursor`, `limit` (default 25/max 250), and sort.
* `GET /wiki/api/v2/pages/{id}/descendants` returns minimal content-tree objects in top-to-bottom order, with `parentId`, relative `depth`, and `childPosition`; `depth` defaults to 2 and is constrained to 1–10. It is cursor-paginated.
* `GET /wiki/api/v2/pages/{id}/children` is explicitly marked **Deprecated** in the current v2 reference. It returns only child pages and should not be confused with `direct-children`, which returns all supported hierarchical content types ([ancestors](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-ancestors/#api-pages-id-ancestors-get); [children](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-children/); [descendants](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-descendants/); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

These hierarchy endpoints intentionally return summaries. A caller that needs a page body or expanded metadata must follow the page ID to `GET /pages/{id}`; a child/descendant summary is not a page document ([children](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-children/); [descendants](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-descendants/)).

### 3. CQL search

The current v2 OpenAPI has no `/search` path. CQL is therefore a v1 contract for this scope:

* `GET /wiki/rest/api/content/search?cql=...` returns `ContentArray`. The request requires `cql` and accepts `cqlcontext`, `expand[]`, `cursor`, and `limit`; the documented result contains `results`, `size`, and `_links` with cursor URLs. The embedded `Content` model can carry ID, type, status, title, space, version, ancestors, body representations, restrictions, metadata, and other expansions ([content CQL](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-content/#api-wiki-rest-api-content-search-get)).
* `GET /wiki/rest/api/search?cql=...` returns `SearchPageResponseSearchResult`: `results`, `start`, `limit`, `size`, `totalSize`, `cqlQuery`, `searchDuration`, optional `archivedResultCount`, and `_links`. Each `SearchResult` has title, excerpt, URL, breadcrumbs, entity type, last modified, optional content/space/user summaries, and score. The documented cursor response contains `_links.next` and `_links.prev` ([search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/#api-wiki-rest-api-search-get); [v1 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/swagger.v3.json)).

CQL itself is structured as field/operator/value, supports boolean grouping, ordering, ranges, text matching, and ancestor/content/space/title fields. Atlassian’s CQL guide says CQL values for creator/mention/etc. use account IDs, full names (subject to profile visibility), or public names rather than usernames ([advanced searching using CQL](https://developer.atlassian.com/cloud/confluence/advanced-searching-using-cql/)).

Do not build a user-search feature on the normal `/wiki/rest/api/search` endpoint. The current docs say it no longer supports user-specific fields (`user`, `user.fullname`, `user.accountid`, `user.userkey`); user-specific CQL goes to `/wiki/rest/api/search/user`. This is also recorded in Atlassian’s deprecation notice, which dates the change to 20 January 2020 ([search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/#api-wiki-rest-api-search-get); [deprecation notice](https://developer.atlassian.com/cloud/confluence/deprecation-notice-search-api/)).

A separate Atlassian modernization notice says the old `start` parameter for both `/rest/api/search` and `/rest/api/content/search` was to be deprecated in favor of `cursor`, and instructs clients to follow returned `prev`/`next` links rather than construct them. That notice still displays a July 15, 2020 removal date while the current reference exposes cursor examples, so the date is historical/ambiguous rather than a current removal schedule ([search modernization notice](https://developer.atlassian.com/cloud/confluence/change-notice-moderize-search-rest-apis/)).

The CQL docs describe cursor `next`/`prev` links and say `limit` is capped at 25 when `body.export_view` or `body.styled_view` is expanded. The CQL endpoint is search-index backed; index freshness and cursor lifetime are not specified in the reference and require a live probe ([v1 search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/#api-wiki-rest-api-search-get); [CQL content search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-content/#api-wiki-rest-api-content-search-get)).

### 4. Body representations and conversion

The v2 page and comment write schemas accept:

```json
{"representation":"storage","value":"<string>"}
```

or a nested object with one representation property, for example:

```json
{"storage":{"representation":"storage","value":"<string>"}}
```

The representation enum in `PageBodyWrite`/`CommentBodyWrite` is `storage`, `atlas_doc_format`, or `wiki`. A read `BodyType` always couples a representation label with a string value. In a single-page response `BodySingle` can contain `storage`, `atlas_doc_format`, and `view`; bulk bodies are narrower and contain storage/ADF. `view` and other rendered formats are read representations, not documented as lossless mutation inputs ([v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

The single-page `body-format` selector allows more read formats than collection selectors. The v2 OpenAPI defines `PrimaryBodyRepresentation` as storage/ADF for list-like operations and `PrimaryBodyRepresentationSingle` as storage, ADF, view, export_view, anonymous_export_view, styled_view, and editor. This asymmetry means a list walk should not be expected to contain the same body payload as a direct fetch ([v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

The v1 content-body API documents asynchronous conversion and its currently supported conversions: ADF to editor/export_view/storage/styled_view/view; storage to ADF/editor/export_view/styled_view/view; and editor to storage. It says a completed result is available for five minutes. This conversion service is useful evidence about representations, but it is a separate v1 asynchronous API—not a guarantee that every page endpoint accepts every listed format ([content body conversion](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-content-body/)).

### 5. Versions, updates, and concurrency signals

A v2 page `version` carries version number, timestamp, message, minor-edit flag, and author account ID. `GET /wiki/api/v2/pages/{id}/versions` is cursor-paginated, accepts body format, sort, cursor, and limit, and returns version records with a nested page summary/body. A current changelog exception says that when `body-format` is supplied, this endpoint caps results at 50 and returns 200 even if a larger limit is requested; without `body-format`, the ordinary 250 maximum is unaffected. `GET /wiki/api/v2/pages/{page-id}/versions/{version-number}` retrieves details for one page version. The version list and page response are therefore the documented way to discover a current/base version before an update ([version API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-version/); [page API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/); [1 June 2026 changelog](https://developer.atlassian.com/cloud/confluence/changelog/)).

The page update request is not a patch document. Its required fields are `id`, `status`, `title`, `body`, and `version`. The version schema says to send current number + 1; draft status requires version 1. It also documents that `spaceId` cannot move a page to another space, while `parentId` can move it under another parent in the same space and `ownerId` can transfer ownership. Updating `current` content attempts reconciliation with an existing draft; materially divergent versions may cause the provided content to override the draft. Restoration from trashed/deleted to current changes status without updating page contents ([page update](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/#api-pages-id-put); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

`PUT /wiki/api/v2/pages/{id}/title` is a separate title-only mutation whose documented body is just required `status` and `title`; the operation does not expose a version field in its request contract. `DELETE /wiki/api/v2/pages/{id}` returns 204; the documented controls distinguish default trashing, `draft=true` permanent draft deletion, and `purge=true` permanent deletion of a trashed page, with purge requiring space-admin permission. These are separate mutation semantics from a versioned body update ([page title update](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/#api-pages-id-title-put); [page delete](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/#api-pages-id-delete)).

The v2 OpenAPI response lists for page `PUT` contain 200, 400, 401, and 404; they do not enumerate 409, ETag, or `If-Match`. The v1 general reference says special `X-Atlassian-Token: no-check` headers are required only where a method description says so, and the page update description does not say so. The rate-limit page recommends ETags/conditional headers for avoiding unchanged reads, but that is not a page-write precondition contract ([v2 page update](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/#api-pages-id-put); [v1 introduction](https://developer.atlassian.com/cloud/confluence/rest/v1/intro/); [rate limiting](https://developer.atlassian.com/cloud/confluence/rate-limiting/)).

**Guarantee versus ambiguity:** version-number sequencing is documented; the exact stale-writer response, whether an ETag is emitted, whether `If-Match` is honored, and whether the update endpoint ever returns 409 are not established by the current endpoint contract. A ResourceFS write path must not claim stronger optimistic-locking behavior until a controlled tenant probe records it.

### 6. Comments and comment limitations

The v2 comment group distinguishes footer comments (page-level discussion) from inline comments (text-anchored discussion):

* `GET /wiki/api/v2/pages/{id}/footer-comments` returns root footer comments for a page; `GET /pages/{id}/inline-comments` returns root inline comments. Both are cursor-paginated and accept body format, status, sort, cursor, and limit; inline listing also accepts resolution status.
* `GET /wiki/api/v2/footer-comments/{id}/children` and `/inline-comments/{id}/children` return replies. The top-level collection endpoints also exist for all comments, but page-root endpoints are the relevant page contract.
* `POST /wiki/api/v2/footer-comments` accepts one container ID (`pageId`, `blogPostId`, `attachmentId`, or `customContentId`) for a top-level comment or `parentCommentId` for a reply, plus a body. A successful create returns 201 and the docs expose a `Location` header for the created comment. `POST /wiki/api/v2/inline-comments` uses page/blog post or parent comment plus body and `inlineCommentProperties`; it likewise returns 201 and a `Location` header.
* Inline top-level creation uses `textSelection`, zero-based `textSelectionMatchIndex`, and `textSelectionMatchCount`; the docs say this selects the text to highlight. Replies do not use the selection object.
* `PUT /footer-comments/{comment-id}` updates body text and a version object. The version number should be one higher than the current comment version. `PUT /inline-comments/{comment-id}` updates body and/or `resolved`; a dangling inline comment cannot be updated. Both return the updated comment; deletes are permanent and return 204 ([comment API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-comment/); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

Comment bodies use the same representation-tagged strings as page bodies. Footer comment models contain IDs for page/blog post/attachment/custom content and parent comment, status, version, body, and optional properties/operations/likes/versions. Inline models add resolution state and the original selection/marker metadata ([v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

The comment docs do not promise that arbitrary text selections remain anchored after page edits, nor do they define how a failed/mismatched selection is represented beyond the `dangling` resolution state. They also do not mark body/container properties as required in every generated schema even though the narrative requires a target and body. Validate these cases against a test tenant before exposing them as a ResourceFS write guarantee.

### 7. Pagination shape and limits

For v2 multi-entity endpoints, the introduction says to request `limit`/`cursor`, then follow the HTTP `Link` header’s relative `next` URL. The same relative URL is available as `_links.next` in the JSON body. If there are no further results, neither the `Link` header nor `_links.next` is present. v2 common collection limits are default 25, minimum 1, maximum 250; the page-version endpoint is a documented exception when `body-format` is present, where the June 2026 changelog caps each response at 50. Descendants additionally constrain depth ([v2 introduction](https://developer.atlassian.com/cloud/confluence/rest/v2/intro/); [v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json); [changelog](https://developer.atlassian.com/cloud/confluence/changelog/)).

The v2 wrappers have `results` and `_links` (`next` and `base`). They do not have the v1-style `start`/`totalSize` contract. Optional nested fields such as labels/properties/versions have a different `meta.hasMore`/`meta.cursor` shape and a self link; that cursor is for the nested field endpoint, not necessarily for the outer collection ([v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)).

The v1 CQL examples also use cursor `next`/`prev` links. `GET /content/search` and `/search` should therefore be treated as cursor-based even though the v1 general introduction still documents offset `start` pagination for ordinary v1 collections. The search endpoint’s own current contract takes precedence for CQL search ([v1 content search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-content/#api-wiki-rest-api-content-search-get); [v1 search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/#api-wiki-rest-api-search-get); [v1 introduction](https://developer.atlassian.com/cloud/confluence/rest/v1/intro/)).

### 8. Authentication, authorization, and permissions

For direct REST calls, Atlassian documents Basic authentication with an Atlassian account email as username and API token as password. It calls Basic auth less secure and recommends it for scripts/manual calls; Forge uses its framework authentication, Connect uses JWT, and developer-console apps can use OAuth 2.0 authorization code grants (3LO) ([Basic auth for REST APIs](https://developer.atlassian.com/cloud/confluence/basic-auth-for-rest-apis/); [v2 introduction](https://developer.atlassian.com/cloud/confluence/rest/v2/intro/)).

OAuth 3LO uses an authorization-code flow at `auth.atlassian.com`, then exchanges the code for a bearer token and discovers accessible resources/cloud IDs before calling `api.atlassian.com/ex/confluence/{cloudid}/{api}`. Atlassian documents rotating refresh tokens and a 90-day inactivity window. This is distinct from direct site Basic auth and should be treated as a separate credential mode ([OAuth 2.0 3LO](https://developer.atlassian.com/cloud/confluence/oauth-2-3lo-apps/)).

The v2 endpoint pages publish granular OAuth/Forge scopes and Connect `READ`/`WRITE` scopes per operation. Relevant examples are:

| Operation | Granular scope in endpoint docs |
| --- | --- |
| Read page/list pages | `read:page:confluence` |
| Create/update/title-update page | `write:page:confluence` |
| Read spaces | `read:space:confluence` |
| Read page children/descendants | `read:hierarchical-content:confluence` |
| Read ancestors | `read:content.metadata:confluence` |
| Read comments | `read:comment:confluence` |
| Create/update comments | `write:comment:confluence` |
| Delete comments | `delete:comment:confluence` |

The endpoint pages also state the Confluence permission required: view the page and space for reads; view plus create-page/update-page permission for page mutations; view plus create-comments permission for comment writes; and delete-comments permission for comment deletion. The scopes reference explicitly says scopes do not override Confluence permissions ([page API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/); [children](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-children/); [comment](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-comment/); [scopes](https://developer.atlassian.com/cloud/confluence/scopes-for-oauth-2-3lo-and-forge-apps/)).

Basic auth can behave differently from a challenge-driven HTTP client because Confluence permits anonymous access by default; Atlassian says clients may need to send the Authorization header proactively ([Basic auth](https://developer.atlassian.com/cloud/confluence/basic-auth-for-rest-apis/)).

Atlassian’s privacy migration guidance says personal identifiers such as username/userKey were removed from many user operations and accountId is the durable user identifier; email/profile fields may be null due to profile privacy. This matters when recording page/comment authors: preserve account IDs and do not require display names or email addresses ([user privacy API migration](https://developer.atlassian.com/cloud/confluence/deprecation-notice-user-privacy-api-migration-guide/)).

### 9. Errors and status signals

The generated v2 operation contracts document these status families:

* page list/read: 200; invalid request 400; unauthenticated 401; missing page/space on applicable operations 404;
* page create: documented 200, with 400/401/404/413; note that this is the page endpoint’s published status, not the usual 201 assumption;
* page update/title update: 200, with 400/401/404;
* page delete: 204, with 400/401/404;
* comment create: 201, with 400/401/404; comment read/update: 200, with 400/401/404; comment delete: 204, with 400/401/404;
* v1 `/wiki/rest/api/search`: 200, 400, or 403; v1 `/content/search`: 200, 400, or 401 in its generated contract.

These are documented endpoint responses, not an exhaustive list of every proxy/platform failure. In particular, the v2 page update operation does not list 409; conflict behavior is unresolved rather than guaranteed absent ([page API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/); [comment API](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-comment/); [v1 search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/); [v1 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/swagger.v3.json)).

The v1 general REST introduction says error responses also return a body “similar to” this shape: `statusCode`, `data` (`authorized`, `valid`, `errors[]`, `successful`), and top-level `message`; an error’s translated message has `translation` and `args`. It is an example shape, not a versioned v2 error schema ([v1 introduction](https://developer.atlassian.com/cloud/confluence/rest/v1/intro/)).

### 10. Rate limits

Atlassian’s current Confluence rate-limit page states:

* a 429 response means Too Many Requests and clients should pause/retry after the specified delay;
* points-based quotas apply to Forge, Connect, and OAuth 2.0 (3LO) apps, while API-token traffic is described as unaffected by that new points-based change and still subject to existing burst limits;
* the page describes a default global pool of 65,000 points/hour and reviewed per-tenant pools, but quotas/tier assignment are service policy rather than a page endpoint guarantee;
* documented headers include `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`, `X-RateLimit-NearLimit`, `RateLimit-Reason`, and `Retry-After` on 429;
* during the transition, beta-prefixed headers (`Beta-Retry-After`, `Beta-RateLimit*`, and `X-Beta-RateLimit-*`) may be informational; the page says enforcement drops `Beta-` for `RateLimit`/`RateLimit-Policy` and that existing X-RateLimit headers continue for other limit types; and
* the recommended retry guidance is to retry only an idempotent operation when `Retry-After` is present, with exponential backoff and jitter.

The same page says any REST API can return a rate-limit response. It also contains both “enforcement begins” and “beta/informational” wording, so the active header set and token/app distinction should be recorded from a low-volume live request for the relevant authentication mode ([rate limiting](https://developer.atlassian.com/cloud/confluence/rate-limiting/)).

## Implications for ResourceFS

These are evidence-backed constraints, not an implementation plan:

1. **Keep Confluence IDs opaque.** The published response schemas use strings while path parameters use int64. Parsing IDs into a narrow integer risks overflow or losing the exact route value.
2. **Represent a page document as metadata plus explicit body representations.** Preserve the representation label and raw string; do not conflate storage XML, ADF JSON-as-string, rendered view HTML, and wiki markup. A direct read is the authoritative source for a body; list and hierarchy endpoints are summaries or narrower bulk bodies.
3. **Use v2 for page/space/tree/comment reads and writes, and isolate v1 CQL search.** The two API versions have different result wrappers, status lists, and pagination details. A single generic “Atlassian list” decoder would obscure contract differences.
4. **Treat cursors and links as opaque server-provided continuation state.** Follow `_links.next`/`Link` for v2 and the endpoint-specific cursor links for v1 CQL; stop when absent. Do not synthesize offsets from result counts.
5. **Treat page mutation as versioned replacement.** A successful update requires a base version and full required payload. The documented contract does not establish ETag/If-Match semantics or a 409 shape; surface that uncertainty rather than promising compare-and-swap stronger than the API documents.
6. **Make comment support explicit and bounded.** Footer comments, inline comments, and replies are separate resources. Inline anchoring can become dangling; footer editing cannot resolve; inline editing can resolve but cannot update dangling comments. Deletion is permanent.
7. **Do not infer authorization from scopes alone.** A source must preserve the authenticated principal’s visibility boundary and distinguish “not visible/not found” from a permission denial only when the observed HTTP contract allows that distinction.
8. **Do not hard-code create status 201 for pages.** The current page-create docs publish 200, while comment and space create docs publish 201. Callers should accept the endpoint’s documented status and decode the returned page.
9. **Make rate-limit handling response-driven.** Honor `Retry-After`; preserve relevant rate-limit headers for diagnostics; avoid retrying non-idempotent mutations blindly. The published beta/enforcement transition makes a live header probe necessary.
10. **Retire deprecated child-page browsing.** Prefer direct children for the content tree. Keep the deprecated child-pages endpoint only as a compatibility observation, not as a new contract assumption.
11. **Preserve account IDs independently of profile fields.** User privacy changes mean display names, usernames, and emails can be absent or unstable; page/comment authorship should not depend on those fields.

## Unresolved questions requiring a live authenticated probe

A probe should use a disposable Confluence Cloud tenant, a page with a draft/current version, a child tree, footer and inline comments, and two principals with deliberately different permissions. Record status, headers, and raw JSON without storing credentials.

1. **Identifier routing:** Do IDs returned as strings round-trip through endpoints whose OpenAPI path parameters are int64? Are IDs ever outside a language’s safe integer range? Do leading/formatting details matter?
2. **Direct read bodies:** For each `body-format`, which body keys and representation labels are returned? What is omitted for list responses? How do `status=draft`, `get-draft`, and historical `version` interact?
3. **CQL:** Compare `/content/search` with `/search` for page, space, title, ancestor, and account-ID queries. Measure index visibility delay after create/update, result ordering, cursor replay/expiry, and the actual cap when body expansions are requested.
4. **Pagination:** Confirm whether every v2 collection emits both HTTP `Link` and JSON `_links.next`, whether relative URLs require the original `/wiki` context, how out-of-range limits are clamped/rejected, and cursor behavior after mutations.
5. **Page create:** Test published versus draft creation, omitted title/body fields, `root-level`, `parentId`, `embedded`, `private`, and `subtype=live`. Confirm the actual create response status and whether the returned body is storage, ADF, or both.
6. **Page update/concurrency:** Run two writers from the same base version. Record stale-version status/body, ETag presence, `If-Match` acceptance, any 409/412 response, whether version numbers advance exactly once, and read-after-write visibility. Test current-to-draft reconciliation and draft version 1 behavior separately.
7. **Move/ownership/delete:** Verify same-space parent moves, cross-space rejection, owner transfer permissions, title-only update version behavior, default trashing, draft deletion, purge permissions, and restoration semantics.
8. **Comment creation/editing:** Test top-level and reply footer comments, inline selection match counts/indexes, edited text that invalidates an anchor, comment version increments, resolution/reopen, and update of a dangling comment. Record whether empty or malformed body/container combinations return 400 or another status.
9. **Permissions:** With a principal lacking site use, space view, page view, page update, page create, comment create, or comment delete, record which status/body is returned and whether inaccessible content is omitted from lists/search.
10. **Authentication:** Confirm Basic email/API-token auth, OAuth 3LO scopes, and app/Forge behavior against the same page. Check whether anonymous requests receive a challenge or simply an anonymous response.
11. **Rate limits:** Capture headers on ordinary page/list/search calls under API-token and app/OAuth authentication. Do not intentionally exhaust a tenant; verify presence/meaning of `Retry-After`, X-RateLimit, beta, and RateLimit headers from normal traffic and any controlled sandbox quota signal.
12. **Deprecations/privacy:** Confirm that `/pages/{id}/children` remains callable, whether it emits a warning header, and that `/direct-children` has the intended all-content behavior. Probe missing username/email/display-name fields and ensure accountId is present for authors where permitted.

## Primary-source references

* [Confluence Cloud REST API v2 introduction](https://developer.atlassian.com/cloud/confluence/rest/v2/intro/)
* [Confluence Cloud REST API v2 OpenAPI](https://dac-static.atlassian.com/cloud/confluence/openapi-v2.v3.json)
* [v2 page operations](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-page/)
* [v2 space operations](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-space/)
* [v2 ancestors](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-ancestors/)
* [v2 children (including deprecated child-pages operation)](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-children/)
* [v2 descendants](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-descendants/)
* [v2 versions](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-version/)
* [Confluence Cloud changelog (version-listing limit change)](https://developer.atlassian.com/cloud/confluence/changelog/)
* [v2 comments](https://developer.atlassian.com/cloud/confluence/rest/v2/api-group-comment/)
* [Confluence Cloud REST API v1 introduction/status codes](https://developer.atlassian.com/cloud/confluence/rest/v1/intro/)
* [v1 content CQL search](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-content/#api-wiki-rest-api-content-search-get)
* [v1 search API](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/#api-wiki-rest-api-search-get)
* [v1 REST OpenAPI](https://dac-static.atlassian.com/cloud/confluence/swagger.v3.json)
* [Advanced searching using CQL](https://developer.atlassian.com/cloud/confluence/advanced-searching-using-cql/)
* [Changes to Confluence Cloud Search APIs and deprecation notice](https://developer.atlassian.com/cloud/confluence/deprecation-notice-search-api/)
* [Upcoming changes to modernize search](https://developer.atlassian.com/cloud/confluence/change-notice-moderize-search-rest-apis/)
* [Content body conversion](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-content-body/)
* [Basic auth for REST APIs](https://developer.atlassian.com/cloud/confluence/basic-auth-for-rest-apis/)
* [OAuth 2.0 3LO apps](https://developer.atlassian.com/cloud/confluence/oauth-2-3lo-apps/)
* [Confluence OAuth 2.0/Forge scopes](https://developer.atlassian.com/cloud/confluence/scopes-for-oauth-2-3lo-and-forge-apps/)
* [User privacy API migration/deprecation guidance](https://developer.atlassian.com/cloud/confluence/deprecation-notice-user-privacy-api-migration-guide/)
* [Confluence Cloud rate limiting](https://developer.atlassian.com/cloud/confluence/rate-limiting/)
