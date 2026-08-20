//! Source-neutral ResourceFS Behavior Contract.

mod discovery;
mod error;
mod read;
mod reference;
mod resource;
mod selector;
mod session;
mod source;
mod version;

#[cfg(feature = "test-support")]
pub use discovery::DiscoveryRetainGate;
pub use discovery::{
    DiscoveryAdapter, DiscoveryDiagnostic, DiscoveryEngine, GlobEntry, GlobKind, GlobLimits,
    GlobOptions, GlobRequest, GlobResult, GlobTarget, MAX_DISCOVERY_PATTERN_BYTES,
    MAX_DISCOVERY_RESULTS, SearchEngine, SearchGroup, SearchLimits, SearchLine, SearchOptions,
    SearchRecord, SearchRequest, SearchResult, SearchSourceResult, SearchTarget, SourceGlobResult,
};
pub use error::{ErrorCategory, ResourceError};
pub use read::{ReadEngine, ReadRequest, TextLimits};
pub use reference::{
    ArtifactAddress, LineRange, LineSelector, MAX_PATH_REFERENCE_BYTES, MAX_WORKSPACE_ROOTS,
    PathReference, ProjectionSelector, ResourceAddress, SelectedWorkspaceAddress, WorkspaceAddress,
    WorkspacePath, WorkspaceRoot, WorkspaceRootId, WorkspaceRootSet,
};
pub use resource::{
    ArtifactProjectionOrigin, BEHAVIOR_CONTRACT_VERSION, MAX_ARTIFACT_BYTES, MAX_TEXT_BYTES,
    MAX_TEXT_COLUMNS, MAX_TEXT_LINES, ReadResource, SourceResource, TEXT_CONTENT_TYPE,
};
pub use selector::{SelectedText, select_utf8};
pub use session::{
    ArtifactId, MAX_SESSION_ARTIFACTS, MAX_SESSION_BYTES, OperationGuard, PathSession,
    SessionStorage, SessionToken,
};
pub use source::SourceAdapter;
pub use version::VersionTag;
