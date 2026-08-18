# Make complete text tool results the Kiro compatibility floor

Kiro CLI is the required first-release harness. Every `rfs_*` result will therefore contain a complete, non-empty TextContent representation in addition to structured content, every output schema will have an object root, and Recovery References will be rendered as inline text rather than `resource_link` blocks. MCP Resources remain a mirrored application-facing surface, not a dependency of the model-controlled workflow, because current Kiro does not expose resource reads to the agent and may discard structured content.
