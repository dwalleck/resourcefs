//! Source-neutral ResourceFS Behavior Contract.

mod error;
mod reference;
mod resource;
mod source;
mod version;

pub use error::{ErrorCategory, ResourceError};
pub use reference::{
    MAX_PATH_REFERENCE_BYTES, MAX_WORKSPACE_ROOTS, PathReference, ProjectionSelector,
    SelectedWorkspaceAddress, WorkspaceAddress, WorkspacePath, WorkspaceRoot, WorkspaceRootId,
    WorkspaceRootSet,
};
pub use resource::{
    BEHAVIOR_CONTRACT_VERSION, MAX_TEXT_BYTES, MAX_TEXT_COLUMNS, MAX_TEXT_LINES, ReadResource,
    TEXT_CONTENT_TYPE,
};
pub use source::SourceAdapter;
pub use version::VersionTag;
