use std::{borrow::Cow, fmt, sync::Arc, time::Duration};

use resourcefs_core::{
    OperationGuard, PathReference, PathSession, ReadEngine, ReadRequest, ResourceError,
    SourceAdapter, TextLimits,
};
#[cfg(feature = "test-support")]
use resourcefs_sources::StorageFailurePoint;
use resourcefs_sources::{
    ArtifactSource, ClientRoot, CompiledSources, FilesystemSource, RootRefresh, SessionStore,
    StoredSession,
};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::schema_for_type, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ClientResult, Implementation,
        ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo,
        ServerRequest, Tool,
    },
    service::{
        NotificationContext, PeerRequestOptions, RequestContext, RoleServer, RxJsonRpcMessage,
        TxJsonRpcMessage,
    },
    tool, tool_router,
    transport::Transport,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, de::Error as _};
use tokio::sync::{Mutex, OnceCell, watch};

use crate::{
    BoxError,
    render::{self, ReadToolOutput},
};

const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] =
    &[ProtocolVersion::V_2026_07_28, ProtocolVersion::V_2025_11_25];
const SESSION_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

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

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadLimitsInput {
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    #[schemars(with = "usize", range(min = 1, max = 49_152))]
    bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    #[schemars(with = "usize", range(min = 1, max = 3_000))]
    lines: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    #[schemars(with = "usize", range(min = 1, max = 512))]
    columns: Option<usize>,
}

fn deserialize_optional_limit<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: Deserializer<'de>,
{
    usize::deserialize(deserializer).map(Some)
}

fn deserialize_read_limits<'de, D>(deserializer: D) -> Result<ReadLimitsInput, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if !value.is_object() {
        return Err(D::Error::custom("limits must be an object"));
    }
    serde_json::from_value(value).map_err(D::Error::custom)
}

impl ReadLimitsInput {
    fn into_text_limits(self) -> Result<TextLimits, ResourceError> {
        TextLimits::new(self.bytes, self.lines, self.columns)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    path: String,
    #[serde(default, deserialize_with = "deserialize_read_limits")]
    limits: ReadLimitsInput,
}

struct DisconnectState {
    session: PathSession,
    result: OnceCell<Result<(), ResourceError>>,
}

impl DisconnectState {
    fn new(session: PathSession) -> Self {
        Self {
            session,
            result: OnceCell::new(),
        }
    }

    fn invalidate(&self) {
        self.session.invalidate();
    }

    async fn disconnect(&self) {
        self.invalidate();
        self.result
            .get_or_init(|| self.session.mark_disconnected())
            .await;
    }

    async fn finish(&self) -> Result<(), ResourceError> {
        self.disconnect().await;
        self.result
            .get()
            .expect("disconnect result initialized")
            .clone()
    }
}

struct DisconnectTransport<T> {
    inner: T,
    disconnect: Arc<DisconnectState>,
}

impl<T> DisconnectTransport<T> {
    fn new(inner: T, disconnect: Arc<DisconnectState>) -> Self {
        Self { inner, disconnect }
    }
}

impl<T> Drop for DisconnectTransport<T> {
    fn drop(&mut self) {
        self.disconnect.invalidate();
    }
}

impl<T> Transport<RoleServer> for DisconnectTransport<T>
where
    T: Transport<RoleServer>,
{
    type Error = T::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleServer>>> + Send {
        let receive = self.inner.receive();
        let disconnect = Arc::clone(&self.disconnect);
        async move {
            let message = receive.await;
            if message.is_none() {
                disconnect.disconnect().await;
            }
            message
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let close = self.inner.close();
        let disconnect = Arc::clone(&self.disconnect);
        async move {
            disconnect.disconnect().await;
            close.await
        }
    }
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
        description = "Read one Resource by Path Reference. Relative paths resolve only under the configured Primary Workspace Root. Optional limits may only lower the 49,152-byte, 3,000-line, and 512-column text ceilings. Follow continuationReference for the next page; recoveryReference names the complete immutable selected projection. Returns complete non-empty text and equivalent structured content; operational failures are tool errors with stable categories.",
        output_schema = schema_for_type::<ReadToolOutput>()
    )]
    async fn read(
        &self,
        Parameters(input): Parameters<ReadInput>,
    ) -> Result<CallToolResult, String> {
        let limits = input
            .limits
            .into_text_limits()
            .map_err(|error| error.to_string())?;
        self.execute_read(input.path, limits, &OperationGuard::new())
            .await
    }

    async fn execute_read(
        &self,
        path: String,
        limits: TextLimits,
        operation: &OperationGuard,
    ) -> Result<CallToolResult, String> {
        let reference = match PathReference::parse(&path) {
            Ok(reference) => reference,
            Err(error) => return render::failure(&path, &error),
        };
        let request = ReadRequest { reference, limits };
        match self.read_engine.read(request, operation).await {
            Ok(resource) => render::success(&path, resource),
            Err(error) => render::failure(&path, &error),
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
        if request.name != "rfs_read" {
            return Err(McpError::invalid_params("tool not found", None));
        }
        // `ToolRouter::call` converts `Parameters` deserialization failures into
        // tool-result errors. ResourceFS classifies input-schema failures as MCP
        // invalid-params errors, so validate the complete shape before root/source I/O.
        let input: ReadInput = serde_json::from_value(serde_json::Value::Object(
            request.arguments.unwrap_or_default(),
        ))
        .map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_read arguments: {error}"), None)
        })?;
        let limits = input.limits.into_text_limits().map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_read arguments: {error}"), None)
        })?;
        self.refresh_client_roots(&context).await;

        let operation = OperationGuard::new();
        let cancellation = context.ct.clone();
        let pending = self.execute_read(input.path, limits, &operation);
        tokio::pin!(pending);
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                operation.cancel();
                pending.await
            }
            result = &mut pending => result,
        }
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
                "Use rfs_read with a relative path under the Primary Workspace Root or a canonical rfs://workspace/<root>/<path> reference. Optional limits only lower the binary ceilings. Follow continuationReference to page; retain recoveryReference for the complete immutable projection.",
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

#[cfg(feature = "test-support")]
async fn open_session_store() -> Result<SessionStore, ResourceError> {
    match std::env::var_os("RESOURCEFS_TEST_SESSION_ROOT") {
        Some(root) => SessionStore::open(root).await,
        None => SessionStore::open_default().await,
    }
}

#[cfg(not(feature = "test-support"))]
async fn open_session_store() -> Result<SessionStore, ResourceError> {
    SessionStore::open_default().await
}

#[cfg(feature = "test-support")]
async fn configure_test_storage_failure(stored_session: &StoredSession) -> Result<(), BoxError> {
    let Some(failure) = std::env::var_os("RESOURCEFS_TEST_STORAGE_FAILURE") else {
        return Ok(());
    };
    let failure = failure.into_string().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "RESOURCEFS_TEST_STORAGE_FAILURE must be UTF-8",
        )
    })?;
    let point = match failure.as_str() {
        "write" => StorageFailurePoint::Write,
        "disconnect" => StorageFailurePoint::Disconnect,
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("unknown RESOURCEFS_TEST_STORAGE_FAILURE value '{failure}'"),
            )
            .into());
        }
    };
    stored_session.storage_for_test().fail_next(point).await;
    Ok(())
}

#[cfg(feature = "test-support")]
async fn start_test_delivery_gate(
    source: &FilesystemSource,
) -> Result<Option<tokio::task::JoinHandle<Result<(), BoxError>>>, BoxError> {
    let Some(directory) = std::env::var_os("RESOURCEFS_TEST_DELIVERY_GATE") else {
        return Ok(None);
    };
    let identity = std::env::var("RESOURCEFS_TEST_DELIVERY_IDENTITY").map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "RESOURCEFS_TEST_DELIVERY_IDENTITY is required with RESOURCEFS_TEST_DELIVERY_GATE",
        )
    })?;
    let directory = std::path::PathBuf::from(directory);
    let gate = source.arm_test_delivery_gate(identity).await;
    Ok(Some(tokio::spawn(async move {
        gate.wait_until_entered().await;
        tokio::fs::write(directory.join("entered"), b"entered\n").await?;
        let release = directory.join("release");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !tokio::fs::try_exists(&release).await? {
            if tokio::time::Instant::now() >= deadline {
                gate.release();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "test delivery gate release was not created",
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        gate.release();
        Ok(())
    })))
}

async fn heartbeat_session(
    stored_session: StoredSession,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), ResourceError> {
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            () = tokio::time::sleep(SESSION_HEARTBEAT_INTERVAL) => {
                stored_session.heartbeat().await?;
            }
        }
    }
}

pub(crate) async fn serve(source: FilesystemSource) -> Result<(), BoxError> {
    #[cfg(feature = "test-support")]
    let test_delivery_gate = start_test_delivery_gate(&source).await?;
    let session_store = open_session_store().await?;
    let stored_session = session_store.create_session().await?;
    #[cfg(feature = "test-support")]
    configure_test_storage_failure(&stored_session).await?;
    let (heartbeat_shutdown, heartbeat_stop) = watch::channel(false);
    let mut heartbeat = tokio::spawn(heartbeat_session(stored_session.clone(), heartbeat_stop));
    let session = stored_session.path_session().clone();
    let disconnect = Arc::new(DisconnectState::new(session.clone()));
    let sources: Arc<dyn SourceAdapter> = Arc::new(CompiledSources::new(
        source.clone(),
        ArtifactSource::new(session.clone()),
    ));
    let read_engine = ReadEngine::new(sources, session);
    let (stdin, stdout) = rmcp::transport::stdio();
    let stdio = rmcp::transport::async_rw::AsyncRwTransport::<RoleServer, _, _>::new(stdin, stdout);
    let transport = DisconnectTransport::new(stdio, Arc::clone(&disconnect));
    let running = match ResourceFsServer::new(source, read_engine)
        .serve(transport)
        .await
    {
        Ok(running) => running,
        Err(error) => {
            heartbeat_shutdown.send_replace(true);
            let heartbeat_result = heartbeat.await;
            let disconnect_result = disconnect.finish().await;
            heartbeat_result??;
            disconnect_result?;
            return Err(error.into());
        }
    };
    let cancellation = running.cancellation_token();
    let waiting = running.waiting();
    tokio::pin!(waiting);
    let (service_result, heartbeat_result) = tokio::select! {
        result = &mut waiting => {
            heartbeat_shutdown.send_replace(true);
            (result, heartbeat.await)
        }
        result = &mut heartbeat => {
            cancellation.cancel();
            (waiting.await, result)
        }
    };
    let disconnect_result = disconnect.finish().await;
    #[cfg(feature = "test-support")]
    let delivery_gate_result = match test_delivery_gate {
        Some(task) => task.await?,
        None => Ok(()),
    };
    service_result?;
    heartbeat_result??;
    disconnect_result?;
    #[cfg(feature = "test-support")]
    delivery_gate_result?;
    Ok(())
}
