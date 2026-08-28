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
                    .map_err(invalid_success_response)?;
                Ok(render::comment(&comment))
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
            .fetch(request, operation)
            .await
            .map_err(MutationCommitFailure::Unknown)?;
        classify_mutation_response(&response, 201, true)?;
        let canonical = match target {
            GithubCreationTarget::Issue { repository } => {
                let issue: Issue =
                    wire::decode(response.body()).map_err(MutationCommitFailure::Unknown)?;
                super::validate_issue_identity(&issue, issue.number)
                    .map_err(MutationCommitFailure::Unknown)?;
                if issue.pull_request.is_some() {
                    return Err(MutationCommitFailure::Unknown(malformed_upstream(
                        "GitHub issue creation response changed object kind",
                    )));
                }
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
            return Err(invalid_creation_document(format!(
                "frontmatter field '{key}' must not be duplicated"
            )));
        }
        let value = raw_value.trim();
        if value.is_empty() {
            return Err(invalid_creation_document(format!(
                "frontmatter field '{key}' must have a scalar value"
            )));
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
        return serde_json::from_str::<String>(value).map_err(|_| {
            invalid_creation_document("double-quoted frontmatter strings must use JSON escapes")
        });
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
        || value.starts_with(['[', '{', '&', '*', '!', '|', '>', '@', '`', '%'])
        || value.starts_with("- ")
        || value.starts_with("? ")
        || value.contains(" #")
        || value.contains(": ")
        || matches!(value, "null" | "Null" | "NULL" | "~" | "true" | "false")
    {
        return Err(invalid_creation_document(
            "frontmatter value uses unsupported YAML syntax; quote one-line strings",
        ));
    }
    Ok(value.to_owned())
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
    if let Some(key) = fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid_creation_document(format!(
            "frontmatter field '{key}' is not supported"
        )));
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
        match mutation {
            SourceMutation::Replace {
                target,
                expected: _,
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
                let authoritative = self.patch_field(&field, &content, operation).await?;
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
                    canonical_reference: canonical,
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
