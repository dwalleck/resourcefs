use resourcefs_core::{ReadAcquisitionLimits, ReadRequest, TextLimits};

use super::*;
use crate::acquisition::AcquisitionInput;

/// Mirrors the error `ReadEngine` reports at its next liveness checkpoint after
/// cancellation, preserving the established read cancellation category.
fn cancelled_read_error() -> ResourceError {
    ResourceError::new(
        ErrorCategory::SourceUnavailable,
        "Path Session is no longer active",
    )
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

impl ReadLimitsInput {
    fn into_text_limits(self) -> Result<TextLimits, ResourceError> {
        TextLimits::new(self.bytes, self.lines, self.columns)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadInput {
    path: String,
    #[serde(default)]
    numbered: bool,
    #[serde(default, deserialize_with = "deserialize_object_limits")]
    limits: ReadLimitsInput,
    #[serde(default, deserialize_with = "deserialize_acquisition")]
    #[schemars(with = "AcquisitionInput")]
    acquisition: Option<AcquisitionInput>,
}

fn deserialize_acquisition<'de, D>(deserializer: D) -> Result<Option<AcquisitionInput>, D::Error>
where
    D: Deserializer<'de>,
{
    AcquisitionInput::deserialize(deserializer).map(Some)
}

impl ResourceFsServer {
    pub(super) async fn read_input(&self, input: ReadInput) -> Result<CallToolResult, String> {
        let limits = input
            .limits
            .into_text_limits()
            .map_err(|error| error.to_string())?;
        let acquisition = input
            .acquisition
            .map(AcquisitionInput::into_limits)
            .transpose()
            .map_err(|error| error.to_string())?;
        self.execute_read(
            &input.path,
            limits,
            input.numbered,
            acquisition,
            &OperationGuard::new(),
        )
        .await
    }

    async fn execute_read(
        &self,
        path: &str,
        limits: TextLimits,
        numbered: bool,
        acquisition: Option<ReadAcquisitionLimits>,
        operation: &OperationGuard,
    ) -> Result<CallToolResult, String> {
        let reference = match PathReference::parse(path) {
            Ok(reference) => reference,
            Err(error) => return render::failure(path, &error),
        };
        let request = ReadRequest {
            reference,
            limits,
            numbered,
            acquisition,
        };
        match self.read_engine.read(request, operation).await {
            Ok(resource) => render::success(path, resource),
            Err(error) => render::failure(path, &error),
        }
    }

    pub(super) async fn dispatch_read(
        &self,
        request: CallToolRequestParams,
        context: &RequestContext<RoleServer>,
        cancellation: watch::Receiver<bool>,
    ) -> Result<CallToolResponse, McpError> {
        let input: ReadInput = deserialize_input(request).map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_read arguments: {error}"), None)
        })?;
        let limits = input.limits.into_text_limits().map_err(|error| {
            McpError::invalid_params(format!("invalid rfs_read arguments: {error}"), None)
        })?;
        let acquisition = input
            .acquisition
            .map(AcquisitionInput::into_limits)
            .transpose()
            .map_err(|error| {
                McpError::invalid_params(format!("invalid rfs_read arguments: {error}"), None)
            })?;
        let operation = OperationGuard::new();
        let cancelled_result = || render::failure(&input.path, &cancelled_read_error());
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
        let pending =
            self.execute_read(&input.path, limits, input.numbered, acquisition, &operation);
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
