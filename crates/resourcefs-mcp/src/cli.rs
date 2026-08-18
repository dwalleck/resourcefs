use std::{error::Error, fmt, path::PathBuf, str::FromStr, sync::Arc};

use clap::{Args, Parser, Subcommand};
use resourcefs_core::{RootName, SourceAdapter};
use resourcefs_sources::FilesystemSource;

use crate::{BoxError, server};

#[derive(Debug, Parser)]
#[command(
    name = "resourcefs",
    version,
    about = "Path-shaped Resources over local stdio MCP"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve ResourceFS over standard input and standard output.
    Serve(ServeArgs),
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Declare the one launch Workspace Root as NAME=PATH.
    #[arg(long = "root", required = true, value_name = "NAME=PATH")]
    roots: Vec<RootArgument>,

    /// Name the launch root used for relative Path References.
    #[arg(long, value_name = "NAME")]
    primary_root: RootNameArgument,
}

#[derive(Debug, Clone)]
struct RootArgument {
    name: RootName,
    path: PathBuf,
}

impl FromStr for RootArgument {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (name, path) = value
            .split_once('=')
            .ok_or_else(|| "Workspace Root must use NAME=PATH syntax".to_owned())?;
        let name = RootName::new(name.to_owned()).map_err(|error| error.to_string())?;
        if path.is_empty() {
            return Err("Workspace Root backing path must not be empty".to_owned());
        }
        Ok(Self {
            name,
            path: PathBuf::from(path),
        })
    }
}

#[derive(Debug, Clone)]
struct RootNameArgument(RootName);

impl FromStr for RootNameArgument {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        RootName::new(value.to_owned())
            .map(Self)
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug)]
struct CliError(String);

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CliError {}

pub async fn run_cli() -> Result<(), BoxError> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(arguments) => serve(arguments).await,
    }
}

async fn serve(arguments: ServeArgs) -> Result<(), BoxError> {
    let [root] = <[RootArgument; 1]>::try_from(arguments.roots).map_err(|roots: Vec<_>| {
        CliError(format!(
            "exactly one launch Workspace Root is required; received {}",
            roots.len()
        ))
    })?;
    if root.name != arguments.primary_root.0 {
        return Err(CliError(format!(
            "Primary Workspace Root '{}' does not match configured root '{}'",
            arguments.primary_root.0, root.name
        ))
        .into());
    }

    let root_name = root.name;
    let source = FilesystemSource::new(root_name.clone(), root.path).await?;
    let source: Arc<dyn SourceAdapter> = Arc::new(source);
    server::serve(source, root_name).await
}
