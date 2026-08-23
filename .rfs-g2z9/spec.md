# Spec: Read allowlisted HTTPS Resources safely

## Request (verbatim)
> Ahh, then lets do rfs-g2z9 instead

Issue rfs-g2z9 — "Make HTTPS a safe Portable Source: allowlisted references return bounded reader-mode Markdown/text or raw bodies, support search and recovery, and enforce host, redirect, resolved-address, private-network, credential, timeout, and cancellation policy. This slice also proves required and Degraded Source lifecycle and connectivity probes."

Substrate charter (tracker note, 2026-08-22): build this as the project's one bounded HTTP substrate, then HTTPS reader mode on top of it; `rfs-45ww`/`rfs-by2z` (GitHub) and `rfs-azd3` (downstream MCP over Streamable HTTP) consume it rather than adding a second client or policy layer.

## What this is

ResourceFS gains its first network Source Adapter: `https://` references to profile-allowlisted origins resolve to bounded, deterministic reader-mode Markdown by default, or the bounded original body under `:raw`. Underneath it sits the project's single bounded HTTP substrate, which owns allowlist enforcement, per-redirect and per-resolved-address authorization, the private-network grant, body/time ceilings, and cancellation — so later network sources reuse one audited policy path instead of adding their own.

## Roles

- **Coding agent**: reads and searches allowlisted web documents through `rfs_read`/`rfs_search` using the same Path Reference grammar, limits, selectors, and `artifact://` recovery as every other source; never configures or bypasses policy.
- **ResourceFS operator**: declares HTTPS origins in the Server Profile (`base_url`, optional credential, `allow_private_network`, `required`), and is the only party who can widen what the agent may reach.

## Behavior

### Read an allowlisted document in reader mode
- **Given**: a Server Profile declaring an HTTPS origin whose `base_url` prefixes the requested URL, the source is available, and the response is an HTML document within the fetch ceiling.
- **When**: the coding agent calls `rfs_read` with that `https://…` reference.
- **Then**: ResourceFS performs one fresh request, extracts reader-mode Markdown from the **complete** document using the in-house bounded extractor, and returns it through the common bounded-read contract — content-derived Version Tag over the extracted Markdown, `mutable: false`, `text/markdown` content type, and, when the Markdown exceeds the effective text ceilings, a lossless `artifact://` recovery reference plus continuation.

### Read the original body with `:raw`
- **Given**: the same allowlisted, available reference.
- **When**: the coding agent calls `rfs_read` with a `:raw` projection (optionally combined with a line selector).
- **Then**: ResourceFS returns the charset-decoded original response body under the same bounded-read contract, with the Version Tag derived over that body and the content type reflecting the response, applying no extraction.

### Reject an over-ceiling document
- **Given**: an allowlisted reference whose response body exceeds the fetch ceiling.
- **When**: the coding agent calls `rfs_read` in reader mode.
- **Then**: ResourceFS stops reading at the ceiling, performs **no extraction**, and returns `limit_exceeded` naming the ceiling and directing the caller to `:raw` with a selector for a bounded slice. Extraction never runs over a truncated document.

### Search an allowlisted document
- **Given**: an allowlisted, available reference.
- **When**: the coding agent calls `rfs_search` with that reference as the target.
- **Then**: ResourceFS fetches it once for that call and returns matching lines over the reader-mode Markdown through the shared discovery engine and the common bounded/paginated result contract.

### Authorize every redirect hop and resolved address
- **Given**: an allowlisted reference whose request redirects, and/or whose host resolves to one or more IP addresses.
- **When**: ResourceFS performs the request.
- **Then**: each redirect target is re-checked against the origin allowlist before it is followed, and each resolved address is checked against the private/special-address policy before connection; the connection is made to the **validated address** so a later re-resolution cannot substitute a different one. A hop or address that fails policy returns `permission_denied` without following it. Redirect depth is bounded; exceeding it returns `limit_exceeded`.

### Require a distinct grant for private and special addresses
- **Given**: an allowlisted origin whose `allow_private_network` is false, resolving to a loopback, link-local, private, or otherwise special address.
- **When**: the coding agent reads or searches it.
- **Then**: ResourceFS returns `permission_denied` and makes no connection to that address. The same request against an origin whose `allow_private_network` is true proceeds.

### Deny an unallowlisted reference without leaking
- **Given**: an `https://` reference matching no declared origin, or one whose configured credential is unavailable.
- **When**: the coding agent reads or searches it.
- **Then**: ResourceFS returns `permission_denied` (unallowlisted) or `source_unavailable` (credential unavailable) whose text names the reference and the policy that rejected it, and never includes credential material, resolved IP addresses, or upstream response bodies.

### Honor cancellation and the request timeout
- **Given**: an in-flight HTTPS read.
- **When**: the MCP request is cancelled, or the request exceeds its timeout ceiling.
- **Then**: cancellation returns `cancelled` and the timeout returns `source_unavailable`; in both cases the connection is abandoned, no partial content is returned as if complete, and no Path Session state is published for the abandoned call.

### Fail startup for a required source; degrade an optional one
- **Given**: a Server Profile declaring an HTTPS source as `required: true` (or `false`) whose connectivity probe fails.
- **When**: `resourcefs serve` starts.
- **Then**: a failing **required** source fails startup with a non-zero exit and a diagnostic naming the source id; a failing **optional** source enters the visible Degraded state, the server starts, every other source keeps working, and only that source's references fail with `source_unavailable`.

### Report connectivity through an explicit probe
- **Given**: a configured HTTPS source.
- **When**: the operator runs the explicit non-mutating probe (`resourcefs check` probe mode).
- **Then**: it reports reachability per source without mutating anything and without running during ordinary reads.

## Success criteria

- **Binary / structural / security**: a request to a host that resolves to a private, loopback, link-local, or special address is refused with `permission_denied` and **no socket is opened to that address** unless the origin's `allow_private_network` is true; checked by a direct adapter test using a resolver fixture that returns such addresses, asserting connection attempts by observation, not by return value alone.
- **Binary / structural / security**: the connection is established to the exact address that passed policy — a resolver fixture returning a policy-passing address on the first resolution and a private address on a second resolution never results in a connection to the second; checked by a DNS-rebinding fixture.
- **Binary / structural / security**: every redirect hop is re-authorized against the origin allowlist before being followed; a redirect chain that leaves the allowlist returns `permission_denied` and the off-allowlist URL is never requested, checked by a fake-server fixture that records every received request path.
- **Quantitative**: a response body exceeding the fetch ceiling returns `limit_exceeded` with no extraction performed, and a body at exactly the ceiling succeeds; measured by boundary fixtures at ceiling and ceiling+1 bytes.
- **Quantitative**: redirect chains deeper than the configured depth return `limit_exceeded`; measured at depth and depth+1.
- **Binary / structural / security**: reader-mode extraction is deterministic — the same input document yields byte-identical Markdown and the identical Version Tag across runs and processes; checked by golden fixtures over a corpus including malformed, script/style-heavy, unicode, and deeply nested documents.
- **Binary / structural / security**: no error message, log line, probe report, or tool result contains credential material, and none contains a resolved IP address; checked by unique-sentinel scans over every observable channel across success and every failure mode, following the rfs-r9m6 secret-sink pattern.
- **Binary / structural / security**: reads, searches, limits, selectors, cancellation, and `artifact://` recovery for HTTPS use the common contracts with no HTTPS-specific result fields; checked by stdio MCP contract assertions.
- **Binary / structural / security**: a `required: true` source whose probe fails prevents startup with a non-zero exit; an optional one starts Degraded and fails only its own references; checked by CLI contract tests following the rfs-r9m6 exit/channel matrix.
- **Binary / structural / security**: the bounded HTTP substrate is the single point of network egress — no other module constructs an HTTP client or performs its own allowlist/address policy; checked by an architecture-contract scan asserting the client crate is referenced in exactly one module.
- **Quantitative**: an HTTPS read completes within the request timeout ceiling and a cancelled read abandons its connection promptly; measured against a deliberately slow fake server.

## Out of scope

This change does NOT include: HTTPS mutation of any kind (the source is read-only); HTTP (non-TLS) origins; authentication flows beyond a configured static credential reference; response caching within or across Path Sessions; JavaScript execution or headless rendering; boilerplate/navigation stripping heuristics (the extractor is a defined tag-subset pass, not a readability heuristic); content negotiation beyond charset/MIME selection between reader mode and `:raw`; GitHub-specific behavior (`rfs-45ww`/`rfs-by2z`, which consume this substrate); downstream MCP over Streamable HTTP (`rfs-azd3`, likewise); cookie jars or session affinity; and proxy configuration.

## Related issues

- **rfs-r9m6** (closed): built and validated the HTTPS profile shape this change consumes — `HttpsSourceProfile` with per-origin `base_url`, `allow_private_network`, credential reference, the `required` flag, `MutationGrants`, and `StaticProbe::Network`. Configuration is done; this issue adds the runtime.
- **rfs-vl0u** (closed): the `DiscoveryAdapter` search engine HTTPS search routes through.
- **rfs-34pz** (closed): bounded reads, selectors, `artifact://` recovery, and Path Session ownership reused unchanged.
- **rfs-73dz** (closed): `OperationGuard` cancellation semantics reused; its read-only classification means `mutable: false` for HTTPS.
- **rfs-ewh2** (closed): `SourceCatalogMetadata` — `https://` must self-list in `rfs://` once mounted.
- **rfs-45ww / rfs-by2z** (open, dependents): GitHub over native HTTP APIs consumes this substrate.
- **rfs-azd3** (open, dependent): downstream MCP over allowlisted Streamable HTTP consumes this substrate.

## Decisions

| Question | Decision | Rationale | Implication |
|---|---|---|---|
| How does reader-mode extraction work? | An in-house bounded HTML→Markdown pass: drop `script`/`style`/`svg`, keep a defined tag subset (headings, paragraphs, lists, links, code, tables), normalize whitespace. | Requester selected "In-house bounded extraction". Deterministic by construction, so content-derived Version Tags stay stable and golden fixtures are viable; one small version-pinned tokenizer instead of a heuristic extractor whose upgrades would churn tags for unchanged upstream content. | No boilerplate/nav stripping; a cluttered page yields cluttered Markdown. Extraction rules are golden-tested and change only deliberately. |
| How do the fetch ceiling and extraction interact? | Stream to a hard fetch ceiling; over-ceiling returns `limit_exceeded` with `:raw`+selector guidance and performs **no** extraction. Extraction runs only over a complete document; the resulting Markdown then obeys the normal text ceilings with `artifact://` recovery. | Requester selected "Fetch ceiling, then extract complete". Extracting from truncated markup can silently mangle or swallow content, and a `bounded` flag cannot express "and possibly wrong". | Very large pages are unreadable in reader mode by design; `:raw` with a selector remains the escape hatch. |
| Are HTTPS responses cached in a Path Session? | No — every `rfs_read`/`rfs_search` performs a fresh request. | Requester selected "No cache — every read fetches". Always current, no invalidation story, no competition for the session budget. | One round trip per call; the same reference may return different content and Version Tags within a session, which is safe because HTTPS is read-only. Continuations still come from the immutable artifact, never a re-fetch. |
| Which HTTP client and TLS backend? | **`reqwest` 0.13.2**, selected by the requester on 2026-08-23 with `evidence.md` P1–P3 in hand. Address authorization lives **inside** a `Resolve` implementation supplied to `ClientBuilder::dns_resolver`; per-hop vetoes use `redirect::Policy::custom`; bounded bodies use `bytes_stream()`. All are ungated public hooks — no fork, no private API. | The probe proved reqwest calls the resolver once per new connection and connects to exactly the addresses it returns, and that returning `Err` opens no socket at all. The `hyper`+`hyper-util`+`rustls` alternative satisfies the same requirement but would require hand-writing redirect following, pooling, decompression, and charset handling — all security-sensitive, and the probe evidence covers reqwest's implementations rather than ones we would write. | Accepts a large transitive tree (to be vetted in `deny.toml`) in exchange for audited, probe-verified security paths. |
| Does the fetch ceiling bound bytes transferred? | No — it bounds what ResourceFS **accepts and retains**, not network transfer. | `evidence.md` P3 learning: with a 1 MiB client ceiling on a 64 MiB body the server still flushed ~3 MiB before observing the peer leave. Claiming the stronger guarantee would be false. | Documentation and any operator-facing description state the accept/retain bound explicitly; the criterion is measured on retained bytes. |
| May canonical output derive from a `Debug` rendering? | No — never. Extraction and any content feeding a Version Tag must be built from explicit field access, not `{:?}`. | `evidence.md` P4 learning: the probe's first canonicalisation used `{:?}` on a `StrTendril`, embedding tendril's inline-vs-heap storage discriminator, so the digest changed with string length. The hand-authored oracle caught it. | Golden fixtures include a case that crosses the tendril inline/heap storage boundary. |
| What is the fetch ceiling? | **Proposed: 8 MiB**, lowerable by the Server Profile, never raisable. | Large enough for any reasonable document, far below the 64 MiB artifact ceiling so a complete body plus its extraction stays bounded. | Confirm or adjust at sign-off. |
| What is the redirect depth limit? | **Proposed: 5 hops**, then `limit_exceeded`. | Matches common client defaults and bounds re-authorization work. | Confirm or adjust at sign-off. |
| What is the request timeout? | **Proposed: 30 seconds total per request**, lowerable by the Server Profile. | Bounds a hung upstream without failing slow-but-legitimate documents. | Confirm or adjust at sign-off. |
| Is HTTPS mutable? | No — read-only in 1.0; `mutable: false`, and every write/edit returns `unsupported_mutation`. | Issue title and DESIGN.md scope HTTPS as a read source; mutation belongs to GitHub's native API path. | No `MutationAdapter` for this source. |
| What scopes a reachable URL? | The requested URL must be prefixed by a declared origin's `base_url`. | rfs-r9m6 already models `base_url` per origin. | Path-level scoping, not host-level; an operator can expose a subtree without exposing a whole host. |
| Empty set | An allowlisted document with no extractable content yields an explicit non-empty status line, not empty content; a search with no match returns the common empty-page status line. | Common bounded-result contract. | No empty tool result on any path. |
| Max scale | Fetch ceiling 8 MiB; redirect depth 5; text ceilings and `artifact://` recovery as inherited. | Ceilings above. | Boundary-tested at each limit and one over. |
| Null / missing field | A response with no charset falls back to a declared default and is decoded deterministically; a missing content type is treated as opaque and served through `:raw` semantics. | Determinism requirement. | Decoding rules are golden-tested. |
| Concurrent writes | N/A — the source is read-only; concurrent reads share no mutable state. | Read-only scope. | No serialization needed. |
| Permission denied / unauthenticated | Unallowlisted origin, off-allowlist redirect, or ungranted private address → `permission_denied`; unavailable credential → `source_unavailable`. Neither leaks credentials or resolved addresses. | Policy-before-state precedence inherited from rfs-73dz. | Denial never reveals whether the remote resource exists. |
| Partial failure | A response abandoned mid-body (timeout, cancellation, connection reset) returns an error; partial content is never returned as if complete, and no artifact is published for it. | Lossless-or-fail posture. | Callers retry explicitly. |
| Retries / idempotency | ResourceFS performs no automatic retries; each call is one request. | The project already refuses automatic retry of unknown upstream outcomes (DESIGN.md). | A transient failure surfaces to the caller, who re-reads. |
| Soft-deleted records | N/A — no deletion lifecycle for a read-only remote source. | No applicable domain object. | An upstream 404 is `not_found`. |
| Multi-tenancy boundaries | Policy is per Server Profile and process-wide; recovery artifacts remain per Path Session. | Existing session/authority model. | One connection cannot observe another's artifacts. |
| Time-zone / DST | N/A — Version Tags are content-derived; no wall-clock value appears in the observable contract. | Content-derived state. | Timeouts are durations, not timestamps. |
| Replication lag | N/A — no replicated store; upstream freshness is the origin's concern. | Explicit scope. | No consistency claim beyond "this is what the origin returned". |
| Cache invalidation | N/A — no cache exists (decision above). | No cache. | Nothing to invalidate. |

## Approval

Requester approval (verbatim): "I agree"
Date: 2026-08-23

Confirmed at sign-off without objection: fetch ceiling 8 MiB, redirect depth 5 hops, request timeout 30 seconds — each lowerable by the Server Profile and never raisable.

Deferred decision resolved 2026-08-23 after `prove-it-prototype` recorded P1–P4 PASS: the requester selected **`reqwest` 0.13.2** as the HTTP client. Two probe learnings were added to the Decisions table at the same time (the fetch ceiling bounds accepted/retained bytes rather than network transfer; canonical output must never derive from a `Debug` rendering).

Requester approval (verbatim): "reqwest 0.13.2"
Date: 2026-08-23
