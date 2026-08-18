# Expose ResourceFS through local stdio MCP

ResourceFS will use the official Rust `rmcp` SDK as a local stdio server, advertise MCP 2026-07-28 while negotiating supported older versions, and expose model-controlled `rfs_*` tools as the canonical interface. Every resolvable Path Reference will also be mirrored through bounded MCP Resources/templates when negotiated, while complete text results remain authoritative for clients that omit richer features.
