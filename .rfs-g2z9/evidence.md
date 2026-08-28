# Evidence: rfs-g2z9

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | An async Rust HTTP client can let ResourceFS authorize a resolved address and then connect to *that exact address*, closing the DNS-rebinding/TOCTOU window the SSRF control exists to prevent. | When a resolver hook returns address A, does the connection land on A — and when the hook denies, is any connection attempted at all? | PASS |
| P2 | Each redirect hop can be inspected and vetoed **before** it is requested, so an off-allowlist URL is never sent. | When a redirect points off the allowlist and policy stops it, does the off-allowlist server receive any request? | PASS |
| P3 | A response body can be streamed and abandoned at a byte ceiling without buffering it whole, and an in-flight request can be cancelled by dropping its future. | After the client stops at a 1 MiB ceiling on a 64 MiB body, how many bytes did the server actually flush — and does it stop? | PASS |
| P4 | The HTML tokenizer is byte-deterministic for identical input across repeated runs and across separate processes, so content-derived Version Tags over extracted output are stable. | Do the canonical token digests for a fixed corpus agree across two independent processes, and does the small document match a hand-authored token stream? | PASS |
| N1 | Reader-mode/`:raw` split, fetch ceiling (8 MiB), redirect depth (5), timeout (30 s), no session caching, `base_url` scoping, denial-error shape. | N/A — requester-approved behavior fixed in `spec.md`; the spec owns it, not this stage. | N/A — specification owns the behavior |
| N2 | Throughput, latency budgets, and memory bounds for the substrate at production scale. | N/A — performance and scale targets for code not yet built. | N/A — design/checkpoint territory |
| N3 | Placement of policy (core) versus transport (sources), and the substrate's public interface. | N/A — a claim about the feature being built; nothing exists yet. | N/A — `falsifiable-design` territory |

## Data

- Source: production-shaped.
- Shape: real TCP sockets on loopback, real HTTP/1.1 request and response framing, a 64 MiB streamed body matching the untrusted-remote-input shape the fetch ceiling must bound, and an HTML corpus covering the document classes the extractor will meet (well-formed, malformed/unclosed, script- and style-bearing, Unicode including astral characters, attribute-heavy, 256-level nesting, entity-bearing, and a doctype name long enough to cross tendril's inline/heap storage boundary).
- Safety: **no external network egress.** Every request targets a loopback listener started by the probe itself; hostnames use the reserved `.invalid` TLD and are resolved only by the probe's own in-process resolver hook, so no name ever reaches a system resolver. The dependency build ran `cargo build --offline` against the existing local registry cache, adding nothing to the workspace. The probe compiles a standalone Cargo project under a temporary directory and touches no production code, no repository file, and no operator-owned data. Approval: N/A — generated production-shaped data on loopback is safe.

## Probe

- File: `probe_http_substrate.py` (drives four standalone Rust binaries in a temporary Cargo project, mirroring the rfs-73dz probe pattern)
- Mechanism:
  - **P1** binds two listeners sharing one port on two distinct loopback IPs (`127.0.0.1` and `127.0.0.2`) — distinguished by IP because reqwest documents that an explicit URL port overrides the resolver's port. A custom `reqwest::dns::Resolve` returns the *authorized* address on its first call and a *rebound* address afterwards; a second client's resolver denies outright.
  - **P2** serves a redirect to an off-allowlist origin and a redirect that stays on-allowlist, with `reqwest::redirect::Policy::custom` applying an allowlist to `attempt.url()`.
  - **P3** streams a 64 MiB body while the client stops at a 1 MiB ceiling and drops the stream; a second scenario drops an in-flight request future under a timeout.
  - **P4** canonicalizes the html5ever token stream (attributes sorted, storage-independent rendering) and SHA-256s it.
- Run: `./.rfs-g2z9/probe_http_substrate.py`

## Oracle

- Mechanism — each computes the same answer through a mechanism that cannot share the client library's failure mode:
  - **P1**: **server-side accept counters.** Ground truth is which listener actually accepted a TCP connection, recorded by the listener — never the client's report of where it connected.
  - **P2**: **server-side request-path logs.** Each listener records every path it actually received; the off-allowlist path must appear in no log.
  - **P3**: **server-side flushed-byte counters.** The server counts bytes it actually wrote and flushed before the peer went away, independent of any client-side accounting.
  - **P4**: **cross-process digest comparison** (two separate OS processes, so no shared in-process state can mask instability) plus a **hand-authored token stream** for the small document, written from the markup by reading the HTML tokenizer specification rather than by running the probe.
- Run: the oracles execute inside the same probe invocation; the P4 cross-process comparison is performed by the Python driver over two independent binary runs.
- Reproducibility: the whole probe was executed twice end to end. Every verdict-critical field was identical between runs — P1 accept counts (1 authorized / 1 rebound; 0 / 0 under denial), P2's empty off-allowlist path log, P3's `serverWroteWholeBody = false` and its 3,145,728-byte flush count, P3's `serverStoppedWriting = true`, and all P4 digests. The recorded numbers are stable, not a single lucky sample.

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | Request 1 → HTTP 200; request 2 (resolver now returns the rebound address) → also succeeds; resolver called **2 times for 2 requests**. Denying resolver → request errors. | Authorized listener accepted **1**, rebound listener accepted **1** — each request connected to exactly the address its resolver call returned. Under denial: **0 new accepts on both listeners**, resolver called once. | PASS |
| P2 | Policy inspected `http://offsite.invalid:PORT/secret` and `http://allowed.invalid:PORT/landing` **before** either was requested. Off-allowlist chain terminated at the 302 with the final URL still `/hop-offsite`; on-allowlist chain returned 200. | Off-allowlist server path log is **empty (`[]`)** — `/secret` was never requested. Allowed server log is exactly `["/hop-offsite", "/hop-onsite", "/landing"]`. | PASS |
| P3 | Declared body 67,108,864 bytes; client read 1,245,131 bytes then dropped the stream. Cancellation scenario timed out early as designed. | Server flushed **3,145,728 of 67,108,864 bytes (4.7%)** before the peer closed; `serverWroteWholeBody = false`. Under cancellation the server flushed 393,216 bytes at cancel and 458,752 after settling, then stopped. | PASS |
| P4 | Digests for all 8 corpus documents; small-document token stream emitted. | Digests **identical across two separate processes** (e.g. `long_doctype` = `bc0319f5…` in both); all 8 stable on repeat within a process; small-document stream **matches the hand-authored expectation exactly**. | PASS |

## Validated / learned

- **P1 — learning that sharpens the design.** Believed: the client might re-resolve between validation and connect, leaving a TOCTOU gap. Observed: reqwest calls the resolver **once per new connection** and connects to **exactly** the addresses that call returned; a resolver that returns `Err` prevents any connection attempt entirely (0 accepts on both listeners). The consequence is that **the resolver hook is itself the enforcement point** — there is no "validate, then connect" gap to close, because validation *is* the resolution step. Policy belongs inside the `Resolve` implementation, not beside it. Corroborated structurally: reqwest's TLS variants (`NativeTls(HttpConnector, …)`, `Rustls { http: HttpConnector, … }`) wrap the *same* `HttpConnector<DynResolver>`, so the hook applies identically on the HTTPS path and TLS cannot change the peer address.
- **P2 — validated prior understanding.** `redirect::Policy::custom` receives each hop's URL before the request is issued, and `attempt.stop()` prevents it. The off-allowlist server's empty path log is direct evidence the URL was never sent, not merely that the response was discarded.
- **P3 — learning with a design consequence.** Believed: a byte ceiling bounds the data transferred. Observed: with a 1 MiB client ceiling the server still flushed ~3 MiB before noticing the peer had gone. The ceiling bounds **what ResourceFS buffers and exposes**, not what the peer transmits — socket and client buffers sit in between. The design must therefore state the ceiling as a bound on accepted/retained bytes, and must not assume network transfer equals accepted bytes. Cancellation by dropping the future does propagate: the server stopped well short of the full body.
- **P4 — learning caught by the oracle, exactly as intended.** The first canonicalization rendered the doctype with `{:?}` on its `StrTendril`, producing `Tendril<UTF8>(inline: "html")`. The hand-authored oracle disagreed, which exposed that the record embedded tendril's **inline-versus-heap storage discriminator** — a value that changes with string length and would have made "deterministic" digests silently unstable for longer names. The probe was wrong, not the oracle: the fix renders the doctype *name* as a plain string, and a `long_doctype` case was added whose name deliberately crosses the inline/heap boundary. It now digests identically across processes. This is a hazard the production extractor must avoid too: canonical output must never be derived from a `Debug` rendering of a library type.

### Client-crate finding (decides the pending architecture decision)

`reqwest` **0.13.2** satisfies P1–P3 with ungated, public hooks and no bespoke connector:

- `ClientBuilder::dns_resolver<R: Resolve>` — the policy point. `Resolve::resolve(&self, Name) -> Resolving` yields `Addrs = Box<dyn Iterator<Item = SocketAddr>>`; returning only policy-passing addresses authorizes, and returning `Err` denies before any socket is opened.
- `ClientBuilder::resolve` / `resolve_to_addrs` — static pinning for a known host, layered on top of the custom resolver.
- `redirect::Policy::custom` with `Attempt::url()` / `previous()` / `follow()` / `stop()` — per-hop authorization before the request is sent.
- Body streaming via `bytes_stream()` (the `stream` feature) and cancellation by dropping the future, both composing with tokio.

`hyper` + `hyper-util` + `rustls` would also satisfy P1 by giving full connector control, but requires hand-building the connector, redirect handling, and body plumbing that reqwest already exposes — strictly more code for the same guarantees. No blocking client was evaluated, because the codebase is tokio-based and a blocking client would be a poor fit regardless.

**Residual, recorded rather than papered over:** the probe exercised the mechanism over plain HTTP, not TLS. Address authorization happens in the connector *below* TLS, and the structural reading above shows the TLS variants reuse the same resolver-driven `HttpConnector` — but the TLS path was not executed. Two items follow for `falsifiable-design` (both design concerns, not unverified premises): certificate verification must remain bound to the **hostname** while the connection is pinned to the validated **address**; and connection pooling means a reused connection does not re-enter the resolver, so the design should state that pooled reuse to an already-authorized address is the intended behavior.

## Live read smoke (2026-08-27)

P1–P6 proved the substrate's mechanisms; the HTTPS Source Adapter itself had only ever read fixture HTML from a local TLS listener. `crates/resourcefs-sources/tests/https_live_smoke.rs` (ignored; `RFS_LIVE=1`; run by `scripts/live-smoke.sh`) now drives the real `HttpsSource` + `HttpSubstrate` against public origins, GET only. First run: H4 failed — a real `404` page was rendered and returned as a successful 859-byte Resource because nothing consulted the status (the fixtures had only ever served `200` to the adapter). Filed and fixed as **rfs-0ox5**; the permanent fence is `https_status_contract.rs`. Second run: 5 rows, all PASS, 1.4 s.

| ID | Row | Observation | Verdict |
|----|-----|-------------|---------|
| H1 | `https://doc.rust-lang.org/stable/book/ch01-01-installation.html` reader mode | 6,826 bytes / 165 lines of article containing `Installation` and `rustup`, no markup; a second read produced the same Version Tag. | PASS |
| H2 | same reference `:raw`, then `:1-5` | 30,474 bytes of served markup under the same canonical identity; the slice was at most five lines. | PASS |
| H3 | `https://docs.rs/serde` | The real `302` to `/serde/latest/serde/` was followed within the origin; 7,604 bytes rendered under the requested identity. | PASS |
| H4 | a real `404` page; `https://www.rust-lang.org/` outside the allowlist | `not_found` (after rfs-0ox5); `permission_denied` before egress. | PASS |
| H5 | `rfs_search rustup` over H1 | 12 records attributed to the canonical URL; the first hit's line (29) matched the read's line 29 verbatim. | PASS |

## Related issues

- Consulted (copied from `spec.md`; no upstream search repeated): **rfs-r9m6** (closed) built and validated the HTTPS profile shape this change consumes — `HttpsSourceProfile` with per-origin `base_url`, `allow_private_network`, credential reference, the `required` flag, `MutationGrants`, and `StaticProbe::Network`; **rfs-vl0u** (closed) the `DiscoveryAdapter` search engine HTTPS search routes through; **rfs-34pz** (closed) bounded reads, selectors, `artifact://` recovery, and Path Session ownership reused unchanged; **rfs-73dz** (closed) `OperationGuard` cancellation semantics reused, and its read-only classification means `mutable: false` for HTTPS; **rfs-ewh2** (closed) `SourceCatalogMetadata`, so `https://` must self-list in `rfs://` once mounted; **rfs-45ww / rfs-by2z** (open, dependents) GitHub over native HTTP APIs consumes this substrate; **rfs-azd3** (open, dependent) downstream MCP over allowlisted Streamable HTTP consumes this substrate.
- Filed: none during the prove-it phase — every premise passed and no underlying-system defect was found. The single disagreement (P4) was a probe defect, fixed and re-run, with the learning recorded above. Filed later from the 2026-08-27 live smoke: **rfs-0ox5** (upstream error pages were returned as Resources; closed with the fix).
