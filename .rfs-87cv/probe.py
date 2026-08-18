#!/usr/bin/env python3
"""Compile and run a minimal rmcp server/client handshake outside production code."""

from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

CARGO_TOML = """\
[package]
name = "resourcefs-rmcp-probe"
version = "0.0.0"
edition = "2024"

[dependencies]
anyhow = "1"
rmcp = { version = "=3.1.3", features = ["client", "server", "macros"] }
schemars = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["io-util", "macros", "rt-multi-thread"] }
"""

MAIN_RS = r'''
use std::borrow::Cow;

use anyhow::{Context, Result};
use rmcp::{
    ClientHandler, ServerHandler, ServiceExt,
    handler::server::{
        router::tool::ToolRouter,
        tool::schema_for_type,
        wrapper::Parameters,
    },
    model::{
        CallToolRequestParams, CallToolResult, ClientInfo, ContentBlock, ProtocolVersion,
        ServerCapabilities, ServerInfo,
    },
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const EXACT_TEXT: &str = "fixture text\n";

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadRequest {
    path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ReadResult {
    contract_version: &'static str,
    path: String,
    canonical_reference: String,
    content_type: &'static str,
    content: &'static str,
    bounded: bool,
}

#[derive(Debug, Clone)]
struct ProbeServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl ProbeServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "rfs_read",
        description = "Read one workspace file",
        output_schema = schema_for_type::<ReadResult>()
    )]
    async fn read(
        &self,
        Parameters(request): Parameters<ReadRequest>,
    ) -> Result<CallToolResult, String> {
        let structured = serde_json::to_value(ReadResult {
            contract_version: "1.0.0",
            canonical_reference: format!("rfs://workspace/workspace/{}", request.path),
            path: request.path,
            content_type: "text/plain; charset=utf-8",
            content: EXACT_TEXT,
            bounded: false,
        })
        .map_err(|error| error.to_string())?;

        let mut result = CallToolResult::success(vec![ContentBlock::text(EXACT_TEXT)]);
        result.structured_content = Some(structured);
        Ok(result)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ProbeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[derive(Debug, Clone)]
struct VersionedClient {
    protocol_version: ProtocolVersion,
}

impl ClientHandler for VersionedClient {
    fn get_info(&self) -> ClientInfo {
        let mut info = ClientInfo::default();
        info.protocol_version = self.protocol_version.clone();
        info
    }
}

async fn exercise(requested: ProtocolVersion) -> Result<Value> {
    let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
    let server_task = tokio::spawn(async move {
        ProbeServer::new()
            .serve(server_transport)
            .await
            .context("start probe server")
    });
    let client = VersionedClient {
        protocol_version: requested.clone(),
    }
    .serve(client_transport)
    .await
    .context("start probe client")?;
    let server = server_task.await.context("join probe server startup")??;

    let server_info = client.peer_info().context("server initialization result")?;
    let tools = client.list_tools(None).await.context("tools/list")?;
    let tool = tools
        .tools
        .iter()
        .find(|tool| tool.name == "rfs_read")
        .context("rfs_read in tools/list")?;
    let input_schema_type = tool
        .input_schema
        .get("type")
        .and_then(Value::as_str)
        .context("input schema root type")?;
    let output_schema_type = tool
        .output_schema
        .as_ref()
        .and_then(|schema| schema.get("type"))
        .and_then(Value::as_str)
        .context("output schema root type")?;

    let arguments = json!({"path": "fixture.txt"})
        .as_object()
        .context("object arguments")?
        .clone();
    let result = client
        .call_tool(CallToolRequestParams::new("rfs_read").with_arguments(arguments))
        .await
        .context("tools/call")?;
    let text = result
        .content
        .first()
        .and_then(ContentBlock::as_text)
        .context("text result")?
        .text
        .clone();

    let observation = json!({
        "requested": requested.as_str(),
        "negotiated": server_info.protocol_version.as_str(),
        "toolsCapability": server_info.capabilities.tools.is_some(),
        "resourcesCapability": server_info.capabilities.resources.is_some(),
        "promptsCapability": server_info.capabilities.prompts.is_some(),
        "inputSchemaType": input_schema_type,
        "outputSchemaType": output_schema_type,
        "text": text,
        "structuredContent": result.structured_content,
        "isError": result.is_error,
    });

    client.cancel().await.context("cancel client")?;
    server.cancel().await.context("cancel server")?;
    Ok(observation)
}

#[tokio::main]
async fn main() -> Result<()> {
    let supported: Cow<'static, [ProtocolVersion]> = ProbeServer::new().supported_protocol_versions();
    let observations = vec![
        exercise(ProtocolVersion::V_2026_07_28).await?,
        exercise(ProtocolVersion::V_2025_11_25).await?,
    ];
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "rmcp": "3.1.3",
            "latest": ProtocolVersion::LATEST.as_str(),
            "supports20260728": supported.contains(&ProtocolVersion::V_2026_07_28),
            "supports20251125": supported.contains(&ProtocolVersion::V_2025_11_25),
            "observations": observations,
        }))?
    );
    Ok(())
}
'''


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="resourcefs-rmcp-probe-") as temp_dir:
        root = Path(temp_dir)
        (root / "src").mkdir()
        (root / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
        (root / "src" / "main.rs").write_text(MAIN_RS, encoding="utf-8")
        completed = subprocess.run(
            ["cargo", "run", "--quiet", "--manifest-path", str(root / "Cargo.toml")],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
        )
        print(completed.stdout, end="")


if __name__ == "__main__":
    main()
