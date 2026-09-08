//! ResourceFS MCP and CLI adapter.

use std::error::Error;

mod acquisition;
mod cli;
mod launch;
mod logging;
mod profile;
mod render;
mod server;
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod test_support;

pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

pub use cli::{CliFailure, CliOutcome, run_cli};
pub use profile::{
    MAX_ALLOWLIST_ENTRIES, MAX_PROFILE_BYTES, ProfileDocument, ProfileError, ProfileErrorKind,
    profile_schema_json,
};
