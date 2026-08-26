# Evidence: rfs-45ww

## Premise checklist
| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | GitHub REST exposes pull requests through issue-compatible endpoints while retaining a machine-readable PR discriminator. | Does `GET /repos/rust-lang/rust/issues/159232` return the same repository-scoped `number` as the PR endpoint and include a `pull_request` discriminator? | PASS |
| P2 | Conversation comments, review submissions, and inline review comments are distinct endpoint/object shapes with stable numeric identities. | Do the three fixed-PR collection endpoints return distinct typed shapes and stable numeric `id` fields for issue comments, reviews, and pull review comments? | PASS |
| P3 | GitHub supplies both a unified-diff representation and ordered per-file diff metadata, with textual patches not guaranteed for every file. | Does the fixed PR return unified diff text under the documented media type and ordered file records whose `patch` field may be absent or null? | PASS |
| P4 | GitHub responses expose the headers needed for bounded pagination and conditional revalidation. | Does a one-item list response carry a parseable `Link` continuation, and does a matching `If-None-Match` repeat return `304` without a response body? | PASS |
| P5 | GitHub success/error responses expose machine-readable status/header signals, but some authorization, absence, and secondary-rate-limit cases may be intentionally ambiguous. | Which classifications are supported by numeric status and documented headers without matching a human error message? | PASS |
| P6 | The approved numeric `:page:<n>` continuation can map to GitHub's numeric `page=n` request even when the returned Link also contains an opaque cursor. | At one observation, do the exact returned Link target and the same list request with only `page=2` return the same first object ID? | PASS |
| N1 | Canonical Resource paths, aggregate rendering, cache lifetime, pagination ceiling, retry policy, and `ResourceError` mappings. | N/A — these are approved behavior decisions in `spec.md`, not claims about an existing system. | N/A — specification behavior |
| N2 | The current source-neutral HTTP seam lacks arbitrary request headers and response-header access. | N/A — current repository evidence directly shows `HttpRequest` contains only `url` and `BoundedHttpResponse` exposes only status/final URL/content type/body/truncated in `crates/resourcefs-sources/src/http/mod.rs`. | N/A — current applicable repository evidence |
| N3 | GitHub JSON decoding can be added without an architecture decision. | N/A — `tests/architecture_contract.rs` currently forbids the relevant serialization dependencies in `resourcefs-sources`; placement is a design question, not an external premise. | N/A — design-stage placement question |

## Data
- Source: production-shaped public read-only GitHub REST data.
- Shape: public `rust-lang/rust` pull request `159232`, its issue-compatible object, conversation comments, review submissions, inline review comments, files, unified diff, one-item repository issue listing, and a deliberately nonexistent issue number; plus public `github/docs` pull request `41852`, whose binary PNG file has no textual `patch`.
- Safety: all requests are unauthenticated or credential-backed GETs against public state; the probe sends no mutation verb or body, records only schema/count/header facts, and never prints credential material. No snapshot approval is required because the data is public and production-shaped.

## Probe
- File: `probe_github_api.py`
- Mechanism: Python standard-library HTTP requests compute compact status/header/schema/count facts directly from the public GitHub REST API without using reqwest, serde, ResourceFS production code, or `gh`.
- Run: `python .rfs-45ww/probe_github_api.py`

## Oracle
- Mechanism: the official GitHub REST/OpenAPI documentation fixes endpoint and header contracts; independent `gh api` plus `jq` requests compute the fixed PR's live counts/statuses through a different HTTP client and JSON parser than both the probe and planned production implementation.
- Run:
  - `gh api repos/rust-lang/rust/issues/159232 --jq '{number: .number, has_pull_request: has("pull_request")}'`
  - `gh api repos/rust-lang/rust/pulls/159232 --jq '{number: .number}'`
  - `gh api 'repos/rust-lang/rust/issues/159232/comments?per_page=100' --jq '{count: length, fields: (.[0] | {id, body, user, created_at, updated_at, issue_url} | keys)}'`
  - `gh api 'repos/rust-lang/rust/pulls/159232/reviews?per_page=100' --jq '{count: length, fields: (.[0] | {id, body, user, state, submitted_at, commit_id} | keys)}'`
  - `gh api 'repos/rust-lang/rust/pulls/159232/comments?per_page=100' --jq '{count: length, fields: (.[0] | {id, body, user, path, diff_hunk, created_at, updated_at, pull_request_review_id} | keys)}'`
  - `gh api 'repos/rust-lang/rust/pulls/159232/files?per_page=100' --jq '{count: length, filenames: map(.filename), patches_present: map(has("patch") and (.patch != null))}'`
  - `gh api 'repos/github/docs/pulls/41852/files?per_page=100' --jq 'map({filename, has_patch: has("patch"), patch_is_null: (.patch == null)})'`
  - `gh api -H 'Accept: application/vnd.github.diff' repos/rust-lang/rust/pulls/159232`
  - `gh api --include 'repos/rust-lang/rust/issues?state=all&sort=updated&direction=desc&per_page=1&page=1' --jq 'length'`
  - `gh api --include -H 'If-None-Match: W/"38898110829b26258ac8f0fe210ecc15066a2461aaa98074a467cee162446d85"' 'repos/rust-lang/rust/issues?state=all&sort=updated&direction=desc&per_page=1&page=1'`
  - `gh api --include repos/rust-lang/rust/issues/999999999`
  - `gh api 'repos/rust-lang/rust/issues?state=all&sort=updated&direction=desc&per_page=1&page=2' --jq '.[0].id'`
  - `gh api 'repositories/724712/issues?state=all&sort=updated&direction=desc&per_page=1&page=2&after=Y3Vyc29yOnYyOpLPAAABoDxzrOjPAAAAATkD-N0%3D' --jq '.[0].id'`
- Expected `304` and `404` cause `gh` to exit nonzero after printing the response; the response itself is the oracle observation. The ETag and `after` values above are the exact values returned during this run.
- Official sources:
  - [Repository issues](https://docs.github.com/en/rest/issues/issues?apiVersion=2026-03-10#list-repository-issues): issue endpoints may return PRs, distinguished by `pull_request`; issue `id` and PR `id` differ; `number` is the repository-scoped path identity; list supports `state=all`, `sort=updated`, `direction=desc`, `per_page` up to 100, and numeric `page`.
  - [Issue comments](https://docs.github.com/en/rest/issues/comments?apiVersion=2026-03-10#list-issue-comments): issue-style conversation comments for both issues and PRs, distinct from review comments.
  - [Pull-request reviews](https://docs.github.com/en/rest/pulls/reviews?apiVersion=2026-03-10#list-reviews-for-a-pull-request): reviews are chronological groups with numeric IDs; `submitted_at` can be absent and `commit_id` can be null.
  - [Pull-request review comments](https://docs.github.com/en/rest/pulls/comments?apiVersion=2026-03-10#list-review-comments-on-a-pull-request): inline diff comments are distinct objects; `pull_request_review_id` may be null.
  - [Pull-request files](https://docs.github.com/en/rest/pulls/pulls?apiVersion=2026-03-10#list-pull-requests-files): at most 3,000 files, 100 per page; `patch` is an optional string in the official OpenAPI schema, with no documented binary/large-file omission rule.
  - [Pagination](https://docs.github.com/en/rest/using-the-rest-api/using-pagination-in-the-rest-api): Link targets are authoritative and may use `page`, `before`/`after`, or `since`; clients should not synthesize later URLs.
  - [REST best practices](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api#use-conditional-requests): most, not all, endpoints supply ETag; the exact ETag and stable request URL produce `304` when unchanged.
  - [Authentication](https://docs.github.com/en/rest/authentication/authenticating-to-the-rest-api): invalid credentials can return `401`, while missing/insufficient access may return `403` or intentionally hidden `404`.
  - [Rate limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api): primary exhaustion is `403` or `429` with `X-RateLimit-Remaining: 0`; `Retry-After` is optional; a secondary limit may otherwise be identifiable only from its human message.
  - [Pinned official OpenAPI snapshot](https://github.com/github/rest-api-description/blob/8114b0d0e23240dfe45374e2daf01651a4729210/descriptions/api.github.com/api.github.com.2026-03-10.yaml): response schemas and nullable/optional field contracts used above.

## Comparisons
| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | Issue and PR endpoints both returned `200`, repository-scoped number `159232`, and the issue-compatible object had a `pull_request` object. | `gh` independently returned `{\"has_pull_request\":true,\"number\":159232}` and PR number `159232`; official schemas say the issue and PR `id` values differ, so canonical paths must use `number`, not cross-endpoint `id`. | PASS |
| P2 | Conversation/review/inline endpoints returned 7/4/6 objects; every sampled row had a unique integer `id` and the probed type-specific fields. | `gh` returned the same 7/4/6 counts and sampled field sets. Official schemas add that `submitted_at` may be absent, `commit_id` may be null, and `pull_request_review_id` may be null. | PASS |
| P3 | Diff request returned `200`, `application/vnd.github.diff`, 7,546 bytes beginning `diff --git`; files returned four paths in upstream order; the binary PNG fixture omitted `patch`. | `gh` returned the same diff text, same ordered four paths, and `{has_patch:false, patch_is_null:true}` for the PNG; the official media/schema contracts agree. | PASS |
| P4 | The same one-item list response returned one row, `Link: <...page=2...>; rel=\"next\"`, and an ETag; repeating that exact list URL with `If-None-Match` returned `304` and zero body bytes. | `gh --include` independently returned `200`, one row, ETag and `rel=\"next\"`; repeating the exact list URL with that ETag returned `304 Not Modified` and no body. | PASS |
| P5 | Success returned `200`; missing object returned `404` with JSON keys `documentation_url`, `message`, `status`; both carried numeric `X-RateLimit-*` headers. | `gh` agreed. Official docs establish primary-rate signals (`403`/`429` plus remaining `0`) but also establish two ambiguities: hidden/unauthorized resources may return `404`, and secondary rate limits may lack both `Retry-After` and remaining `0`, leaving only human message text. | PASS |
| P6 | The returned Link target and a direct numeric `page=2` request both returned `200` with first object ID `5221733773`. | Independent `gh` requests for the Link's cursor-bearing target and direct numeric page 2 both returned first ID `5221733773`. | PASS |

## Validated / learned
- P1: new learning — repository-scoped `number` agrees across issue-compatible and PR endpoints, but their `id` values name different object representations. Canonical `issue://`/`pr://` paths must use `number`; stable comment/review Field paths use each collection's numeric `id`.
- P2: new learning — the three endpoint families and IDs are distinct as expected, but review `submitted_at`, review `commit_id`, and inline `pull_request_review_id` are optional/nullable. Typed wire values must represent those absences rather than reject valid responses.
- P3: validated prior understanding — the pull endpoint serves unified diff media, the files endpoint preserves upstream file order, and binary file rows can omit `patch`; patch absence is valid data, not malformed JSON.
- P4: new learning — GitHub's current `Link` continuation may combine `page=2` with an opaque `after` cursor. The adapter must follow the returned Link target rather than synthesize the next URL from page number alone; ResourceFS may still render its approved typed `:page:2` continuation while caching the upstream target inside the Path Session.
- P5: new learning — no machine-only matrix can perfectly distinguish all permission, absence, and secondary-rate-limit responses. `401`, primary-limit remaining `0`, `429`, and `Retry-After` are machine-classifiable; upstream `404` intentionally collapses hidden and absent resources, while a secondary-limit `403` without rate headers is distinguishable only by human message text. The re-approved spec chooses the stable collapse: all `404` is `not_found`; `429` or signaled rate-limit `403` is `source_unavailable`; other `403` is `permission_denied`; message prose is never matched.
- P6: validated prior understanding for the observed fixed query — numeric page 2 and GitHub's cursor-bearing Link target selected the same next object. The adapter can expose the approved numeric continuation, but must still treat Link as authoritative while following pages inside one operation.

## Related issues
- Consulted: rfs-jk0d (accepted GitHub source and direct-adapter behavior), rfs-g2z9 (bounded HTTP substrate evidence), rfs-by2z (future GitHub mutation boundary), rfs-q12y (HTTP dependency vetting).
- Filed: none.
