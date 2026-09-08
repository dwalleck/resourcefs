use std::{
    borrow::Cow,
    collections::{HashMap, hash_map::Entry},
    fmt,
    future::Future,
    sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard},
    time::Duration,
};

use resourcefs_core::{
    DiscoveryEngine, ErrorCategory, GlobLimits, GlobOptions, GlobRequest, GlobTarget,
    MutationEngine, OperationGuard, OperationId, PathReference, PathSession, ReadEngine,
    ResourceError, SearchLimits, SearchOptions, SearchRequest, SearchTarget, ServerLimits,
    VersionTag, WriteRequest,
};
#[cfg(feature = "test-support")]
use resourcefs_sources::StorageFailurePoint;
use resourcefs_sources::{
    ArtifactSource, ClientRoot, CompiledSources, FilesystemSource, GithubSourceMount, HttpsSource,
    LocalSource, RootRefresh, SessionStorageConfig, SessionStore, StoredSession,
};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, tool::schema_for_type, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ClientNotification, ClientRequest,
        ClientResult, Implementation, JsonRpcMessage, ListToolsResult, PaginatedRequestParams,
        ProtocolVersion, RequestId, ServerCapabilities, ServerInfo, ServerRequest, Tool,
    },
    service::{
        NotificationContext, PeerRequestOptions, RequestContext, RoleServer, RxJsonRpcMessage,
        TxJsonRpcMessage,
    },
    tool, tool_router,
    transport::Transport,
};
use schemars::JsonSchema;
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeOwned, Error as _},
};
use tokio::sync::{Mutex, OnceCell, watch};

use crate::{
    BoxError,
    launch::LaunchPlan,
    logging::{LogLevel, LogSink},
    render::{self, GlobToolOutput, MutationToolOutput, ReadToolOutput, SearchToolOutput},
};

mod read;
use read::ReadInput;

#[cfg(test)]
mod jira_query_tests;

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

fn deserialize_optional_limit<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: Deserializer<'de>,
{
    usize::deserialize(deserializer).map(Some)
}

fn deserialize_object_limits<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if !value.is_object() {
        return Err(D::Error::custom("limits must be an object"));
    }
    serde_json::from_value(value).map_err(D::Error::custom)
}

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WriteInput {
    path: String,
    content: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    #[schemars(with = "String")]
    if_version: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    #[schemars(with = "String")]
    operation_id: Option<String>,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditInput {
    patch: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SearchLimitsInput {
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    #[schemars(with = "usize", range(min = 1, max = 1_000))]
    max_results: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    #[schemars(with = "usize", range(min = 1, max = 49_152))]
    max_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    #[schemars(with = "usize", range(min = 1, max = 3_000))]
    max_lines: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    #[schemars(with = "usize", range(min = 1, max = 512))]
    max_columns: Option<usize>,
}

impl SearchLimitsInput {
    fn into_search_limits(self) -> Result<SearchLimits, ResourceError> {
        SearchLimits::new(
            self.max_results,
            self.max_bytes,
            self.max_lines,
            self.max_columns,
        )
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GlobLimitsInput {
    #[serde(default, deserialize_with = "deserialize_optional_limit")]
    #[schemars(with = "usize", range(min = 1, max = 1_000))]
    max_results: Option<usize>,
}

impl GlobLimitsInput {
    fn into_glob_limits(self) -> Result<GlobLimits, ResourceError> {
        GlobLimits::new(self.max_results)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SearchInput {
    pattern: String,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    #[schemars(with = "String")]
    path: Option<String>,
    #[serde(default = "default_true")]
    case_sensitive: bool,
    #[serde(default = "default_true")]
    gitignore: bool,
    #[serde(default = "default_false")]
    hidden: bool,
    #[serde(default)]
    skip: usize,
    #[serde(default, deserialize_with = "deserialize_object_limits")]
    limits: SearchLimitsInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GlobInput {
    path: String,
    #[serde(default = "default_true")]
    case_sensitive: bool,
    #[serde(default = "default_true")]
    gitignore: bool,
    #[serde(default = "default_false")]
    hidden: bool,
    #[serde(default)]
    skip: usize,
    #[serde(default, deserialize_with = "deserialize_object_limits")]
    limits: GlobLimitsInput,
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

/// Keeps ResourceFS cancellation response semantics independent of rmcp's
/// request-token handling. rmcp removes a cancelled request from its response
/// pool before the handler finishes, which would discard the complete
/// `cancelled` tool result required by the Behavior Contract.
#[derive(Debug)]
struct RequestCancellationEntry {
    sender: watch::Sender<bool>,
    requests: usize,
}

#[derive(Debug, Default)]
struct RequestCancellations {
    active: StdMutex<HashMap<RequestId, RequestCancellationEntry>>,
}

impl RequestCancellations {
    fn active(&self) -> StdMutexGuard<'_, HashMap<RequestId, RequestCancellationEntry>> {
        match self.active.lock() {
            Ok(active) => active,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn begin(&self, id: RequestId) {
        match self.active().entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(RequestCancellationEntry {
                    sender: watch::channel(false).0,
                    requests: 1,
                });
            }
            Entry::Occupied(mut entry) => {
                entry.get_mut().requests = entry.get().requests.saturating_add(1);
            }
        }
    }

    fn receiver(&self, id: &RequestId) -> Option<watch::Receiver<bool>> {
        self.active().get(id).map(|entry| entry.sender.subscribe())
    }

    fn cancel(&self, id: &RequestId) -> bool {
        let sender = self.active().get(id).map(|entry| entry.sender.clone());
        if let Some(sender) = sender {
            sender.send_replace(true);
            true
        } else {
            false
        }
    }

    fn finish(&self, id: &RequestId) {
        let mut active = self.active();
        let Entry::Occupied(mut entry) = active.entry(id.clone()) else {
            return;
        };
        if entry.get().requests > 1 {
            entry.get_mut().requests -= 1;
        } else {
            entry.remove();
        }
    }
}

struct RequestCancellationLease {
    cancellations: Arc<RequestCancellations>,
    id: RequestId,
}

impl RequestCancellationLease {
    fn new(cancellations: Arc<RequestCancellations>, id: RequestId) -> Self {
        Self { cancellations, id }
    }
}

impl Drop for RequestCancellationLease {
    fn drop(&mut self) {
        self.cancellations.finish(&self.id);
    }
}

struct DisconnectTransport<T> {
    inner: T,
    disconnect: Arc<DisconnectState>,
    cancellations: Arc<RequestCancellations>,
}

impl<T> DisconnectTransport<T> {
    fn new(
        inner: T,
        disconnect: Arc<DisconnectState>,
        cancellations: Arc<RequestCancellations>,
    ) -> Self {
        Self {
            inner,
            disconnect,
            cancellations,
        }
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
        let disconnect = Arc::clone(&self.disconnect);
        let cancellations = Arc::clone(&self.cancellations);
        async move {
            loop {
                let Some(message) = self.inner.receive().await else {
                    disconnect.disconnect().await;
                    return None;
                };
                match &message {
                    JsonRpcMessage::Request(request)
                        if matches!(&request.request, ClientRequest::CallToolRequest(_)) =>
                    {
                        cancellations.begin(request.id.clone());
                    }
                    JsonRpcMessage::Notification(notification) => {
                        if let ClientNotification::CancelledNotification(cancelled) =
                            &notification.notification
                        {
                            // Only consume cancellation for an inbound tool
                            // request. Server-initiated requests such as
                            // roots/list must still reach rmcp's responder pool.
                            if cancelled
                                .params
                                .request_id
                                .as_ref()
                                .is_some_and(|request_id| cancellations.cancel(request_id))
                            {
                                continue;
                            }
                        }
                    }
                    _ => {}
                }
                return Some(message);
            }
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
    discovery_engine: DiscoveryEngine,
    mutation_engine: MutationEngine,
    root_sync: Arc<RootSync>,
    cancellations: Arc<RequestCancellations>,
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
    fn new(
        source: FilesystemSource,
        read_engine: ReadEngine,
        mutation_engine: MutationEngine,
        discovery_engine: DiscoveryEngine,
        cancellations: Arc<RequestCancellations>,
    ) -> Self {
        Self {
            read_engine,
            discovery_engine,
            mutation_engine,
            source,
            root_sync: Arc::new(RootSync {
                state: Mutex::new(RootSyncState::Unseen),
                acquisition: Mutex::new(()),
            }),
            cancellations,
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
        let refreshed = self
            .refresh_client_roots_with_cancellation(context, std::future::pending())
            .await;
        debug_assert!(refreshed, "pending cancellation future cannot fire");
    }

    async fn refresh_client_roots_with_cancellation<C>(
        &self,
        context: &RequestContext<RoleServer>,
        cancelled: C,
    ) -> bool
    where
        C: Future<Output = ()>,
    {
        if !request_supports_roots(context) {
            return true;
        }
        tokio::pin!(cancelled);
        let _acquisition = tokio::select! {
            biased;
            _ = &mut cancelled => return false,
            acquisition = self.root_sync.acquisition.lock() => acquisition,
        };
        while let Some(refresh) = self.take_pending_root_refresh().await {
            let acquisition = self.source.start_client_root_acquisition(refresh);
            match request_client_roots(context, acquisition.remaining(), cancelled.as_mut()).await {
                Ok(roots) => {
                    let _ = self
                        .source
                        .complete_client_root_refresh(acquisition, roots)
                        .await;
                }
                Err(ClientRootsRequestError::Cancelled) => {
                    self.source.fail_client_root_refresh(acquisition).await;
                    self.mark_client_roots_pending(true).await;
                    return false;
                }
                Err(ClientRootsRequestError::Failed) => {
                    self.source.fail_client_root_refresh(acquisition).await;
                }
            }
        }
        true
    }

    #[tool(
        name = "rfs_read",
        description = "Start with rfs_read of rfs:// to discover mounted sources. Read one Resource by Path Reference. Relative paths resolve only under the configured Primary Workspace Root. Optional limits may only lower the 49,152-byte, 3,000-line, and 512-column text ceilings. Follow continuationReference for the next page; recoveryReference names the complete immutable selected projection; when present, sourceContinuationReference names the next upstream page of a paginated source that remains after the continuation chain is exhausted. Returns complete non-empty text and equivalent structured content; operational failures are tool errors with stable categories.",
        output_schema = schema_for_type::<ReadToolOutput>()
    )]
    async fn read(
        &self,
        Parameters(input): Parameters<ReadInput>,
    ) -> Result<CallToolResult, String> {
        self.read_input(input).await
    }

    #[tool(
        name = "rfs_search",
        description = "Start with rfs_read of rfs:// to discover mounted sources. Search matching lines in Workspace, Artifact, HTTPS, and configured issue:// or pr:// Resources. Omit path to search the Primary Workspace Root recursively; an explicit path targets exactly one Resource or source-native repository collection. Defaults: caseSensitive=true, gitignore=true, hidden=false, skip=0; caseSensitive=false selects Unicode-aware case-insensitive matching, gitignore=false includes ignored Workspace entries, hidden=true includes hidden Workspace entries. The pattern is matched literally when it has no metacharacters, otherwise with Rust regex first and PCRE2 when Rust rejects the syntax; a pattern both engines reject is an invalid_pattern tool error, and a pattern over 65,536 UTF-8 bytes is limit_exceeded. Optional limits may only lower the 1,000-record, 49,152-byte, 3,000-line, and 512-column ceilings. Follow continuationReference for the next page; recoveryReference names the complete immutable result; when present, sourceContinuationReference names the next upstream page of a paginated source collection that remains after the continuation chain is exhausted. Returns complete non-empty text and equivalent structured content; operational failures are tool errors with stable categories.",
        output_schema = schema_for_type::<SearchToolOutput>()
    )]
    async fn search(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> Result<CallToolResult, String> {
        let limits = input
            .limits
            .into_search_limits()
            .map_err(|error| error.to_string())?;
        let options = SearchOptions::new(input.case_sensitive, input.gitignore, input.hidden);
        let target = match &input.path {
            Some(path) => match PathReference::parse(path) {
                Ok(reference) => SearchTarget::resource(reference),
                Err(error) => return render::search_failure(path, &error),
            },
            None => SearchTarget::primary(),
        };
        let request = match SearchRequest::new(target, input.pattern, options, input.skip, limits) {
            Ok(request) => request,
            Err(error) => {
                return render::search_failure(input.path.as_deref().unwrap_or(""), &error);
            }
        };
        let operation = OperationGuard::new();
        self.execute_search(request, input.path.as_deref(), &operation)
            .await
    }

    #[tool(
        name = "rfs_glob",
        description = "Start with rfs_read of rfs:// to discover mounted sources. Enumerate Workspace files and directories and Artifact Resources matching a glob. path is one required glob pattern using canonical / separators: * ? and character classes match within one component, ** crosses directories, {a,b} alternates, and backslash escapes on every platform. Defaults: caseSensitive=true, gitignore=true, hidden=false, skip=0; caseSensitive=false selects ASCII-only case-insensitive matching, gitignore=false includes ignored entries, hidden=true includes hidden entries. An empty or invalid glob is an invalid_pattern tool error, and a pattern over 65,536 UTF-8 bytes is limit_exceeded. Optional limits may only lower the 1,000-entry ceiling. Follow continuationReference for the next page; recoveryReference names the complete immutable result. Returns complete non-empty text and equivalent structured content; operational failures are tool errors with stable categories.",
        output_schema = schema_for_type::<GlobToolOutput>()
    )]
    async fn glob(
        &self,
        Parameters(input): Parameters<GlobInput>,
    ) -> Result<CallToolResult, String> {
        let limits = input
            .limits
            .into_glob_limits()
            .map_err(|error| error.to_string())?;
        let options = GlobOptions::new(input.case_sensitive, input.gitignore, input.hidden);
        let target = match GlobTarget::new(input.path.clone()) {
            Ok(target) => target,
            Err(error) => return render::glob_failure(&input.path, &error),
        };
        let request = GlobRequest::new(target, options, input.skip, limits);
        let operation = OperationGuard::new();
        self.execute_glob(request, &input.path, &operation).await
    }

    #[tool(
        name = "rfs_write",
        description = "Start with rfs_read of rfs:// to discover mounted sources. Create one UTF-8 text Resource when create is granted and the parent exists; replace existing text only when update is granted and ifVersion matches its current content-derived Version Tag. Remote paths ending in /new are write-only Creation Targets and require operationId; the same ID/target/exact content safely repeats only in this Path Session, while changed content conflicts and unknown outcomes require upstream reconciliation with a new ID. operationId is rejected everywhere else. Mutation policy remains authoritative. Returns a compact receipt without repeating Resource content.",
        output_schema = schema_for_type::<MutationToolOutput>()
    )]
    async fn write(
        &self,
        Parameters(input): Parameters<WriteInput>,
    ) -> Result<CallToolResult, String> {
        self.execute_write(input, &OperationGuard::new()).await
    }

    #[tool(
        name = "rfs_edit",
        description = "Start with rfs_read of rfs:// to discover mounted sources. Apply one [canonical-reference#Version-Tag] hashline document using PUT N.=M:, PUT <N:, PUT >N:, PUT >$:, CUT N.=M, sole REM, or sole MV <destination>. Coordinates must come from same-session displayed regions; block-star and register forms are reserved and rejected with the accepted grammar.",
        output_schema = schema_for_type::<MutationToolOutput>()
    )]
    async fn edit(
        &self,
        Parameters(input): Parameters<EditInput>,
    ) -> Result<CallToolResult, String> {
        self.execute_edit(&input.patch, &OperationGuard::new())
            .await
    }

    async fn execute_write(
        &self,
        input: WriteInput,
        operation: &OperationGuard,
    ) -> Result<CallToolResult, String> {
        let operation_id = match input.operation_id {
            Some(value) => match OperationId::parse(value) {
                Ok(operation_id) => Some(operation_id),
                Err(error) => return render::mutation_failure(&input.path, &error),
            },
            None => None,
        };
        let reference = match PathReference::parse(&input.path) {
            Ok(reference) => reference,
            Err(error) => return render::mutation_failure(&input.path, &error),
        };
        let if_version = match input.if_version {
            Some(version) => match VersionTag::parse(version) {
                Ok(version) => Some(version),
                Err(error) => return render::mutation_failure(&input.path, &error),
            },
            None => None,
        };
        let request = match WriteRequest::new(reference, input.content, if_version, operation_id) {
            Ok(request) => request,
            Err(error) => return render::mutation_failure(&input.path, &error),
        };
        match self.mutation_engine.write(request, operation).await {
            Ok(receipt) => render::mutation_success(receipt),
            Err(error) => render::mutation_failure(&input.path, &error),
        }
    }

    async fn execute_edit(
        &self,
        document: &str,
        operation: &OperationGuard,
    ) -> Result<CallToolResult, String> {
        match self.mutation_engine.edit(document, operation).await {
            Ok(receipt) => render::mutation_success(receipt),
            Err(error) => render::mutation_failure("", &error),
        }
    }

    async fn execute_search(
        &self,
        request: SearchRequest,
        requested_path: Option<&str>,
        operation: &OperationGuard,
    ) -> Result<CallToolResult, String> {
        match self.discovery_engine.search(request, operation).await {
            Ok(result) => render::search_success(result),
            Err(error) => render::search_failure(requested_path.unwrap_or(""), &error),
        }
    }

    async fn execute_glob(
        &self,
        request: GlobRequest,
        requested_path: &str,
        operation: &OperationGuard,
    ) -> Result<CallToolResult, String> {
        match self.discovery_engine.glob(request, operation).await {
            Ok(result) => render::glob_success(result),
            Err(error) => render::glob_failure(requested_path, &error),
        }
    }

    async fn dispatch_search(
        &self,
        request: CallToolRequestParams,
        context: &RequestContext<RoleServer>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<CallToolResponse, McpError> {
        let SearchInput {
            pattern,
            path,
            case_sensitive,
            gitignore,
            hidden,
            skip,
            limits,
        } = deserialize_input(request).map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_search arguments: {error}"), None)
        })?;
        let limits = limits.into_search_limits().map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_search arguments: {error}"), None)
        })?;
        let options = SearchOptions::new(case_sensitive, gitignore, hidden);
        let requested_path = path.as_deref().unwrap_or("");
        let target = match path.as_deref() {
            Some(path) => match PathReference::parse(path) {
                Ok(reference) => SearchTarget::resource(reference),
                Err(error) => return search_tool_failure(requested_path, &error),
            },
            None => SearchTarget::primary(),
        };
        let request = match SearchRequest::new(target, pattern, options, skip, limits) {
            Ok(request) => request,
            Err(error) => return search_tool_failure(requested_path, &error),
        };
        let operation = OperationGuard::new();
        let cancelled_result =
            || render::search_failure(requested_path, &cancelled_discovery_error());
        if !self
            .refresh_client_roots_with_cancellation(
                context,
                cancellation_received(cancellation.clone()),
            )
            .await
        {
            operation.cancel();
            return cancelled_result()
                .map(CallToolResponse::from)
                .map_err(|error| McpError::internal_error(error, None));
        }
        let pending = self.execute_search(request, path.as_deref(), &operation);
        let result = run_under_cancellation(
            &operation,
            cancellation_received(cancellation),
            pending,
            cancelled_result,
        )
        .await
        .map_err(|error| McpError::internal_error(error, None))?;
        Ok(result.into())
    }

    async fn dispatch_glob(
        &self,
        request: CallToolRequestParams,
        context: &RequestContext<RoleServer>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<CallToolResponse, McpError> {
        let GlobInput {
            path,
            case_sensitive,
            gitignore,
            hidden,
            skip,
            limits,
        } = deserialize_input(request).map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_glob arguments: {error}"), None)
        })?;
        let limits = limits.into_glob_limits().map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_glob arguments: {error}"), None)
        })?;
        let options = GlobOptions::new(case_sensitive, gitignore, hidden);
        let target = match GlobTarget::new(path.clone()) {
            Ok(target) => target,
            Err(error) => return glob_tool_failure(&path, &error),
        };
        let request = GlobRequest::new(target, options, skip, limits);
        let operation = OperationGuard::new();
        let cancelled_result = || render::glob_failure(&path, &cancelled_discovery_error());
        if !self
            .refresh_client_roots_with_cancellation(
                context,
                cancellation_received(cancellation.clone()),
            )
            .await
        {
            operation.cancel();
            return cancelled_result()
                .map(CallToolResponse::from)
                .map_err(|error| McpError::internal_error(error, None));
        }
        let pending = self.execute_glob(request, &path, &operation);
        let result = run_under_cancellation(
            &operation,
            cancellation_received(cancellation),
            pending,
            cancelled_result,
        )
        .await
        .map_err(|error| McpError::internal_error(error, None))?;
        Ok(result.into())
    }
    async fn dispatch_write(
        &self,
        request: CallToolRequestParams,
        context: &RequestContext<RoleServer>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<CallToolResponse, McpError> {
        let input: WriteInput = deserialize_input(request).map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_write arguments: {error}"), None)
        })?;
        let requested_path = input.path.clone();
        let operation = OperationGuard::new();
        let cancelled_result = || {
            render::mutation_failure(
                &requested_path,
                &ResourceError::new(ErrorCategory::Cancelled, "rfs_write was cancelled"),
            )
        };
        if !self
            .refresh_client_roots_with_cancellation(
                context,
                cancellation_received(cancellation.clone()),
            )
            .await
        {
            operation.cancel();
            return cancelled_result()
                .map(CallToolResponse::from)
                .map_err(|error| McpError::internal_error(error, None));
        }
        let pending = self.execute_write(input, &operation);
        let result = run_under_cancellation(
            &operation,
            cancellation_received(cancellation),
            pending,
            cancelled_result,
        )
        .await
        .map_err(|error| McpError::internal_error(error, None))?;
        Ok(result.into())
    }

    async fn dispatch_edit(
        &self,
        request: CallToolRequestParams,
        context: &RequestContext<RoleServer>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<CallToolResponse, McpError> {
        let input: EditInput = deserialize_input(request).map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_edit arguments: {error}"), None)
        })?;
        let operation = OperationGuard::new();
        let cancelled_result = || {
            render::mutation_failure(
                "",
                &ResourceError::new(ErrorCategory::Cancelled, "rfs_edit was cancelled"),
            )
        };
        if !self
            .refresh_client_roots_with_cancellation(
                context,
                cancellation_received(cancellation.clone()),
            )
            .await
        {
            operation.cancel();
            return cancelled_result()
                .map(CallToolResponse::from)
                .map_err(|error| McpError::internal_error(error, None));
        }
        let pending = self.execute_edit(&input.patch, &operation);
        let result = run_under_cancellation(
            &operation,
            cancellation_received(cancellation),
            pending,
            cancelled_result,
        )
        .await
        .map_err(|error| McpError::internal_error(error, None))?;
        Ok(result.into())
    }
}

async fn cancellation_received(mut cancellation: watch::Receiver<bool>) {
    if cancellation.wait_for(|cancelled| *cancelled).await.is_err() {
        // A closed uncancelled channel means the request already reached another
        // terminal path; it must not be reclassified as client cancellation.
        std::future::pending::<()>().await;
    }
}

fn deserialize_input<T>(request: CallToolRequestParams) -> Result<T, serde_json::Error>
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::Object(
        request.arguments.unwrap_or_default(),
    ))
}

/// One biased cancellation race shared by every tool. Cancellation invalidates
/// ordinary read/discovery work and drops its pending future. Mutation work can
/// transition its guard to committing; once that happens cancellation is masked
/// and this helper awaits the authoritative result instead of reporting an
/// outcome that may disagree with committed state.
async fn run_under_cancellation<T, F, C>(
    operation: &OperationGuard,
    cancelled: C,
    pending: F,
    cancelled_result: impl FnOnce() -> Result<T, String>,
) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
    C: Future<Output = ()>,
{
    tokio::pin!(pending);
    tokio::select! {
        biased;
        _ = cancelled => {
            if operation.cancel() {
                cancelled_result()
            } else {
                pending.as_mut().await
            }
        }
        result = &mut pending => result,
    }
}

/// Mirrors the core discovery `cancelled()` operational error exactly.
fn cancelled_discovery_error() -> ResourceError {
    ResourceError::new(
        ErrorCategory::Cancelled,
        "discovery operation was cancelled",
    )
}

fn search_tool_failure(
    requested_path: &str,
    error: &ResourceError,
) -> Result<CallToolResponse, McpError> {
    render::search_failure(requested_path, error)
        .map(CallToolResponse::from)
        .map_err(|render_error| McpError::internal_error(render_error, None))
}

fn glob_tool_failure(
    requested_path: &str,
    error: &ResourceError,
) -> Result<CallToolResponse, McpError> {
    render::glob_failure(requested_path, error)
        .map(CallToolResponse::from)
        .map_err(|render_error| McpError::internal_error(render_error, None))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientRootsRequestError {
    Cancelled,
    Failed,
}

#[allow(deprecated)]
async fn request_client_roots<C>(
    context: &RequestContext<RoleServer>,
    timeout: Duration,
    cancelled: C,
) -> Result<Vec<ClientRoot>, ClientRootsRequestError>
where
    C: Future<Output = ()>,
{
    let request = ServerRequest::ListRootsRequest(rmcp::model::ListRootsRequest {
        method: Default::default(),
        extensions: Default::default(),
    });
    tokio::pin!(cancelled);
    let mut handle = tokio::select! {
        biased;
        _ = &mut cancelled => return Err(ClientRootsRequestError::Cancelled),
        handle = context
            .peer
            .send_cancellable_request(request, PeerRequestOptions::no_options()) => {
                handle.map_err(|_| ClientRootsRequestError::Failed)?
            }
    };
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let result = tokio::select! {
        biased;
        _ = &mut cancelled => {
            if let Err(error) = handle
                .cancel(Some("parent ResourceFS request was cancelled".to_owned()))
                .await
            {
                eprintln!("resourcefs: failed to cancel roots/list request: {error}");
            }
            return Err(ClientRootsRequestError::Cancelled);
        }
        response = &mut handle.rx => {
            response
                .map_err(|_| ClientRootsRequestError::Failed)?
                .map_err(|_| ClientRootsRequestError::Failed)?
        }
        _ = &mut deadline => {
            if let Err(error) = handle.cancel(Some("request timeout".to_owned())).await {
                eprintln!("resourcefs: failed to time out roots/list request: {error}");
            }
            return Err(ClientRootsRequestError::Failed);
        }
    };
    let ClientResult::ListRootsResult(result) = result else {
        return Err(ClientRootsRequestError::Failed);
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
        // `ToolRouter::call` converts `Parameters` deserialization failures into
        // tool-result errors. ResourceFS classifies input-schema failures as MCP
        // invalid-params errors, so each known tool deserializes only its own
        // deny-unknown schema and validates its complete shape before root/source
        // I/O; unknown tool names never reach root refresh or source I/O.
        let cancellation = self.cancellations.receiver(&context.id).ok_or_else(|| {
            McpError::internal_error("request cancellation state unavailable", None)
        })?;
        let _cancellation_lease =
            RequestCancellationLease::new(Arc::clone(&self.cancellations), context.id.clone());
        let name = request.name.clone();
        match name.as_ref() {
            "rfs_read" => self.dispatch_read(request, &context, cancellation).await,
            "rfs_search" => self.dispatch_search(request, &context, cancellation).await,
            "rfs_glob" => self.dispatch_glob(request, &context, cancellation).await,
            "rfs_write" => self.dispatch_write(request, &context, cancellation).await,
            "rfs_edit" => self.dispatch_edit(request, &context, cancellation).await,
            _ => Err(McpError::invalid_params("tool not found", None)),
        }
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
                "Start with rfs_read of rfs:// to discover mounted sources. Use rfs_read with a relative path under the Primary Workspace Root or a canonical rfs://workspace/<root>/<path> reference. Optional limits only lower the binary ceilings. Follow continuationReference to page; retain recoveryReference for the complete immutable projection.",
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
async fn open_session_store(config: SessionStorageConfig) -> Result<SessionStore, ResourceError> {
    match std::env::var_os("RESOURCEFS_TEST_SESSION_ROOT") {
        Some(root) => {
            let ttl_seconds = config.retention_ttl().as_secs() as i64;
            SessionStore::open_with(SessionStorageConfig::new(root, ttl_seconds)?).await
        }
        None => SessionStore::open_with(config).await,
    }
}

#[cfg(not(feature = "test-support"))]
async fn open_session_store(config: SessionStorageConfig) -> Result<SessionStore, ResourceError> {
    SessionStore::open_with(config).await
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

pub(crate) struct ServeFailure {
    diagnostic: Option<String>,
}

impl ServeFailure {
    fn reported() -> Self {
        Self { diagnostic: None }
    }

    fn unreported(error: impl fmt::Display) -> Self {
        Self {
            diagnostic: Some(error.to_string()),
        }
    }

    pub(crate) fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

pub(crate) async fn serve(plan: LaunchPlan) -> Result<(), ServeFailure> {
    let (source, https, github, limits, session_storage, logging, redactor) = plan.into_parts();
    let logging = LogSink::new(logging, redactor)
        .await
        .map_err(ServeFailure::unreported)?;
    #[cfg(feature = "test-support")]
    if let Some(message) = std::env::var_os("RESOURCEFS_TEST_LOG_MESSAGE") {
        let message = message
            .into_string()
            .map_err(|_| ServeFailure::unreported("test log message was not Unicode"))?;
        logging
            .write(LogLevel::Error, &message)
            .await
            .map_err(ServeFailure::unreported)?;
    }
    match serve_inner(source, https, github, limits, session_storage).await {
        Ok(()) => Ok(()),
        Err(error) => {
            if let Err(logging_error) = logging.write(LogLevel::Error, &error.to_string()).await {
                return Err(ServeFailure::unreported(logging_error));
            }
            Err(ServeFailure::reported())
        }
    }
}

async fn serve_inner(
    source: FilesystemSource,
    https: Option<HttpsSource>,
    github: Option<GithubSourceMount>,
    limits: ServerLimits,
    session_storage: SessionStorageConfig,
) -> Result<(), BoxError> {
    #[cfg(feature = "test-support")]
    let test_delivery_gate = start_test_delivery_gate(&source).await?;
    let session_store = open_session_store(session_storage).await?;
    let stored_session = session_store.create_session(limits).await?;
    #[cfg(feature = "test-support")]
    configure_test_storage_failure(&stored_session).await?;
    let (heartbeat_shutdown, heartbeat_stop) = watch::channel(false);
    let mut heartbeat = tokio::spawn(heartbeat_session(stored_session.clone(), heartbeat_stop));
    let session = stored_session.path_session().clone();
    let github = github
        .map(|mount| mount.bind(session.clone()))
        .transpose()?;
    let disconnect = Arc::new(DisconnectState::new(session.clone()));
    let compiled_sources = Arc::new(
        CompiledSources::new(
            source.clone(),
            ArtifactSource::new(session.clone()),
            LocalSource::new(session.clone()),
            https,
            github,
            None,
        )
        .await?,
    );
    let read_sources = Arc::clone(&compiled_sources);
    let discovery_sources = Arc::clone(&compiled_sources);
    let read_engine = ReadEngine::new(read_sources, session.clone(), limits);
    let mutation_engine = MutationEngine::new(compiled_sources, session.clone());
    let discovery_engine = DiscoveryEngine::new(discovery_sources, session, limits);
    let (stdin, stdout) = rmcp::transport::stdio();
    let stdio = rmcp::transport::async_rw::AsyncRwTransport::<RoleServer, _, _>::new(stdin, stdout);
    let cancellations = Arc::new(RequestCancellations::default());
    let transport =
        DisconnectTransport::new(stdio, Arc::clone(&disconnect), Arc::clone(&cancellations));
    let running = match ResourceFsServer::new(
        source,
        read_engine,
        mutation_engine,
        discovery_engine,
        cancellations,
    )
    .serve(transport)
    .await
    {
        Ok(running) => running,
        Err(error) => {
            heartbeat_shutdown.send_replace(true);
            let heartbeat_result = heartbeat.await;
            let disconnect_result = disconnect.finish().await;
            let retention_result = if disconnect_result.is_ok() {
                stored_session.mark_disconnected().await
            } else {
                Ok(())
            };
            heartbeat_result??;
            disconnect_result?;
            retention_result?;
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
    let retention_result = if disconnect_result.is_ok() {
        stored_session.mark_disconnected().await
    } else {
        Ok(())
    };
    #[cfg(feature = "test-support")]
    let delivery_gate_result = match test_delivery_gate {
        Some(task) => task.await?,
        None => Ok(()),
    };
    service_result?;
    heartbeat_result??;
    disconnect_result?;
    retention_result?;
    #[cfg(feature = "test-support")]
    delivery_gate_result?;
    Ok(())
}
