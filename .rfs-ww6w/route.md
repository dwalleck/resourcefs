# Route: rfs-ww6w

Change: Mirror Path References through bounded MCP Resources, templates, completion, and subscriptions
Date: 2026-08-23

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | **Yes — rmcp 3.1.3's Resources/completion/subscription runtime behavior is unexercised by this repository.** The server advertises only `ServerCapabilities::builder().enable_tools()` (`server.rs:1147`) and implements no resource handler (`list_resources`/`read_resource`/`list_resource_templates`/`complete`/`subscribe`/`unsubscribe` are all absent). Three concrete unknowns, each load-bearing: (a) **there is no plain `enable_resources()` builder** — only `enable_resources_list_changed()` and `enable_resources_subscribe()` (`rmcp/src/model/capabilities.rs:410,417`), so how the Resources capability is advertised at all is unverified; (b) `notify_resource_updated` is documented as sending `notifications/resources/updated` "for an **accepted** URI" and returns `SubscriptionSendError` (`rmcp/src/service/server.rs:307-316`), implying rmcp owns subscription state — whether the server or rmcp tracks subscribed URIs, and what makes a URI "accepted", is unverified; (c) whether notifications actually reach a client over a live stdio session end-to-end is unverified. Reading the SDK source gives API shape, not runtime behavior. This project has twice been surprised by exactly this class of premise: rmcp's cancellation semantics (it drops cancelled requests from the response pool before the handler finishes — documented at `server.rs:237-240`) and Kiro's MCP quirks (discarded `structuredContent`, unreliable `resource_link`, output-schema strictness — DESIGN.md:62-72). Pagination is *not* an unknown: `list_resources` takes `Option<PaginatedRequestParams>` exactly like the already-implemented `list_tools`, so cursor handling is ours and is covered by existing evidence. | yes |
| 2 | Structural boundary | Yes. Adds a new public MCP surface — `resources/list`, `resources/templates/list`, `resources/read`, `completion/complete`, `resources/subscribe`/`unsubscribe` and their notifications — changing advertised `ServerCapabilities` and requiring a new resource-mirror module plus placement decisions for mapping `ResourceAddress` families onto concrete Resources versus URI templates. Touches `resourcefs-mcp` (`server.rs`, `render.rs`) and needs a seam to enumerate the address space without duplicating adapter knowledge. | yes |
| 3 | Production-scale risk | Yes. Resource listing enumerates the resolvable address space (workspace roots, artifacts, scratch) and the acceptance criteria forbid unbounded or keystroke-time network enumeration; completion runs per keystroke and must stay local and deterministic. Both need budgets and stress fixtures that Local has no machinery for. | yes |
| 4 | Explicit behavior | No — unresolved observable decisions: which families mirror as concrete Resources versus URI templates; the mirror's URI identity (canonical `rfs://`/`artifact://`/`local://` versus a distinct mirror scheme) and its relationship to the DESIGN.md rule that MCP Resource identity always uses the canonical private `rfs://` URI; listing page size and cursor encoding; what `completion/complete` completes (schemes, root ids, workspace paths, scratch names) and its bound; subscription granularity (per-Resource, per-root, per-session) and which mutations emit `resources/updated` versus `list_changed`; text-versus-blob and MIME selection per family; behavior for a client that negotiates no Resources capability; and whether bounded/spilled reads mirror the artifact or the bounded page. | no |

Unknown tests: none

## Selected route

Empirical — rmcp's Resources/subscription/completion runtime behavior is an unverified external premise (capability advertisement, subscription ownership, live notification delivery), and the change also adds a public protocol surface with enumeration/completion scale risk and unresolved observable behavior. Precedence: Empirical > Structural > Local.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — T4 no: mirror identity, template/concrete split, completion scope, subscription granularity, and no-capability fallback are unresolved |
| evidence.md, probe.* | prove-it-prototype | required — T1 yes: rmcp capability advertisement, subscription ownership, and live notification delivery must be probed against an independent oracle |
| design.md | falsifiable-design | required — Empirical route |
| plan.md | budgeted-plan | required — Empirical route |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

interrogated-spec → prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `prove-it-prototype` records PASS for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no FAIL.
