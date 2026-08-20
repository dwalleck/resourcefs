//! Source-neutral ResourceFS Behavior Contract.

mod error;
mod reference;
mod resource;
mod selector;
mod source;
mod version;

pub use error::{ErrorCategory, ResourceError};
pub use reference::{
    ArtifactAddress, LineRange, LineSelector, MAX_PATH_REFERENCE_BYTES, MAX_WORKSPACE_ROOTS,
    PathReference, ProjectionSelector, ResourceAddress, SelectedWorkspaceAddress, WorkspaceAddress,
    WorkspacePath, WorkspaceRoot, WorkspaceRootId, WorkspaceRootSet,
};
pub use resource::{
    BEHAVIOR_CONTRACT_VERSION, MAX_ARTIFACT_BYTES, MAX_TEXT_BYTES, MAX_TEXT_COLUMNS,
    MAX_TEXT_LINES, ReadResource, TEXT_CONTENT_TYPE,
};
pub use selector::{SelectedText, select_utf8};
pub use source::SourceAdapter;
pub use version::VersionTag;
