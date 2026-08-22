use std::{
    io::{self, Write},
    path::PathBuf,
    str::FromStr,
};

use clap::{Args, Parser, Subcommand, error::ErrorKind};
use resourcefs_core::{ServerLimits, WorkspaceRootId};
use resourcefs_sources::{
    BackingPathVisibility, FilesystemSource, LaunchRoot, LaunchRootSource, SESSION_CLEANUP_TTL,
    SessionStorageConfig,
};

use crate::{BoxError, profile, profile_schema_json, server};

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
    /// Validate a Server Profile, optionally probing configured dependencies.
    Check(CheckArgs),
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

#[derive(Debug, Args)]
struct CheckArgs {
    /// Server Profile to validate.
    #[arg(long, value_name = "PATH")]
    config: PathBuf,

    /// Perform one bounded, non-mutating availability probe per source.
    #[arg(long)]
    probe: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliOutcome {
    Success,
    UsageError,
    RequiredUnavailable,
}

impl CliOutcome {
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::UsageError => 2,
            Self::RequiredUnavailable => 3,
        }
    }
}

pub async fn run_cli() -> Result<CliOutcome, BoxError> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let outcome = match error.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => CliOutcome::Success,
                _ => CliOutcome::UsageError,
            };
            error.print()?;
            return Ok(outcome);
        }
    };
    match cli.command {
        Command::Serve(arguments) => {
            serve(arguments).await?;
            Ok(CliOutcome::Success)
        }
        Command::Check(arguments) => check(arguments).await,
        Command::Schema => {
            write_schema()?;
            Ok(CliOutcome::Success)
        }
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

async fn check(arguments: CheckArgs) -> Result<CliOutcome, BoxError> {
    match profile::check(&arguments.config, arguments.probe).await {
        Ok(output) => {
            let stdout = io::stdout();
            let mut destination = stdout.lock();
            destination.write_all(output.report().as_bytes())?;
            destination.write_all(b"\n")?;
            destination.flush()?;
            Ok(if output.ok() {
                CliOutcome::Success
            } else {
                CliOutcome::RequiredUnavailable
            })
        }
        Err(error) => {
            let rendered = error.to_string();
            let diagnostic = bounded_diagnostic(&rendered, 4_096);
            let stderr = io::stderr();
            let mut destination = stderr.lock();
            writeln!(destination, "resourcefs: {diagnostic}")?;
            destination.flush()?;
            Ok(CliOutcome::UsageError)
        }
    }
}

fn bounded_diagnostic(message: &str, limit: usize) -> &str {
    if message.len() <= limit {
        return message;
    }
    let mut end = limit;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    &message[..end]
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
    let session_storage =
        SessionStorageConfig::for_current_user(SESSION_CLEANUP_TTL.as_secs() as i64)?;
    server::serve(source, ServerLimits::default(), session_storage).await
}
