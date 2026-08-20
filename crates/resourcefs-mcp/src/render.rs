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
    backing_file_uri: Option<String>,
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
    continuation_reference: Option<String>,
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
    let recovery_reference = resource.recovery_reference().map(str::to_owned);
    let continuation_reference = resource.continuation_reference().map(str::to_owned);
    let mut text = String::with_capacity(
        resource.canonical_reference().len()
            + resource.version_tag().as_str().len()
            + resource.content().len()
            + recovery_reference.as_ref().map_or(0, String::len)
            + continuation_reference.as_ref().map_or(0, String::len)
            + 48,
    );
    writeln!(
        text,
        "[{}#{}]",
        resource.canonical_reference(),
        resource.version_tag()
    )
    .expect("writing a read header to String cannot fail");

    if let Some(reference) = recovery_reference.as_deref() {
        writeln!(text, "Recovery Reference: {reference}")
            .expect("writing a Recovery Reference to String cannot fail");
    }
    if let Some(reference) = continuation_reference.as_deref() {
        writeln!(text, "Continuation Reference: {reference}")
            .expect("writing a continuation reference to String cannot fail");
    }
    text.push_str(resource.content());
    let output = ReadToolOutput {
        ok: true,
        contract_version: BEHAVIOR_CONTRACT_VERSION,
        requested_path: requested_path.to_owned(),
        canonical_reference: Some(resource.canonical_reference().to_owned()),
        backing_file_uri: resource.backing_file_uri().map(str::to_owned),
        content_type: Some(resource.content_type()),
        version_tag: Some(resource.version_tag().to_string()),
        mutable: Some(resource.is_mutable()),
        bounded: Some(resource.is_bounded()),
        recovery_reference,
        continuation_reference,
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
        backing_file_uri: None,
        content_type: None,
        version_tag: None,
        mutable: None,
        bounded: None,
        recovery_reference: None,
        continuation_reference: None,
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

#[cfg(test)]
mod tests {
    use resourcefs_core::{
        MAX_TEXT_BYTES, PathReference, ReadResource, WorkspacePath, WorkspaceRootId,
    };
    use std::time::{Duration, Instant};

    use super::success;

    fn resource() -> ReadResource {
        let reference = PathReference::canonical(
            WorkspaceRootId::new("workspace").expect("static root ID"),
            WorkspacePath::new("visible.txt").expect("static workspace path"),
        );
        ReadResource::text(reference, "content".to_owned()).expect("read resource")
    }

    #[test]
    fn renders_backing_file_uri_only_when_present() {
        let hidden = success("visible.txt", resource()).expect("hidden result");
        assert!(
            hidden
                .structured_content
                .as_ref()
                .expect("structured hidden result")
                .get("backingFileUri")
                .is_none()
        );

        let visible_resource = resource()
            .with_backing_file_uri("file:///workspace/visible.txt")
            .expect("local backing URI");
        let visible = success("visible.txt", visible_resource).expect("visible result");
        let structured = visible
            .structured_content
            .as_ref()
            .expect("structured visible result");
        assert_eq!(
            structured["canonicalReference"],
            "rfs://workspace/workspace/visible.txt"
        );
        assert_eq!(
            structured["backingFileUri"],
            "file:///workspace/visible.txt"
        );
    }

    #[test]
    fn maximum_page_render_stays_within_budget() {
        let reference = PathReference::canonical(
            WorkspaceRootId::new("workspace").expect("static root ID"),
            WorkspacePath::new("maximum.txt").expect("static workspace path"),
        );
        let resource = ReadResource::text(reference, "x".repeat(MAX_TEXT_BYTES))
            .expect("maximum page resource");

        let started = Instant::now();
        let result = success("maximum.txt", resource).expect("maximum page result");
        let elapsed = started.elapsed();

        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|output| output["content"].as_str())
                .map(str::len),
            Some(MAX_TEXT_BYTES)
        );
        if !cfg!(debug_assertions) {
            assert!(
                elapsed <= Duration::from_millis(25),
                "maximum-page render took {elapsed:?}"
            );
        }
    }
}
