# Evidence: rfs-87cv

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | Current published `rmcp` can advertise/support MCP `2026-07-28` and negotiate `2025-11-25`. | In the latest non-prerelease crate, are both revisions in the server's supported set, what is the default/fallback revision, and does a real client/server handshake select each requested supported revision? | PASS |
| P2 | Current `rmcp` can expose a stdio-capable tool contract with object-root input/output schemas and return caller-chosen complete TextContent beside structured content while omitting unsupported optional capabilities. | Can a compiled `rmcp` 3.1.3 server/client exchange list and call `rfs_read` at both required revisions and observe object schema roots, exact text, equivalent structured content, tools-only capabilities, and no tool error? | PASS |
| P3 | ResourceFS error-category mappings, Version Tag format, and initial Behavior Contract version. | N/A — these are ResourceFS contract/design choices, not claims about existing external behavior. | N/A — design decisions, not empirical premises |

## Data

- Source: production-shaped
- Shape: the official published `rmcp` 3.1.3 crate, a compiled in-memory MCP transport carrying real initialize, `tools/list`, and `tools/call` messages for the two required revisions, and a one-file-shaped `rfs_read` result with the accepted object/text fields.
- Safety: the probe compiles in a temporary directory, uses `tokio::io::duplex`, reads only downloaded crate source for the oracle, and neither reads nor mutates production or repository-owned Resource state.

## Probe

- File: `probe.py`
- Mechanism: generate and compile a throwaway Rust crate pinned to `rmcp = 3.1.3`; run a real `rmcp` server/client pair over an in-memory duplex transport for both protocol revisions; call the registered tool and serialize observed negotiation, capabilities, schemas, text, and structured content.
- Run: `python3 .rfs-87cv/probe.py`

## Oracle

- File: `oracle.py`
- Mechanism: statically inspect the independently published `rmcp` 3.1.3 source in Cargo's registry for version constants/defaults, supported-version and negotiation branches, stdio transport, schema validation, and `CallToolResult` representation. Static source inspection has a different failure mechanism from both the compiled runtime probe and the production server.
- Run: `python3 .rfs-87cv/oracle.py`
- Official source corroboration: `rmcp` 3.1.3 on crates.io/docs.rs; inspected symbols include `ProtocolVersion` (`src/model.rs`), `Service::supported_protocol_versions` (`src/service.rs`), `negotiate_protocol_version` (`src/service/server.rs`), `schema_for_input` (`src/handler/server/common.rs`), `CallToolResult` (`src/model.rs`), and `transport::stdio` (`src/transport/io.rs`).

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | `rmcp=3.1.3`; `latest=2025-11-25`; support flags true for both revisions; requested `2026-07-28` negotiated `2026-07-28`; requested `2025-11-25` negotiated `2025-11-25`. | `rmcp=3.1.3`; `LATEST=2025-11-25`; both constants occur in `KNOWN_VERSIONS`; default supported revisions use that list; negotiation echoes a supported client revision and otherwise returns the server fallback. | PASS |
| P2 | At both revisions: input and output schema roots are `object`; tools capability true; resources/prompts false; TextContent is exactly `fixture text\n`; structured `content` is the same text with stable object fields; `isError=false`. | Published source provides stdio transport, enforces object-root input schemas, accepts explicit object output schemas, and represents independent `content: Vec<ContentBlock>` plus `structured_content: Option<Value>` on `CallToolResult`. | PASS |
| P3 | N/A — not executed. | N/A — not executed. | N/A — design decisions, not empirical premises |

## Validated / learned

- P1: learned that `rmcp` 3.1.3 fully supports and negotiates `2026-07-28`, but `ProtocolVersion::LATEST` and the default server fallback remain `2025-11-25`. ResourceFS must explicitly use `V_2026_07_28` for its advertised/default server version instead of relying on `LATEST`, while keeping `V_2025_11_25` in its supported set.
- P2: validated prior understanding — the runtime probe and source oracle agree that the SDK can expose object-root tool schemas, return exact non-empty text beside structured content, advertise only tools, and exercise the same contract at both required protocol revisions.

## Related issues

- Consulted: `rfs-87cv` (this slice), `rfs-jk0d` (accepted 1.0 parent contract), `rfs-cgbq` (future complete multi-root/reference behavior blocked by this slice), and `rfs-ww6w` (future MCP Resource mirror). One bounded repository-native tracker search found no separate prior implementation or SDK-evidence issue.
- Filed: none — both external premises passed and no underlying-system defect or intended future work was discovered.
