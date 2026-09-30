//! ResourceFS MCP and CLI adapter.

// Wall-clock budget policy for this crate's unit tests (rfs-cn1r), loaded once
// here because several test modules use it.
#[cfg(test)]
#[path = "../../resourcefs-core/tests/support/wall_budget.rs"]
mod wall_budget;

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
