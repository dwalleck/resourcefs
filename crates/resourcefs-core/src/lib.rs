//! Source-neutral ResourceFS Behavior Contract.

mod acquisition;
mod discovery;
mod error;
mod http_policy;
mod limits;
mod mutation;
mod probe;
mod read;
mod reference;
mod resource;
mod secret;
mod selector;
mod session;
mod source;
mod version;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use acquisition::ReadAcquisitionLimits;
#[cfg(feature = "test-support")]
pub use discovery::DiscoveryRetainGate;
pub use discovery::{
    DiscoveryAdapter, DiscoveryDiagnostic, DiscoveryEngine, GlobEntry, GlobKind, GlobLimits,
    GlobOptions, GlobRequest, GlobResult, GlobSource, GlobTarget, MAX_DISCOVERY_PATTERN_BYTES,
    MAX_DISCOVERY_RESULTS, SearchEngine, SearchGroup, SearchLimits, SearchLine, SearchOptions,
    SearchRecord, SearchRequest, SearchResult, SearchSourceResult, SearchTarget, SourceGlobResult,
    catalog_discovery_unsupported,
};
pub use error::{
    AccessAmbiguity, AcquisitionLimitKind, ErrorCategory, ErrorReason, HttpStatus, LimitDetail,
    ResourceError, ResourceErrorDetails, RetryGuidance,
};
pub use http_policy::{
    AddressClass, AddressPolicy, AllowedOrigin, HttpCeilings, HttpCeilingsInput,
    MAX_HTTP_FETCH_BYTES, MAX_HTTP_REDIRECT_DEPTH, MAX_HTTP_TIMEOUT_MILLIS, OriginAllowlist,
};
pub use limits::{
    DiscoveryLimitInput, MAX_IMAGE_BYTES, ServerLimits, ServerLimitsInput, StorageLimitInput,
    TextLimitInput,
};
pub use mutation::{
    HashlinePatch, LineNumber, MAX_HASHLINE_PATCH_BYTES, MAX_OPERATION_ID_BYTES,
    MIN_VERSION_PREFIX_HEX, MutationAccess, MutationAdapter, MutationCommitFailure,
    MutationCommitOutcome, MutationEngine, MutationLockSet, MutationOperation, MutationReceipt,
    MutationResourceKey, MutationSourceKey, MutationState, MutationTarget, MutationTargetMode,
    OperationId, OriginalLineRange, PatchOperation, PatchOperationRef, PutTarget, SourceMutation,
    VersionSelector, WriteRequest,
};
pub use probe::{
    MAX_PROBE_DIAGNOSTIC_BYTES, ProbeDiagnostic, ProbeDiagnosticError, ProbeOutcome, ProbeState,
    SourceProbe,
};
pub use read::{ReadEngine, ReadRequest, TextLimits};
pub use reference::{
    ArtifactAddress, AtlassianSiteId, CatalogAddress, ConversationCommentId, DiffFileIndex,
    GithubRepositoryIdentity, HttpsAddress, IssueAddress, IssueNumber, IssueResource, JiraAddress,
    JiraFieldId, JiraIssueId, JiraIssueKey, JiraIssueResource, JiraProjectId, JiraProjectKey,
    JiraQuery, LineRange, LineSelector, LocalAddress, LocalName, MAX_ATLASSIAN_SITE_ID_BYTES,
    MAX_JIRA_ISSUE_ID_BYTES, MAX_JIRA_PROJECT_ID_BYTES, MAX_JIRA_SEGMENT_BYTES,
    MAX_LOCAL_NAME_BYTES, MAX_PATH_REFERENCE_BYTES, MAX_WORKSPACE_ROOTS, PathReference,
    ProjectionSelector, PullRequestAddress, PullRequestNumber, PullRequestResource,
    ResourceAddress, ReviewCommentId, ReviewId, SelectedLocalAddress, SelectedWorkspaceAddress,
    SourceCursor, SourceOffset, WorkspaceAddress, WorkspacePath, WorkspaceRoot, WorkspaceRootId,
    WorkspaceRootSet,
};
pub use resource::{
    ArtifactProjectionOrigin, BEHAVIOR_CONTRACT_VERSION, DisplayedLineRange, MAX_ARTIFACT_BYTES,
    MAX_TEXT_BYTES, MAX_TEXT_COLUMNS, MAX_TEXT_LINES, ReadResource, SourceResource,
    TEXT_CONTENT_TYPE, Utf8ContentType,
};
pub use secret::{Redactor, Secret, SecretError};
pub use selector::{SelectedText, select_utf8};
pub use session::{
    ArtifactId, MAX_MUTATION_OPERATIONS, MAX_SESSION_ARTIFACTS, MAX_SESSION_BYTES,
    MutationOperationLease, MutationOperationOutcome, MutationOperationStart,
    MutationOperationWaiter, OperationGuard, PathSession, ScratchState, SessionCacheEntry,
    SessionCacheKey, SessionStorage, SessionToken,
};
pub use source::SourceAdapter;
pub use version::VersionTag;
