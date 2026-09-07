use std::fmt::Write as _;

use crate::ResourceError;

use super::{
    AtlassianSiteId, PathReference, ProjectionSelector, invalid_reference, percent_decode,
    projection_candidate_split, validate_percent_encoding,
};

pub(crate) const JIRA_PREFIX: &str = "jira://";
/// Maximum decimal length retained for one Jira stable issue ID.
pub const MAX_JIRA_ISSUE_ID_BYTES: usize = 64;
/// Maximum decimal length retained for one Jira stable project ID.
pub const MAX_JIRA_PROJECT_ID_BYTES: usize = 64;
/// Maximum decoded UTF-8 length of one opaque Jira path segment.
pub const MAX_JIRA_SEGMENT_BYTES: usize = 255;

/// Stable Jira issue ID retained as its canonical decimal string without machine narrowing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JiraIssueId(String);

impl JiraIssueId {
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        validate_jira_decimal_id(&value, MAX_JIRA_ISSUE_ID_BYTES, "Jira issue ID")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable Jira project identity, without machine-integer narrowing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JiraProjectId(String);

impl JiraProjectId {
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        validate_jira_decimal_id(&value, MAX_JIRA_PROJECT_ID_BYTES, "Jira project ID")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_jira_decimal_id(value: &str, maximum: usize, label: &str) -> Result<(), ResourceError> {
    if value.is_empty()
        || value.len() > maximum
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid_reference(format!(
            "{label} must be a canonical nonzero decimal string"
        )));
    }
    Ok(())
}

macro_rules! jira_opaque_segment {
    ($name:ident, $label:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
                let value = value.into();
                validate_jira_opaque_segment(&value, $label)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

jira_opaque_segment!(JiraIssueKey, "Jira issue key");
jira_opaque_segment!(JiraFieldId, "Jira field ID");
jira_opaque_segment!(JiraProjectKey, "Jira project key");

fn validate_jira_opaque_segment(value: &str, label: &str) -> Result<(), ResourceError> {
    if value.is_empty()
        || value.len() > MAX_JIRA_SEGMENT_BYTES
        || matches!(value, "." | ".." | "new")
        || value.contains(['/', '\\'])
        || value.chars().any(char::is_control)
    {
        return Err(invalid_reference(format!(
            "{label} must be one safe non-empty UTF-8 path segment of at most {MAX_JIRA_SEGMENT_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Addressable read-only Resource below one stable Jira issue.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum JiraIssueResource {
    Aggregate,
    Fields,
    Field(JiraFieldId),
}

/// Parsed Jira read identity independent of its optional selector.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum JiraAddress {
    Projects {
        site: AtlassianSiteId,
    },
    Project {
        site: AtlassianSiteId,
        project_id: JiraProjectId,
    },
    ProjectKeyAlias {
        site: AtlassianSiteId,
        project_key: JiraProjectKey,
    },
    Issue {
        site: AtlassianSiteId,
        issue_id: JiraIssueId,
        resource: JiraIssueResource,
    },
    IssueKeyAlias {
        site: AtlassianSiteId,
        issue_key: JiraIssueKey,
    },
}

impl JiraAddress {
    pub const fn site(&self) -> &AtlassianSiteId {
        match self {
            Self::Issue { site, .. }
            | Self::IssueKeyAlias { site, .. }
            | Self::Projects { site }
            | Self::Project { site, .. }
            | Self::ProjectKeyAlias { site, .. } => site,
        }
    }

    pub fn issue_id(&self) -> Option<&JiraIssueId> {
        match self {
            Self::Issue { issue_id, .. } => Some(issue_id),
            Self::IssueKeyAlias { .. }
            | Self::Projects { .. }
            | Self::Project { .. }
            | Self::ProjectKeyAlias { .. } => None,
        }
    }

    pub fn issue_key(&self) -> Option<&JiraIssueKey> {
        match self {
            Self::IssueKeyAlias { issue_key, .. } => Some(issue_key),
            Self::Issue { .. }
            | Self::Projects { .. }
            | Self::Project { .. }
            | Self::ProjectKeyAlias { .. } => None,
        }
    }

    pub fn resource(&self) -> Option<&JiraIssueResource> {
        match self {
            Self::Issue { resource, .. } => Some(resource),
            Self::IssueKeyAlias { .. }
            | Self::Projects { .. }
            | Self::Project { .. }
            | Self::ProjectKeyAlias { .. } => None,
        }
    }

    pub(crate) fn canonical_reference(&self) -> String {
        match self {
            Self::Projects { site } => format!("{JIRA_PREFIX}{}/projects", site.as_str()),
            Self::Project { site, project_id } => {
                format!(
                    "{JIRA_PREFIX}{}/projects/{}",
                    site.as_str(),
                    project_id.as_str()
                )
            }
            Self::ProjectKeyAlias { site, project_key } => {
                let mut reference = format!("{JIRA_PREFIX}{}/project-keys/", site.as_str());
                encode_jira_segment(project_key.as_str(), &mut reference);
                reference
            }
            Self::Issue {
                site,
                issue_id,
                resource,
            } => {
                let base = format!(
                    "{JIRA_PREFIX}{}/issues/{}",
                    site.as_str(),
                    issue_id.as_str()
                );
                match resource {
                    JiraIssueResource::Aggregate => base,
                    JiraIssueResource::Fields => format!("{base}/fields"),
                    JiraIssueResource::Field(field_id) => {
                        let mut reference = format!("{base}/fields/");
                        encode_jira_segment(field_id.as_str(), &mut reference);
                        reference
                    }
                }
            }
            Self::IssueKeyAlias { site, issue_key } => {
                let mut reference = format!("{JIRA_PREFIX}{}/issue-keys/", site.as_str());
                encode_jira_segment(issue_key.as_str(), &mut reference);
                reference
            }
        }
    }
}

pub(super) fn parse_jira_address(input: &str) -> Result<JiraAddress, ResourceError> {
    let body = input
        .strip_prefix(JIRA_PREFIX)
        .ok_or_else(|| invalid_reference("malformed Jira reference"))?;
    let segments = body.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        [site, "projects"] => Ok(JiraAddress::Projects {
            site: AtlassianSiteId::new((*site).to_owned())?,
        }),
        [site, "projects", project_id] => Ok(JiraAddress::Project {
            site: AtlassianSiteId::new((*site).to_owned())?,
            project_id: JiraProjectId::new((*project_id).to_owned())?,
        }),
        [site, "project-keys", project_key] => Ok(JiraAddress::ProjectKeyAlias {
            site: AtlassianSiteId::new((*site).to_owned())?,
            project_key: JiraProjectKey::new(parse_canonical_jira_segment(
                project_key,
                "Jira project key",
            )?)?,
        }),
        [site, "issues", issue_id] => Ok(JiraAddress::Issue {
            site: AtlassianSiteId::new((*site).to_owned())?,
            issue_id: JiraIssueId::new((*issue_id).to_owned())?,
            resource: JiraIssueResource::Aggregate,
        }),
        [site, "issue-keys", issue_key] => Ok(JiraAddress::IssueKeyAlias {
            site: AtlassianSiteId::new((*site).to_owned())?,
            issue_key: JiraIssueKey::new(parse_canonical_jira_segment(
                issue_key,
                "Jira issue key",
            )?)?,
        }),
        [site, "issues", issue_id, "fields"] => Ok(JiraAddress::Issue {
            site: AtlassianSiteId::new((*site).to_owned())?,
            issue_id: JiraIssueId::new((*issue_id).to_owned())?,
            resource: JiraIssueResource::Fields,
        }),
        [site, "issues", issue_id, "fields", field_id] => Ok(JiraAddress::Issue {
            site: AtlassianSiteId::new((*site).to_owned())?,
            issue_id: JiraIssueId::new((*issue_id).to_owned())?,
            resource: JiraIssueResource::Field(JiraFieldId::new(parse_canonical_jira_segment(
                field_id,
                "Jira field ID",
            )?)?),
        }),
        _ => Err(invalid_reference("unsupported Jira Resource path")),
    }
}

fn parse_canonical_jira_segment(input: &str, label: &str) -> Result<String, ResourceError> {
    validate_percent_encoding(input)?;
    let decoded = percent_decode(input)?;
    let mut canonical = String::with_capacity(input.len());
    encode_jira_segment(&decoded, &mut canonical);
    if canonical != input {
        return Err(invalid_reference(format!(
            "{label} must use canonical percent encoding"
        )));
    }
    Ok(decoded)
}

fn encode_jira_segment(segment: &str, output: &mut String) {
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(byte));
        } else {
            write!(output, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
}

pub(super) fn parse_jira_reference(
    input: &str,
) -> Result<(JiraAddress, Option<ProjectionSelector>), ResourceError> {
    // Native page markers belong only to Jira; literal workspace/URL interpretations stay intact.
    let split = input
        .find(":offset:")
        .map(|index| (&input[..index], &input[index + 1..]))
        .or_else(|| projection_candidate_split(input));
    let (base, projection) = match split {
        Some((base, selector)) => (base, Some(ProjectionSelector::parse(selector)?)),
        None => (input, None),
    };
    let address = parse_jira_address(base)?;
    if projection
        .as_ref()
        .is_some_and(|selector| selector.source_offset().is_some())
        && !matches!(address, JiraAddress::Projects { .. })
    {
        return Err(invalid_reference(
            "source offset requires a Jira projects collection",
        ));
    }
    Ok((address, projection))
}

pub(crate) fn canonical_record_identity(reference: &PathReference, address: &JiraAddress) -> bool {
    if matches!(address, JiraAddress::ProjectKeyAlias { .. }) {
        return false;
    }
    let canonical = address.canonical_reference();
    match reference.projection() {
        None => reference.requested() == canonical,
        Some(selector) => {
            matches!(address, JiraAddress::Projects { .. })
                && selector.source_offset().is_some()
                && reference.requested() == format!("{canonical}:{}", selector.as_str())
        }
    }
}
