use serde::{Deserialize, de::DeserializeOwned};

use resourcefs_core::{ErrorCategory, ResourceError};

#[derive(Debug, Deserialize)]
pub(super) struct SimpleUser {
    pub(super) login: String,
    pub(super) id: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct Issue {
    pub(super) id: u64,
    pub(super) number: u64,
    pub(super) state: String,
    pub(super) title: String,
    pub(super) body: Option<String>,
    pub(super) user: Option<SimpleUser>,
    pub(super) html_url: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    #[serde(default)]
    pub(super) pull_request: Option<PullRequestMarker>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PullRequestMarker {}

#[derive(Debug, Deserialize)]
pub(super) struct GitRef {
    #[serde(rename = "ref")]
    pub(super) name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PullRequest {
    pub(super) id: u64,
    pub(super) number: u64,
    pub(super) state: String,
    pub(super) title: String,
    pub(super) body: Option<String>,
    pub(super) user: Option<SimpleUser>,
    pub(super) html_url: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) draft: bool,
    pub(super) merged: bool,
    pub(super) merged_at: Option<String>,
    pub(super) head: GitRef,
    pub(super) base: GitRef,
}

#[derive(Debug, Deserialize)]
pub(super) struct PullRequestSummary {
    pub(super) id: u64,
    pub(super) number: u64,
    pub(super) state: String,
    pub(super) title: String,
    pub(super) user: Option<SimpleUser>,
    pub(super) html_url: String,
    pub(super) updated_at: String,
    pub(super) draft: bool,
    pub(super) merged_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConversationComment {
    pub(super) id: u64,
    pub(super) body: Option<String>,
    pub(super) user: Option<SimpleUser>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) issue_url: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct Review {
    pub(super) id: u64,
    pub(super) body: String,
    pub(super) user: Option<SimpleUser>,
    pub(super) state: String,
    #[serde(default)]
    pub(super) submitted_at: Option<String>,
    pub(super) commit_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReviewComment {
    pub(super) id: u64,
    pub(super) body: String,
    pub(super) user: Option<SimpleUser>,
    pub(super) path: String,
    pub(super) diff_hunk: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) pull_request_review_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DiffFile {
    pub(super) filename: String,
    pub(super) status: String,
    #[serde(default)]
    pub(super) patch: Option<String>,
}

pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ResourceError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let decoded = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| malformed_json(error.path().to_string(), &error))?;
    deserializer
        .end()
        .map_err(|error| malformed_json("<root>".to_owned(), &error))?;
    Ok(decoded)
}

fn malformed_json(path: String, error: &dyn std::fmt::Display) -> ResourceError {
    let mut message = format!("GitHub upstream JSON is malformed at '{path}': {error}");
    const MAX_DIAGNOSTIC_BYTES: usize = 512;
    if message.len() > MAX_DIAGNOSTIC_BYTES {
        let mut end = MAX_DIAGNOSTIC_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    ResourceError::new(ErrorCategory::SourceUnavailable, message)
}

#[cfg(feature = "test-support")]
fn require_positive(value: u64, field: &str) -> Result<u64, ResourceError> {
    if value == 0 {
        return Err(ResourceError::new(
            ErrorCategory::SourceUnavailable,
            format!("GitHub upstream field '{field}' must be positive"),
        ));
    }
    Ok(value)
}

#[cfg(feature = "test-support")]
fn string_bytes<'a>(values: impl IntoIterator<Item = &'a str>) -> usize {
    values
        .into_iter()
        .fold(0_usize, |total, value| total.saturating_add(value.len()))
}

#[cfg(feature = "test-support")]
fn optional_bytes(value: &Option<String>) -> usize {
    value.as_deref().map_or(0, str::len)
}

#[cfg(feature = "test-support")]
fn user_bytes(value: &Option<SimpleUser>) -> Result<usize, ResourceError> {
    let Some(user) = value else {
        return Ok(0);
    };
    require_positive(user.id, "user.id")?;
    Ok(user.login.len())
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubWireKindForTest {
    Issue,
    PullRequest,
    ConversationComments,
    Reviews,
    ReviewComments,
    DiffFiles,
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubWireObservation {
    ids: Vec<u64>,
    numbers: Vec<u64>,
    absent_fields: Vec<&'static str>,
    retained_bytes: usize,
}

#[cfg(feature = "test-support")]
impl GithubWireObservation {
    pub fn ids(&self) -> &[u64] {
        &self.ids
    }

    pub fn numbers(&self) -> &[u64] {
        &self.numbers
    }

    pub fn absent_fields(&self) -> &[&'static str] {
        &self.absent_fields
    }

    pub const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

#[cfg(feature = "test-support")]
pub fn inspect_github_wire_for_test(
    kind: GithubWireKindForTest,
    bytes: &[u8],
) -> Result<GithubWireObservation, ResourceError> {
    match kind {
        GithubWireKindForTest::Issue => {
            let value: Issue = decode(bytes)?;
            let id = require_positive(value.id, "id")?;
            let number = require_positive(value.number, "number")?;
            let mut absent_fields = Vec::new();
            if value.body.is_none() {
                absent_fields.push("body");
            }
            if value.user.is_none() {
                absent_fields.push("user");
            }
            let retained_bytes = string_bytes([
                value.state.as_str(),
                value.title.as_str(),
                value.html_url.as_str(),
                value.created_at.as_str(),
                value.updated_at.as_str(),
            ])
            .saturating_add(optional_bytes(&value.body))
            .saturating_add(user_bytes(&value.user)?)
            .saturating_add(usize::from(value.pull_request.is_some()));
            Ok(GithubWireObservation {
                ids: vec![id],
                numbers: vec![number],
                absent_fields,
                retained_bytes,
            })
        }
        GithubWireKindForTest::PullRequest => {
            let value: PullRequest = decode(bytes)?;
            let id = require_positive(value.id, "id")?;
            let number = require_positive(value.number, "number")?;
            let mut absent_fields = Vec::new();
            if value.body.is_none() {
                absent_fields.push("body");
            }
            if value.user.is_none() {
                absent_fields.push("user");
            }
            if value.merged_at.is_none() {
                absent_fields.push("merged_at");
            }
            let retained_bytes = string_bytes([
                value.state.as_str(),
                value.title.as_str(),
                value.html_url.as_str(),
                value.created_at.as_str(),
                value.updated_at.as_str(),
                value.head.name.as_str(),
                value.base.name.as_str(),
            ])
            .saturating_add(optional_bytes(&value.body))
            .saturating_add(user_bytes(&value.user)?)
            .saturating_add(optional_bytes(&value.merged_at))
            .saturating_add(usize::from(value.draft))
            .saturating_add(usize::from(value.merged));
            Ok(GithubWireObservation {
                ids: vec![id],
                numbers: vec![number],
                absent_fields,
                retained_bytes,
            })
        }
        GithubWireKindForTest::ConversationComments => {
            let values: Vec<ConversationComment> = decode(bytes)?;
            let mut ids = Vec::with_capacity(values.len());
            let mut absent_fields = Vec::new();
            let mut retained_bytes = 0_usize;
            for value in values {
                ids.push(require_positive(value.id, "id")?);
                if value.body.is_none() {
                    absent_fields.push("body");
                }
                if value.user.is_none() {
                    absent_fields.push("user");
                }
                retained_bytes = retained_bytes
                    .saturating_add(string_bytes([
                        value.created_at.as_str(),
                        value.updated_at.as_str(),
                        value.issue_url.as_str(),
                    ]))
                    .saturating_add(optional_bytes(&value.body))
                    .saturating_add(user_bytes(&value.user)?);
            }
            Ok(GithubWireObservation {
                ids,
                numbers: Vec::new(),
                absent_fields,
                retained_bytes,
            })
        }
        GithubWireKindForTest::Reviews => {
            let values: Vec<Review> = decode(bytes)?;
            let mut ids = Vec::with_capacity(values.len());
            let mut absent_fields = Vec::new();
            let mut retained_bytes = 0_usize;
            for value in values {
                ids.push(require_positive(value.id, "id")?);
                if value.commit_id.is_none() {
                    absent_fields.push("commit_id");
                }
                if value.submitted_at.is_none() {
                    absent_fields.push("submitted_at");
                }
                if value.user.is_none() {
                    absent_fields.push("user");
                }
                retained_bytes = retained_bytes
                    .saturating_add(string_bytes([value.body.as_str(), value.state.as_str()]))
                    .saturating_add(optional_bytes(&value.submitted_at))
                    .saturating_add(optional_bytes(&value.commit_id))
                    .saturating_add(user_bytes(&value.user)?);
            }
            absent_fields.sort_unstable();
            Ok(GithubWireObservation {
                ids,
                numbers: Vec::new(),
                absent_fields,
                retained_bytes,
            })
        }
        GithubWireKindForTest::ReviewComments => {
            let values: Vec<ReviewComment> = decode(bytes)?;
            let mut ids = Vec::with_capacity(values.len());
            let mut absent_fields = Vec::new();
            let mut retained_bytes = 0_usize;
            for value in values {
                ids.push(require_positive(value.id, "id")?);
                if value.pull_request_review_id.is_none() {
                    absent_fields.push("pull_request_review_id");
                }
                if value.user.is_none() {
                    absent_fields.push("user");
                }
                retained_bytes = retained_bytes
                    .saturating_add(string_bytes([
                        value.body.as_str(),
                        value.path.as_str(),
                        value.diff_hunk.as_str(),
                        value.created_at.as_str(),
                        value.updated_at.as_str(),
                    ]))
                    .saturating_add(user_bytes(&value.user)?);
            }
            Ok(GithubWireObservation {
                ids,
                numbers: Vec::new(),
                absent_fields,
                retained_bytes,
            })
        }
        GithubWireKindForTest::DiffFiles => {
            let values: Vec<DiffFile> = decode(bytes)?;
            let mut absent_fields = Vec::new();
            let mut retained_bytes = 0_usize;
            for value in values {
                if value.patch.is_none() {
                    absent_fields.push("patch");
                }
                retained_bytes = retained_bytes
                    .saturating_add(string_bytes([
                        value.filename.as_str(),
                        value.status.as_str(),
                    ]))
                    .saturating_add(optional_bytes(&value.patch));
            }
            Ok(GithubWireObservation {
                ids: Vec::new(),
                numbers: Vec::new(),
                absent_fields,
                retained_bytes,
            })
        }
    }
}
