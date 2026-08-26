mod render;
mod wire;

use std::sync::Arc;

use async_trait::async_trait;
use resourcefs_core::{
    ErrorCategory, GithubRepositoryIdentity, IssueAddress, IssueResource, OperationGuard,
    PathReference, PullRequestAddress, PullRequestResource, ResourceAddress, ResourceError,
    SourceAdapter, SourceResource,
};
use serde::de::DeserializeOwned;
use url::Url;

use crate::{GithubConfig, HttpRequest, HttpSubstrate};
use wire::{ConversationComment, DiffFile, Issue, PullRequest, Review, ReviewComment, SimpleUser};

#[cfg(feature = "test-support")]
pub use wire::{GithubWireKindForTest, GithubWireObservation, inspect_github_wire_for_test};

#[cfg(feature = "test-support")]
pub fn render_issue_for_test(bytes: &[u8]) -> Result<String, ResourceError> {
    let issue: Issue = wire::decode(bytes)?;
    let repository =
        GithubRepositoryIdentity::parse("owner/repo").expect("hardcoded repository is valid");
    Ok(render::issue_aggregate(&repository, &issue, &mut []))
}
const GITHUB_JSON: &str = "application/vnd.github+json";
const GITHUB_DIFF: &str = "application/vnd.github.diff";
const GITHUB_API_VERSION: &str = "2022-11-28";
const USER_AGENT: &str = "resourcefs/1.0";

#[derive(Clone)]
pub struct GithubSource {
    config: GithubConfig,
    api_base: Url,
    substrate: Arc<HttpSubstrate>,
}

impl std::fmt::Debug for GithubSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GithubSource")
            .field("id", &self.config.id())
            .field("repositories", &self.config.repositories().len())
            .finish_non_exhaustive()
    }
}

impl GithubSource {
    pub fn new(config: GithubConfig, substrate: Arc<HttpSubstrate>) -> Result<Self, ResourceError> {
        let mut api_base = Url::parse(config.api_base_url()).map_err(|_| {
            ResourceError::new(
                ErrorCategory::InvalidReference,
                "GitHub API base URL is invalid",
            )
        })?;
        if !api_base.path().ends_with('/') {
            let mut path = api_base.path().to_owned();
            path.push('/');
            api_base.set_path(&path);
        }
        Ok(Self {
            config,
            api_base,
            substrate,
        })
    }

    fn authorize_repository(
        &self,
        identity: &GithubRepositoryIdentity,
    ) -> Result<(), ResourceError> {
        if self.config.repository(identity).is_none() {
            return Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "GitHub repository is not present in the readable allowlist",
            ));
        }
        Ok(())
    }

    fn endpoint(
        &self,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
    ) -> Result<Url, ResourceError> {
        self.api_base
            .join(&format!(
                "repos/{}/{}/{suffix}",
                repository.owner(),
                repository.repository()
            ))
            .map_err(|_| {
                ResourceError::new(
                    ErrorCategory::InvalidReference,
                    "GitHub API endpoint could not be constructed",
                )
            })
    }

    fn request(url: Url, accept: &str) -> Result<HttpRequest, ResourceError> {
        HttpRequest::get(url)
            .with_header("Accept", accept)
            .and_then(|request| request.with_header("User-Agent", USER_AGENT))
            .and_then(|request| request.with_header("X-GitHub-Api-Version", GITHUB_API_VERSION))
    }

    async fn fetch(
        &self,
        url: Url,
        accept: &str,
        operation: &OperationGuard,
    ) -> Result<crate::BoundedHttpResponse, ResourceError> {
        let response = self
            .substrate
            .fetch(Self::request(url, accept)?, operation)
            .await?;
        self.classify_status(&response)?;
        if response.truncated() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                "GitHub response exceeds the bounded HTTP body ceiling",
            ));
        }
        Ok(response)
    }

    fn classify_status(&self, response: &crate::BoundedHttpResponse) -> Result<(), ResourceError> {
        match response.status() {
            200..=299 => Ok(()),
            401 => Err(github_error(
                ErrorCategory::PermissionDenied,
                "GitHub rejected the configured credential",
            )),
            403 if response.rate_limit_remaining() == Some(0)
                || response.retry_after().is_some() =>
            {
                Err(github_error(
                    ErrorCategory::SourceUnavailable,
                    "GitHub rate limit is exhausted",
                ))
            }
            403 => Err(github_error(
                ErrorCategory::PermissionDenied,
                "GitHub denied the requested operation",
            )),
            404 => Err(github_error(
                ErrorCategory::NotFound,
                "GitHub Resource was not found or is access-hidden",
            )),
            429 => Err(github_error(
                ErrorCategory::SourceUnavailable,
                "GitHub rate limit is exhausted",
            )),
            500..=599 => Err(github_error(
                ErrorCategory::SourceUnavailable,
                "GitHub upstream is unavailable",
            )),
            status => Err(github_error(
                ErrorCategory::SourceUnavailable,
                &format!("GitHub upstream returned HTTP status {status}"),
            )),
        }
    }

    async fn json<T: DeserializeOwned>(
        &self,
        url: Url,
        operation: &OperationGuard,
    ) -> Result<T, ResourceError> {
        let response = self.fetch(url, GITHUB_JSON, operation).await?;
        wire::decode(response.body())
    }

    async fn issue_resource(
        &self,
        address: &IssueAddress,
        operation: &OperationGuard,
    ) -> Result<String, ResourceError> {
        let IssueAddress::Item {
            repository,
            number,
            resource,
        } = address
        else {
            return Err(unsupported_github_projection());
        };
        let number = number.get();
        match resource {
            IssueResource::Aggregate => {
                let issue: Issue = self
                    .json(
                        self.endpoint(repository, &format!("issues/{number}"))?,
                        operation,
                    )
                    .await?;
                let mut comments: Vec<ConversationComment> = self
                    .json(
                        self.endpoint(repository, &format!("issues/{number}/comments"))?,
                        operation,
                    )
                    .await?;
                validate_issue_identity(&issue, number)?;
                validate_comment_ids(&comments)?;
                Ok(render::issue_aggregate(repository, &issue, &mut comments))
            }
            IssueResource::Title | IssueResource::Body => {
                let issue: Issue = self
                    .json(
                        self.endpoint(repository, &format!("issues/{number}"))?,
                        operation,
                    )
                    .await?;
                validate_issue_identity(&issue, number)?;
                Ok(match resource {
                    IssueResource::Title => issue.title,
                    IssueResource::Body => match issue.body {
                        Some(body) => body,
                        None => String::new(),
                    },
                    _ => unreachable!("matched title/body"),
                })
            }
            IssueResource::Comments => {
                let mut comments: Vec<ConversationComment> = self
                    .json(
                        self.endpoint(repository, &format!("issues/{number}/comments"))?,
                        operation,
                    )
                    .await?;
                validate_comment_ids(&comments)?;
                comments.sort_by(|left, right| {
                    left.created_at
                        .cmp(&right.created_at)
                        .then_with(|| left.id.cmp(&right.id))
                });
                Ok(comments
                    .iter()
                    .map(|comment| {
                        format!(
                            "issue://{}/{number}/comments/{}\n{}",
                            repository.as_str(),
                            comment.id,
                            render::comment(comment)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"))
            }
            IssueResource::Comment(id) => {
                let comment: ConversationComment = self
                    .json(
                        self.endpoint(repository, &format!("issues/comments/{}", id.get()))?,
                        operation,
                    )
                    .await?;
                validate_positive(comment.id, "comment.id")?;
                validate_user(&comment.user, "comment.user.id")?;
                if comment.id != id.get() {
                    return Err(malformed_upstream(
                        "GitHub comment ID does not match its path",
                    ));
                }
                Ok(render::comment(&comment))
            }
        }
    }

    async fn pull_request_resource(
        &self,
        address: &PullRequestAddress,
        operation: &OperationGuard,
    ) -> Result<String, ResourceError> {
        let PullRequestAddress::Item {
            repository,
            number,
            resource,
        } = address
        else {
            return Err(unsupported_github_projection());
        };
        let number = number.get();
        match resource {
            PullRequestResource::Aggregate => {
                let pull: PullRequest = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}"))?,
                        operation,
                    )
                    .await?;
                let mut comments: Vec<ConversationComment> = self
                    .json(
                        self.endpoint(repository, &format!("issues/{number}/comments"))?,
                        operation,
                    )
                    .await?;
                let mut reviews: Vec<Review> = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}/reviews"))?,
                        operation,
                    )
                    .await?;
                let mut review_comments: Vec<ReviewComment> = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}/comments"))?,
                        operation,
                    )
                    .await?;
                let files: Vec<DiffFile> = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}/files"))?,
                        operation,
                    )
                    .await?;
                validate_pull_identity(&pull, number)?;
                validate_comment_ids(&comments)?;
                validate_review_ids(&reviews)?;
                validate_review_comment_ids(&review_comments)?;
                validate_files(&files)?;
                Ok(render::pull_request_aggregate(
                    repository,
                    &pull,
                    &mut comments,
                    &mut reviews,
                    &mut review_comments,
                    &files,
                ))
            }
            PullRequestResource::Title | PullRequestResource::Body => {
                let pull: PullRequest = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}"))?,
                        operation,
                    )
                    .await?;
                validate_pull_identity(&pull, number)?;
                Ok(match resource {
                    PullRequestResource::Title => pull.title,
                    PullRequestResource::Body => match pull.body {
                        Some(body) => body,
                        None => String::new(),
                    },
                    _ => unreachable!("matched title/body"),
                })
            }
            PullRequestResource::Comments => {
                let comments: Vec<ConversationComment> = self
                    .json(
                        self.endpoint(repository, &format!("issues/{number}/comments"))?,
                        operation,
                    )
                    .await?;
                validate_comment_ids(&comments)?;
                Ok(comments
                    .iter()
                    .map(render::comment)
                    .collect::<Vec<_>>()
                    .join("\n\n"))
            }
            PullRequestResource::Comment(id) => {
                let comment: ConversationComment = self
                    .json(
                        self.endpoint(repository, &format!("issues/comments/{}", id.get()))?,
                        operation,
                    )
                    .await?;
                validate_positive(comment.id, "comment.id")?;
                validate_user(&comment.user, "comment.user.id")?;
                if comment.id != id.get() {
                    return Err(malformed_upstream(
                        "GitHub comment ID does not match its path",
                    ));
                }
                Ok(render::comment(&comment))
            }
            PullRequestResource::Reviews => {
                let reviews: Vec<Review> = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}/reviews"))?,
                        operation,
                    )
                    .await?;
                validate_review_ids(&reviews)?;
                Ok(reviews
                    .iter()
                    .map(render::review)
                    .collect::<Vec<_>>()
                    .join("\n\n"))
            }
            PullRequestResource::Review(id) => {
                let review: Review = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}/reviews/{}", id.get()))?,
                        operation,
                    )
                    .await?;
                validate_positive(review.id, "review.id")?;
                validate_user(&review.user, "review.user.id")?;
                if review.id != id.get() {
                    return Err(malformed_upstream(
                        "GitHub review ID does not match its path",
                    ));
                }
                Ok(render::review(&review))
            }
            PullRequestResource::ReviewComments => {
                let comments: Vec<ReviewComment> = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}/comments"))?,
                        operation,
                    )
                    .await?;
                validate_review_comment_ids(&comments)?;
                Ok(comments
                    .iter()
                    .map(render::review_comment)
                    .collect::<Vec<_>>()
                    .join("\n\n"))
            }
            PullRequestResource::ReviewComment(id) => {
                let comment: ReviewComment = self
                    .json(
                        self.endpoint(repository, &format!("pulls/comments/{}", id.get()))?,
                        operation,
                    )
                    .await?;
                validate_positive(comment.id, "review_comment.id")?;
                validate_user(&comment.user, "review_comment.user.id")?;
                if comment.id != id.get() {
                    return Err(malformed_upstream(
                        "GitHub review comment ID does not match its path",
                    ));
                }
                Ok(render::review_comment(&comment))
            }
            PullRequestResource::Diff => {
                let response = self
                    .fetch(
                        self.endpoint(repository, &format!("pulls/{number}"))?,
                        GITHUB_DIFF,
                        operation,
                    )
                    .await?;
                String::from_utf8(response.body().to_vec())
                    .map_err(|_| malformed_upstream("GitHub unified diff is not valid UTF-8"))
            }
            PullRequestResource::DiffFile(index) => {
                let files: Vec<DiffFile> = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}/files"))?,
                        operation,
                    )
                    .await?;
                validate_files(&files)?;
                let offset = usize::try_from(index.get() - 1).map_err(|_| {
                    ResourceError::new(
                        ErrorCategory::NotFound,
                        "GitHub diff file index is outside the addressable range",
                    )
                })?;
                let file = files.get(offset).ok_or_else(|| {
                    ResourceError::new(
                        ErrorCategory::NotFound,
                        "GitHub diff file index was not found",
                    )
                })?;
                Ok(render::diff_file(file))
            }
        }
    }
}

#[async_trait]
impl SourceAdapter for GithubSource {
    async fn read(
        &self,
        reference: &PathReference,
        operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        if reference.projection().is_some() {
            return Err(unsupported_github_projection());
        }
        let (canonical, content) = match reference.address() {
            ResourceAddress::Issue(address) => {
                let repository = address
                    .repository()
                    .ok_or_else(unsupported_github_projection)?;
                self.authorize_repository(repository)?;
                (
                    PathReference::issue(address.clone(), None)?,
                    self.issue_resource(address, operation).await?,
                )
            }
            ResourceAddress::PullRequest(address) => {
                let repository = address
                    .repository()
                    .ok_or_else(unsupported_github_projection)?;
                self.authorize_repository(repository)?;
                (
                    PathReference::pull_request(address.clone(), None)?,
                    self.pull_request_resource(address, operation).await?,
                )
            }
            _ => return Err(unsupported_github_projection()),
        };
        SourceResource::text(canonical, content)
    }
}

fn validate_positive(value: u64, field: &str) -> Result<(), ResourceError> {
    if value == 0 {
        return Err(malformed_upstream(&format!(
            "GitHub upstream field '{field}' must be positive"
        )));
    }
    Ok(())
}

fn validate_user(user: &Option<SimpleUser>, field: &str) -> Result<(), ResourceError> {
    if let Some(user) = user {
        validate_positive(user.id, field)?;
        if user.login.is_empty() {
            return Err(malformed_upstream(&format!(
                "GitHub upstream field '{field}' requires a login"
            )));
        }
    }
    Ok(())
}

fn validate_issue_identity(issue: &Issue, expected: u64) -> Result<(), ResourceError> {
    validate_positive(issue.id, "issue.id")?;
    validate_positive(issue.number, "issue.number")?;
    validate_user(&issue.user, "issue.user.id")?;
    if issue.number != expected {
        return Err(malformed_upstream(
            "GitHub issue number does not match its Path Reference",
        ));
    }
    Ok(())
}

fn validate_pull_identity(pull: &PullRequest, expected: u64) -> Result<(), ResourceError> {
    validate_positive(pull.id, "pull_request.id")?;
    validate_positive(pull.number, "pull_request.number")?;
    validate_user(&pull.user, "pull_request.user.id")?;
    if pull.number != expected {
        return Err(malformed_upstream(
            "GitHub pull request number does not match its Path Reference",
        ));
    }
    Ok(())
}

fn validate_unique_ids(
    values: impl IntoIterator<Item = u64>,
    field: &str,
) -> Result<(), ResourceError> {
    let mut ids = std::collections::HashSet::new();
    for id in values {
        validate_positive(id, field)?;
        if !ids.insert(id) {
            return Err(malformed_upstream(&format!(
                "GitHub upstream field '{field}' contains a duplicate identity"
            )));
        }
    }
    Ok(())
}

fn validate_comment_ids(values: &[ConversationComment]) -> Result<(), ResourceError> {
    validate_unique_ids(values.iter().map(|value| value.id), "comment.id")?;
    for value in values {
        validate_user(&value.user, "comment.user.id")?;
    }
    Ok(())
}

fn validate_review_ids(values: &[Review]) -> Result<(), ResourceError> {
    validate_unique_ids(values.iter().map(|value| value.id), "review.id")?;
    for value in values {
        validate_user(&value.user, "review.user.id")?;
    }
    Ok(())
}

fn validate_review_comment_ids(values: &[ReviewComment]) -> Result<(), ResourceError> {
    validate_unique_ids(values.iter().map(|value| value.id), "review_comment.id")?;
    for value in values {
        validate_user(&value.user, "review_comment.user.id")?;
    }
    Ok(())
}

fn validate_files(values: &[DiffFile]) -> Result<(), ResourceError> {
    if values
        .iter()
        .any(|file| file.filename.is_empty() || file.status.is_empty())
    {
        return Err(malformed_upstream(
            "GitHub diff file requires filename and status",
        ));
    }
    Ok(())
}

fn unsupported_github_projection() -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        "GitHub reference does not name a supported read projection",
    )
}

fn malformed_upstream(message: &str) -> ResourceError {
    ResourceError::new(ErrorCategory::SourceUnavailable, message)
}

fn github_error(category: ErrorCategory, message: &str) -> ResourceError {
    ResourceError::new(category, message)
}
