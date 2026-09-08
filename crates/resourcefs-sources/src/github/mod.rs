mod facts;
mod fetch;
mod mutation;
pub(crate) use mutation::GITHUB_MUTATION_SOURCE_KEY;
#[cfg(feature = "test-support")]
pub use mutation::inspect_github_mutation_route_for_test;
mod render;
mod wire;

use std::{collections::HashSet, fmt::Write as _, io::Cursor, sync::Arc};

use async_trait::async_trait;
use resourcefs_core::{
    DiscoveryAdapter, ErrorCategory, GithubRepositoryIdentity, GlobOptions, GlobTarget,
    IssueAddress, IssueResource, OperationGuard, PathReference, PathSession, ProjectionSelector,
    PullRequestAddress, PullRequestResource, ResourceAddress, ResourceError, SearchOptions,
    SearchSourceResult, SearchTarget, SourceAdapter, SourceGlobResult, SourceResource, select_utf8,
};
use url::Url;

use crate::{
    GithubConfig, HttpSubstrate,
    catalog::{SourceCatalogEntry, SourceCatalogMetadata},
    http::BoundedRead,
    pattern::search_document,
};
use wire::{
    ConversationComment, DiffFile, Issue, PullRequest, PullRequestSummary, Review, ReviewComment,
    SimpleUser,
};
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
/// Objects requested per upstream page.
const PAGE_SIZE: u64 = 100;
/// Query for repository collections: every state, most recently updated first.
const COLLECTION_QUERY: &[(&str, &str)] =
    &[("state", "all"), ("sort", "updated"), ("direction", "desc")];

/// A rendered projection plus the typed continuation naming what it omits.
///
/// Only one logical collection carries a structured continuation. An
/// Aggregate composes several collections that may each have further pages;
/// it names every one of them as a `Continuation:` line in its text — the
/// same line every paginated rendering carries, so the reference survives at
/// the end of a retained artifact — but it has no single next page of its own.
struct Rendered {
    content: String,
    continuation: Option<PathReference>,
}

impl Rendered {
    const fn complete(content: String) -> Self {
        Self {
            content,
            continuation: None,
        }
    }

    fn collection(mut content: String, next: Option<PathReference>) -> Self {
        if let Some(reference) = &next {
            write_continuation_line(&mut content, reference);
        }
        Self {
            content,
            continuation: next,
        }
    }

    fn aggregate(mut content: String, continuations: &[PathReference]) -> Self {
        for reference in continuations {
            write_continuation_line(&mut content, reference);
        }
        Self::complete(content)
    }
}

fn write_continuation_line(content: &mut String, reference: &PathReference) {
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str("\nContinuation: ");
    content.push_str(reference.requested());
    content.push('\n');
}

fn page_reference(base: &str, page: u64) -> Result<PathReference, ResourceError> {
    PathReference::parse(format!("{base}:page:{page}"))
}

/// Renders one entry per item, each led by its own canonical reference so a
/// listing row can be read or searched on its own.
fn listing<T>(
    items: &[T],
    reference: impl Fn(&T) -> String,
    render: impl Fn(&T) -> String,
) -> String {
    items
        .iter()
        .map(|item| format!("{}\n{}", reference(item), render(item)))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[derive(Clone)]
pub struct GithubSource {
    config: GithubConfig,
    api_base: Url,
    substrate: Arc<HttpSubstrate>,
    session: PathSession,
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

#[derive(Debug, Clone)]
pub struct GithubSourceMount {
    config: GithubConfig,
    substrate: Arc<HttpSubstrate>,
}

impl GithubSourceMount {
    pub fn new(config: GithubConfig, substrate: Arc<HttpSubstrate>) -> Self {
        Self { config, substrate }
    }

    pub fn bind(self, session: PathSession) -> Result<GithubSource, ResourceError> {
        GithubSource::new(self.config, self.substrate, session)
    }
}

impl GithubSource {
    pub fn new(
        config: GithubConfig,
        substrate: Arc<HttpSubstrate>,
        session: PathSession,
    ) -> Result<Self, ResourceError> {
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
            session,
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

    /// Fetches issue `number` and refuses a pull request wearing an issue
    /// number: GitHub serves pull requests through the issues endpoint too,
    /// and one object must never own two canonical namespaces.
    async fn issue(
        &self,
        repository: &GithubRepositoryIdentity,
        number: u64,
        operation: BoundedRead<'_>,
    ) -> Result<Issue, ResourceError> {
        let issue: Issue = self
            .json(
                self.endpoint(repository, &format!("issues/{number}"))?,
                operation,
            )
            .await?;
        validate_issue_identity(&issue, number)?;
        if issue.pull_request.is_some() {
            return Err(ResourceError::new(
                ErrorCategory::NotFound,
                "GitHub number names a pull request; address it through pr://",
            ));
        }
        Ok(issue)
    }

    /// Fetches pull request `number`; the pulls endpoint itself refuses an
    /// issue number, so the kind gate is the request.
    async fn pull(
        &self,
        repository: &GithubRepositoryIdentity,
        number: u64,
        operation: BoundedRead<'_>,
    ) -> Result<PullRequest, ResourceError> {
        let pull: PullRequest = self
            .json(
                self.endpoint(repository, &format!("pulls/{number}"))?,
                operation,
            )
            .await?;
        validate_pull_identity(&pull, number)?;
        Ok(pull)
    }

    /// Fetches conversation comment `id` and refuses it unless its `issue_url`
    /// names `number` in this repository: the endpoint is repository-wide, so
    /// nothing else ties a comment to the path it was addressed under.
    async fn conversation_comment(
        &self,
        repository: &GithubRepositoryIdentity,
        number: u64,
        id: u64,
        operation: BoundedRead<'_>,
    ) -> Result<ConversationComment, ResourceError> {
        let comment: ConversationComment = self
            .json(
                self.endpoint(repository, &format!("issues/comments/{id}"))?,
                operation,
            )
            .await?;
        validate_positive(comment.id, "comment.id")?;
        validate_user(&comment.user, "comment.user.id")?;
        if comment.id != id {
            return Err(malformed_upstream(
                "GitHub comment ID does not match its path",
            ));
        }
        validate_parent(&comment.issue_url, repository, "issues", number)?;
        Ok(comment)
    }

    /// Fetches inline review comment `id`, refused unless its
    /// `pull_request_url` names `number` in this repository.
    async fn review_comment(
        &self,
        repository: &GithubRepositoryIdentity,
        number: u64,
        id: u64,
        operation: BoundedRead<'_>,
    ) -> Result<ReviewComment, ResourceError> {
        let comment: ReviewComment = self
            .json(
                self.endpoint(repository, &format!("pulls/comments/{id}"))?,
                operation,
            )
            .await?;
        validate_positive(comment.id, "review_comment.id")?;
        validate_user(&comment.user, "review_comment.user.id")?;
        if comment.id != id {
            return Err(malformed_upstream(
                "GitHub review comment ID does not match its path",
            ));
        }
        validate_parent(&comment.pull_request_url, repository, "pulls", number)?;
        Ok(comment)
    }

    async fn issue_resource(
        &self,
        address: &IssueAddress,
        page: Option<u64>,
        operation: BoundedRead<'_>,
    ) -> Result<Rendered, ResourceError> {
        if let IssueAddress::Collection { repository } = address {
            let (mut issues, next): (Vec<Issue>, Option<u64>) = self
                .collection(
                    repository,
                    "issues",
                    COLLECTION_QUERY,
                    page.unwrap_or(1),
                    operation,
                )
                .await?;
            validate_issue_listing(&issues)?;
            issues.retain(|issue| issue.pull_request.is_none());
            issues.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.number.cmp(&right.number))
            });
            let base = format!("issue://{}", repository.as_str());
            let mut rendered = format!("# Issues: {}\n", repository.as_str());
            if issues.is_empty() {
                // Every row of a page can be a pull request, which this
                // collection excludes; "(no issues)" would then contradict the
                // continuation that follows it.
                rendered.push_str(if next.is_some() {
                    "\n(no issues on this page)\n"
                } else {
                    "\n(no issues)\n"
                });
            } else {
                for issue in &issues {
                    writeln!(
                        &mut rendered,
                        "- {base}/{} — {} [{}] by {} (updated {})",
                        issue.number,
                        issue.title,
                        issue.state,
                        render::author(&issue.user),
                        issue.updated_at
                    )
                    .expect("String write");
                }
            }
            let next = next.map(|page| page_reference(&base, page)).transpose()?;
            return Ok(Rendered::collection(rendered, next));
        }
        let IssueAddress::Item {
            repository,
            number,
            resource,
        } = address
        else {
            return Err(unsupported_github_projection());
        };
        if page.is_some() && !matches!(resource, IssueResource::Comments) {
            return Err(unsupported_github_projection());
        }
        let number = number.get();
        let base = format!("issue://{}/{number}", repository.as_str());
        match resource {
            IssueResource::Aggregate => {
                let issue = self.issue(repository, number, operation).await?;
                let (mut comments, next): (Vec<ConversationComment>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("issues/{number}/comments"),
                        &[],
                        1,
                        operation,
                    )
                    .await?;
                validate_comment_ids(&comments)?;
                let rendered = render::issue_aggregate(repository, &issue, &mut comments);
                let continuations = next
                    .map(|page| page_reference(&format!("{base}/comments"), page))
                    .transpose()?;
                Ok(Rendered::aggregate(rendered, continuations.as_slice()))
            }
            IssueResource::Title => Ok(Rendered::complete(
                self.issue(repository, number, operation).await?.title,
            )),
            IssueResource::Body => Ok(Rendered::complete(authoritative_body(
                self.issue(repository, number, operation).await?.body,
            ))),
            IssueResource::Comments => {
                // The kind gate costs one request, but without it a pull
                // request's conversation would render under issue://.
                self.issue(repository, number, operation).await?;
                let (mut comments, next): (Vec<ConversationComment>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("issues/{number}/comments"),
                        &[],
                        page.unwrap_or(1),
                        operation,
                    )
                    .await?;
                validate_comment_ids(&comments)?;
                render::sort_comments(&mut comments);
                let rendered = listing(
                    &comments,
                    |comment| format!("{base}/comments/{}", comment.id),
                    render::comment,
                );
                let next = next
                    .map(|page| page_reference(&format!("{base}/comments"), page))
                    .transpose()?;
                Ok(Rendered::collection(rendered, next))
            }
            IssueResource::CommentsNew => Err(unsupported_github_projection()),
            IssueResource::Comment(id) => {
                self.issue(repository, number, operation).await?;
                let comment = self
                    .conversation_comment(repository, number, id.get(), operation)
                    .await?;
                Ok(Rendered::complete(render::comment(&comment)))
            }
        }
    }

    async fn pull_request_resource(
        &self,
        address: &PullRequestAddress,
        page: Option<u64>,
        operation: BoundedRead<'_>,
    ) -> Result<Rendered, ResourceError> {
        if let PullRequestAddress::Collection { repository } = address {
            let (mut pulls, next): (Vec<PullRequestSummary>, Option<u64>) = self
                .collection(
                    repository,
                    "pulls",
                    COLLECTION_QUERY,
                    page.unwrap_or(1),
                    operation,
                )
                .await?;
            validate_pull_summaries(&pulls)?;
            pulls.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.number.cmp(&right.number))
            });
            let base = format!("pr://{}", repository.as_str());
            let mut rendered = format!("# Pull requests: {}\n", repository.as_str());
            if pulls.is_empty() {
                rendered.push_str(if next.is_some() {
                    "\n(no pull requests on this page)\n"
                } else {
                    "\n(no pull requests)\n"
                });
            } else {
                for pull in &pulls {
                    let merge = if pull.merged_at.is_some() {
                        "merged"
                    } else {
                        pull.state.as_str()
                    };
                    writeln!(
                        &mut rendered,
                        "- {base}/{} — {} [{}{}] by {} (updated {}; {})",
                        pull.number,
                        pull.title,
                        merge,
                        if pull.draft { ", draft" } else { "" },
                        render::author(&pull.user),
                        pull.updated_at,
                        pull.html_url
                    )
                    .expect("String write");
                }
            }
            let next = next.map(|page| page_reference(&base, page)).transpose()?;
            return Ok(Rendered::collection(rendered, next));
        }
        let PullRequestAddress::Item {
            repository,
            number,
            resource,
        } = address
        else {
            return Err(unsupported_github_projection());
        };
        if page.is_some()
            && !matches!(
                resource,
                PullRequestResource::Comments
                    | PullRequestResource::Reviews
                    | PullRequestResource::ReviewComments
                    | PullRequestResource::Diff
            )
        {
            return Err(unsupported_github_projection());
        }
        let number = number.get();
        let base = format!("pr://{}/{number}", repository.as_str());
        match resource {
            PullRequestResource::Facts => Err(unsupported_github_projection()),
            PullRequestResource::Aggregate => {
                let pull = self.pull(repository, number, operation).await?;
                let (mut comments, comment_next): (Vec<ConversationComment>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("issues/{number}/comments"),
                        &[],
                        1,
                        operation,
                    )
                    .await?;
                let (mut reviews, review_next): (Vec<Review>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("pulls/{number}/reviews"),
                        &[],
                        1,
                        operation,
                    )
                    .await?;
                let (mut review_comments, review_comment_next): (Vec<ReviewComment>, Option<u64>) =
                    self.collection(
                        repository,
                        &format!("pulls/{number}/comments"),
                        &[],
                        1,
                        operation,
                    )
                    .await?;
                let (files, file_next): (Vec<DiffFile>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("pulls/{number}/files"),
                        &[],
                        1,
                        operation,
                    )
                    .await?;
                validate_comment_ids(&comments)?;
                validate_review_ids(&reviews)?;
                validate_review_comment_ids(&review_comments)?;
                validate_files(&files)?;
                let rendered = render::pull_request_aggregate(
                    repository,
                    &pull,
                    &mut comments,
                    &mut reviews,
                    &mut review_comments,
                    &files,
                );
                let mut continuations = Vec::new();
                for (collection, next) in [
                    ("comments", comment_next),
                    ("reviews", review_next),
                    ("review-comments", review_comment_next),
                    ("diff", file_next),
                ] {
                    if let Some(page) = next {
                        continuations.push(page_reference(&format!("{base}/{collection}"), page)?);
                    }
                }
                Ok(Rendered::aggregate(rendered, &continuations))
            }
            PullRequestResource::Title => Ok(Rendered::complete(
                self.pull(repository, number, operation).await?.title,
            )),
            PullRequestResource::Body => Ok(Rendered::complete(authoritative_body(
                self.pull(repository, number, operation).await?.body,
            ))),
            PullRequestResource::Comments => {
                // `issues/{number}/comments` serves plain issues too; the pulls
                // endpoint is the kind gate that keeps an issue out of pr://.
                self.pull(repository, number, operation).await?;
                let (mut comments, next): (Vec<ConversationComment>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("issues/{number}/comments"),
                        &[],
                        page.unwrap_or(1),
                        operation,
                    )
                    .await?;
                validate_comment_ids(&comments)?;
                render::sort_comments(&mut comments);
                let rendered = listing(
                    &comments,
                    |comment| format!("{base}/comments/{}", comment.id),
                    render::comment,
                );
                let next = next
                    .map(|page| page_reference(&format!("{base}/comments"), page))
                    .transpose()?;
                Ok(Rendered::collection(rendered, next))
            }
            PullRequestResource::CommentsNew => Err(unsupported_github_projection()),
            PullRequestResource::Comment(id) => {
                self.pull(repository, number, operation).await?;
                let comment = self
                    .conversation_comment(repository, number, id.get(), operation)
                    .await?;
                Ok(Rendered::complete(render::comment(&comment)))
            }
            PullRequestResource::Reviews => {
                let (mut reviews, next): (Vec<Review>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("pulls/{number}/reviews"),
                        &[],
                        page.unwrap_or(1),
                        operation,
                    )
                    .await?;
                validate_review_ids(&reviews)?;
                render::sort_reviews(&mut reviews);
                let rendered = listing(
                    &reviews,
                    |review| format!("{base}/reviews/{}", review.id),
                    render::review,
                );
                let next = next
                    .map(|page| page_reference(&format!("{base}/reviews"), page))
                    .transpose()?;
                Ok(Rendered::collection(rendered, next))
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
                Ok(Rendered::complete(render::review(&review)))
            }
            PullRequestResource::ReviewComments => {
                let (mut comments, next): (Vec<ReviewComment>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("pulls/{number}/comments"),
                        &[],
                        page.unwrap_or(1),
                        operation,
                    )
                    .await?;
                validate_review_comment_ids(&comments)?;
                render::sort_review_comments(&mut comments);
                let rendered = listing(
                    &comments,
                    |comment| format!("{base}/review-comments/{}", comment.id),
                    render::review_comment,
                );
                let next = next
                    .map(|page| page_reference(&format!("{base}/review-comments"), page))
                    .transpose()?;
                Ok(Rendered::collection(rendered, next))
            }
            PullRequestResource::ReviewComment(id) => {
                let comment = self
                    .review_comment(repository, number, id.get(), operation)
                    .await?;
                Ok(Rendered::complete(render::review_comment(&comment)))
            }
            PullRequestResource::Diff => {
                let Some(page) = page else {
                    let response = self
                        .fetch(
                            self.endpoint(repository, &format!("pulls/{number}"))?,
                            GITHUB_DIFF,
                            operation,
                        )
                        .await?;
                    let diff = String::from_utf8(response.body().to_vec()).map_err(|_| {
                        malformed_upstream("GitHub unified diff is not valid UTF-8")
                    })?;
                    return Ok(Rendered::complete(diff));
                };
                let (files, next): (Vec<DiffFile>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("pulls/{number}/files"),
                        &[],
                        page,
                        operation,
                    )
                    .await?;
                validate_files(&files)?;
                let base_index = page
                    .checked_sub(1)
                    .and_then(|page| page.checked_mul(PAGE_SIZE))
                    .ok_or_else(|| {
                        ResourceError::new(
                            ErrorCategory::LimitExceeded,
                            "GitHub diff page index overflowed",
                        )
                    })?;
                let rendered = files
                    .iter()
                    .enumerate()
                    .map(|(offset, file)| {
                        format!(
                            "{base}/diff/{} — {} ({})",
                            base_index + offset as u64 + 1,
                            file.filename,
                            file.status
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let next = next
                    .map(|page| page_reference(&format!("{base}/diff"), page))
                    .transpose()?;
                Ok(Rendered::collection(rendered, next))
            }
            PullRequestResource::DiffFile(index) => {
                // Diff files are numbered across the same 100-per-page listing
                // the aggregate and `/diff:page:<n>` advertise, so the index
                // resolves on the page that lists it — never on the endpoint's
                // default page size, which stops at 30.
                let zero_based = index.get().saturating_sub(1);
                let page = zero_based / PAGE_SIZE + 1;
                let offset = usize::try_from(zero_based % PAGE_SIZE).map_err(|_| {
                    ResourceError::new(
                        ErrorCategory::NotFound,
                        "GitHub diff file index is outside the addressable range",
                    )
                })?;
                let files: Vec<DiffFile> = self
                    .json(
                        self.page_url(repository, &format!("pulls/{number}/files"), &[], page)?,
                        operation,
                    )
                    .await?;
                validate_files(&files)?;
                let file = files.get(offset).ok_or_else(|| {
                    ResourceError::new(
                        ErrorCategory::NotFound,
                        "GitHub diff file index was not found",
                    )
                })?;
                Ok(Rendered::complete(render::diff_file(file)))
            }
        }
    }

    async fn read_resource(
        &self,
        reference: &PathReference,
        operation: BoundedRead<'_>,
    ) -> Result<SourceResource, ResourceError> {
        if reference.is_creation_target() {
            return Err(unsupported_github_projection());
        }
        let projection = reference.projection();
        let page = projection.and_then(ProjectionSelector::page_offset);
        let (canonical, rendered) = match reference.address() {
            ResourceAddress::Issue(address) => {
                let repository = address
                    .repository()
                    .ok_or_else(unsupported_github_projection)?;
                self.authorize_repository(repository)?;
                (
                    PathReference::issue(address.clone(), None)?,
                    self.issue_resource(address, page, operation).await?,
                )
            }
            ResourceAddress::PullRequest(address) => {
                let repository = address
                    .repository()
                    .ok_or_else(unsupported_github_projection)?;
                self.authorize_repository(repository)?;
                (
                    PathReference::pull_request(address.clone(), None)?,
                    self.pull_request_resource(address, page, operation).await?,
                )
            }
            _ => return Err(unsupported_github_projection()),
        };
        let mutable = self.field_mutability(&canonical)?;
        let Rendered {
            content,
            continuation,
        } = rendered;
        let resource =
            if projection.is_some_and(|projection| projection.line_selection().is_some()) {
                let selected = select_utf8(Cursor::new(content), projection)?;
                SourceResource::selected_text(canonical, selected)?
            } else {
                SourceResource::text(canonical, content)?
            }
            .with_mutability(mutable);
        Ok(match continuation {
            Some(next) => resource.with_continuation(&next),
            None => resource,
        })
    }
}

#[async_trait]
impl SourceAdapter for GithubSource {
    async fn read(
        &self,
        reference: &PathReference,
        operation: &OperationGuard,
        acquisition: Option<&resourcefs_core::ReadAcquisitionLimits>,
    ) -> Result<SourceResource, ResourceError> {
        if matches!(
            reference.address(),
            ResourceAddress::PullRequest(PullRequestAddress::Item {
                resource: PullRequestResource::Facts,
                ..
            })
        ) {
            return self.read_facts(reference, operation, acquisition).await;
        }
        if acquisition.is_some() {
            return Err(ResourceError::new(
                ErrorCategory::UnsupportedProjection,
                "acquisition controls are not supported for this resource",
            )
            .with_details(resourcefs_core::ResourceErrorDetails::new(
                resourcefs_core::ErrorReason::AcquisitionControlsUnsupported,
            )));
        }
        let timeout = self.substrate.ceilings().timeout();
        let operation = self.substrate.begin_read(operation)?;
        tokio::time::timeout(timeout, self.read_resource(reference, operation))
            .await
            .map_err(|_| {
                ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "GitHub operation exceeded the configured logical deadline",
                )
            })?
    }
}

impl SourceCatalogMetadata for GithubSource {
    fn catalog_entries(&self) -> Result<Vec<SourceCatalogEntry>, ResourceError> {
        Ok(vec![
            SourceCatalogEntry::new(
                "issue://",
                "issue://<owner>/<repository>[/<number>[/title|body|comments/<id>]][:selector]",
                "issue://owner/repository/42",
                None,
            )?,
            SourceCatalogEntry::new(
                "pr://",
                "pr://<owner>/<repository>[/<number>[/title|body|facts|comments|reviews|review-comments|diff]][:selector]",
                "pr://owner/repository/42",
                None,
            )?,
        ])
    }
}

#[async_trait]
impl DiscoveryAdapter for GithubSource {
    async fn search(
        &self,
        target: &SearchTarget,
        pattern: &str,
        options: SearchOptions,
        operation: &OperationGuard,
    ) -> Result<SearchSourceResult, ResourceError> {
        let reference = target
            .reference()
            .ok_or_else(unsupported_github_projection)?;
        // A search covers the whole rendered Resource — or the whole typed
        // upstream page — never a line selection, so a hit's line number is
        // the line an unselected read of the record's reference displays. The
        // page spelling stays: page 2 is a different set of objects, not an
        // offset into page 1, and only its own reference reproduces the hit.
        let page = reference
            .projection()
            .filter(|projection| projection.page_offset().is_some())
            .cloned();
        let searched = match reference.address() {
            ResourceAddress::Issue(address) => PathReference::issue(address.clone(), page)?,
            ResourceAddress::PullRequest(address) => {
                PathReference::pull_request(address.clone(), page)?
            }
            _ => return Err(unsupported_github_projection()),
        };
        let source = self.read(&searched, operation, None).await?;
        let continuation = source
            .continuation()
            .map(|next| PathReference::parse(next.to_owned()))
            .transpose()?;
        let content = source.content().to_owned();
        let pattern = pattern.to_owned();
        let case_sensitive = options.case_sensitive();
        let mut result = tokio::task::spawn_blocking(move || {
            search_document(&content, &searched, &pattern, case_sensitive)
        })
        .await
        .map_err(|error| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                format!("GitHub discovery worker failed: {error}"),
            )
        })??;
        if let Some(continuation) = continuation {
            result = result.with_source_continuation(continuation);
        }
        Ok(result)
    }

    async fn glob(
        &self,
        _target: &GlobTarget,
        _options: GlobOptions,
        _operation: &OperationGuard,
    ) -> Result<SourceGlobResult, ResourceError> {
        Err(ResourceError::new(
            ErrorCategory::UnsupportedProjection,
            "GitHub Resources cannot be enumerated by glob; read a repository collection",
        ))
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

/// Checks that an upstream parent URL ends in `repos/{owner}/{repo}/{family}/
/// {number}`, so a comment fetched through a repository-wide endpoint is served
/// only beneath the issue or pull request that owns it.
fn validate_parent(
    url: &str,
    repository: &GithubRepositoryIdentity,
    family: &str,
    number: u64,
) -> Result<(), ResourceError> {
    let parsed =
        Url::parse(url).map_err(|_| malformed_upstream("GitHub parent URL is malformed"))?;
    let segments = parsed
        .path_segments()
        .map(Iterator::collect::<Vec<_>>)
        .ok_or_else(|| malformed_upstream("GitHub parent URL is malformed"))?;
    let owned = matches!(
        segments.as_slice(),
        [.., "repos", owner, name, observed_family, observed_number]
            if owner.eq_ignore_ascii_case(repository.owner())
                && name.eq_ignore_ascii_case(repository.repository())
                && *observed_family == family
                && observed_number.parse::<u64>().ok() == Some(number)
    );
    if !owned {
        return Err(ResourceError::new(
            ErrorCategory::NotFound,
            "GitHub comment does not belong to the addressed issue or pull request",
        ));
    }
    Ok(())
}

#[expect(
    clippy::manual_unwrap_or_default,
    reason = "null remains typed absence until the approved empty body Field rendering boundary"
)]
fn authoritative_body(body: Option<String>) -> String {
    match body {
        Some(body) => body,
        None => String::new(),
    }
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

fn validate_issue_listing(values: &[Issue]) -> Result<(), ResourceError> {
    validate_unique_ids(values.iter().map(|value| value.id), "issue.id")?;
    let mut numbers = HashSet::new();
    for value in values {
        validate_positive(value.number, "issue.number")?;
        validate_user(&value.user, "issue.user.id")?;
        if !numbers.insert(value.number)
            || value.state.is_empty()
            || value.title.is_empty()
            || value.html_url.is_empty()
            || value.updated_at.is_empty()
        {
            return Err(malformed_upstream(
                "GitHub issue listing contains invalid required fields",
            ));
        }
    }
    Ok(())
}

fn validate_pull_summaries(values: &[PullRequestSummary]) -> Result<(), ResourceError> {
    validate_unique_ids(values.iter().map(|value| value.id), "pull_request.id")?;
    let mut numbers = HashSet::new();
    for value in values {
        validate_positive(value.number, "pull_request.number")?;
        validate_user(&value.user, "pull_request.user.id")?;
        if !numbers.insert(value.number)
            || value.state.is_empty()
            || value.title.is_empty()
            || value.html_url.is_empty()
            || value.updated_at.is_empty()
        {
            return Err(malformed_upstream(
                "GitHub pull request listing contains invalid required fields",
            ));
        }
    }
    Ok(())
}

fn validate_unique_ids(
    values: impl IntoIterator<Item = u64>,
    field: &str,
) -> Result<(), ResourceError> {
    let mut ids = HashSet::new();
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
        if value.issue_url.is_empty() {
            return Err(malformed_upstream(
                "GitHub conversation comment requires issue_url",
            ));
        }
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
        if value.pull_request_url.is_empty() {
            return Err(malformed_upstream(
                "GitHub review comment requires pull_request_url",
            ));
        }
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
