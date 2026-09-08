# Atlassian fixture operator: package reuse

Date: 2026-09-07. Context: rfs-br1u; implementation has not been migrated. This is package research, not an approved architecture or performance benchmark.

## Recommendation

Reuse general HTTP/JSON/CLI facilities; retain a small ResourceFS-owned lifecycle implementation. If selecting Python, prefer HTTPX plus explicit endpoint calls over adopting an SDK as the lifecycle owner. If selecting Rust, reqwest/serde/clap/tokio already exist in the workspace. The reason for custom policy is the current ownership and recovery contract, not speculative flexibility.

No turnkey replacement covering both products and the fixture safety contract was verified in this bounded search. That is not a claim of global nonexistence. An Atlassian SDK can remove CRUD plumbing but does not establish our marker authority, uncertain-write recovery or final absence proof.

## Required custom responsibilities

- Manifest namespace and exact ownership markers; foreign-object refusal.
- Ownership revalidation immediately before destructive actions.
- Provisioner and reader credential separation.
- Locking, atomic receipts and recovery after an uncertain creation response.
- Bounded request/pagination/poll handling and trusted continuation URLs.
- Distinguishing trash, absent owner containers, missing collections and completed asynchronous deletion.
- Removing receipts only after authoritative absence, including Jira project trash.

These requirements are evidenced by the current operator and `.rfs-br1u/evidence.md`; they are not a reason to rewrite HTTP, TLS, JSON or command-line parsing ourselves.

## Python options

### HTTPX

Official documentation establishes pooled connections through `Client`, raw status/headers/body access, response streaming, explicit network timeouts, no redirect following by default, and injectable transports including `MockTransport`.

The optional `HTTPTransport(retries=N)` retries ConnectError/ConnectTimeout, not arbitrary read/write/status failures. Mutation replay and outcome reconciliation remain our policy. Network-operation timeouts are not a total fixture-run deadline; response streaming must still enforce our byte ceiling.

This removes repeated curl/jq processes while preserving exact endpoint and response semantics. Expected speed improvement from connection reuse and fewer processes is an inference; no comparative benchmark was run.

Sources:
- https://www.python-httpx.org/advanced/clients/
- https://www.python-httpx.org/advanced/transports/
- https://www.python-httpx.org/compatibility/
- https://www.python-httpx.org/api/

### atlassian-python-api

A credible CRUD SDK, not legacy-only: current docs name ConfluenceCloud and explicit ConfluenceV2; v2 content-property operations exist. Main inspected upstream source via GitHub file_read:

- `get_v2_content_properties(content_type, content_id, cursor=None, limit=25)` returns `list(self._get_paged(...))`. This convenience method has no `key` filter parameter or explicit total-page budget; `limit` is not a global resource bound.
- The underlying `request` supports raw responses through advanced_mode and `allow_redirects=False`, but defaults `allow_redirects=True`; inspected high-level get/post/delete signatures do not expose that parameter.
- `retry_with_header=True` defaults to a single 429 Retry-After retry even when exponential backoff is disabled, without an HTTP-method gate in that handler. Enabling urllib3 backoff uses `allowed_methods=None`.
- `_response_handler` returns None on JSON decoding failure, which would need explicit handling to preserve the operator's missing/corrupt distinction.

These are configuration/interface mismatches, not a declaration that the SDK is unsafe or unusable. Raw lower-level calls and deliberate configuration can preserve our contract, but reduce the convenience advantage. Current upstream source/docs were inspected, not a pinned installed release.

Sources:
- https://atlassian-python-api.readthedocs.io/confluence.html
- https://github.com/atlassian-api/atlassian-python-api/blob/HEAD/atlassian/confluence/cloud/content_properties.py
- https://github.com/atlassian-api/atlassian-python-api/blob/HEAD/atlassian/rest_client.py

### pycontribs jira

Published source exposes `delete_project(pid, enable_undo=True)`: passing False performs permanent deletion, while the default permits trash recovery. The helper returns `r.ok`, not exact success status or response headers. The client defaults max_retries=3 and timeout=None; ResilientSession retry decisions do not gate on the HTTP method. Explicit mutation retry policy and timeout configuration are therefore necessary.

This is useful for broader Jira automation, but adding it just for fixture CRUD does not remove the lifecycle work. It supplies no Confluence implementation.

Sources:
- https://jira.readthedocs.io/_modules/jira/client.html#JIRA.delete_project
- https://jira.readthedocs.io/_modules/jira/resilientsession.html#ResilientSession

## Rust options

### Existing workspace foundations

At source 7de8d14, Cargo.toml already defines reqwest 0.13.2, serde, serde_json, tokio and clap. reqwest Client pools connections; this stack can implement a standalone development operator without depending on ResourceFS production adapters. Sharing low-level third-party libraries is compatible with keeping the fixture oracle independent of the adapter under test.

Sources:
- repository Cargo.toml lines 32, 44–51
- https://docs.rs/reqwest/latest/reqwest/struct.Client.html
- https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html

### jira_v3_openapi 1.6.1

Strong verified Jira endpoint coverage: generated project lifecycle and property operations, with issue/comment operations in its endpoint inventory. Package metadata identifies OpenAPI generation and reqwest ^0.12.

A concrete loss of information matters here: `delete_project_asynchronously` returns `Result<(), ...>` while its own documentation tells callers to follow the response Location header. The inspected public result does not expose that success header. Inference: callers needing it must bypass/extend the helper or obtain task identity separately.

This is a Jira endpoint client, not a combined fixture reconciler. Its reqwest 0.12 dependency would also introduce a different dependency line from the current workspace unless adapted.

Sources:
- https://docs.rs/crate/jira_v3_openapi/1.6.1
- https://docs.rs/jira_v3_openapi/1.6.1/jira_v3_openapi/apis/projects_api/
- https://docs.rs/jira_v3_openapi/1.6.1/jira_v3_openapi/apis/projects_api/fn.delete_project_asynchronously.html

### Other bounded candidates

- jc-conf 0.2.0 supplies page CRUD and space reads; its inspected space module has list/find_by_key/resolve_id/get, not space create/delete. Its public module inventory did not cover ownership properties or longtasks. jc-jira 0.2.0 covers issue-oriented operations, not the required project lifecycle.
- lib-client-confluence 0.1.0 has a narrower page/space-reading interface, with no space creation/deletion or longtask methods in the inspected public Client.
- threatflux-atlassian-sdk 0.5.1 explicitly describes focused Jira automation, not a complete Jira/Confluence SDK.

Sources:
- https://docs.rs/jc-conf/0.2.0/jc_conf/space/
- https://docs.rs/jc-conf/0.2.0/jc_conf/
- https://docs.rs/jc-jira/0.2.0/jc_jira/
- https://docs.rs/lib-client-confluence/0.1.0/lib_client_confluence/struct.Client.html
- https://docs.rs/threatflux-atlassian-sdk/0.5.1/threatflux_atlassian_sdk/

## Ready-made provisioning screen

Terraform Registry inventories for procorp-solutions/jira, surajrajput1024/atlassian and fabiogermann/confluence did not cover the full issue/comment/page fixture graph across both products. The inspected Jira providers focus on projects/configuration; the inspected Confluence provider inventory covers spaces/permissions/groups, not pages. That is sufficient to reject these as turnkey replacements without claiming their deletion internals lack particular safety checks. Provider implementations were not audited.

Sources:
- https://registry.terraform.io/v1/providers/procorp-solutions/jira
- https://registry.terraform.io/v1/providers/surajrajput1024/atlassian
- https://registry.terraform.io/v1/providers/fabiogermann/confluence

## Decision boundary

Recommended composition: a library supplies transport and parsing; our operator supplies fixture intent, ownership and durable recovery. Python/HTTPX favors maintenance speed; Rust/existing workspace libraries favor compile-time state constraints and one dependency ecosystem. Neither language choice is approved by this research. No packages were installed, no migration code was written, and no performance claim was measured.
