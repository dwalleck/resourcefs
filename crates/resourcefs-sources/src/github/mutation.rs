use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Serialize;

use resourcefs_core::{
    ConversationCommentId, ErrorCategory, GithubRepositoryIdentity, IssueAddress, IssueNumber,
    IssueResource, MutationAccess, MutationAdapter, MutationCommitFailure, MutationCommitOutcome,
    MutationSourceKey, MutationState, MutationTarget, MutationTargetMode, OperationGuard,
    PathReference, PullRequestAddress, PullRequestNumber, PullRequestResource, ResourceAddress,
    ResourceError, SourceMutation, VersionTag,
};

use crate::{BoundedHttpResponse, HttpRequest, http::HttpFetchFailure};

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

    fn mutation_suffix(&self) -> String {
        match self {
            Self::IssueTitle { number, .. } | Self::IssueBody { number, .. } => {
                format!("issues/{number}")
            }
            Self::IssueComment { id, .. } | Self::PullComment { id, .. } => {
                format!("issues/comments/{id}")
            }
            Self::PullTitle { number, .. } | Self::PullBody { number, .. } => {
                format!("pulls/{number}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GithubCreationTarget {
    Issue {
        repository: GithubRepositoryIdentity,
    },
    IssueComment {
        repository: GithubRepositoryIdentity,
        number: u64,
    },
    Pull {
        repository: GithubRepositoryIdentity,
    },
    PullComment {
        repository: GithubRepositoryIdentity,
        number: u64,
    },
}

impl GithubCreationTarget {
    fn parse(reference: &PathReference) -> Result<Self, ResourceError> {
        if reference.projection().is_some() {
            return Err(invalid_creation_document(
                "Creation Target writes do not accept projection selectors",
            ));
        }
        match reference.address() {
            ResourceAddress::Issue(IssueAddress::New { repository }) => Ok(Self::Issue {
                repository: repository.clone(),
            }),
            ResourceAddress::Issue(IssueAddress::Item {
                repository,
                number,
                resource: IssueResource::CommentsNew,
            }) => Ok(Self::IssueComment {
                repository: repository.clone(),
                number: number.get(),
            }),
            ResourceAddress::PullRequest(PullRequestAddress::New { repository }) => {
                Ok(Self::Pull {
                    repository: repository.clone(),
                })
            }
            ResourceAddress::PullRequest(PullRequestAddress::Item {
                repository,
                number,
                resource: PullRequestResource::CommentsNew,
            }) => Ok(Self::PullComment {
                repository: repository.clone(),
                number: number.get(),
            }),
            _ => Err(unsupported_field_mutation()),
        }
    }

    fn repository(&self) -> &GithubRepositoryIdentity {
        match self {
            Self::Issue { repository }
            | Self::IssueComment { repository, .. }
            | Self::Pull { repository }
            | Self::PullComment { repository, .. } => repository,
        }
    }
    fn mutation_suffix(&self) -> String {
        match self {
            Self::Issue { .. } => "issues".to_owned(),
            Self::Pull { .. } => "pulls".to_owned(),
            Self::IssueComment { number, .. } => format!("issues/{number}/comments"),
            Self::PullComment { number, .. } => format!("issues/{number}/comments"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CreationSubmission {
    Issue {
        title: String,
        body: String,
    },
    IssueComment {
        body: String,
    },
    Pull {
        title: String,
        head: String,
        base: String,
        draft: Option<bool>,
        body: String,
    },
    PullComment {
        body: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FrontmatterValue {
    String(String),
    Boolean(bool),
}

#[derive(Serialize)]
struct TitleUpdate<'a> {
    title: &'a str,
}

#[derive(Serialize)]
struct BodyUpdate<'a> {
    body: &'a str,
}

#[derive(Serialize)]
struct IssueCreation<'a> {
    title: &'a str,
    body: &'a str,
}

#[derive(Serialize)]
struct PullCreation<'a> {
    title: &'a str,
    head: &'a str,
    base: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    draft: Option<bool>,
    body: &'a str,
}

impl GithubSource {
    pub(super) fn field_mutability(
        &self,
        reference: &PathReference,
    ) -> Result<bool, ResourceError> {
        match GithubFieldTarget::parse(reference) {
            Ok(target) => Ok(self
                .config
                .repository(target.repository())
                .is_some_and(|repository| repository.grants().update())),
            Err(error) if error.category() == ErrorCategory::UnsupportedMutation => Ok(false),
            Err(error) => Err(error),
        }
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

    async fn field_snapshot(
        &self,
        target: &GithubFieldTarget,
        operation: &OperationGuard,
    ) -> Result<(String, u64), MutationCommitFailure> {
        match target {
            GithubFieldTarget::IssueTitle { repository, number } => {
                let issue = self.mutation_issue(repository, *number, operation).await?;
                Ok((issue.title, issue.id))
            }
            GithubFieldTarget::IssueBody { repository, number } => {
                let issue = self.mutation_issue(repository, *number, operation).await?;
                Ok((authoritative_body(issue.body), issue.id))
            }
            GithubFieldTarget::IssueComment {
                repository,
                number,
                id,
            } => {
                self.mutation_issue(repository, *number, operation).await?;
                let comment = self
                    .mutation_comment(repository, *number, *id, operation)
                    .await?;
                Ok((render::comment(&comment), comment.id))
            }
            GithubFieldTarget::PullTitle { repository, number } => {
                let pull = self.mutation_pull(repository, *number, operation).await?;
                Ok((pull.title, pull.id))
            }
            GithubFieldTarget::PullBody { repository, number } => {
                let pull = self.mutation_pull(repository, *number, operation).await?;
                Ok((authoritative_body(pull.body), pull.id))
            }
            GithubFieldTarget::PullComment {
                repository,
                number,
                id,
            } => {
                self.mutation_pull(repository, *number, operation).await?;
                let comment = self
                    .mutation_comment(repository, *number, *id, operation)
                    .await?;
                Ok((render::comment(&comment), comment.id))
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
        validate_object_web_url(&issue.html_url, repository, "issues", number)
            .map_err(MutationCommitFailure::Conclusive)?;
        validate_required_wire_string(&issue.title, "issue.title")
            .map_err(MutationCommitFailure::Conclusive)?;
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
        validate_object_web_url(&pull.html_url, repository, "pull", number)
            .map_err(MutationCommitFailure::Conclusive)?;
        validate_required_wire_string(&pull.title, "pull.title")
            .map_err(MutationCommitFailure::Conclusive)?;
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
    ) -> Result<(String, u64), MutationCommitFailure> {
        let url = self.endpoint(target.repository(), &target.mutation_suffix())?;
        let body = match target {
            GithubFieldTarget::IssueTitle { .. } | GithubFieldTarget::PullTitle { .. } => {
                serde_json::to_vec(&TitleUpdate { title: content })
            }
            GithubFieldTarget::IssueBody { .. }
            | GithubFieldTarget::IssueComment { .. }
            | GithubFieldTarget::PullBody { .. }
            | GithubFieldTarget::PullComment { .. } => {
                serde_json::to_vec(&BodyUpdate { body: content })
            }
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
            .fetch_attempt(request, operation, self.substrate.ceilings().fetch_bytes())
            .await
            .map_err(map_http_mutation_failure)?;
        classify_mutation_response(&response, 200, true)?;
        self.validate_patch_response(target, response.body())
    }

    fn validate_patch_response(
        &self,
        target: &GithubFieldTarget,
        body: &[u8],
    ) -> Result<(String, u64), MutationCommitFailure> {
        match target {
            GithubFieldTarget::IssueTitle { repository, number }
            | GithubFieldTarget::IssueBody { repository, number } => {
                let issue: Issue = wire::decode(body).map_err(MutationCommitFailure::Unknown)?;
                super::validate_issue_identity(&issue, *number)
                    .map_err(MutationCommitFailure::Unknown)?;
                if issue.pull_request.is_some() {
                    return Err(MutationCommitFailure::Unknown(malformed_upstream(
                        "GitHub issue mutation response changed object kind",
                    )));
                }
                validate_object_web_url(&issue.html_url, repository, "issues", *number)
                    .map_err(invalid_success_response)?;
                let content = match target {
                    GithubFieldTarget::IssueTitle { .. } => {
                        require_non_blank_success(&issue.title, "issue.title")?;
                        issue.title
                    }
                    GithubFieldTarget::IssueBody { .. } => authoritative_body(issue.body),
                    _ => unreachable!("issue response target"),
                };
                Ok((content, issue.id))
            }
            GithubFieldTarget::PullTitle { repository, number }
            | GithubFieldTarget::PullBody { repository, number } => {
                let pull: PullRequest =
                    wire::decode(body).map_err(MutationCommitFailure::Unknown)?;
                super::validate_pull_identity(&pull, *number)
                    .map_err(MutationCommitFailure::Unknown)?;
                validate_object_web_url(&pull.html_url, repository, "pull", *number)
                    .map_err(invalid_success_response)?;
                let content = match target {
                    GithubFieldTarget::PullTitle { .. } => {
                        require_non_blank_success(&pull.title, "pull.title")?;
                        pull.title
                    }
                    GithubFieldTarget::PullBody { .. } => authoritative_body(pull.body),
                    _ => unreachable!("pull response target"),
                };
                Ok((content, pull.id))
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
                    .map_err(invalid_success_response)?;
                Ok((render::comment(&comment), comment.id))
            }
        }
    }

    fn authorize_creation(
        &self,
        target: &GithubCreationTarget,
        access: MutationAccess,
    ) -> Result<(), ResourceError> {
        let repository = self.config.repository(target.repository()).ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::PermissionDenied,
                "GitHub repository is not present in the readable allowlist",
            )
        })?;
        match access {
            MutationAccess::Create if repository.grants().create() => Ok(()),
            MutationAccess::Create => Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "GitHub repository create grant is required for Creation Targets",
            )),
            MutationAccess::Update | MutationAccess::Delete => Err(ResourceError::new(
                ErrorCategory::UnsupportedMutation,
                "GitHub Creation Targets accept create only",
            )),
        }
    }

    async fn create_target(
        &self,
        target: &GithubCreationTarget,
        submission: &CreationSubmission,
        operation: &OperationGuard,
    ) -> Result<PathReference, MutationCommitFailure> {
        match target {
            GithubCreationTarget::IssueComment { repository, number } => {
                self.mutation_issue(repository, *number, operation).await?;
            }
            GithubCreationTarget::PullComment { repository, number } => {
                self.mutation_pull(repository, *number, operation).await?;
            }
            GithubCreationTarget::Issue { .. } | GithubCreationTarget::Pull { .. } => {}
        }
        let url = self.endpoint(target.repository(), &target.mutation_suffix())?;
        let encoded = match (target, submission) {
            (GithubCreationTarget::Issue { .. }, CreationSubmission::Issue { title, body }) => {
                serde_json::to_vec(&IssueCreation { title, body })
            }
            (
                GithubCreationTarget::Pull { .. },
                CreationSubmission::Pull {
                    title,
                    head,
                    base,
                    draft,
                    body,
                },
            ) => serde_json::to_vec(&PullCreation {
                title,
                head,
                base,
                draft: *draft,
                body,
            }),
            (
                GithubCreationTarget::IssueComment { .. },
                CreationSubmission::IssueComment { body },
            )
            | (
                GithubCreationTarget::PullComment { .. },
                CreationSubmission::PullComment { body },
            ) => serde_json::to_vec(&BodyUpdate { body }),
            _ => {
                return Err(MutationCommitFailure::Conclusive(ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "GitHub Creation Target submission kind is inconsistent",
                )));
            }
        };
        let encoded = encoded.map_err(|error| {
            MutationCommitFailure::Conclusive(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("GitHub creation request JSON could not be encoded: {error}"),
            ))
        })?;
        let request = Self::request_with_github_headers(HttpRequest::post_json(url, encoded)?)?;
        let response = self
            .substrate
            .fetch_attempt(request, operation, self.substrate.ceilings().fetch_bytes())
            .await
            .map_err(map_http_mutation_failure)?;
        classify_mutation_response(&response, 201, true)?;
        let canonical = match target {
            GithubCreationTarget::Issue { repository } => {
                let issue: Issue =
                    wire::decode(response.body()).map_err(MutationCommitFailure::Unknown)?;
                super::validate_issue_identity(&issue, issue.number)
                    .map_err(MutationCommitFailure::Unknown)?;
                validate_object_web_url(&issue.html_url, repository, "issues", issue.number)
                    .map_err(invalid_success_response)?;
                if issue.pull_request.is_some() {
                    return Err(MutationCommitFailure::Unknown(malformed_upstream(
                        "GitHub issue creation response changed object kind",
                    )));
                }
                require_non_blank_success(&issue.title, "issue.title")?;
                PathReference::issue(
                    IssueAddress::Item {
                        repository: repository.clone(),
                        number: IssueNumber::new(issue.number)
                            .map_err(MutationCommitFailure::Unknown)?,
                        resource: IssueResource::Aggregate,
                    },
                    None,
                )
                .map_err(MutationCommitFailure::Unknown)?
            }
            GithubCreationTarget::Pull { repository } => {
                let pull: PullRequest =
                    wire::decode(response.body()).map_err(MutationCommitFailure::Unknown)?;
                super::validate_pull_identity(&pull, pull.number)
                    .map_err(MutationCommitFailure::Unknown)?;
                validate_object_web_url(&pull.html_url, repository, "pull", pull.number)
                    .map_err(invalid_success_response)?;
                require_non_blank_success(&pull.title, "pull.title")?;
                PathReference::pull_request(
                    PullRequestAddress::Item {
                        repository: repository.clone(),
                        number: PullRequestNumber::new(pull.number)
                            .map_err(MutationCommitFailure::Unknown)?,
                        resource: PullRequestResource::Aggregate,
                    },
                    None,
                )
                .map_err(MutationCommitFailure::Unknown)?
            }
            GithubCreationTarget::IssueComment { repository, number }
            | GithubCreationTarget::PullComment { repository, number } => {
                let comment: ConversationComment =
                    wire::decode(response.body()).map_err(MutationCommitFailure::Unknown)?;
                super::validate_positive(comment.id, "comment.id")
                    .map_err(MutationCommitFailure::Unknown)?;
                super::validate_user(&comment.user, "comment.user.id")
                    .map_err(MutationCommitFailure::Unknown)?;
                super::validate_parent(&comment.issue_url, repository, "issues", *number)
                    .map_err(invalid_success_response)?;
                let resource = IssueResource::Comment(
                    ConversationCommentId::new(comment.id)
                        .map_err(MutationCommitFailure::Unknown)?,
                );
                if matches!(target, GithubCreationTarget::IssueComment { .. }) {
                    PathReference::issue(
                        IssueAddress::Item {
                            repository: repository.clone(),
                            number: IssueNumber::new(*number)
                                .map_err(MutationCommitFailure::Unknown)?,
                            resource,
                        },
                        None,
                    )
                    .map_err(MutationCommitFailure::Unknown)?
                } else {
                    PathReference::pull_request(
                        PullRequestAddress::Item {
                            repository: repository.clone(),
                            number: PullRequestNumber::new(*number)
                                .map_err(MutationCommitFailure::Unknown)?,
                            resource: PullRequestResource::Comment(
                                ConversationCommentId::new(comment.id)
                                    .map_err(MutationCommitFailure::Unknown)?,
                            ),
                        },
                        None,
                    )
                    .map_err(MutationCommitFailure::Unknown)?
                }
            }
        };
        self.session
            .cache_remove_namespace(GITHUB_CACHE_NAMESPACE)
            .await
            .map_err(MutationCommitFailure::Unknown)?;
        Ok(canonical)
    }
}

fn parse_creation_submission(
    target: &GithubCreationTarget,
    content: &str,
) -> Result<CreationSubmission, ResourceError> {
    match target {
        GithubCreationTarget::IssueComment { .. } => {
            require_non_blank(content, "conversation comment")?;
            Ok(CreationSubmission::IssueComment {
                body: content.to_owned(),
            })
        }
        GithubCreationTarget::PullComment { .. } => {
            require_non_blank(content, "conversation comment")?;
            Ok(CreationSubmission::PullComment {
                body: content.to_owned(),
            })
        }
        GithubCreationTarget::Issue { .. } => {
            let (mut fields, body) = parse_frontmatter(content)?;
            reject_unknown_fields(&fields, &["title"])?;
            let title = take_string(&mut fields, "title")?;
            require_non_blank(&title, "issue title")?;
            Ok(CreationSubmission::Issue {
                title,
                body: body.to_owned(),
            })
        }
        GithubCreationTarget::Pull { .. } => {
            let (mut fields, body) = parse_frontmatter(content)?;
            reject_unknown_fields(&fields, &["title", "head", "base", "draft"])?;
            let title = take_string(&mut fields, "title")?;
            let head = take_string(&mut fields, "head")?;
            let base = take_string(&mut fields, "base")?;
            require_non_blank(&title, "pull request title")?;
            require_non_blank(&head, "pull request head")?;
            require_non_blank(&base, "pull request base")?;
            let draft = match fields.remove("draft") {
                None => None,
                Some(FrontmatterValue::Boolean(value)) => Some(value),
                Some(FrontmatterValue::String(_)) => {
                    return Err(invalid_creation_document(
                        "pull request frontmatter field 'draft' must be true or false",
                    ));
                }
            };
            Ok(CreationSubmission::Pull {
                title,
                head,
                base,
                draft,
                body: body.to_owned(),
            })
        }
    }
}

fn parse_frontmatter(
    content: &str,
) -> Result<(BTreeMap<String, FrontmatterValue>, &str), ResourceError> {
    let opening = if content.starts_with("---\r\n") {
        5
    } else if content.starts_with("---\n") {
        4
    } else {
        return Err(invalid_creation_document(
            "creation document must begin with an exact '---' delimiter line",
        ));
    };
    let mut offset = opening;
    let (frontmatter, body) = loop {
        let remaining = &content[offset..];
        let newline = remaining.find('\n');
        let (line_end, next) = match newline {
            Some(relative) => (offset + relative, offset + relative + 1),
            None => (content.len(), content.len()),
        };
        let line = content[offset..line_end]
            .strip_suffix('\r')
            .unwrap_or(&content[offset..line_end]);
        if line == "---" {
            break (&content[opening..offset], &content[next..]);
        }
        if newline.is_none() {
            return Err(invalid_creation_document(
                "creation document frontmatter is missing its closing '---' delimiter",
            ));
        }
        offset = next;
    };
    let mut fields = BTreeMap::new();
    for raw_line in frontmatter.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        let (key, raw_value) = line.split_once(':').ok_or_else(|| {
            invalid_creation_document("frontmatter lines must use one 'key: value' scalar")
        })?;
        if key.is_empty()
            || key.trim() != key
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            return Err(invalid_creation_document(
                "frontmatter keys must be lowercase ASCII without whitespace",
            ));
        }
        if fields.contains_key(key) {
            return Err(invalid_creation_document(
                "frontmatter fields must not be duplicated",
            ));
        }
        let value = raw_value.trim();
        if value.is_empty() {
            return Err(invalid_creation_document(
                "frontmatter field must have a scalar value",
            ));
        }
        let parsed = if key == "draft" {
            match value {
                "true" => FrontmatterValue::Boolean(true),
                "false" => FrontmatterValue::Boolean(false),
                _ => FrontmatterValue::String(parse_string_scalar(value)?),
            }
        } else {
            FrontmatterValue::String(parse_string_scalar(value)?)
        };
        fields.insert(key.to_owned(), parsed);
    }
    Ok((fields, body))
}

fn parse_string_scalar(value: &str) -> Result<String, ResourceError> {
    if value.starts_with('"') {
        let decoded = serde_json::from_str::<String>(value).map_err(|_| {
            invalid_creation_document("double-quoted frontmatter strings must use JSON escapes")
        })?;
        if decoded.contains(['\n', '\r']) {
            return Err(invalid_creation_document(
                "frontmatter strings must remain on one logical line",
            ));
        }
        return Ok(decoded);
    }
    if value.starts_with('\'') {
        if !value.ends_with('\'') || value.len() < 2 {
            return Err(invalid_creation_document(
                "single-quoted frontmatter string is not closed",
            ));
        }
        let inner = &value[1..value.len() - 1];
        let mut output = String::new();
        let mut chars = inner.chars().peekable();
        while let Some(character) = chars.next() {
            if character == '\'' {
                if chars.next_if_eq(&'\'').is_none() {
                    return Err(invalid_creation_document(
                        "single quote inside a quoted scalar must be doubled",
                    ));
                }
                output.push('\'');
            } else {
                output.push(character);
            }
        }
        return Ok(output);
    }
    if value.ends_with('"')
        || value.ends_with('\'')
        || value.starts_with(['[', '{', '&', '*', '!', '|', '>', '@', '`', '%', '#'])
        || value.starts_with("- ")
        || value.starts_with("? ")
        || matches!(value, "-" | "?" | ":" | "..." | "---")
        || has_yaml_comment(value)
        || value.contains(": ")
        || matches!(value, "null" | "Null" | "NULL" | "~" | "true" | "false")
        || looks_numeric_yaml_scalar(value)
    {
        return Err(invalid_creation_document(
            "frontmatter value uses unsupported YAML syntax; quote one-line strings",
        ));
    }
    Ok(value.to_owned())
}

fn has_yaml_comment(value: &str) -> bool {
    let mut previous = None;
    for character in value.chars() {
        if character == '#' && previous.is_none_or(char::is_whitespace) {
            return true;
        }
        previous = Some(character);
    }
    false
}

fn looks_numeric_yaml_scalar(value: &str) -> bool {
    let normalized = value.replace('_', "");
    let unsigned = normalized
        .strip_prefix(['+', '-'])
        .unwrap_or(normalized.as_str());
    let lower = unsigned.to_ascii_lowercase();
    lower.starts_with("0x")
        || lower.starts_with("0o")
        || matches!(lower.as_str(), ".inf" | ".nan")
        || normalized.parse::<i128>().is_ok()
        || normalized.parse::<f64>().is_ok()
}

fn take_string(
    fields: &mut BTreeMap<String, FrontmatterValue>,
    key: &str,
) -> Result<String, ResourceError> {
    match fields.remove(key) {
        Some(FrontmatterValue::String(value)) => Ok(value),
        Some(FrontmatterValue::Boolean(_)) => Err(invalid_creation_document(format!(
            "frontmatter field '{key}' must be a string"
        ))),
        None => Err(invalid_creation_document(format!(
            "frontmatter field '{key}' is required"
        ))),
    }
}

fn reject_unknown_fields(
    fields: &BTreeMap<String, FrontmatterValue>,
    allowed: &[&str],
) -> Result<(), ResourceError> {
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid_creation_document(
            "frontmatter contains an unsupported field",
        ));
    }
    Ok(())
}

fn require_non_blank(value: &str, label: &str) -> Result<(), ResourceError> {
    if value.trim().is_empty() {
        return Err(invalid_creation_document(format!(
            "{label} must not be blank"
        )));
    }
    Ok(())
}

fn invalid_creation_document(message: impl Into<String>) -> ResourceError {
    ResourceError::new(ErrorCategory::InvalidReference, message)
}

#[async_trait]
impl MutationAdapter for GithubSource {
    async fn resolve(
        &self,
        reference: &PathReference,
        access: MutationAccess,
    ) -> Result<MutationTarget, ResourceError> {
        let source_key = MutationSourceKey::new(GITHUB_MUTATION_SOURCE_KEY)?;
        if reference.is_creation_target() {
            let target = GithubCreationTarget::parse(reference)?;
            self.authorize_creation(&target, access)?;
            MutationTarget::new(
                reference.clone(),
                source_key,
                MutationTargetMode::CreationTarget,
            )
        } else {
            let target = GithubFieldTarget::parse(reference)?;
            self.authorize_field_update(&target, access)?;
            MutationTarget::new(
                reference.clone(),
                source_key,
                MutationTargetMode::AuthoritativeText,
            )
        }
    }

    fn validate_write(&self, target: &MutationTarget, content: &str) -> Result<(), ResourceError> {
        match target.mode() {
            MutationTargetMode::AuthoritativeText => {
                let field = GithubFieldTarget::parse(target.canonical_reference())?;
                Self::validate_replacement(&field, content)
            }
            MutationTargetMode::CreationTarget => {
                let creation = GithubCreationTarget::parse(target.canonical_reference())?;
                parse_creation_submission(&creation, content).map(|_| ())
            }
            MutationTargetMode::AuthoredText => Err(ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "GitHub Source Adapter received an authored-text target mode",
            )),
        }
    }

    async fn load(
        &self,
        target: &MutationTarget,
        _access: MutationAccess,
        operation: &OperationGuard,
    ) -> Result<MutationState, ResourceError> {
        if matches!(target.mode(), MutationTargetMode::CreationTarget) {
            return Ok(MutationState::Missing);
        }
        let target = GithubFieldTarget::parse(target.canonical_reference())?;
        let (content, _) = self
            .field_snapshot(&target, operation)
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
        match mutation {
            SourceMutation::Replace {
                target,
                expected,
                content,
            } => {
                if target.mode() != MutationTargetMode::AuthoritativeText {
                    return Err(MutationCommitFailure::Conclusive(ResourceError::new(
                        ErrorCategory::SourceUnavailable,
                        "GitHub Field mutation target mode is invalid",
                    )));
                }
                let field = GithubFieldTarget::parse(target.canonical_reference())?;
                Self::validate_replacement(&field, &content)?;
                let (current, object_id) = self.field_snapshot(&field, operation).await?;
                if VersionTag::from_content(current.as_bytes()) != expected {
                    return Err(MutationCommitFailure::Conclusive(ResourceError::new(
                        ErrorCategory::VersionConflict,
                        "replacement Version Tag no longer matches authoritative GitHub content",
                    )));
                }
                let (authoritative, response_id) =
                    self.patch_field(&field, &content, operation).await?;
                if response_id != object_id {
                    return Err(MutationCommitFailure::Unknown(malformed_upstream(
                        "GitHub mutation response object ID changed after authoritative preflight",
                    )));
                }
                self.session
                    .cache_remove_namespace(GITHUB_CACHE_NAMESPACE)
                    .await
                    .map_err(MutationCommitFailure::Unknown)?;
                Ok(MutationCommitOutcome::AuthoritativeText {
                    version_tag: VersionTag::from_content(authoritative.as_bytes()),
                })
            }
            SourceMutation::Create { target, content } => {
                if target.mode() != MutationTargetMode::CreationTarget {
                    return Err(MutationCommitFailure::Conclusive(ResourceError::new(
                        ErrorCategory::SourceUnavailable,
                        "GitHub Creation Target mode is invalid",
                    )));
                }
                let creation = GithubCreationTarget::parse(target.canonical_reference())?;
                let submission = parse_creation_submission(&creation, &content)?;
                let canonical = self
                    .create_target(&creation, &submission, operation)
                    .await?;
                Ok(MutationCommitOutcome::CreationTarget {
                    canonical_reference: Box::new(canonical),
                })
            }
            SourceMutation::Delete { .. } | SourceMutation::Move { .. } => Err(
                MutationCommitFailure::Conclusive(unsupported_field_mutation()),
            ),
        }
    }
}
#[cfg(feature = "test-support")]
pub fn inspect_github_mutation_route_for_test(
    reference: &str,
) -> Result<(String, String), ResourceError> {
    let reference = PathReference::parse(reference)?;
    if reference.is_creation_target() {
        let target = GithubCreationTarget::parse(&reference)?;
        Ok(("POST".to_owned(), target.mutation_suffix()))
    } else {
        let target = GithubFieldTarget::parse(&reference)?;
        Ok(("PATCH".to_owned(), target.mutation_suffix()))
    }
}
fn map_http_mutation_failure(failure: HttpFetchFailure) -> MutationCommitFailure {
    let unknown = failure.is_unknown_outcome();
    let error = failure.into_error();
    if unknown {
        MutationCommitFailure::Unknown(error)
    } else {
        MutationCommitFailure::Conclusive(error)
    }
}

fn validate_object_web_url(
    value: &str,
    repository: &GithubRepositoryIdentity,
    family: &str,
    number: u64,
) -> Result<(), ResourceError> {
    let url =
        url::Url::parse(value).map_err(|_| malformed_upstream("GitHub object URL is malformed"))?;
    if url.scheme() != "https" {
        return Err(malformed_upstream("GitHub object URL must use HTTPS"));
    }
    let segments = url
        .path_segments()
        .ok_or_else(|| malformed_upstream("GitHub object URL has no hierarchical path"))?
        .collect::<Vec<_>>();
    let [owner, repo, actual_family, actual_number] = segments
        .as_slice()
        .split_at(segments.len().saturating_sub(4))
        .1
    else {
        return Err(malformed_upstream(
            "GitHub object URL does not name a repository object",
        ));
    };
    let actual_number = actual_number
        .parse::<u64>()
        .map_err(|_| malformed_upstream("GitHub object URL number is invalid"))?;
    if !owner.eq_ignore_ascii_case(repository.owner())
        || !repo.eq_ignore_ascii_case(repository.repository())
        || *actual_family != family
        || actual_number != number
    {
        return Err(malformed_upstream(
            "GitHub object URL does not match its requested repository identity",
        ));
    }
    Ok(())
}

fn validate_required_wire_string(value: &str, field: &str) -> Result<(), ResourceError> {
    if value.trim().is_empty() {
        return Err(malformed_upstream(&format!(
            "GitHub upstream field '{field}' must not be blank"
        )));
    }
    Ok(())
}

fn require_non_blank_success(value: &str, field: &str) -> Result<(), MutationCommitFailure> {
    validate_required_wire_string(value, field).map_err(invalid_success_response)
}

fn invalid_success_response(error: ResourceError) -> MutationCommitFailure {
    MutationCommitFailure::Unknown(ResourceError::new(
        ErrorCategory::SourceUnavailable,
        format!(
            "GitHub mutation success response is invalid: {}",
            error.message()
        ),
    ))
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
