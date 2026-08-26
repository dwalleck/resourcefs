mod render;
mod wire;

use std::{io::Cursor, sync::Arc};

use async_trait::async_trait;
use resourcefs_core::{
    DiscoveryAdapter, ErrorCategory, GithubRepositoryIdentity, GlobOptions, GlobTarget,
    IssueAddress, IssueResource, OperationGuard, PathReference, PathSession, PullRequestAddress,
    PullRequestResource, ResourceAddress, ResourceError, SearchOptions, SearchSourceResult,
    SearchTarget, SessionCacheEntry, SessionCacheKey, SourceAdapter, SourceGlobResult,
    SourceResource, select_utf8,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::{
    GithubConfig, HttpRequest, HttpSubstrate,
    catalog::{SourceCatalogEntry, SourceCatalogMetadata},
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

#[derive(Debug, Serialize, Deserialize)]
struct CacheMetadata {
    etag: String,
    link: Option<String>,
}

struct FetchedResponse {
    body: Arc<[u8]>,
    link: Option<String>,
}

impl FetchedResponse {
    fn body(&self) -> &[u8] {
        &self.body
    }
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

    fn request(url: Url, accept: &str, etag: Option<&str>) -> Result<HttpRequest, ResourceError> {
        let request = HttpRequest::get(url)
            .with_header("Accept", accept)
            .and_then(|request| request.with_header("User-Agent", USER_AGENT))
            .and_then(|request| request.with_header("X-GitHub-Api-Version", GITHUB_API_VERSION))?;
        match etag {
            Some(etag) => request.with_header("If-None-Match", etag),
            None => Ok(request),
        }
    }

    fn cache_key(url: &Url, accept: &str) -> Result<SessionCacheKey, ResourceError> {
        SessionCacheKey::new("github-http", format!("{accept}\n{url}"))
    }

    async fn fetch(
        &self,
        url: Url,
        accept: &str,
        operation: &OperationGuard,
    ) -> Result<FetchedResponse, ResourceError> {
        let key = Self::cache_key(&url, accept)?;
        let cached = self.session.cache_get(&key).await?;
        let cached_metadata = cached
            .as_ref()
            .map(|entry| serde_json::from_slice::<CacheMetadata>(entry.metadata()))
            .transpose()
            .map_err(|_| malformed_upstream("GitHub session cache metadata is corrupt"))?;
        let mut attempt = 0_u8;
        loop {
            let request = Self::request(
                url.clone(),
                accept,
                cached_metadata
                    .as_ref()
                    .map(|metadata| metadata.etag.as_str()),
            )?;
            let response = match self.substrate.fetch_attempt(request, operation).await {
                Ok(response) => response,
                Err(failure) if attempt == 0 && failure.is_retryable() => {
                    attempt = 1;
                    continue;
                }
                Err(failure) => return Err(failure.into_error()),
            };
            if response.status() == 304 {
                let (Some(entry), Some(metadata)) = (cached.as_ref(), cached_metadata.as_ref())
                else {
                    return Err(malformed_upstream(
                        "GitHub returned 304 without a session cache entry",
                    ));
                };
                return Ok(FetchedResponse {
                    body: Arc::clone(entry.content_arc()),
                    link: metadata.link.clone(),
                });
            }
            if attempt == 0
                && matches!(response.status(), 429 | 503)
                && let Some(delay) = response.retry_after()
            {
                attempt = 1;
                tokio::select! {
                    biased;
                    () = operation.cancelled() => {
                        return Err(ResourceError::new(
                            ErrorCategory::Cancelled,
                            "GitHub retry wait was cancelled",
                        ));
                    }
                    () = tokio::time::sleep(delay) => {}
                }
                continue;
            }
            self.classify_status(&response)?;
            if response.truncated() {
                return Err(ResourceError::new(
                    ErrorCategory::LimitExceeded,
                    "GitHub response exceeds the bounded HTTP body ceiling",
                ));
            }
            let etag = response.etag().map(str::to_owned);
            let link = response.link().map(str::to_owned);
            let body = response.into_body();
            let entry = SessionCacheEntry::new(
                etag.as_ref().map_or_else(Vec::new, |etag| {
                    serde_json::to_vec(&CacheMetadata {
                        etag: etag.clone(),
                        link: link.clone(),
                    })
                    .expect("CacheMetadata serialization cannot fail")
                }),
                body,
            )?;
            if etag.is_some() {
                self.session.cache_put(key.clone(), entry.clone()).await?;
            } else {
                self.session.cache_remove(&key).await?;
            }
            return Ok(FetchedResponse {
                body: Arc::clone(entry.content_arc()),
                link,
            });
        }
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

    fn next_link(
        &self,
        link: Option<&str>,
        repository: &GithubRepositoryIdentity,
        expected_path: &str,
    ) -> Result<Option<Url>, ResourceError> {
        let Some(target) = link.and_then(|link| {
            link.split(',').find_map(|part| {
                let part = part.trim();
                part.contains("rel=\"next\"").then(|| {
                    part.split_once('<')
                        .and_then(|(_, rest)| rest.split_once('>'))
                        .map(|(url, _)| url)
                })?
            })
        }) else {
            return Ok(None);
        };
        let target = Url::parse(target)
            .map_err(|_| malformed_upstream("GitHub pagination Link is malformed"))?;
        if target.scheme() != self.api_base.scheme()
            || target.host_str() != self.api_base.host_str()
            || target.port_or_known_default() != self.api_base.port_or_known_default()
            || target.path() != expected_path
            || !target.path().contains(&format!(
                "/repos/{}/{}/",
                repository.owner(),
                repository.repository()
            ))
        {
            return Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "GitHub pagination Link leaves its repository endpoint authority",
            ));
        }
        Ok(Some(target))
    }

    async fn collection<T: DeserializeOwned>(
        &self,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
        parameters: &[(&str, &str)],
        start_page: u64,
        operation: &OperationGuard,
    ) -> Result<(Vec<T>, Option<u64>), ResourceError> {
        let mut next = self.endpoint(repository, suffix)?;
        {
            let mut query = next.query_pairs_mut();
            for (name, value) in parameters {
                query.append_pair(name, value);
            }
            query.append_pair("per_page", "100");
            query.append_pair("page", &start_page.to_string());
        }
        let expected_path = next.path().to_owned();
        let mut values = Vec::new();
        for _ in 0..10 {
            let response = self.fetch(next, GITHUB_JSON, operation).await?;
            let following = self.next_link(response.link.as_deref(), repository, &expected_path)?;
            let mut page: Vec<T> = wire::decode(response.body())?;
            values.append(&mut page);
            let Some(link) = following else {
                return Ok((values, None));
            };
            next = link;
        }
        let continuation = next
            .query_pairs()
            .find_map(|(name, value)| (name == "page").then(|| value.parse::<u64>().ok()))
            .flatten()
            .ok_or_else(|| malformed_upstream("GitHub next Link does not name a numeric page"))?;
        Ok((values, Some(continuation)))
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
        page: Option<u64>,
        operation: &OperationGuard,
    ) -> Result<String, ResourceError> {
        if let IssueAddress::Collection { repository } = address {
            let (mut issues, continuation): (Vec<Issue>, Option<u64>) = self
                .collection(
                    repository,
                    "issues",
                    &[("state", "all"), ("sort", "updated"), ("direction", "desc")],
                    page.unwrap_or(1),
                    operation,
                )
                .await?;
            for issue in &issues {
                validate_issue_identity(issue, issue.number)?;
            }
            issues.retain(|issue| issue.pull_request.is_none());
            issues.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.number.cmp(&right.number))
            });
            let mut rendered = format!("# Issues: {}\n", repository.as_str());
            if issues.is_empty() {
                rendered.push_str("\n(no issues)\n");
            } else {
                for issue in issues {
                    use std::fmt::Write as _;
                    writeln!(
                        &mut rendered,
                        "- issue://{}/{} — {} [{}] by {} (updated {})",
                        repository.as_str(),
                        issue.number,
                        issue.title,
                        issue.state,
                        issue
                            .user
                            .as_ref()
                            .map_or("[deleted]", |user| user.login.as_str()),
                        issue.updated_at
                    )
                    .expect("String write");
                }
            }
            if let Some(page) = continuation {
                use std::fmt::Write as _;
                writeln!(
                    &mut rendered,
                    "\nContinuation: issue://{}:page:{page}",
                    repository.as_str()
                )
                .expect("String write");
            }
            return Ok(rendered);
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
        match resource {
            IssueResource::Aggregate => {
                let issue: Issue = self
                    .json(
                        self.endpoint(repository, &format!("issues/{number}"))?,
                        operation,
                    )
                    .await?;
                let (mut comments, continuation): (Vec<ConversationComment>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("issues/{number}/comments"),
                        &[],
                        1,
                        operation,
                    )
                    .await?;
                validate_issue_identity(&issue, number)?;
                validate_comment_ids(&comments)?;
                let mut rendered = render::issue_aggregate(repository, &issue, &mut comments);
                if let Some(page) = continuation {
                    use std::fmt::Write as _;
                    writeln!(
                        &mut rendered,
                        "\nContinuation: issue://{}/{number}/comments:page:{page}",
                        repository.as_str()
                    )
                    .expect("String write");
                }
                Ok(rendered)
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
                    IssueResource::Body => authoritative_body(issue.body),
                    _ => unreachable!("matched title/body"),
                })
            }
            IssueResource::Comments => {
                let (mut comments, continuation): (Vec<ConversationComment>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("issues/{number}/comments"),
                        &[],
                        page.unwrap_or(1),
                        operation,
                    )
                    .await?;
                validate_comment_ids(&comments)?;
                comments.sort_by(|left, right| {
                    left.created_at
                        .cmp(&right.created_at)
                        .then_with(|| left.id.cmp(&right.id))
                });
                let mut rendered = comments
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
                    .join("\n\n");
                if let Some(page) = continuation {
                    use std::fmt::Write as _;
                    writeln!(
                        &mut rendered,
                        "\nContinuation: issue://{}/{number}/comments:page:{page}",
                        repository.as_str()
                    )
                    .expect("String write");
                }
                Ok(rendered)
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
        page: Option<u64>,
        operation: &OperationGuard,
    ) -> Result<String, ResourceError> {
        if let PullRequestAddress::Collection { repository } = address {
            let (mut pulls, continuation): (Vec<PullRequestSummary>, Option<u64>) = self
                .collection(
                    repository,
                    "pulls",
                    &[("state", "all"), ("sort", "updated"), ("direction", "desc")],
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
            let mut rendered = format!("# Pull requests: {}\n", repository.as_str());
            if pulls.is_empty() {
                rendered.push_str("\n(no pull requests)\n");
            } else {
                for pull in pulls {
                    use std::fmt::Write as _;
                    let merge = if pull.merged_at.is_some() {
                        "merged"
                    } else {
                        pull.state.as_str()
                    };
                    writeln!(
                        &mut rendered,
                        "- pr://{}/{} — {} [{}{}] by {} (updated {}; {})",
                        repository.as_str(),
                        pull.number,
                        pull.title,
                        merge,
                        if pull.draft { ", draft" } else { "" },
                        pull.user
                            .as_ref()
                            .map_or("[deleted]", |user| user.login.as_str()),
                        pull.updated_at,
                        pull.html_url
                    )
                    .expect("String write");
                }
            }
            if let Some(page) = continuation {
                use std::fmt::Write as _;
                writeln!(
                    &mut rendered,
                    "\nContinuation: pr://{}:page:{page}",
                    repository.as_str()
                )
                .expect("String write");
            }
            return Ok(rendered);
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
        match resource {
            PullRequestResource::Aggregate => {
                let pull: PullRequest = self
                    .json(
                        self.endpoint(repository, &format!("pulls/{number}"))?,
                        operation,
                    )
                    .await?;
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
                validate_pull_identity(&pull, number)?;
                validate_comment_ids(&comments)?;
                validate_review_ids(&reviews)?;
                validate_review_comment_ids(&review_comments)?;
                validate_files(&files)?;
                let mut rendered = render::pull_request_aggregate(
                    repository,
                    &pull,
                    &mut comments,
                    &mut reviews,
                    &mut review_comments,
                    &files,
                );
                use std::fmt::Write as _;
                for (collection, next) in [
                    ("comments", comment_next),
                    ("reviews", review_next),
                    ("review-comments", review_comment_next),
                    ("diff", file_next),
                ] {
                    if let Some(page) = next {
                        writeln!(
                            &mut rendered,
                            "\nContinuation: pr://{}/{number}/{collection}:page:{page}",
                            repository.as_str()
                        )
                        .expect("String write");
                    }
                }
                Ok(rendered)
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
                    PullRequestResource::Body => authoritative_body(pull.body),
                    _ => unreachable!("matched title/body"),
                })
            }
            PullRequestResource::Comments => {
                let (comments, continuation): (Vec<ConversationComment>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("issues/{number}/comments"),
                        &[],
                        page.unwrap_or(1),
                        operation,
                    )
                    .await?;
                validate_comment_ids(&comments)?;
                let mut rendered = comments
                    .iter()
                    .map(render::comment)
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if let Some(page) = continuation {
                    use std::fmt::Write as _;
                    writeln!(
                        &mut rendered,
                        "\nContinuation: pr://{}/{number}/comments:page:{page}",
                        repository.as_str()
                    )
                    .expect("String write");
                }
                Ok(rendered)
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
                let (reviews, continuation): (Vec<Review>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("pulls/{number}/reviews"),
                        &[],
                        page.unwrap_or(1),
                        operation,
                    )
                    .await?;
                validate_review_ids(&reviews)?;
                let mut rendered = reviews
                    .iter()
                    .map(render::review)
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if let Some(page) = continuation {
                    use std::fmt::Write as _;
                    writeln!(
                        &mut rendered,
                        "\nContinuation: pr://{}/{number}/reviews:page:{page}",
                        repository.as_str()
                    )
                    .expect("String write");
                }
                Ok(rendered)
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
                let (comments, continuation): (Vec<ReviewComment>, Option<u64>) = self
                    .collection(
                        repository,
                        &format!("pulls/{number}/comments"),
                        &[],
                        page.unwrap_or(1),
                        operation,
                    )
                    .await?;
                validate_review_comment_ids(&comments)?;
                let mut rendered = comments
                    .iter()
                    .map(render::review_comment)
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if let Some(page) = continuation {
                    use std::fmt::Write as _;
                    writeln!(
                        &mut rendered,
                        "\nContinuation: pr://{}/{number}/review-comments:page:{page}",
                        repository.as_str()
                    )
                    .expect("String write");
                }
                Ok(rendered)
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
                if let Some(page) = page {
                    let (files, continuation): (Vec<DiffFile>, Option<u64>) = self
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
                        .and_then(|page| page.checked_mul(100))
                        .ok_or_else(|| {
                            ResourceError::new(
                                ErrorCategory::LimitExceeded,
                                "GitHub diff page index overflowed",
                            )
                        })?;
                    let mut rendered = files
                        .iter()
                        .enumerate()
                        .map(|(offset, file)| {
                            format!(
                                "pr://{}/{number}/diff/{} — {} ({})",
                                repository.as_str(),
                                base_index + offset as u64 + 1,
                                file.filename,
                                file.status
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if let Some(page) = continuation {
                        use std::fmt::Write as _;
                        writeln!(
                            &mut rendered,
                            "\nContinuation: pr://{}/{number}/diff:page:{page}",
                            repository.as_str()
                        )
                        .expect("String write");
                    }
                    Ok(rendered)
                } else {
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

    async fn read_resource(
        &self,
        reference: &PathReference,
        operation: &OperationGuard,
    ) -> Result<SourceResource, ResourceError> {
        let projection = reference.projection();
        let page = projection.and_then(resourcefs_core::ProjectionSelector::page_offset);
        let (canonical, content) = match reference.address() {
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
        if projection.is_some_and(|projection| projection.line_selection().is_some()) {
            let selected = select_utf8(Cursor::new(content), projection)?;
            SourceResource::selected_text(canonical, selected)
        } else {
            SourceResource::text(canonical, content)
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
        tokio::time::timeout(
            self.substrate.ceilings().timeout(),
            self.read_resource(reference, operation),
        )
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
                "pr://<owner>/<repository>[/<number>[/title|body|comments|reviews|review-comments|diff]][:selector]",
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
        if !matches!(
            reference.address(),
            ResourceAddress::Issue(_) | ResourceAddress::PullRequest(_)
        ) {
            return Err(unsupported_github_projection());
        }
        let source = self.read(reference, operation).await?;
        let canonical = PathReference::parse(source.canonical_reference().to_owned())?;
        let mut lines = source.content().lines().collect::<Vec<_>>();
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        let mut continuations = Vec::new();
        while let Some(line) = lines
            .last()
            .and_then(|line| line.strip_prefix("Continuation: "))
        {
            continuations.push(PathReference::parse(line)?);
            lines.pop();
            while lines.last().is_some_and(|line| line.is_empty()) {
                lines.pop();
            }
        }
        continuations.reverse();
        let content = lines.join("\n");
        let continuation = match continuations.as_slice() {
            [] => None,
            [continuation] => Some(continuation.clone()),
            many => {
                let manifest = many
                    .iter()
                    .map(PathReference::requested)
                    .collect::<Vec<_>>()
                    .join("\n");
                let address = self.session.retain(&manifest, operation).await?;
                Some(PathReference::artifact(address, None)?)
            }
        };
        let pattern = pattern.to_owned();
        let case_sensitive = options.case_sensitive();
        let mut result = tokio::task::spawn_blocking(move || {
            search_document(&content, &canonical, &pattern, case_sensitive)
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

fn validate_pull_summaries(values: &[PullRequestSummary]) -> Result<(), ResourceError> {
    validate_unique_ids(values.iter().map(|value| value.id), "pull_request.id")?;
    let mut numbers = std::collections::HashSet::new();
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
