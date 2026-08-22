use std::{
    io::{self, Write},
    path::PathBuf,
    str::FromStr,
};

use clap::{Args, Parser, Subcommand};
use resourcefs_core::WorkspaceRootId;
use resourcefs_sources::{BackingPathVisibility, FilesystemSource, LaunchRoot, LaunchRootSource};

use crate::{BoxError, profile_schema_json, server};

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
    /// Print the current strict Server Profile JSON Schema.
    Schema,
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Declare one or more launch Workspace Roots as ID=PATH.
    #[arg(long = "root", required = true, value_name = "ID=PATH")]
    roots: Vec<RootArgument>,

    /// Select the launch root used for relative Path References.
    #[arg(long, value_name = "ID")]
    primary_root: Option<RootIdArgument>,
}

#[derive(Debug, Clone)]
struct RootArgument {
    id: WorkspaceRootId,
    path: PathBuf,
}

impl FromStr for RootArgument {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (id, path) = value
            .split_once('=')
            .ok_or_else(|| "Workspace Root must use ID=PATH syntax".to_owned())?;
        let id = WorkspaceRootId::new(id.to_owned()).map_err(|error| error.to_string())?;
        if path.is_empty() {
            return Err("Workspace Root backing path must not be empty".to_owned());
        }
        Ok(Self {
            id,
            path: PathBuf::from(path),
        })
    }
}

#[derive(Debug, Clone)]
struct RootIdArgument(WorkspaceRootId);

impl FromStr for RootIdArgument {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        WorkspaceRootId::new(value.to_owned())
            .map(Self)
            .map_err(|error| error.to_string())
    }
}

impl From<RootArgument> for LaunchRoot {
    fn from(root: RootArgument) -> Self {
        Self {
            id: root.id,
            path: root.path,
        }
    }
}

pub async fn run_cli() -> Result<(), BoxError> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(arguments) => serve(arguments).await,
        Command::Schema => write_schema(),
    }
}

fn write_schema() -> Result<(), BoxError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    output.write_all(profile_schema_json().as_bytes())?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

async fn serve(arguments: ServeArgs) -> Result<(), BoxError> {
    let roots = arguments.roots.into_iter().map(LaunchRoot::from).collect();
    let primary_selector = arguments
        .primary_root
        .map(|primary| primary.0.as_str().to_owned());
    let source = FilesystemSource::new(
        LaunchRootSource::Cli(roots),
        primary_selector,
        BackingPathVisibility::Hidden,
    )
    .await?;
    server::serve(source).await
}
