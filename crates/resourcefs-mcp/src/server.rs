use std::{borrow::Cow, fmt, sync::Arc};

use resourcefs_core::{PathReference, RootName, SourceAdapter};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::schema_for_type, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    BoxError,
    render::{self, ReadToolOutput},
};

const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] =
    &[ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    path: String,
}

#[derive(Clone)]
struct ResourceFsServer {
    source: Arc<dyn SourceAdapter>,
    primary_root: RootName,
    tool_router: ToolRouter<Self>,
}

impl fmt::Debug for ResourceFsServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceFsServer")
            .field("primary_root", &self.primary_root)
            .finish_non_exhaustive()
    }
}

#[tool_router(router = tool_router)]
impl ResourceFsServer {
    fn new(source: Arc<dyn SourceAdapter>, primary_root: RootName) -> Self {
        Self {
            source,
            primary_root,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "rfs_read",
        description = "Read one Resource by Path Reference. Relative paths resolve only under the configured Primary Workspace Root. Returns complete non-empty text and equivalent structured content; operational failures are tool errors with stable categories.",
        output_schema = schema_for_type::<ReadToolOutput>()
    )]
    async fn read(
        &self,
        Parameters(input): Parameters<ReadInput>,
    ) -> Result<CallToolResult, String> {
        let reference = match PathReference::parse(&input.path, &self.primary_root) {
            Ok(reference) => reference,
            Err(error) => return render::failure(&input.path, &error),
        };
        match self.source.read(&reference).await {
            Ok(resource) => render::success(&input.path, resource),
            Err(error) => render::failure(&input.path, &error),
        }
    }
}

impl ServerHandler for ResourceFsServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if request.name != "rfs_read" {
            return Err(McpError::invalid_params("tool not found", None));
        }
        // `ToolRouter::call` converts `Parameters` deserialization failures into
        // tool-result errors. ResourceFS classifies input-schema failures as MCP
        // invalid-params errors, so validate once here and call the typed handler.
        let input = serde_json::from_value(serde_json::Value::Object(
            request.arguments.unwrap_or_default(),
        ))
        .map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_read arguments: {error}"), None)
        })?;
        let result = self
            .read(Parameters(input))
            .await
            .map_err(|error| McpError::internal_error(error, None))?;
        Ok(result.into())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            ..Default::default()
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
            .with_server_info(Implementation::new(
                "resourcefs",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Use rfs_read with a relative path under the Primary Workspace Root or a canonical rfs://workspace/<root>/<path> reference.",
            )
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }
}

pub(crate) async fn serve(
    source: Arc<dyn SourceAdapter>,
    primary_root: RootName,
) -> Result<(), BoxError> {
    let running = ResourceFsServer::new(source, primary_root)
        .serve(rmcp::transport::stdio())
        .await?;
    running.waiting().await?;
    Ok(())
}
