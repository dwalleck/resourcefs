//! Compiled ResourceFS Source Adapters.

mod filesystem;

#[cfg(feature = "test-support")]
pub use filesystem::TestDeliveryGate;
pub use filesystem::{
    BackingPathVisibility, ClientRoot, FilesystemSource, LaunchRoot, LaunchRootSource,
    RootAcquisition, RootRefresh, RootRefreshOutcome,
};
