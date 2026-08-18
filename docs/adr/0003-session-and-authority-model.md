# Scope references and authority explicitly

One MCP connection owns one Path Session; exact OMP-style URI schemes are server-relative handles whose state expires at disconnect and whose cache files are garbage-collected after a TTL. Filesystem access is confined to canonical client roots or explicit fallback roots, Workspace Mutations and Source Mutations require explicit per-target grants, network adapters and hosts are allowlisted, and `local://` Session Scratch remains writable without granting workspace mutation.
