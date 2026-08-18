use std::fmt::Write as _;

use resourcefs_core::{BEHAVIOR_CONTRACT_VERSION, ReadResource, ResourceError};
use rmcp::model::{CallToolResult, ContentBlock};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ReadToolOutput {
    ok: bool,
    contract_version: &'static str,
    requested_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mutable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bounded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ReadErrorOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReadErrorOutput {
    category: String,
    message: String,
}

pub(crate) fn success(
    requested_path: &str,
    resource: ReadResource,
) -> Result<CallToolResult, String> {
    let mut text = String::with_capacity(
        resource.canonical_reference().len()
            + resource.version_tag().as_str().len()
            + resource.content().len()
            + 4,
    );
    writeln!(
        text,
        "[{}#{}]",
        resource.canonical_reference(),
        resource.version_tag()
    )
    .expect("writing a read header to String cannot fail");
    text.push_str(resource.content());

    let output = ReadToolOutput {
        ok: true,
        contract_version: BEHAVIOR_CONTRACT_VERSION,
        requested_path: requested_path.to_owned(),
        canonical_reference: Some(resource.canonical_reference().to_owned()),
        content_type: Some(resource.content_type()),
        version_tag: Some(resource.version_tag().to_string()),
        mutable: Some(resource.is_mutable()),
        bounded: Some(resource.is_bounded()),
        recovery_reference: None,
        content: Some(resource.content().to_owned()),
        error: None,
    };
    let structured = serde_json::to_value(output)
        .map_err(|error| format!("failed to serialize rfs_read result: {error}"))?;
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(structured);
    Ok(result)
}

pub(crate) fn failure(
    requested_path: &str,
    error: &ResourceError,
) -> Result<CallToolResult, String> {
    let mut text = String::with_capacity(requested_path.len() + error.message().len() + 64);
    write!(
        text,
        "[rfs_error]\ncategory: {}\npath: {}\nmessage: {}",
        error.category(),
        requested_path,
        error.message()
    )
    .expect("writing an error result to String cannot fail");

    let output = ReadToolOutput {
        ok: false,
        contract_version: BEHAVIOR_CONTRACT_VERSION,
        requested_path: requested_path.to_owned(),
        canonical_reference: None,
        content_type: None,
        version_tag: None,
        mutable: None,
        bounded: None,
        recovery_reference: None,
        content: None,
        error: Some(ReadErrorOutput {
            category: error.category().as_str().to_owned(),
            message: error.message().to_owned(),
        }),
    };
    let structured = serde_json::to_value(output).map_err(|serialization_error| {
        format!("failed to serialize rfs_read error: {serialization_error}")
    })?;
    let mut result = CallToolResult::error(vec![ContentBlock::text(text)]);
    result.structured_content = Some(structured);
    Ok(result)
}
