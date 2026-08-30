# Evidence: rfs-pm0y

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | Jira's direct issue endpoint accepts either stable ID or key and returns the found issue rather than redirecting an alias. | Does current Jira REST v3 document one GET path whose required `issueIdOrKey` accepts ID/key, performs case-insensitive and moved-key fallback, returns the found key, and does not redirect? | PASS |
| P2 | One issue response carries stable identity, exact field values, field names, and field schemas in shapes the adapter can validate. | Does the current `IssueBean` schema expose `id`, `key`, `self`, unconstrained JSON field values, string field names, and `JsonTypeBean` metadata whose only required member is `type`? | PASS |
| P3 | Complete ADF is recognizable as a typed JSON document without converting it to Markdown authority. | Does the current canonical ADF schema require root `type: doc`, `version: 1`, and array `content`, while the official guide identifies ADF as rich-text JSON used by Jira issue comments and textarea fields? | PASS |
| P4 | Jira issue reads do not provide a documented ETag guarantee, so caching must remain opportunistic and revalidated only when a usable validator is actually returned. | Does the current issue-read operation document an `ETag` response header? | PASS |
| P5 | Atlassian API-token authentication is HTTP Basic over the account email and token, not a bearer token. | Does current Atlassian guidance require `Basic base64(email:api_token)` in the `Authorization` header? | PASS |
| N1 | ResourceFS retry, deadline, cancellation, and 304-generation behavior. | N/A — repository behavior already implemented and verified by closed prerequisite `rfs-i0c9`; this ticket reuses `BoundedRead` and the existing session-cache contract rather than changing it. | N/A — current repository evidence |
| N2 | Canonical JSON encoding, deterministic Markdown layout, warnings, ResourceError mapping, and identifier grammar. | N/A — these are requested feature behavior and design choices fixed by `rfs-pm0y`, parent `rfs-0zbv`, and closed decision tickets, not claims about an existing external system. | N/A — specified behavior |

## Data

- Source: production-shaped — Atlassian's current published Jira REST v3 OpenAPI document, canonical ADF JSON Schema, and API-token Basic-authentication guide.
- Shape: the vendor schemas and wire-authentication instructions used to publish the production endpoint path, response model, field metadata model, response statuses, complete ADF document grammar, and Authorization header; Jira schema version `1001.0.0-SNAPSHOT-699dda19a3d49050afba1f0e24e0b62d363c1be4`, SHA-256 `7b92e6a64584be28d2222e38e8752db7b9f422aa91ed0d20995cfaf6168293d4`; resolved ADF schema `@atlaskit/adf-schema@57.2.5`, SHA-256 `75f080928a970250eb8289e9cae5374e3c2a6c0ac3ca22478acaa9d3f39484a3`; normalized Basic-auth fact-set SHA-256 `2533908d075f45cf9f860436789fe5db9a8ea275da5c14250b6c8c72573e4cc5` (raw documentation HTML is intentionally not pinned because page-shell metadata changes independently of the four extracted facts).
- Safety: read-only anonymous fetches of public vendor documentation; no tenant, credentials, production state, or mutation involved. | Approval: N/A — safe public production-shaped data.

## Probe

- File: `probe_openapi.py`
- Mechanism: Python's JSON decoder fetches the published Jira OpenAPI and ADF schemas, follows schema references, and emits only the endpoint, identity, field-metadata, status, validator, and ADF-root facts named by P1–P4; a separate literal extraction from the rendered Atlassian authentication guide checks P5's email/token, colon-join, base64, and Basic-scheme instructions. It neither imports ResourceFS production code nor reuses the future Rust decoder or credential composer.
- Run: `python .rfs-pm0y/probe_openapi.py`

## Oracle

- Mechanism: manual inspection of Atlassian's separately rendered REST operation page, ADF structure guide, and Basic-authentication example, cross-checked against the source-cited research note at commit `cfab7e42a998764f1d7b0e446c1a652a48ff3162`; this uses vendor prose/examples and human comparison rather than the probe's schema traversal/literal extraction and differs from the future Rust wire decoder and base64 composer.
- Run: Read `https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/:3790-3995`, read `https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/`, inspect lines 138–215 of `https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/`, and run `git show cfab7e42a998764f1d7b0e446c1a652a48ff3162:docs/research/jira-cloud-contracts.md`.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | `GET /rest/api/3/issue/{issueIdOrKey}`; required path parameter; accepts ID/key; case-insensitive and moved-issue fallback; returns found key; no redirect; statuses 200/401/404. | Rendered operation says the issue is identified by ID or key, unmatched input gets case-insensitive/moved-issue search, the found issue's key is returned without 302/other redirect, and responses are 200/401/404. | PASS |
| P2 | `IssueBean` exposes `id`, `key`, `self`; `fields.additionalProperties` is unconstrained JSON; `names` values are strings; `schema` values are `JsonTypeBean`; `JsonTypeBean.type` is required; `IssueBean` itself declares no required members. | The rendered operation returns `IssueBean`; the source-cited research table independently records returned `id`, `key`, `self`, fields, and field metadata (`id`/name/schema). Its nullable/optional warning agrees that authority-bearing members must be validated at point of use rather than trusted as serde-required by the vendor schema. | PASS |
| P3 | ADF root resolves to `doc_node`; required members are `content`, `type`, `version`; type is `doc`; version is `1`; content is an array. | The ADF guide calls ADF a JSON object for Jira rich text, shows `{version: 1, type: "doc", content: [...]}`, requires `type`, root `version`, and block `content`, and warns schema-valid nodes may still be unsupported by a particular implementation. | PASS |
| P4 | The 200 issue-read response has no documented `ETag` header in the OpenAPI operation. | The rendered operation lists 200/401/404 and the response model but no ETag or conditional-read guarantee; the research note likewise treats undocumented validators as non-authority. | PASS |
| P5 | The guide contains all four facts: account email plus API token, `useremail:api_token`, base64 encoding, and `Authorization: Basic`. | The rendered example directs the caller to build `useremail:api_token`, base64-encode it, and supply `Authorization: Basic <encoded>`; `curl -u email:token` is the equivalent positive example. | PASS |

## Validated / learned

- P1: validated prior understanding — the current vendor contract explicitly supports stable ID and mutable-key lookup through one endpoint and returns found identity without redirect authority.
- P2: validated with a sharper boundary — all needed members exist, but `IssueBean` declares none required. The adapter must deserialize optional native members and then atomically reject missing identity, fields, names, or per-present-field schema/name authority instead of letting serde invent defaults.
- P3: validated prior understanding — complete ADF has a distinct root grammar and remains JSON authority; product support is narrower than schema validity, so unsupported well-formed nodes belong in deterministic projection warnings rather than destructive normalization.
- P4: validated prior understanding — no ETag is promised. The adapter may echo and revalidate an observed usable ETag through the shared cache, but must fetch unconditionally when none is present and must never require one for correctness.
- P5: validated prior understanding — API tokens use Basic authentication over the UTF-8 `email:token` pair. Composition must occur inside a Secret-preserving credential constructor; neither the email/token pair nor its base64 value may become a debuggable/displayable `String` outside the credential boundary.

## Related issues

- Consulted: `rfs-4df1` (Jira Cloud contracts), `rfs-nrjp` (lossless native fields and projections), `rfs-lcuh` (canonical Atlassian identities), `rfs-lbps` (operational behavior/evidence), `rfs-i0c9` (bounded HTTP-date retries), `rfs-0zbv` (consolidated destination contract).
- Filed: none — the comparisons agreed and revealed no underlying-system defect or separate future-work gap.
