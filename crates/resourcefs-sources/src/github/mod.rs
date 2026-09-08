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
    SearchSourceResult, SearchTarget, SessionCacheEntry, SessionCacheKey, SourceAdapter,
    SourceGlobResult, SourceResource, select_utf8,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::{
    GithubConfig, HttpRequest, HttpSubstrate,
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
pub(super) const GITHUB_CACHE_NAMESPACE: &str = "github-http";
/// Objects requested per upstream page, and the pages followed inside one
/// operation: together they bound a logical collection at 1,000 objects.
const PAGE_SIZE: u64 = 100;
const MAX_PAGES_PER_OPERATION: u64 = 10;
/// Query for repository collections: every state, most recently updated first.
const COLLECTION_QUERY: &[(&str, &str)] =
    &[("state", "all"), ("sort", "updated"), ("direction", "desc")];

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

    /// The URL of one upstream page of a collection, at the fixed page size.
    fn page_url(
        &self,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
        parameters: &[(&str, &str)],
        page: u64,
    ) -> Result<Url, ResourceError> {
        let mut url = self.endpoint(repository, suffix)?;
        {
            let mut query = url.query_pairs_mut();
            for (name, value) in parameters {
                query.append_pair(name, value);
            }
            query.append_pair("per_page", &PAGE_SIZE.to_string());
            query.append_pair("page", &page.to_string());
        }
        Ok(url)
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
        SessionCacheKey::new(GITHUB_CACHE_NAMESPACE, format!("{accept}\n{url}"))
    }

    async fn fetch(
        &self,
        url: Url,
        accept: &str,
        operation: BoundedRead<'_>,
    ) -> Result<FetchedResponse, ResourceError> {
        let key = Self::cache_key(&url, accept)?;
        let cache_generation = self
            .session
            .cache_generation(GITHUB_CACHE_NAMESPACE)
            .await?;
        let mut cached = self.session.cache_get(&key).await?;
        let mut cached_metadata = cached
            .as_ref()
            .map(|entry| serde_json::from_slice::<CacheMetadata>(entry.metadata()))
            .transpose()
            .map_err(|_| malformed_upstream("GitHub session cache metadata is corrupt"))?;
        // A validator the request header ceiling cannot carry is no validator:
        // drop the entry and fetch unconditionally rather than leave a URL
        // unreadable for the rest of the session behind a retained ETag.
        if let Some(metadata) = cached_metadata.as_ref()
            && let Err(error) = Self::request(url.clone(), accept, Some(&metadata.etag))
        {
            if error.category() != ErrorCategory::LimitExceeded {
                return Err(error);
            }
            self.session
                .cache_remove_if_generation(&key, cache_generation)
                .await?;
            cached = None;
            cached_metadata = None;
        }
        let validator = cached_metadata
            .as_ref()
            .map(|metadata| metadata.etag.as_str());
        let request = Self::request(url, accept, validator)?;
        let response = operation.fetch(request).await?;
        if response.status() == 304 {
            if self
                .session
                .cache_generation(GITHUB_CACHE_NAMESPACE)
                .await?
                != cache_generation
            {
                return Err(github_error(
                    ErrorCategory::SourceUnavailable,
                    "GitHub read was invalidated by a concurrent mutation; retry the read",
                ));
            }
            let (Some(entry), Some(metadata)) = (cached.as_ref(), cached_metadata.as_ref()) else {
                return Err(malformed_upstream(
                    "GitHub returned 304 without a session cache entry",
                ));
            };
            return Ok(FetchedResponse {
                body: Arc::clone(entry.content_arc()),
                link: metadata.link.clone(),
            });
        }
        self.classify_status(&response)?;
        if response.truncated() {
            return Err(ResourceError::new(
                ErrorCategory::LimitExceeded,
                "GitHub response exceeds the bounded HTTP body ceiling",
            ));
        }
        let etag = response.etag().map(str::to_owned);
        // An unreadable Link is a corrupt continuation, not the last page.
        let link = response.link()?.map(str::to_owned);
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
            self.cache(key, &entry, cache_generation).await?;
        } else {
            self.session
                .cache_remove_if_generation(&key, cache_generation)
                .await?;
        }
        Ok(FetchedResponse {
            body: Arc::clone(entry.content_arc()),
            link,
        })
    }

    /// Caches a validated response for conditional revalidation.
    ///
    /// A ceiling refusal is not a read failure: the substrate accepted the
    /// bytes and they are returned to the caller uncached. Any previous entry
    /// under the key is dropped so a later `304` can never revive content the
    /// refused response replaced. Every other cache failure is propagated.
    async fn cache(
        &self,
        key: SessionCacheKey,
        entry: &SessionCacheEntry,
        generation: u64,
    ) -> Result<(), ResourceError> {
        match self
            .session
            .cache_put_if_generation(key.clone(), entry.clone(), generation)
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if error.category() == ErrorCategory::LimitExceeded => {
                self.session
                    .cache_remove_if_generation(&key, generation)
                    .await?;
                Ok(())
            }
            Err(error) => Err(error),
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

    /// Resolves the `rel="next"` target of a page's Link header, confined to
    /// the same API origin and the same repository endpoint family.
    ///
    /// Two path spellings are accepted for the family: the `/repos/{owner}/
    /// {repo}/{suffix}` form this adapter requests, and `/repositories/{id}/
    /// {suffix}`, which is what the live API actually returns (evidence P4).
    /// The numeric id is the upstream's own name for the repository it was
    /// just asked about, so following it discloses nothing new to the origin.
    fn next_link(
        &self,
        link: Option<&str>,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
    ) -> Result<Option<Url>, ResourceError> {
        let Some(link) = link else {
            return Ok(None);
        };
        let Some(target) = next_link_target(link)? else {
            return Ok(None);
        };
        let target = Url::parse(target).map_err(|_| malformed_link())?;
        let repository_path = self.endpoint(repository, suffix)?.path().to_owned();
        let same_origin = target.scheme() == self.api_base.scheme()
            && target.host_str() == self.api_base.host_str()
            && target.port_or_known_default() == self.api_base.port_or_known_default();
        let same_family = target.path().eq_ignore_ascii_case(&repository_path)
            || self.is_repository_id_path(target.path(), suffix);
        if !same_origin || !same_family {
            return Err(ResourceError::new(
                ErrorCategory::PermissionDenied,
                "GitHub pagination Link leaves its repository endpoint authority",
            ));
        }
        Ok(Some(target))
    }

    fn is_repository_id_path(&self, path: &str, suffix: &str) -> bool {
        let Some(rest) = path.strip_prefix(self.api_base.path()) else {
            return false;
        };
        let Some(rest) = rest.strip_prefix("repositories/") else {
            return false;
        };
        let Some((id, rest)) = rest.split_once('/') else {
            return false;
        };
        !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()) && rest == suffix
    }

    /// Fetches one logical collection from `start_page`, following at most
    /// [`MAX_PAGES_PER_OPERATION`] Link targets, and returns the objects with
    /// the number of the first page not fetched.
    async fn collection<T: DeserializeOwned>(
        &self,
        repository: &GithubRepositoryIdentity,
        suffix: &str,
        parameters: &[(&str, &str)],
        start_page: u64,
        operation: BoundedRead<'_>,
    ) -> Result<(Vec<T>, Option<u64>), ResourceError> {
        let mut next = self.page_url(repository, suffix, parameters, start_page)?;
        let mut values = Vec::new();
        for _ in 0..MAX_PAGES_PER_OPERATION {
            let response = self.fetch(next, GITHUB_JSON, operation).await?;
            let following = self.next_link(response.link.as_deref(), repository, suffix)?;
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
        operation: BoundedRead<'_>,
    ) -> Result<T, ResourceError> {
        let response = self.fetch(url, GITHUB_JSON, operation).await?;
        wire::decode(response.body())
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

/// Extracts the `rel="next"` target of a Link header (RFC 8288), or `None`
/// when the header names no next relation.
///
/// Strict on shape: a link-value that does not open with `<`, never closes
/// its `>`, or carries an unparseable parameter is an error, never "no next
/// page" — an unlabeled partial listing is exactly what a lenient parse would
/// produce. Lenient on spelling: `rel=next`, `rel="next"`, `rel="next last"`,
/// and any parameter-name case all name the same relation.
fn next_link_target(link: &str) -> Result<Option<&str>, ResourceError> {
    let mut rest = link.trim_start();
    let mut next = None;
    while !rest.is_empty() {
        let Some(after_open) = rest.strip_prefix('<') else {
            return Err(malformed_link());
        };
        let Some((target, after_target)) = after_open.split_once('>') else {
            return Err(malformed_link());
        };
        let (parameters, remainder) = split_first_outside_quotes(after_target, b',');
        if link_value_is_next(parameters)? && next.replace(target).is_some() {
            return Err(malformed_link());
        }
        rest = remainder.unwrap_or("").trim_start();
    }
    Ok(next)
}

fn link_value_is_next(parameters: &str) -> Result<bool, ResourceError> {
    let mut is_next = false;
    let mut rest = Some(parameters);
    while let Some(remaining) = rest {
        let (parameter, remainder) = split_first_outside_quotes(remaining, b';');
        rest = remainder;
        let parameter = parameter.trim();
        if parameter.is_empty() {
            continue;
        }
        let Some((name, value)) = parameter.split_once('=') else {
            return Err(malformed_link());
        };
        if name.trim().eq_ignore_ascii_case("rel")
            && value
                .trim()
                .trim_matches('"')
                .split_ascii_whitespace()
                .any(|relation| relation.eq_ignore_ascii_case("next"))
        {
            is_next = true;
        }
    }
    Ok(is_next)
}

/// Splits `value` at the first `delimiter` that sits outside a quoted-string,
/// returning the head and the remainder after the delimiter, if any.
fn split_first_outside_quotes(value: &str, delimiter: u8) -> (&str, Option<&str>) {
    let mut quoted = false;
    let mut escaped = false;
    for (index, byte) in value.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            _ if byte == delimiter && !quoted => {
                return (&value[..index], Some(&value[index + 1..]));
            }
            _ => {}
        }
    }
    (value, None)
}

fn malformed_link() -> ResourceError {
    malformed_upstream("GitHub pagination Link is malformed")
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
