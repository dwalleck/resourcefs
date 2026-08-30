use std::fmt::Write as _;

use resourcefs_core::{
    AllowedOrigin, AtlassianSiteId, ErrorCategory, JiraAddress, JiraIssueResource, PathReference,
    ResourceError,
};

use super::wire::{JiraField, JiraIssue, StrictJson};

pub(crate) const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
pub(crate) const ADF_CONTENT_TYPE: &str = "application/vnd.atlassian.adf+json; charset=utf-8";

#[derive(Debug, Clone)]
pub(crate) struct ProjectionWarning {
    pub(crate) kind: &'static str,
    pub(crate) native_type: String,
    pub(crate) path: String,
    pub(crate) field_reference: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedField {
    pub(crate) id: String,
    pub(crate) canonical_json: String,
    pub(crate) content_type: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct RenderedIssue {
    pub(crate) canonical_reference: String,
    pub(crate) aggregate: String,
    pub(crate) field_index: String,
    pub(crate) fields: Vec<RenderedField>,
    pub(crate) warnings: Vec<ProjectionWarning>,
}

pub(crate) fn render_issue(
    site: &AtlassianSiteId,
    issue: &JiraIssue,
) -> Result<RenderedIssue, ResourceError> {
    let canonical_reference = canonical_issue_reference(site, issue)?;
    let mut aggregate = format!(
        "# Jira Issue {}\n\nCanonical Reference: {canonical_reference}\nIssue ID: {}\nIssue Key: {}\n\n## Fields\n\n",
        issue.key.as_str(),
        issue.id.as_str(),
        issue.key.as_str()
    );
    let mut field_index = format!(
        "# Jira Fields {}\n\nCanonical Issue: {canonical_reference}\n\n",
        issue.key.as_str()
    );
    if issue.fields.is_empty() {
        aggregate.push_str("No visible fields.\n");
        field_index.push_str("No visible fields.\n");
    }

    let mut fields = Vec::with_capacity(issue.fields.len());
    let mut warnings = Vec::new();
    for field in issue.fields.values() {
        let field_reference = canonical_field_reference(site, issue, field)?;
        let name = quoted(&field.name);
        let native_type = quoted(&field.native_type);
        writeln!(
            &mut field_index,
            "Field ID: {}\nName: {name}\nNative Type: {native_type}\nMutable: false\nReference: {field_reference}\n",
            field.id.as_str()
        )
        .expect("writing to a String cannot fail");

        writeln!(
            &mut aggregate,
            "### {}\n\nName: {name}\nNative Type: {native_type}\nMutable: false\nReference: {field_reference}\n\nValue:\n",
            field.id.as_str()
        )
        .expect("writing to a String cannot fail");

        let content_type = if is_adf_candidate(&field.value) {
            let rendered = render_adf(&field.value, &field_reference, &mut warnings)?;
            aggregate.push_str(&rendered);
            if !aggregate.ends_with('\n') {
                aggregate.push('\n');
            }
            ADF_CONTENT_TYPE
        } else {
            aggregate.push_str("```json\n");
            aggregate.push_str(&field.canonical_json);
            aggregate.push_str("\n```\n");
            JSON_CONTENT_TYPE
        };
        aggregate.push('\n');
        fields.push(RenderedField {
            id: field.id.as_str().to_owned(),
            canonical_json: field.canonical_json.clone(),
            content_type,
        });
    }

    if !warnings.is_empty() {
        aggregate.push_str("## Projection Warnings\n\n");
        for warning in &warnings {
            writeln!(
                &mut aggregate,
                "- Field: {}\n  Kind: {}\n  Native Type: {}\n  Path: {}\n",
                warning.field_reference,
                warning.kind,
                quoted(&warning.native_type),
                warning.path
            )
            .expect("writing to a String cannot fail");
        }
    }
    Ok(RenderedIssue {
        canonical_reference,
        aggregate,
        field_index,
        fields,
        warnings,
    })
}

fn canonical_issue_reference(
    site: &AtlassianSiteId,
    issue: &JiraIssue,
) -> Result<String, ResourceError> {
    PathReference::jira(
        JiraAddress::Issue {
            site: site.clone(),
            issue_id: issue.id.clone(),
            resource: JiraIssueResource::Aggregate,
        },
        None,
    )
    .map(|reference| reference.requested().to_owned())
}

fn canonical_field_reference(
    site: &AtlassianSiteId,
    issue: &JiraIssue,
    field: &JiraField,
) -> Result<String, ResourceError> {
    PathReference::jira(
        JiraAddress::Issue {
            site: site.clone(),
            issue_id: issue.id.clone(),
            resource: JiraIssueResource::Field(field.id.clone()),
        },
        None,
    )
    .map(|reference| reference.requested().to_owned())
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("serializing an in-memory string cannot fail")
}

fn is_adf_candidate(value: &StrictJson) -> bool {
    matches!(
        value,
        StrictJson::Object(object)
            if matches!(object.get("type"), Some(StrictJson::String(kind)) if kind == "doc")
    )
}

fn render_adf(
    value: &StrictJson,
    field_reference: &str,
    warnings: &mut Vec<ProjectionWarning>,
) -> Result<String, ResourceError> {
    let StrictJson::Object(document) = value else {
        return Err(malformed_adf("ADF document must be an object"));
    };
    match document.get("version") {
        Some(StrictJson::Number(version)) if version.as_u64() == Some(1) => {}
        _ => return Err(malformed_adf("ADF document version must be 1")),
    }
    let Some(StrictJson::Array(content)) = document.get("content") else {
        return Err(malformed_adf("ADF document content must be an array"));
    };
    let mut output = String::new();
    render_nodes(content, "/content", field_reference, &mut output, warnings)?;
    Ok(output)
}

fn render_nodes(
    nodes: &[StrictJson],
    path: &str,
    field_reference: &str,
    output: &mut String,
    warnings: &mut Vec<ProjectionWarning>,
) -> Result<(), ResourceError> {
    for (index, node) in nodes.iter().enumerate() {
        render_node(
            node,
            &format!("{path}/{index}"),
            field_reference,
            output,
            warnings,
        )?;
    }
    Ok(())
}

fn render_node(
    node: &StrictJson,
    path: &str,
    field_reference: &str,
    output: &mut String,
    warnings: &mut Vec<ProjectionWarning>,
) -> Result<(), ResourceError> {
    let StrictJson::Object(node) = node else {
        return Err(malformed_adf("ADF node must be an object"));
    };
    let kind = required_adf_string(node, "type", "ADF node type")?;
    match kind {
        "paragraph" => {
            let content = required_adf_content(node, "paragraph")?;
            render_nodes(
                content,
                &format!("{path}/content"),
                field_reference,
                output,
                warnings,
            )?;
            output.push_str("\n\n");
        }
        "heading" => {
            let level = match node.get("attrs") {
                Some(StrictJson::Object(attrs)) => match attrs.get("level") {
                    Some(StrictJson::Number(level)) => level.as_u64(),
                    _ => None,
                },
                _ => None,
            }
            .filter(|level| (1..=6).contains(level))
            .ok_or_else(|| malformed_adf("ADF heading level must be between 1 and 6"))?;
            for _ in 0..level {
                output.push('#');
            }
            output.push(' ');
            let content = required_adf_content(node, "heading")?;
            render_nodes(
                content,
                &format!("{path}/content"),
                field_reference,
                output,
                warnings,
            )?;
            output.push_str("\n\n");
        }
        "text" => {
            let text = required_adf_string(node, "text", "ADF text node text")?;
            let mut rendered = escape_markdown(text);
            if let Some(marks) = node.get("marks") {
                let StrictJson::Array(marks) = marks else {
                    return Err(malformed_adf("ADF text marks must be an array"));
                };
                for (index, mark) in marks.iter().enumerate() {
                    rendered = render_mark(
                        mark,
                        &format!("{path}/marks/{index}"),
                        field_reference,
                        rendered,
                        warnings,
                    )?;
                }
            }
            output.push_str(&rendered);
        }
        "hardBreak" => output.push_str("  \n"),
        "rule" => output.push_str("\n---\n"),
        unsupported => {
            let content = optional_adf_content(node)?;
            write!(output, "[Unsupported ADF node: {unsupported}]")
                .expect("writing to a String cannot fail");
            warnings.push(ProjectionWarning {
                kind: "node",
                native_type: unsupported.to_owned(),
                path: path.to_owned(),
                field_reference: field_reference.to_owned(),
            });
            if let Some(content) = content {
                render_nodes(
                    content,
                    &format!("{path}/content"),
                    field_reference,
                    output,
                    warnings,
                )?;
            }
        }
    }
    Ok(())
}

fn render_mark(
    mark: &StrictJson,
    path: &str,
    field_reference: &str,
    rendered: String,
    warnings: &mut Vec<ProjectionWarning>,
) -> Result<String, ResourceError> {
    let StrictJson::Object(mark) = mark else {
        return Err(malformed_adf("ADF mark must be an object"));
    };
    let kind = required_adf_string(mark, "type", "ADF mark type")?;
    let wrapped = match kind {
        "strong" => format!("**{rendered}**"),
        "em" => format!("_{rendered}_"),
        "strike" => format!("~~{rendered}~~"),
        "code" => format!("`{rendered}`"),
        unsupported => {
            warnings.push(ProjectionWarning {
                kind: "mark",
                native_type: unsupported.to_owned(),
                path: path.to_owned(),
                field_reference: field_reference.to_owned(),
            });
            format!("[Unsupported ADF mark: {unsupported}]{rendered}")
        }
    };
    Ok(wrapped)
}

fn required_adf_string<'a>(
    object: &'a std::collections::BTreeMap<String, StrictJson>,
    field: &str,
    label: &str,
) -> Result<&'a str, ResourceError> {
    match object.get(field) {
        Some(StrictJson::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(malformed_adf(format!("{label} must be a non-empty string"))),
    }
}

fn required_adf_content<'a>(
    node: &'a std::collections::BTreeMap<String, StrictJson>,
    kind: &str,
) -> Result<&'a [StrictJson], ResourceError> {
    match node.get("content") {
        Some(StrictJson::Array(content)) => Ok(content),
        _ => Err(malformed_adf(format!(
            "ADF {kind} content must be an array"
        ))),
    }
}

fn optional_adf_content(
    node: &std::collections::BTreeMap<String, StrictJson>,
) -> Result<Option<&[StrictJson]>, ResourceError> {
    match node.get("content") {
        Some(StrictJson::Array(content)) => Ok(Some(content)),
        Some(_) => Err(malformed_adf(
            "unsupported ADF node content must be an array when present",
        )),
        None => Ok(None),
    }
}

fn escape_markdown(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '*' | '_' | '~' | '`' | '[' | ']') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn malformed_adf(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::SourceUnavailable, message)
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraRenderFieldObservation {
    pub id: String,
    pub canonical_json: String,
    pub content_type: &'static str,
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraProjectionWarningObservation {
    pub kind: &'static str,
    pub native_type: String,
    pub path: String,
    pub field_reference: String,
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraRenderObservation {
    pub canonical_reference: String,
    pub aggregate: String,
    pub field_index: String,
    pub fields: Vec<JiraRenderFieldObservation>,
    pub warnings: Vec<JiraProjectionWarningObservation>,
}

#[cfg(feature = "test-support")]
pub fn inspect_jira_render_for_test(
    body: &[u8],
    origin: &str,
    site: &str,
    lookup: super::wire::JiraWireLookupForTest,
) -> Result<JiraRenderObservation, ResourceError> {
    use super::wire::{JiraLookup, decode_issue};
    use resourcefs_core::{JiraIssueId, JiraIssueKey};

    let origin = AllowedOrigin::new(origin, false)?;
    let site = AtlassianSiteId::new(site)?;
    let stable_id;
    let lookup = match &lookup {
        super::wire::JiraWireLookupForTest::StableId(value) => {
            stable_id = JiraIssueId::new(value.clone())?;
            JiraLookup::StableId(&stable_id)
        }
        super::wire::JiraWireLookupForTest::IssueKey(value) => {
            JiraIssueKey::new(value.clone())?;
            JiraLookup::IssueKeyAlias
        }
    };
    let issue = decode_issue(body, &origin, lookup)?;
    let rendered = render_issue(&site, &issue)?;
    Ok(JiraRenderObservation {
        canonical_reference: rendered.canonical_reference,
        aggregate: rendered.aggregate,
        field_index: rendered.field_index,
        fields: rendered
            .fields
            .into_iter()
            .map(|field| JiraRenderFieldObservation {
                id: field.id,
                canonical_json: field.canonical_json,
                content_type: field.content_type,
            })
            .collect(),
        warnings: rendered
            .warnings
            .into_iter()
            .map(|warning| JiraProjectionWarningObservation {
                kind: warning.kind,
                native_type: warning.native_type,
                path: warning.path,
                field_reference: warning.field_reference,
            })
            .collect(),
    })
}
