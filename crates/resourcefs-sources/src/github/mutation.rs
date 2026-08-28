use async_trait::async_trait;
use serde::Serialize;

use resourcefs_core::{
    ErrorCategory, GithubRepositoryIdentity, IssueAddress, IssueResource, MutationAccess,
    MutationAdapter, MutationCommitFailure, MutationCommitOutcome, MutationSourceKey,
    MutationState, MutationTarget, MutationTargetMode, OperationGuard, PathReference,
    PullRequestAddress, PullRequestResource, ResourceAddress, ResourceError, SourceMutation,
    VersionTag,
};

use crate::{BoundedHttpResponse, HttpRequest};

use super::{
    GITHUB_API_VERSION, GITHUB_CACHE_NAMESPACE, GITHUB_JSON, GithubSource, USER_AGENT,
    authoritative_body, malformed_upstream, render,
    wire::{self, ConversationComment, Issue, PullRequest},
};

pub(crate) const GITHUB_MUTATION_SOURCE_KEY: &str = "github";
#[derive(Debug, Clone, PartialEq, Eq)]
enum GithubFieldTarget {
    IssueTitle {
        repository: GithubRepositoryIdentity,
        number: u64,
    },
    IssueBody {
        repository: GithubRepositoryIdentity,
        number: u64,
    },
    IssueComment {
        repository: GithubRepositoryIdentity,
        number: u64,
        id: u64,
    },
    PullTitle {
        repository: GithubRepositoryIdentity,
        number: u64,
    },
    PullBody {
        repository: GithubRepositoryIdentity,
        number: u64,
    },
    PullComment {
        repository: GithubRepositoryIdentity,
        number: u64,
        id: u64,
    },
}

impl GithubFieldTarget {
    fn parse(reference: &PathReference) -> Result<Self, ResourceError> {
        if reference.projection().is_some() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "GitHub Field mutation requires a canonical reference without a selector",
            ));
        }
        match reference.address() {
            ResourceAddress::Issue(IssueAddress::Item {
                repository,
                number,
                resource,
            }) => match resource {
                IssueResource::Title => Ok(Self::IssueTitle {
                    repository: repository.clone(),
                    number: number.get(),
                }),
                IssueResource::Body => Ok(Self::IssueBody {
                    repository: repository.clone(),
                    number: number.get(),
                }),
                IssueResource::Comment(id) => Ok(Self::IssueComment {
                    repository: repository.clone(),
                    number: number.get(),
                    id: id.get(),
                }),
                IssueResource::Aggregate | IssueResource::Comments | IssueResource::CommentsNew => {
                    Err(unsupported_field_mutation())
                }
            },
            ResourceAddress::PullRequest(PullRequestAddress::Item {
                repository,
                number,
                resource,
            }) => match resource {
                PullRequestResource::Title => Ok(Self::PullTitle {
                    repository: repository.clone(),
                    number: number.get(),
                }),
                PullRequestResource::Body => Ok(Self::PullBody {
                    repository: repository.clone(),
                    number: number.get(),
                }),
                PullRequestResource::Comment(id) => Ok(Self::PullComment {
                    repository: repository.clone(),
                    number: number.get(),
                    id: id.get(),
                }),
                PullRequestResource::Aggregate
                | PullRequestResource::Comments
                | PullRequestResource::CommentsNew
                | PullRequestResource::Reviews
                | PullRequestResource::Review(_)
                | PullRequestResource::ReviewComments
                | PullRequestResource::ReviewComment(_)
                | PullRequestResource::Diff
                | PullRequestResource::DiffFile(_) => Err(unsupported_field_mutation()),
            },
            ResourceAddress::Issue(_) | ResourceAddress::PullRequest(_) => {
                Err(unsupported_field_mutation())
            }
            _ => Err(ResourceError::new(
                ErrorCategory::UnsupportedMutation,
                "GitHub mutation accepts only issue:// or pr:// Field references",
            )),
        }
    }

    fn repository(&self) -> &GithubRepositoryIdentity {
        match self {
            Self::IssueTitle { repository, .. }
            | Self::IssueBody { repository, .. }
            | Self::IssueComment { repository, .. }
            | Self::PullTitle { repository, .. }
            | Self::PullBody { repository, .. }
            | Self::PullComment { repository, .. } => repository,
        }
    }
}

#[derive(Serialize)]
struct TitleUpdate<'a> {
    title: &'a str,
}

#[derive(Serialize)]
struct BodyUpdate<'a> {
    body: &'a str,
}

impl GithubSource {
    pub(super) fn field_is_mutable(&self, reference: &PathReference) -> bool {
        GithubFieldTarget::parse(reference)
            .ok()
            .and_then(|target| self.config.repository(target.repository()))
            .is_some_and(|repository| repository.grants().update())
    }

    fn authorize_field_update(
        &self,
        target: &GithubFieldTarget,
        access: MutationAccess,
    ) -> Result<(), ResourceError> {
        let repository = self.config.repository(target.repository()).ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::PermissionDenied,
                "GitHub repository is not present in the readable allowlist",
            )
        })?;
        match access {
            MutationAccess::Update if repository.grants().update() => Ok(()),
            MutationAccess::Update => Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "GitHub repository update grant is required for Field replacement",
            )),
            MutationAccess::Create => Err(ResourceError::new(
                ErrorCategory::VersionConflict,
                "GitHub Field replacement requires ifVersion",
            )),
            MutationAccess::Delete => Err(ResourceError::new(
                ErrorCategory::UnsupportedMutation,
                "GitHub Field deletion is not supported",
            )),
        }
    }

    fn request_with_github_headers(request: HttpRequest) -> Result<HttpRequest, ResourceError> {
        request
            .with_header("Accept", GITHUB_JSON)?
            .with_header("User-Agent", USER_AGENT)?
            .with_header("X-GitHub-Api-Version", GITHUB_API_VERSION)
    }

    async fn uncached_json<T: serde::de::DeserializeOwned>(
        &self,
        url: url::Url,
        operation: &OperationGuard,
    ) -> Result<T, MutationCommitFailure> {
        let request = Self::request_with_github_headers(HttpRequest::get(url))?;
        let response = self
            .substrate
            .fetch(request, operation)
            .await
            .map_err(MutationCommitFailure::Conclusive)?;
        classify_mutation_response(&response, 200, false)?;
        wire::decode(response.body()).map_err(MutationCommitFailure::Conclusive)
    }

    async fn field_content(
        &self,
        target: &GithubFieldTarget,
        operation: &OperationGuard,
    ) -> Result<String, MutationCommitFailure> {
        match target {
            GithubFieldTarget::IssueTitle { repository, number } => Ok(self
                .mutation_issue(repository, *number, operation)
                .await?
                .title),
            GithubFieldTarget::IssueBody { repository, number } => Ok(authoritative_body(
                self.mutation_issue(repository, *number, operation)
                    .await?
                    .body,
            )),
            GithubFieldTarget::IssueComment {
                repository,
                number,
                id,
            } => {
                self.mutation_issue(repository, *number, operation).await?;
                let comment = self
                    .mutation_comment(repository, *number, *id, operation)
                    .await?;
                Ok(render::comment(&comment))
            }
            GithubFieldTarget::PullTitle { repository, number } => Ok(self
                .mutation_pull(repository, *number, operation)
                .await?
                .title),
            GithubFieldTarget::PullBody { repository, number } => Ok(authoritative_body(
                self.mutation_pull(repository, *number, operation)
                    .await?
                    .body,
            )),
            GithubFieldTarget::PullComment {
                repository,
                number,
                id,
            } => {
                self.mutation_pull(repository, *number, operation).await?;
                let comment = self
                    .mutation_comment(repository, *number, *id, operation)
                    .await?;
                Ok(render::comment(&comment))
            }
        }
    }

    async fn mutation_issue(
        &self,
        repository: &GithubRepositoryIdentity,
        number: u64,
        operation: &OperationGuard,
    ) -> Result<Issue, MutationCommitFailure> {
        let issue: Issue = self
            .uncached_json(
                self.endpoint(repository, &format!("issues/{number}"))?,
                operation,
            )
            .await?;
        super::validate_issue_identity(&issue, number)
            .map_err(MutationCommitFailure::Conclusive)?;
        if issue.pull_request.is_some() {
            return Err(MutationCommitFailure::Conclusive(ResourceError::new(
                ErrorCategory::NotFound,
                "GitHub number names a pull request; address it through pr://",
            )));
        }
        Ok(issue)
    }

    async fn mutation_pull(
        &self,
        repository: &GithubRepositoryIdentity,
        number: u64,
        operation: &OperationGuard,
    ) -> Result<PullRequest, MutationCommitFailure> {
        let pull: PullRequest = self
            .uncached_json(
                self.endpoint(repository, &format!("pulls/{number}"))?,
                operation,
            )
            .await?;
        super::validate_pull_identity(&pull, number).map_err(MutationCommitFailure::Conclusive)?;
        Ok(pull)
    }

    async fn mutation_comment(
        &self,
        repository: &GithubRepositoryIdentity,
        number: u64,
        id: u64,
        operation: &OperationGuard,
    ) -> Result<ConversationComment, MutationCommitFailure> {
        let comment: ConversationComment = self
            .uncached_json(
                self.endpoint(repository, &format!("issues/comments/{id}"))?,
                operation,
            )
            .await?;
        super::validate_positive(comment.id, "comment.id")
            .map_err(MutationCommitFailure::Conclusive)?;
        super::validate_user(&comment.user, "comment.user.id")
            .map_err(MutationCommitFailure::Conclusive)?;
        if comment.id != id {
            return Err(MutationCommitFailure::Conclusive(malformed_upstream(
                "GitHub comment ID does not match its path",
            )));
        }
        super::validate_parent(&comment.issue_url, repository, "issues", number)
            .map_err(MutationCommitFailure::Conclusive)?;
        Ok(comment)
    }

    fn validate_replacement(
        target: &GithubFieldTarget,
        content: &str,
    ) -> Result<(), ResourceError> {
        let blank_forbidden = matches!(
            target,
            GithubFieldTarget::IssueTitle { .. }
                | GithubFieldTarget::IssueComment { .. }
                | GithubFieldTarget::PullTitle { .. }
                | GithubFieldTarget::PullComment { .. }
        );
        if blank_forbidden && content.trim().is_empty() {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "GitHub titles and conversation comments must not be blank",
            ));
        }
        Ok(())
    }

    async fn patch_field(
        &self,
        target: &GithubFieldTarget,
        content: &str,
        operation: &OperationGuard,
    ) -> Result<String, MutationCommitFailure> {
        let (url, body) = match target {
            GithubFieldTarget::IssueTitle { repository, number } => (
                self.endpoint(repository, &format!("issues/{number}"))?,
                serde_json::to_vec(&TitleUpdate { title: content }),
            ),
            GithubFieldTarget::IssueBody { repository, number } => (
                self.endpoint(repository, &format!("issues/{number}"))?,
                serde_json::to_vec(&BodyUpdate { body: content }),
            ),
            GithubFieldTarget::IssueComment { repository, id, .. }
            | GithubFieldTarget::PullComment { repository, id, .. } => (
                self.endpoint(repository, &format!("issues/comments/{id}"))?,
                serde_json::to_vec(&BodyUpdate { body: content }),
            ),
            GithubFieldTarget::PullTitle { repository, number } => (
                self.endpoint(repository, &format!("pulls/{number}"))?,
                serde_json::to_vec(&TitleUpdate { title: content }),
            ),
            GithubFieldTarget::PullBody { repository, number } => (
                self.endpoint(repository, &format!("pulls/{number}"))?,
                serde_json::to_vec(&BodyUpdate { body: content }),
            ),
        };
        let body = body.map_err(|error| {
            MutationCommitFailure::Conclusive(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("GitHub mutation request JSON could not be encoded: {error}"),
            ))
        })?;
        let request = Self::request_with_github_headers(HttpRequest::patch_json(url, body)?)?;
        let response = self
            .substrate
            .fetch(request, operation)
            .await
            .map_err(MutationCommitFailure::Unknown)?;
        classify_mutation_response(&response, 200, true)?;
        self.validate_patch_response(target, response.body())
    }

    fn validate_patch_response(
        &self,
        target: &GithubFieldTarget,
        body: &[u8],
    ) -> Result<String, MutationCommitFailure> {
        match target {
            GithubFieldTarget::IssueTitle { number, .. }
            | GithubFieldTarget::IssueBody { number, .. } => {
                let issue: Issue = wire::decode(body).map_err(MutationCommitFailure::Unknown)?;
                super::validate_issue_identity(&issue, *number)
                    .map_err(MutationCommitFailure::Unknown)?;
                if issue.pull_request.is_some() {
                    return Err(MutationCommitFailure::Unknown(malformed_upstream(
                        "GitHub issue mutation response changed object kind",
                    )));
                }
                Ok(match target {
                    GithubFieldTarget::IssueTitle { .. } => issue.title,
                    GithubFieldTarget::IssueBody { .. } => authoritative_body(issue.body),
                    _ => unreachable!("issue response target"),
                })
            }
            GithubFieldTarget::PullTitle { number, .. }
            | GithubFieldTarget::PullBody { number, .. } => {
                let pull: PullRequest =
                    wire::decode(body).map_err(MutationCommitFailure::Unknown)?;
                super::validate_pull_identity(&pull, *number)
                    .map_err(MutationCommitFailure::Unknown)?;
                Ok(match target {
                    GithubFieldTarget::PullTitle { .. } => pull.title,
                    GithubFieldTarget::PullBody { .. } => authoritative_body(pull.body),
                    _ => unreachable!("pull response target"),
                })
            }
            GithubFieldTarget::IssueComment {
                repository,
                number,
                id,
            }
            | GithubFieldTarget::PullComment {
                repository,
                number,
                id,
            } => {
                let comment: ConversationComment =
                    wire::decode(body).map_err(MutationCommitFailure::Unknown)?;
                super::validate_positive(comment.id, "comment.id")
                    .map_err(MutationCommitFailure::Unknown)?;
                super::validate_user(&comment.user, "comment.user.id")
                    .map_err(MutationCommitFailure::Unknown)?;
                if comment.id != *id {
                    return Err(MutationCommitFailure::Unknown(malformed_upstream(
                        "GitHub comment mutation response ID does not match its path",
                    )));
                }
                super::validate_parent(&comment.issue_url, repository, "issues", *number)
                    .map_err(MutationCommitFailure::Unknown)?;
                Ok(render::comment(&comment))
            }
        }
    }
}

#[async_trait]
impl MutationAdapter for GithubSource {
    async fn resolve(
        &self,
        reference: &PathReference,
        access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError> {
        let target = GithubFieldTarget::parse(reference)?;
        self.authorize_field_update(&target, access)?;
        MutationTarget::new(
            reference.clone(),
            MutationSourceKey::new(GITHUB_MUTATION_SOURCE_KEY)?,
            MutationTargetMode::AuthoritativeText,
        )
    }

    fn validate_write(&self, target: &MutationTarget, content: &str) -> Result<(), ResourceError> {
        let field = GithubFieldTarget::parse(target.canonical_reference())?;
        Self::validate_replacement(&field, content)
    }

    async fn load(
        &self,
        target: &MutationTarget,
        _access: MutationAccess,
        operation: &OperationGuard,
    ) -> Result<MutationState, ResourceError> {
        let target = GithubFieldTarget::parse(target.canonical_reference())?;
        let content = self
            .field_content(&target, operation)
            .await
            .map_err(MutationCommitFailure::into_error)?;
        Ok(MutationState::Text {
            version_tag: VersionTag::from_content(content.as_bytes()),
            content,
        })
    }

    async fn commit(
        &self,
        mutation: SourceMutation,
        operation: &OperationGuard,
    ) -> Result<MutationCommitOutcome, MutationCommitFailure> {
        if !operation.is_committing() {
            return Err(MutationCommitFailure::Conclusive(ResourceError::new(
                ErrorCategory::Cancelled,
                "GitHub mutation commit requires a committing operation guard",
            )));
        }
        let SourceMutation::Replace {
            target,
            expected: _,
            content,
        } = mutation
        else {
            return Err(MutationCommitFailure::Conclusive(
                unsupported_field_mutation(),
            ));
        };
        if target.mode() != MutationTargetMode::AuthoritativeText {
            return Err(MutationCommitFailure::Conclusive(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "GitHub Field mutation target mode is invalid",
            )));
        }
        let field = GithubFieldTarget::parse(target.canonical_reference())?;
        Self::validate_replacement(&field, &content)?;
        let authoritative = self.patch_field(&field, &content, operation).await?;
        self.session
            .cache_remove_namespace(GITHUB_CACHE_NAMESPACE)
            .await
            .map_err(MutationCommitFailure::Unknown)?;
        Ok(MutationCommitOutcome::AuthoritativeText {
            version_tag: VersionTag::from_content(authoritative.as_bytes()),
        })
    }
}

fn classify_mutation_response(
    response: &BoundedHttpResponse,
    expected_success: u16,
    mutation_sent: bool,
) -> Result<(), MutationCommitFailure> {
    let status = response.status();
    if status == expected_success {
        if response.truncated() {
            let error = ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "GitHub mutation response exceeds the bounded HTTP body ceiling; reconcile upstream state",
            );
            return Err(if mutation_sent {
                MutationCommitFailure::Unknown(error)
            } else {
                MutationCommitFailure::Conclusive(error)
            });
        }
        return Ok(());
    }
    let error = match status {
        200..=299 => ResourceError::new(
            ErrorCategory::SourceUnavailable,
            format!("GitHub mutation returned unexpected success status {status}"),
        ),
        300..=399 => ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "GitHub mutation redirect was not followed",
        ),
        400 | 422 => ResourceError::new(
            ErrorCategory::InvalidReference,
            "GitHub rejected the mutation input",
        ),
        401 => ResourceError::new(
            ErrorCategory::PermissionDenied,
            "GitHub rejected the configured credential",
        ),
        403 if response.rate_limit_remaining() == Some(0) || response.retry_after().is_some() => {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "GitHub rate limit is exhausted",
            )
        }
        403 => ResourceError::new(
            ErrorCategory::PermissionDenied,
            "GitHub denied the requested mutation",
        ),
        404 | 410 => ResourceError::new(
            ErrorCategory::NotFound,
            "GitHub mutation target was not found or is access-hidden",
        ),
        409 => ResourceError::new(
            ErrorCategory::VersionConflict,
            "GitHub rejected the mutation because upstream state conflicts",
        ),
        429 => ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "GitHub rate limit is exhausted",
        ),
        500..=599 => ResourceError::new(
            ErrorCategory::SourceUnavailable,
            "GitHub mutation outcome is unknown after an upstream server failure; reconcile upstream state",
        ),
        _ => ResourceError::new(
            ErrorCategory::SourceUnavailable,
            format!("GitHub mutation returned HTTP status {status}"),
        ),
    };
    if mutation_sent && (status >= 500 || (200..=299).contains(&status)) {
        Err(MutationCommitFailure::Unknown(error))
    } else {
        Err(MutationCommitFailure::Conclusive(error))
    }
}

fn unsupported_field_mutation() -> ResourceError {
    ResourceError::new(
        ErrorCategory::UnsupportedMutation,
        "GitHub mutation supports only existing title, body, and stable conversation-comment Fields",
    )
}
