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
    catalog_discovery_unsupported, DiscoveryAdapter, DiscoveryDiagnostic, DiscoveryEngine,
    GlobEntry, GlobKind, GlobLimits, GlobOptions, GlobRequest, GlobResult, GlobSource, GlobTarget,
    SearchEngine, SearchGroup, SearchLimits, SearchLine, SearchOptions, SearchRecord,
    SearchRequest, SearchResult, SearchSourceResult, SearchTarget, SourceGlobResult,
    MAX_DISCOVERY_PATTERN_BYTES, MAX_DISCOVERY_RESULTS,
};
pub use error::{
    AccessAmbiguity, AcquisitionLimitKind, ErrorCategory, ErrorReason, HttpStatus, LimitDetail,
    ResourceError, ResourceErrorDetails, RetryGuidance,
};
pub use http_policy::{
    AddressClass, AddressPolicy, AllowedOrigin, HttpCeilings, HttpCeilingsInput, OriginAllowlist,
    MAX_HTTP_FETCH_BYTES, MAX_HTTP_REDIRECT_DEPTH, MAX_HTTP_TIMEOUT_MILLIS,
};
pub use limits::{
    DiscoveryLimitInput, ServerLimits, ServerLimitsInput, StorageLimitInput, TextLimitInput,
    MAX_IMAGE_BYTES,
};
pub use mutation::{
    HashlinePatch, LineNumber, MutationAccess, MutationAdapter, MutationCommitFailure,
    MutationCommitOutcome, MutationEngine, MutationLockSet, MutationOperation, MutationReceipt,
    MutationResourceKey, MutationSourceKey, MutationState, MutationTarget, MutationTargetMode,
    OperationId, OriginalLineRange, PatchOperation, PatchOperationRef, PutTarget, SourceMutation,
    VersionSelector, WriteRequest, MAX_HASHLINE_PATCH_BYTES, MAX_OPERATION_ID_BYTES,
    MIN_VERSION_PREFIX_HEX,
};
pub use probe::{
    ProbeDiagnostic, ProbeDiagnosticError, ProbeOutcome, ProbeState, SourceProbe,
    MAX_PROBE_DIAGNOSTIC_BYTES,
};
pub use read::{ReadEngine, ReadRequest, TextLimits};
pub use reference::{
    ArtifactAddress, AtlassianSiteId, CatalogAddress, ConversationCommentId, DiffFileIndex,
    GithubRepositoryIdentity, HttpsAddress, IssueAddress, IssueNumber, IssueResource, JiraAddress,
    JiraFieldId, JiraIssueId, JiraIssueKey, JiraIssueResource, JiraProjectId, JiraProjectKey,
    JiraQuery, LineRange, LineSelector, LocalAddress, LocalName, PathReference, ProjectionSelector,
    PullRequestAddress, PullRequestFact, PullRequestNumber, PullRequestResource, ResourceAddress,
    ReviewCommentId, ReviewId, SelectedLocalAddress, SelectedWorkspaceAddress, SourceCursor,
    SourceOffset, WorkspaceAddress, WorkspacePath, WorkspaceRoot, WorkspaceRootId,
    WorkspaceRootSet, MAX_ATLASSIAN_SITE_ID_BYTES, MAX_JIRA_ISSUE_ID_BYTES,
    MAX_JIRA_PROJECT_ID_BYTES, MAX_JIRA_SEGMENT_BYTES, MAX_LOCAL_NAME_BYTES,
    MAX_PATH_REFERENCE_BYTES, MAX_WORKSPACE_ROOTS,
};
pub use resource::{
    ArtifactProjectionOrigin, DisplayedLineRange, ReadResource, SourceResource, Utf8ContentType,
    BEHAVIOR_CONTRACT_VERSION, MAX_ARTIFACT_BYTES, MAX_TEXT_BYTES, MAX_TEXT_COLUMNS,
    MAX_TEXT_LINES, TEXT_CONTENT_TYPE,
};
pub use secret::{Redactor, Secret, SecretError};
pub use selector::{select_utf8, SelectedText};
pub use session::{
    ArtifactId, MutationOperationLease, MutationOperationOutcome, MutationOperationStart,
    MutationOperationWaiter, OperationGuard, PathSession, ScratchState, SessionCacheEntry,
    SessionCacheKey, SessionStorage, SessionToken, MAX_MUTATION_OPERATIONS, MAX_SESSION_ARTIFACTS,
    MAX_SESSION_BYTES,
};
pub use source::{SourceAdapter, reject_acquisition};
pub use version::VersionTag;
