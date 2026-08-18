//! ResourceFS MCP and CLI adapter.

use std::error::Error;

mod cli;
mod render;
mod server;

pub type BoxError = Box<dyn Error + Send + Sync + 'static>;

pub use cli::run_cli;
