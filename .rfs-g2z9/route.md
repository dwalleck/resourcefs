# Route: rfs-g2z9

Change: Read allowlisted HTTPS Resources safely, on the project's one bounded HTTP substrate
Date: 2026-08-23

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | **Yes — and the sharpest premise is a security control.** The workspace has **no HTTP client, TLS, or HTML-extraction dependency at all** (`Cargo.toml` network-adjacent deps are `url` alone), so every runtime premise is unverified. Load-bearing unknowns, in order of risk: (a) **per-resolved-address authorization.** The acceptance criteria require that "every redirect and resolved address is reauthorized" and that private/special addresses need a distinct grant — a DNS-rebinding/SSRF defense. Enforcing it correctly means resolving the host, validating the resulting IP against policy, and connecting to *that validated address*; a client that re-resolves internally after validation reintroduces the TOCTOU the control exists to close. Whether any candidate Rust client exposes that hook is unverified and decides the dependency. (b) **per-redirect interception** — authorizing each hop rather than letting the client follow blindly. (c) **bounded body reads, timeouts, and cancellation** interacting with the existing `OperationGuard`. (d) **reader-mode determinism** — HTML→Markdown extraction must be byte-deterministic to support golden fixtures and content-derived Version Tags. None of this is covered by existing evidence: r9m6 verified *configuration*, not transport. | yes |
| 2 | Structural boundary | Yes. Introduces the workspace's first HTTP client/TLS dependency (an architecture decision requiring user approval per the contract's approval semantics); adds an `https://` address family to the public Path Reference grammar and a new Source Adapter; and — per this issue's substrate charter — must place a reusable bounded-HTTP substrate that `rfs-45ww`/`rfs-by2z` (GitHub) and `rfs-azd3` (Streamable HTTP downstream MCP) consume rather than adding a second client or policy layer. Placement of policy (core, source-neutral types) versus transport (sources) is a cross-module decision. | yes |
| 3 | Production-scale risk | Yes. Network I/O introduces latency, timeout, and cancellation behavior the repository has never carried; bodies must be bounded before buffering (the 64 MiB/text ceilings apply to untrusted remote input); redirect chains must be depth-bounded; and probes must not run network enumeration on a hot path. Budgets and stress fixtures are required, and Local has none. | yes |
| 4 | Explicit behavior | No — unresolved observable decisions: which client crate and TLS backend (informed by premise (a)); reader-mode extraction approach and its determinism guarantees; what `:raw` returns versus reader mode and how MIME/charset selects between them; redirect depth limit and whether cross-origin redirects require re-allowlisting; how an origin's `base_url` constrains reachable paths; response-size and timeout ceilings and whether they are profile-lowerable; whether responses are cached within a Path Session; how search targets an HTTPS Resource; what a Degraded HTTPS source returns per reference; and exactly which fields a policy-denial error may surface without leaking credentials or internal addresses. | no |

Unknown tests: none

## Selected route

Empirical — the transport, its address-authorization hooks, and reader-mode determinism are unverified external premises whose resolution decides the workspace's first HTTP dependency; the change also adds a public address family plus a shared substrate seam and carries genuine network-scale risk. Precedence: Empirical > Structural > Local.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | required — T4 no: reader-mode/`:raw` split, redirect and origin scoping, ceilings, degraded behavior, and denial-error shape are unresolved |
| evidence.md, probe.* | prove-it-prototype | required — T1 yes: candidate clients must be probed for validated-address connection, per-redirect interception, bounded/cancellable body reads, and deterministic extraction, against an independent oracle |
| design.md | falsifiable-design | required — Empirical route |
| plan.md | budgeted-plan | required — Empirical route |

Oracle checkpoint in `checkpointed-build`: required — Empirical route

## Downstream sequence

interrogated-spec → prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

Note on ordering: the client-crate decision is an architecture choice the requester owns, and it depends on premise (a). The interrogation therefore pins observable behavior and records the dependency choice as pending probe evidence; `prove-it-prototype` evaluates candidates against the validated-address requirement; the requester selects with that evidence in hand before `falsifiable-design` commits placement.

## Terminal criterion

Empirical — `prove-it-prototype` records PASS for every empirical premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no FAIL.
