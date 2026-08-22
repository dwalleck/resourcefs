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
    ChildEnvironment, CommandSpec, ConfigurationError, CredentialHeader, DownstreamMcpConfig,
    DownstreamServer, DownstreamTransport, EnvironmentValue, GithubConfig, GithubRepository,
    HttpsConfig, HttpsOrigin, MAX_COMMAND_ARGUMENT_BYTES, MAX_COMMAND_ARGUMENTS,
    MAX_COMMAND_ENVIRONMENT_ENTRIES, MAX_CONFIGURATION_ENTRIES, MAX_CONFIGURATION_ID_BYTES,
    MAX_SCHEME_CLAIM_BYTES, MutationGrants, MutationSupport, SchemeClaim, SecretReference,
    SshConfig, SshHost, validate_configuration_id,
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
