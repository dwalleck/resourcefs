//! Compiled ResourceFS Source Adapters.

mod artifact;
mod compiled;
mod configuration;
mod filesystem;
mod pattern;
mod session_storage;
pub use artifact::ArtifactSource;
pub use compiled::CompiledSources;
pub use configuration::{
    ConfigurationError, MAX_CONFIGURATION_ID_BYTES, MutationGrants, MutationSupport,
    validate_configuration_id,
};

#[cfg(feature = "test-support")]
pub use filesystem::TestDeliveryGate;
pub use filesystem::{
    BackingPathVisibility, ClientRoot, FilesystemSource, LaunchRoot, LaunchRootSource,
    RootAcquisition, RootRefresh, RootRefreshOutcome,
};
#[cfg(feature = "test-support")]
pub use session_storage::StorageFailurePoint;
pub use session_storage::{
    CleanupReport, DiskSessionStorage, SESSION_CLEANUP_TTL, SessionStore, StoredSession,
};
