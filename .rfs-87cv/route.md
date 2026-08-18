# Route: rfs-87cv

Change: Serve and read one contained workspace file through the real stdio MCP interface.
Date: 2026-08-18

## Route tests

| # | Test | Evidence | Verdict |
|---|------|----------|---------|
| 1 | Empirical premise | The accepted contract requires the official Rust `rmcp` SDK, MCP `2026-07-28`, negotiation with `2025-11-25` where supported, capability gating, object-root tool schemas, and complete text plus structured results (`DESIGN.md:19-56`, ADR 0002, ADR 0004). The repository has no Rust implementation, dependency lockfile, SDK probe, or current `evidence.md`; current `rmcp` protocol-version and result-rendering behavior is therefore an unverified external premise. | yes |
| 2 | Structural boundary | This first product slice creates the public `resourcefs` CLI/stdio protocol surface, the `rfs_read` input/output schemas, stable error categories, the three accepted crate/module boundaries, and the Source Adapter seam (`DESIGN.md:269-279`). | yes |
| 3 | Production-scale risk | This slice reads only a small UTF-8 file under the existing 48 KiB/3,000-line/512-column hard result ceilings. It introduces no concurrency, large-input, spill, mutation, or remote-source behavior; containment is a security invariant covered by direct and process tests rather than a production-scale budget. | no |
| 4 | Explicit behavior | Given `resourcefs serve` is launched with exactly one named Primary Workspace Root, when an MCP client initializes over stdio using a supported revision, then initialization succeeds without non-protocol stdout and unsupported optional capabilities remain unadvertised. Given that connection, when the client lists tools, then `rfs_read` is present and both input and output schemas have object roots. Given a small contained UTF-8 file and either a relative reference under the Primary Workspace Root or its canonical `rfs://workspace/<root>/<path>` reference, when `rfs_read` is called through MCP, then the result contains the file's exact complete text and an equivalent structured object with stable contract/reference/content fields. Given a missing path, malformed reference, directory used as a file, lexical escape, or canonical target outside the Workspace Root, when `rfs_read` is called, then the server returns a tool error with a stable semantic category and does not expose outside content. Given the filesystem Source Adapter directly, the same success, containment, UTF-8, and error-mapping contracts hold without the MCP layer; given the compiled process, an end-to-end MCP test proves launch, negotiation, discovery, and read. | yes |

Unknown tests: none

## Selected route

Empirical — the public boundary is structural and its required current `rmcp` protocol/version behavior has no applicable evidence.

## Required artifacts

| Artifact | Owner | Status |
|---|---|---|
| route.md | change-workflow | this file |
| spec.md | interrogated-spec | N/A — behavior is fully explicit in T4 and the accepted Behavior Contract |
| evidence.md, probe.* | prove-it-prototype | required — Empirical route must verify current `rmcp` protocol/version and result-schema behavior |
| design.md | falsifiable-design | required — Empirical route and new public/module boundaries |
| plan.md | budgeted-plan | required — Empirical route checkpointed implementation |

Oracle checkpoint in `checkpointed-build`: required — the compiled process over stdio MCP independently checks the adapter-level read result and schemas.

## Downstream sequence

prove-it-prototype → falsifiable-design → budgeted-plan → checkpointed-build

## Terminal criterion

Empirical — `evidence.md` records PASS for every `rmcp` premise, every later artifact satisfies its owning stage's completion criterion, and `checkpointed-build` records no FAIL.
