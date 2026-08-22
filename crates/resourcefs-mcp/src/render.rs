use std::fmt::Write as _;

use resourcefs_core::{
    BEHAVIOR_CONTRACT_VERSION, DiscoveryDiagnostic, GlobKind, GlobResult, ReadResource,
    ResourceError, SearchEngine, SearchResult,
};
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

/// Object-rooted structured content for `rfs_search` results and errors.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SearchToolOutput {
    ok: bool,
    contract_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine: Option<EngineOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<SearchGroupOutput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<Vec<DiscoveryDiagnosticOutput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    returned_records: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_records: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    continuation_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<DiscoveryErrorOutput>,
}

/// Object-rooted structured content for `rfs_glob` results and errors.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct GlobToolOutput {
    ok: bool,
    contract_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    entries: Option<Vec<GlobEntryOutput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<Vec<DiscoveryDiagnosticOutput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    returned_records: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_records: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    continuation_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<DiscoveryErrorOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum EngineOutput {
    RustRegex,
    Pcre2,
}

impl From<SearchEngine> for EngineOutput {
    fn from(engine: SearchEngine) -> Self {
        match engine {
            SearchEngine::RustRegex => Self::RustRegex,
            SearchEngine::Pcre2 => Self::Pcre2,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum GlobKindOutput {
    File,
    Directory,
    Artifact,
}

impl From<GlobKind> for GlobKindOutput {
    fn from(kind: GlobKind) -> Self {
        match kind {
            GlobKind::File => Self::File,
            GlobKind::Directory => Self::Directory,
            GlobKind::Artifact => Self::Artifact,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SearchGroupOutput {
    reference: String,
    lines: Vec<SearchLineOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SearchLineOutput {
    line: u64,
    text: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GlobEntryOutput {
    reference: String,
    kind: GlobKindOutput,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveryDiagnosticOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    reference: Option<String>,
    category: String,
    message: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveryErrorOutput {
    category: String,
    message: String,
}

fn discovery_diagnostic_output(diagnostic: &DiscoveryDiagnostic) -> DiscoveryDiagnosticOutput {
    DiscoveryDiagnosticOutput {
        reference: diagnostic.reference().map(str::to_owned),
        category: diagnostic.category().as_str().to_owned(),
        message: diagnostic.message().to_owned(),
    }
}

pub(crate) fn search_success(result: SearchResult) -> Result<CallToolResult, String> {
    let recovery_reference = result.recovery_reference().map(str::to_owned);
    let continuation_reference = result.continuation_reference().map(str::to_owned);
    let mut text = String::with_capacity(
        result.text().len()
            + recovery_reference.as_ref().map_or(0, String::len)
            + continuation_reference.as_ref().map_or(0, String::len)
            + 48,
    );
    if let Some(reference) = recovery_reference.as_deref() {
        writeln!(text, "Recovery Reference: {reference}")
            .expect("writing a Recovery Reference to String cannot fail");
    }
    if let Some(reference) = continuation_reference.as_deref() {
        writeln!(text, "Continuation Reference: {reference}")
            .expect("writing a continuation reference to String cannot fail");
    }
    text.push_str(result.text());
    let output = SearchToolOutput {
        ok: true,
        contract_version: BEHAVIOR_CONTRACT_VERSION,
        engine: Some(result.engine().into()),
        groups: Some(
            result
                .groups()
                .iter()
                .map(|group| SearchGroupOutput {
                    reference: group.reference().to_owned(),
                    lines: group
                        .lines()
                        .iter()
                        .map(|line| SearchLineOutput {
                            line: line.line(),
                            text: line.text().to_owned(),
                        })
                        .collect(),
                })
                .collect(),
        ),
        diagnostics: Some(
            result
                .diagnostics()
                .iter()
                .map(discovery_diagnostic_output)
                .collect(),
        ),
        returned_records: Some(result.returned_records()),
        total_records: Some(result.total_records()),
        recovery_reference,
        continuation_reference,
        error: None,
    };
    let structured = serde_json::to_value(output)
        .map_err(|error| format!("failed to serialize rfs_search result: {error}"))?;
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(structured);
    Ok(result)
}

pub(crate) fn glob_success(result: GlobResult) -> Result<CallToolResult, String> {
    let recovery_reference = result.recovery_reference().map(str::to_owned);
    let continuation_reference = result.continuation_reference().map(str::to_owned);
    let mut text = String::with_capacity(
        result.text().len()
            + recovery_reference.as_ref().map_or(0, String::len)
            + continuation_reference.as_ref().map_or(0, String::len)
            + 48,
    );
    if let Some(reference) = recovery_reference.as_deref() {
        writeln!(text, "Recovery Reference: {reference}")
            .expect("writing a Recovery Reference to String cannot fail");
    }
    if let Some(reference) = continuation_reference.as_deref() {
        writeln!(text, "Continuation Reference: {reference}")
            .expect("writing a continuation reference to String cannot fail");
    }
    text.push_str(result.text());
    let output = GlobToolOutput {
        ok: true,
        contract_version: BEHAVIOR_CONTRACT_VERSION,
        entries: Some(
            result
                .entries()
                .iter()
                .map(|entry| GlobEntryOutput {
                    reference: entry.reference().to_owned(),
                    kind: entry.kind().into(),
                })
                .collect(),
        ),
        diagnostics: Some(
            result
                .diagnostics()
                .iter()
                .map(discovery_diagnostic_output)
                .collect(),
        ),
        returned_records: Some(result.returned_records()),
        total_records: Some(result.total_records()),
        recovery_reference,
        continuation_reference,
        error: None,
    };
    let structured = serde_json::to_value(output)
        .map_err(|error| format!("failed to serialize rfs_glob result: {error}"))?;
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(structured);
    Ok(result)
}

pub(crate) fn search_failure(
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

    let output = SearchToolOutput {
        ok: false,
        contract_version: BEHAVIOR_CONTRACT_VERSION,
        engine: None,
        groups: None,
        diagnostics: None,
        returned_records: None,
        total_records: None,
        recovery_reference: None,
        continuation_reference: None,
        error: Some(DiscoveryErrorOutput {
            category: error.category().as_str().to_owned(),
            message: error.message().to_owned(),
        }),
    };
    let structured = serde_json::to_value(output)
        .map_err(|error| format!("failed to serialize rfs_search error: {error}"))?;
    let mut result = CallToolResult::error(vec![ContentBlock::text(text)]);
    result.structured_content = Some(structured);
    Ok(result)
}

pub(crate) fn glob_failure(
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

    let output = GlobToolOutput {
        ok: false,
        contract_version: BEHAVIOR_CONTRACT_VERSION,
        entries: None,
        diagnostics: None,
        returned_records: None,
        total_records: None,
        recovery_reference: None,
        continuation_reference: None,
        error: Some(DiscoveryErrorOutput {
            category: error.category().as_str().to_owned(),
            message: error.message().to_owned(),
        }),
    };
    let structured = serde_json::to_value(output)
        .map_err(|error| format!("failed to serialize rfs_glob error: {error}"))?;
    let mut result = CallToolResult::error(vec![ContentBlock::text(text)]);
    result.structured_content = Some(structured);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::Arc,
        time::{Duration, Instant},
    };

    use resourcefs_core::{
        BEHAVIOR_CONTRACT_VERSION, DiscoveryEngine, ErrorCategory, GlobKind, GlobLimits,
        GlobOptions, GlobRequest, GlobTarget, MAX_TEXT_BYTES, OperationGuard, PathReference,
        ReadResource, ResourceError, SearchEngine, SearchLimits, SearchOptions, SearchRequest,
        SearchTarget, WorkspacePath, WorkspaceRootId,
    };
    use resourcefs_sources::{
        ArtifactSource, BackingPathVisibility, CompiledSources, FilesystemSource, LaunchRoot,
        LaunchRootSource, SessionStore, StoredSession,
    };
    use rmcp::model::{CallToolResult, ContentBlock};
    use tempfile::TempDir;

    use super::{glob_failure, glob_success, search_failure, search_success, success};

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

    struct Fixture {
        _temporary: TempDir,
        session: StoredSession,
        engine: DiscoveryEngine,
    }

    async fn fixture() -> Fixture {
        let temporary = TempDir::new().expect("render fixture temporary directory");
        let root = temporary.path().join("workspace");
        fs::create_dir(&root).expect("render fixture workspace root");
        let store = SessionStore::open(temporary.path().join("cache"))
            .await
            .expect("render fixture session store");
        let session = store
            .create_session(resourcefs_core::ServerLimits::default())
            .await
            .expect("render fixture session");
        let filesystem = FilesystemSource::new(
            LaunchRootSource::Cli(vec![LaunchRoot {
                id: WorkspaceRootId::new("workspace").expect("render fixture root ID"),
                path: root,
            }]),
            None,
            BackingPathVisibility::Hidden,
        )
        .await
        .expect("render fixture filesystem source");
        let compiled = Arc::new(CompiledSources::new(
            filesystem,
            ArtifactSource::new(session.path_session().clone()),
        ));
        let engine = DiscoveryEngine::new(
            compiled,
            session.path_session().clone(),
            resourcefs_core::ServerLimits::default(),
        );
        Fixture {
            _temporary: temporary,
            session,
            engine,
        }
    }

    fn write_workspace(fixture: &Fixture, path: &str, content: &str) {
        fs::write(
            fixture._temporary.path().join("workspace").join(path),
            content,
        )
        .expect("render fixture workspace file");
    }

    fn rendered_text(result: &CallToolResult) -> &str {
        match result.content.first() {
            Some(ContentBlock::Text(block)) => block.text.as_str(),
            _ => panic!("expected a text content block"),
        }
    }

    #[tokio::test]
    async fn search_success_text_and_structured_agree() {
        let fixture = fixture().await;
        write_workspace(&fixture, "a.txt", "alpha\nneedle one\nneedle two\n");
        let request = SearchRequest::new(
            SearchTarget::primary(),
            "needle",
            SearchOptions::default(),
            0,
            SearchLimits::default(),
        )
        .expect("render fixture search request");
        let result = fixture
            .engine
            .search(request, &OperationGuard::new())
            .await
            .expect("render fixture search");
        assert_eq!(result.engine(), SearchEngine::RustRegex);
        assert_eq!(result.returned_records(), 2);
        assert_eq!(result.total_records(), 2);

        let rendered = search_success(result).expect("rendered search success");
        assert_eq!(
            rendered_text(&rendered),
            "rfs://workspace/workspace/a.txt\t2\tneedle one\n\
             rfs://workspace/workspace/a.txt\t3\tneedle two\n"
        );
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured search success");
        assert!(structured["ok"].as_bool() == Some(true));
        assert_eq!(structured["contractVersion"], BEHAVIOR_CONTRACT_VERSION);
        assert_eq!(structured["engine"], "rust_regex");
        assert_eq!(structured["returnedRecords"].as_u64(), Some(2));
        assert_eq!(structured["totalRecords"].as_u64(), Some(2));
        assert_eq!(
            structured["groups"][0]["reference"],
            "rfs://workspace/workspace/a.txt"
        );
        assert_eq!(
            structured["groups"][0]["lines"][0]["line"].as_u64(),
            Some(2)
        );
        assert_eq!(structured["groups"][0]["lines"][0]["text"], "needle one");
        assert_eq!(
            structured["groups"][0]["lines"][1]["line"].as_u64(),
            Some(3)
        );
        assert_eq!(structured["groups"][0]["lines"][1]["text"], "needle two");
        assert_eq!(structured["diagnostics"].as_array().map(Vec::len), Some(0));
        assert!(structured.get("recoveryReference").is_none());
        assert!(structured.get("continuationReference").is_none());
        assert!(structured.get("error").is_none());
        assert!(structured.get("requestedPath").is_none());
    }

    #[tokio::test]
    async fn search_no_match_returns_empty_structured_and_text() {
        let fixture = fixture().await;
        write_workspace(&fixture, "a.txt", "alpha\nbeta\n");
        let request = SearchRequest::new(
            SearchTarget::primary(),
            "needle",
            SearchOptions::default(),
            0,
            SearchLimits::default(),
        )
        .expect("render fixture search request");
        let result = fixture
            .engine
            .search(request, &OperationGuard::new())
            .await
            .expect("render fixture search");
        assert_eq!(result.returned_records(), 0);
        assert_eq!(result.total_records(), 0);
        assert!(result.groups().is_empty());
        assert!(result.recovery_reference().is_none());
        assert!(result.continuation_reference().is_none());

        let rendered = search_success(result).expect("rendered no-match search");
        assert_eq!(rendered_text(&rendered), "No matches found.\n");
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured no-match search");
        assert!(structured["ok"].as_bool() == Some(true));
        assert_eq!(structured["engine"], "rust_regex");
        assert_eq!(structured["groups"].as_array().map(Vec::len), Some(0));
        assert_eq!(structured["returnedRecords"].as_u64(), Some(0));
        assert_eq!(structured["totalRecords"].as_u64(), Some(0));
        assert!(structured.get("recoveryReference").is_none());
        assert!(structured.get("continuationReference").is_none());
        assert!(structured.get("error").is_none());
    }

    #[tokio::test]
    async fn search_reports_pcre2_fallback_engine() {
        let fixture = fixture().await;
        write_workspace(&fixture, "pcre.txt", "xy\n");
        let request = SearchRequest::new(
            SearchTarget::primary(),
            r"(?<=x)y",
            SearchOptions::default(),
            0,
            SearchLimits::default(),
        )
        .expect("render fixture search request");
        let result = fixture
            .engine
            .search(request, &OperationGuard::new())
            .await
            .expect("render fixture PCRE2 search");
        assert_eq!(result.engine(), SearchEngine::Pcre2);

        let rendered = search_success(result).expect("rendered PCRE2 search");
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured PCRE2 search");
        assert_eq!(structured["engine"], "pcre2");
        assert_eq!(structured["groups"][0]["lines"][0]["text"], "xy");
    }

    #[tokio::test]
    async fn search_recovery_and_continuation_references_are_rendered() {
        let fixture = fixture().await;
        write_workspace(&fixture, "a.txt", "needle one\nneedle two\nneedle three\n");
        let request = SearchRequest::new(
            SearchTarget::primary(),
            "needle",
            SearchOptions::default(),
            0,
            SearchLimits::new(Some(1), None, None, None).expect("one-record limits"),
        )
        .expect("render fixture search request");
        let result = fixture
            .engine
            .search(request, &OperationGuard::new())
            .await
            .expect("render fixture search");
        assert_eq!(result.returned_records(), 1);
        assert_eq!(result.total_records(), 3);
        let recovery = result
            .recovery_reference()
            .expect("recovery reference")
            .to_owned();
        let continuation = result
            .continuation_reference()
            .expect("continuation reference")
            .to_owned();

        let rendered = search_success(result).expect("rendered recovery search");
        let expected = format!(
            "Recovery Reference: {recovery}\n\
             Continuation Reference: {continuation}\n\
             rfs://workspace/workspace/a.txt\t1\tneedle one\n"
        );
        assert_eq!(rendered_text(&rendered), expected.as_str());
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured recovery search");
        assert_eq!(structured["recoveryReference"], recovery.as_str());
        assert_eq!(structured["continuationReference"], continuation.as_str());
        assert_eq!(structured["returnedRecords"].as_u64(), Some(1));
        assert_eq!(structured["totalRecords"].as_u64(), Some(3));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn search_reports_escaping_link_diagnostic() {
        let fixture = fixture().await;
        write_workspace(&fixture, "a.txt", "needle\n");
        let outside = fixture._temporary.path().join("outside");
        fs::write(&outside, "needle outside\n").expect("outside fixture file");
        std::os::unix::fs::symlink(
            &outside,
            fixture._temporary.path().join("workspace").join("escape"),
        )
        .expect("escaping link fixture");
        let request = SearchRequest::new(
            SearchTarget::primary(),
            "needle",
            SearchOptions::default(),
            0,
            SearchLimits::default(),
        )
        .expect("render fixture search request");
        let result = fixture
            .engine
            .search(request, &OperationGuard::new())
            .await
            .expect("render fixture search");
        assert_eq!(result.returned_records(), 1);
        assert_eq!(result.diagnostics().len(), 1);
        let diagnostic = &result.diagnostics()[0];
        assert_eq!(diagnostic.category(), ErrorCategory::PermissionDenied);
        assert_eq!(
            diagnostic.reference(),
            Some("rfs://workspace/workspace/escape")
        );

        let rendered = search_success(result).expect("rendered diagnostic search");
        assert!(
            rendered_text(&rendered)
                .contains("!rfs://workspace/workspace/escape\tpermission_denied\t")
        );
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured diagnostic search");
        assert_eq!(
            structured["diagnostics"][0]["reference"],
            "rfs://workspace/workspace/escape"
        );
        assert_eq!(
            structured["diagnostics"][0]["category"],
            "permission_denied"
        );
        assert!(
            !structured["diagnostics"][0]["message"]
                .as_str()
                .expect("diagnostic message")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn glob_success_text_and_structured_agree_with_kinds() {
        let fixture = fixture().await;
        write_workspace(&fixture, "a.txt", "alpha\n");
        write_workspace(&fixture, "b.txt", "beta\n");
        fs::create_dir(fixture._temporary.path().join("workspace").join("docs"))
            .expect("render fixture docs directory");
        write_workspace(&fixture, "docs/readme.md", "readme\n");
        let request = GlobRequest::new(
            GlobTarget::new("**/*").expect("render fixture glob target"),
            GlobOptions::default(),
            0,
            GlobLimits::default(),
        );
        let result = fixture
            .engine
            .glob(request, &OperationGuard::new())
            .await
            .expect("render fixture glob");
        assert_eq!(result.returned_records(), 4);
        assert_eq!(result.total_records(), 4);
        assert_eq!(result.entries()[2].kind(), GlobKind::Directory);

        let rendered = glob_success(result).expect("rendered glob success");
        assert_eq!(
            rendered_text(&rendered),
            "rfs://workspace/workspace/a.txt\n\
             rfs://workspace/workspace/b.txt\n\
             rfs://workspace/workspace/docs/\n\
             rfs://workspace/workspace/docs/readme.md\n"
        );
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured glob success");
        assert!(structured["ok"].as_bool() == Some(true));
        assert_eq!(structured["contractVersion"], BEHAVIOR_CONTRACT_VERSION);
        assert_eq!(structured["returnedRecords"].as_u64(), Some(4));
        assert_eq!(structured["totalRecords"].as_u64(), Some(4));
        assert_eq!(
            structured["entries"][0]["reference"],
            "rfs://workspace/workspace/a.txt"
        );
        assert_eq!(structured["entries"][0]["kind"], "file");
        assert_eq!(structured["entries"][1]["kind"], "file");
        assert_eq!(
            structured["entries"][2]["reference"],
            "rfs://workspace/workspace/docs"
        );
        assert_eq!(structured["entries"][2]["kind"], "directory");
        assert_eq!(
            structured["entries"][3]["reference"],
            "rfs://workspace/workspace/docs/readme.md"
        );
        assert_eq!(structured["entries"][3]["kind"], "file");
        assert_eq!(structured["diagnostics"].as_array().map(Vec::len), Some(0));
        assert!(structured.get("engine").is_none());
        assert!(structured.get("recoveryReference").is_none());
        assert!(structured.get("error").is_none());
    }

    #[tokio::test]
    async fn glob_lists_session_artifacts_with_artifact_kind() {
        let fixture = fixture().await;
        fixture
            .session
            .path_session()
            .retain("artifact body", &OperationGuard::new())
            .await
            .expect("retained artifact");
        let request = GlobRequest::new(
            GlobTarget::new("artifact://*").expect("artifact glob target"),
            GlobOptions::default(),
            0,
            GlobLimits::default(),
        );
        let result = fixture
            .engine
            .glob(request, &OperationGuard::new())
            .await
            .expect("artifact glob");
        assert_eq!(result.returned_records(), 1);
        assert_eq!(result.total_records(), 1);
        assert_eq!(result.entries()[0].kind(), GlobKind::Artifact);
        assert!(result.entries()[0].reference().starts_with("artifact://"));
        let expected_reference = result.entries()[0].reference().to_owned();

        let rendered = glob_success(result).expect("rendered artifact glob");
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured artifact glob");
        assert_eq!(
            structured["entries"][0]["reference"],
            expected_reference.as_str()
        );
        assert_eq!(structured["entries"][0]["kind"], "artifact");
    }

    #[tokio::test]
    async fn glob_recovery_reference_is_rendered() {
        let fixture = fixture().await;
        write_workspace(&fixture, "a.txt", "alpha\n");
        write_workspace(&fixture, "b.txt", "beta\n");
        let request = GlobRequest::new(
            GlobTarget::new("**/*").expect("render fixture glob target"),
            GlobOptions::default(),
            0,
            GlobLimits::new(Some(1)).expect("one-entry limits"),
        );
        let result = fixture
            .engine
            .glob(request, &OperationGuard::new())
            .await
            .expect("render fixture glob");
        assert_eq!(result.returned_records(), 1);
        assert_eq!(result.total_records(), 2);
        let recovery = result
            .recovery_reference()
            .expect("recovery reference")
            .to_owned();
        let continuation = result
            .continuation_reference()
            .expect("continuation reference")
            .to_owned();

        let rendered = glob_success(result).expect("rendered recovery glob");
        let expected = format!(
            "Recovery Reference: {recovery}\n\
             Continuation Reference: {continuation}\n\
             rfs://workspace/workspace/a.txt\n"
        );
        assert_eq!(rendered_text(&rendered), expected.as_str());
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured recovery glob");
        assert_eq!(structured["recoveryReference"], recovery.as_str());
        assert_eq!(structured["continuationReference"], continuation.as_str());
        assert_eq!(structured["returnedRecords"].as_u64(), Some(1));
        assert_eq!(structured["totalRecords"].as_u64(), Some(2));
    }

    #[test]
    fn search_and_glob_failures_use_rfs_error_shape() {
        let error = ResourceError::new(
            ErrorCategory::InvalidPattern,
            "pattern rejected by both engines",
        );
        let search = search_failure("rfs://workspace/workspace/a.txt", &error)
            .expect("rendered search failure");
        assert_eq!(
            rendered_text(&search),
            "[rfs_error]\ncategory: invalid_pattern\npath: rfs://workspace/workspace/a.txt\nmessage: pattern rejected by both engines"
        );
        assert_eq!(search.is_error, Some(true));
        let structured = search
            .structured_content
            .as_ref()
            .expect("structured search failure");
        assert!(structured["ok"].as_bool() == Some(false));
        assert_eq!(structured["contractVersion"], BEHAVIOR_CONTRACT_VERSION);
        assert_eq!(structured["error"]["category"], "invalid_pattern");
        assert_eq!(
            structured["error"]["message"],
            "pattern rejected by both engines"
        );
        assert!(structured.get("engine").is_none());
        assert!(structured.get("groups").is_none());
        assert!(structured.get("diagnostics").is_none());
        assert!(structured.get("returnedRecords").is_none());
        assert!(structured.get("totalRecords").is_none());
        assert!(structured.get("recoveryReference").is_none());
        assert!(structured.get("requestedPath").is_none());

        let glob = glob_failure("*.txt", &error).expect("rendered glob failure");
        assert_eq!(
            rendered_text(&glob),
            "[rfs_error]\ncategory: invalid_pattern\npath: *.txt\nmessage: pattern rejected by both engines"
        );
        assert_eq!(glob.is_error, Some(true));
        let structured = glob
            .structured_content
            .as_ref()
            .expect("structured glob failure");
        assert!(structured["ok"].as_bool() == Some(false));
        assert_eq!(structured["error"]["category"], "invalid_pattern");
        assert_eq!(
            structured["error"]["message"],
            "pattern rejected by both engines"
        );
        assert!(structured.get("entries").is_none());
        assert!(structured.get("engine").is_none());
        assert!(structured.get("requestedPath").is_none());
    }

    #[tokio::test]
    async fn maximum_search_page_render_stays_within_budget() {
        let fixture = fixture().await;
        let mut content = String::with_capacity(7_000);
        for _ in 0..1_000 {
            content.push_str("needle\n");
        }
        write_workspace(&fixture, "maximum.txt", &content);
        let request = SearchRequest::new(
            SearchTarget::primary(),
            "needle",
            SearchOptions::default(),
            0,
            SearchLimits::default(),
        )
        .expect("render fixture search request");
        let result = fixture
            .engine
            .search(request, &OperationGuard::new())
            .await
            .expect("render fixture maximum search");
        assert_eq!(result.returned_records(), 1_000);
        assert_eq!(result.total_records(), 1_000);
        assert!(result.recovery_reference().is_none());
        let expected_text_len = result.text().len();
        assert!(
            expected_text_len <= MAX_TEXT_BYTES,
            "maximum search page stays within 48 KiB"
        );

        let started = Instant::now();
        let rendered = search_success(result).expect("rendered maximum search page");
        let elapsed = started.elapsed();
        assert_eq!(rendered_text(&rendered).len(), expected_text_len);
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured maximum search");
        assert_eq!(structured["returnedRecords"].as_u64(), Some(1_000));
        assert_eq!(structured["totalRecords"].as_u64(), Some(1_000));
        assert_eq!(
            structured["groups"][0]["lines"].as_array().map(Vec::len),
            Some(1_000)
        );
        if !cfg!(debug_assertions) {
            assert!(
                elapsed <= Duration::from_millis(25),
                "maximum-page render took {elapsed:?}"
            );
        }
    }

    #[tokio::test]
    async fn maximum_glob_page_render_stays_within_budget() {
        let fixture = fixture().await;
        for index in 0..1_000 {
            write_workspace(&fixture, &format!("max-{index:04}.txt"), "x\n");
        }
        let request = GlobRequest::new(
            GlobTarget::new("**/*").expect("render fixture glob target"),
            GlobOptions::default(),
            0,
            GlobLimits::default(),
        );
        let result = fixture
            .engine
            .glob(request, &OperationGuard::new())
            .await
            .expect("render fixture maximum glob");
        assert_eq!(result.returned_records(), 1_000);
        assert_eq!(result.total_records(), 1_000);
        assert!(result.recovery_reference().is_none());
        let expected_text_len = result.text().len();

        let started = Instant::now();
        let rendered = glob_success(result).expect("rendered maximum glob page");
        let elapsed = started.elapsed();
        assert_eq!(rendered_text(&rendered).len(), expected_text_len);
        let structured = rendered
            .structured_content
            .as_ref()
            .expect("structured maximum glob");
        assert_eq!(structured["entries"].as_array().map(Vec::len), Some(1_000));
        if !cfg!(debug_assertions) {
            assert!(
                elapsed <= Duration::from_millis(25),
                "maximum-page render took {elapsed:?}"
            );
        }
    }
}
