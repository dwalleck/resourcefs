# Related issues (prior art)

- **rfs-34pz** (closed) — built the Path Session, `artifact://` immutable recovery Resources, `DiskSessionStorage`, the 64 MiB object / 256 MiB session quotas, disconnect invalidation, and TTL cleanup. Session Scratch shares this storage and quota accounting; `artifact://` is the immutable sibling of `local://`.
- **rfs-73dz** (closed) — built the source-neutral `MutationEngine` + `MutationAdapter{resolve,load,commit}` seam, hashline parser, seen-region snapshots, Version Tags, per-canonical-Resource serialization, and receipts. rfs-60g1 is the first non-filesystem `MutationAdapter` and the generalization test of that seam.
- **rfs-vl0u** (closed) — `DiscoveryAdapter` (search/glob). Scratch search/glob route through the same discovery engine.
- **rfs-ewh2** (closed) — `SourceCatalogMetadata`; `local://` must self-list in `rfs://` once mounted (a catalog test currently asserts `local://` is absent until then).
- **rfs-73dz** decision — catalogs and `artifact://` are immutable (`permission_denied`); `local://` is the writable session-owned family, distinct from both.
- **Dependent: rfs-ww6w** — MCP Resources/templates mirror; consumes `local://` once it exists.

DESIGN.md already pins (do not re-interrogate): writable without Workspace Root mutation permission; same 64 MiB object / 256 MiB session ceilings; atomic limit-exceeded that never evicts a live Resource; belongs to one Path Session; expires logically at disconnect; content-derived Version Tags.
