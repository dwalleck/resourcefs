#!/usr/bin/env python3
"""Statically verify the published rmcp 3.1.3 source used by the runtime probe."""

from __future__ import annotations

import json
import re
from pathlib import Path

VERSION = "3.1.3"


def require(pattern: str, text: str, claim: str) -> str:
    match = re.search(pattern, text, re.MULTILINE | re.DOTALL)
    if match is None:
        raise RuntimeError(f"published rmcp source does not prove: {claim}")
    return match.group(0)


def main() -> None:
    matches = sorted(
        (Path.home() / ".cargo" / "registry" / "src").glob(f"*/rmcp-{VERSION}")
    )
    if len(matches) != 1:
        raise RuntimeError(f"expected one downloaded rmcp {VERSION} source, found {matches}")
    root = matches[0]

    model = (root / "src" / "model.rs").read_text(encoding="utf-8")
    service = (root / "src" / "service.rs").read_text(encoding="utf-8")
    server = (root / "src" / "service" / "server.rs").read_text(encoding="utf-8")
    schema = (root / "src" / "handler" / "server" / "common.rs").read_text(
        encoding="utf-8"
    )
    transport = (root / "src" / "transport" / "io.rs").read_text(encoding="utf-8")

    require(r'V_2026_07_28.*?"2026-07-28"', model, "2026-07-28 constant")
    require(r'V_2025_11_25.*?"2025-11-25"', model, "2025-11-25 constant")
    latest = require(r"pub const LATEST: Self = Self::V_(\d{4}_\d{2}_\d{2});", model, "LATEST")
    latest_version = latest.removeprefix("pub const LATEST: Self = Self::V_").split(";")[0].replace("_", "-")
    require(
        r"KNOWN_VERSIONS.*?V_2026_07_28.*?V_2025_11_25",
        model,
        "both revisions in KNOWN_VERSIONS",
    )
    require(
        r"fn supported_protocol_versions\(&self\).*?ProtocolVersion::KNOWN_VERSIONS",
        service,
        "default supported revisions",
    )
    require(
        r"fn negotiate_protocol_version.*?server_supported\.contains\(client_requested\).*?client_requested\.clone\(\).*?server_fallback",
        server,
        "supported client revision echo and fallback negotiation",
    )
    require(r"pub fn stdio\(\).*?tokio::io::stdin.*?tokio::io::stdout", transport, "stdio transport")
    require(
        r"fn validate_and_strip.*?if t == \"object\".*?requires tool .*?root type 'object'.*?pub fn schema_for_input.*?validate_and_strip",
        schema,
        "object-root input schema enforcement",
    )
    require(r"pub struct CallToolResult.*?structured_content: Option<Value>", model, "structured content field")
    require(r"pub fn success\(content: Vec<ContentBlock>\)", model, "caller-supplied text content")

    print(
        json.dumps(
            {
                "rmcp": VERSION,
                "source": str(root),
                "latest": latest_version,
                "supports20260728": True,
                "supports20251125": True,
                "negotiation": "echo supported client revision; otherwise server fallback",
                "stdioTransport": True,
                "inputSchemaObjectEnforced": True,
                "customTextAndStructuredContentRepresentable": True,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
