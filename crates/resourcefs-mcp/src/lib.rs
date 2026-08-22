//! ResourceFS MCP and CLI adapter.

use std::error::Error;

mod cli;
mod profile;
mod render;
mod server;

pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

pub use cli::run_cli;
pub use profile::{
    MAX_ALLOWLIST_ENTRIES, MAX_PROFILE_BYTES, ProfileDocument, ProfileError, ProfileErrorKind,
    profile_schema_json,
};
