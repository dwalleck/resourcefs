use std::{borrow::Cow, fmt, sync::Arc, time::Duration};

use resourcefs_core::{
    OperationGuard, PathReference, ReadEngine, ReadRequest, SourceAdapter, TextLimits,
};
use resourcefs_sources::{
    ArtifactSource, ClientRoot, CompiledSources, FilesystemSource, RootRefresh, SessionStore,
};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::schema_for_type, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ClientResult, Implementation,
        ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo,
        ServerRequest, Tool,
    },
    service::{NotificationContext, PeerRequestOptions, RequestContext, RoleServer},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::{
    BoxError,
    render::{self, ReadToolOutput},
};

const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] =
    &[ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25];

#[derive(Debug)]
enum RootSyncState {
    Unseen,
    Idle,
    Pending(RootRefresh),
}

#[derive(Debug)]
struct RootSync {
    state: Mutex<RootSyncState>,
    acquisition: Mutex<()>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    path: String,
}

#[derive(Clone)]
struct ResourceFsServer {
    source: FilesystemSource,
    read_engine: ReadEngine,
    root_sync: Arc<RootSync>,
    tool_router: ToolRouter<Self>,
}

impl fmt::Debug for ResourceFsServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceFsServer")
            .finish_non_exhaustive()
    }
}

#[tool_router(router = tool_router)]
impl ResourceFsServer {
    fn new(source: FilesystemSource, read_engine: ReadEngine) -> Self {
        Self {
            read_engine,
            source,
            root_sync: Arc::new(RootSync {
                state: Mutex::new(RootSyncState::Unseen),
                acquisition: Mutex::new(()),
            }),
            tool_router: Self::tool_router(),
        }
    }

    async fn mark_client_roots_pending(&self, supersede_existing: bool) {
        let mut state = self.root_sync.state.lock().await;
        if !supersede_existing && !matches!(*state, RootSyncState::Unseen) {
            return;
        }
        let refresh = self.source.begin_client_root_refresh().await;
        *state = RootSyncState::Pending(refresh);
    }

    async fn take_pending_root_refresh(&self) -> Option<RootRefresh> {
        let mut state = self.root_sync.state.lock().await;
        let refresh = match *state {
            RootSyncState::Unseen => self.source.begin_client_root_refresh().await,
            RootSyncState::Pending(refresh) => refresh,
            RootSyncState::Idle => return None,
        };
        *state = RootSyncState::Idle;
        Some(refresh)
    }

    async fn refresh_client_roots(&self, context: &RequestContext<RoleServer>) {
        if !request_supports_roots(context) {
            return;
        }
        let _acquisition = self.root_sync.acquisition.lock().await;
        while let Some(refresh) = self.take_pending_root_refresh().await {
            let acquisition = self.source.start_client_root_acquisition(refresh);
            match request_client_roots(context, acquisition.remaining()).await {
                Ok(roots) => {
                    let _ = self
                        .source
                        .complete_client_root_refresh(acquisition, roots)
                        .await;
                }
                Err(()) => {
                    self.source.fail_client_root_refresh(acquisition).await;
                }
            }
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
        let reference = match PathReference::parse(&input.path) {
            Ok(reference) => reference,
            Err(error) => return render::failure(&input.path, &error),
        };
        let request = ReadRequest {
            reference,
            limits: TextLimits::default(),
        };
        match self.read_engine.read(request, &OperationGuard::new()).await {
            Ok(resource) => render::success(&input.path, resource),
            Err(error) => render::failure(&input.path, &error),
        }
    }
}

#[allow(deprecated)]
async fn request_client_roots(
    context: &RequestContext<RoleServer>,
    timeout: Duration,
) -> Result<Vec<ClientRoot>, ()> {
    let request = ServerRequest::ListRootsRequest(rmcp::model::ListRootsRequest {
        method: Default::default(),
        extensions: Default::default(),
    });
    let handle = context
        .peer
        .send_cancellable_request(request, PeerRequestOptions::with_timeout(timeout))
        .await
        .map_err(|_| ())?;
    let result = handle.await_response().await.map_err(|_| ())?;
    let ClientResult::ListRootsResult(result) = result else {
        return Err(());
    };
    Ok(result
        .roots
        .into_iter()
        .map(|root| ClientRoot {
            uri: root.uri,
            name: root.name,
        })
        .collect())
}

impl ServerHandler for ResourceFsServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        self.refresh_client_roots(&context).await;
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
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        self.refresh_client_roots(&context).await;
        Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            ..Default::default()
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        if notification_supports_roots(&context) {
            self.mark_client_roots_pending(false).await;
        }
    }

    async fn on_roots_list_changed(&self, context: NotificationContext<RoleServer>) {
        if notification_supports_roots(&context) {
            self.mark_client_roots_pending(true).await;
        }
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

#[allow(deprecated)]
fn request_supports_roots(context: &RequestContext<RoleServer>) -> bool {
    context
        .client_capabilities()
        .is_some_and(|capabilities| capabilities.roots.is_some())
}

#[allow(deprecated)]
fn notification_supports_roots(context: &NotificationContext<RoleServer>) -> bool {
    context
        .peer
        .peer_info()
        .is_some_and(|info| info.capabilities.roots.is_some())
}

pub(crate) async fn serve(source: FilesystemSource) -> Result<(), BoxError> {
    let session_store = SessionStore::open_default().await?;
    let stored_session = session_store.create_session().await?;
    let session = stored_session.path_session().clone();
    let sources: Arc<dyn SourceAdapter> = Arc::new(CompiledSources::new(
        source.clone(),
        ArtifactSource::new(session.clone()),
    ));
    let read_engine = ReadEngine::new(sources, session);
    let running = ResourceFsServer::new(source, read_engine)
        .serve(rmcp::transport::stdio())
        .await?;
    let result = running.waiting().await;
    stored_session.mark_disconnected().await?;
    result?;
    Ok(())
}
