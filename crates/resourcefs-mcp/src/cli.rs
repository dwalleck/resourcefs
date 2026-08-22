use std::{
    fmt,
    io::{self, Write},
    path::PathBuf,
    str::FromStr,
};

use clap::{Args, Parser, Subcommand, error::ErrorKind};
use resourcefs_core::WorkspaceRootId;
use resourcefs_sources::LaunchRoot;

use crate::{
    launch::{LaunchError, LaunchErrorKind, LaunchPlan},
    profile, profile_schema_json, server,
};

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
    /// Strict Server Profile supplying launch authority and policy.
    #[arg(
        long,
        value_name = "PATH",
        required_unless_present = "roots",
        conflicts_with_all = ["roots", "primary_root"]
    )]
    config: Option<PathBuf>,

    /// Declare one or more launch Workspace Roots as ID=PATH.
    #[arg(
        long = "root",
        value_name = "ID=PATH",
        required_unless_present = "config",
        conflicts_with = "config"
    )]
    roots: Vec<RootArgument>,

    /// Select the launch root used for relative Path References.
    #[arg(long, value_name = "ID", requires = "roots", conflicts_with = "config")]
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
        Self::read_only(root.id, root.path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliOutcome {
    Success,
    UsageError,
    RequiredUnavailable,
    InternalFailure,
}

impl CliOutcome {
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::UsageError => 2,
            Self::RequiredUnavailable => 3,
            Self::InternalFailure => 1,
        }
    }
}

#[derive(Debug)]
pub struct CliFailure {
    outcome: CliOutcome,
    diagnostic: Option<String>,
}

impl CliFailure {
    fn configuration(error: impl fmt::Display) -> Self {
        Self::new(CliOutcome::UsageError, error)
    }

    fn internal(error: impl fmt::Display) -> Self {
        Self::new(CliOutcome::InternalFailure, error)
    }

    fn silent_internal() -> Self {
        Self {
            outcome: CliOutcome::InternalFailure,
            diagnostic: None,
        }
    }

    fn from_launch(error: LaunchError) -> Self {
        let outcome = match error.kind() {
            LaunchErrorKind::Configuration => CliOutcome::UsageError,
            LaunchErrorKind::RequiredUnavailable => CliOutcome::RequiredUnavailable,
            LaunchErrorKind::Internal => CliOutcome::InternalFailure,
        };
        Self::new(outcome, error)
    }

    fn new(outcome: CliOutcome, diagnostic: impl fmt::Display) -> Self {
        Self {
            outcome,
            diagnostic: Some(bounded_diagnostic(&diagnostic.to_string(), 4_096)),
        }
    }

    pub const fn exit_code(&self) -> u8 {
        self.outcome.exit_code()
    }

    pub fn report(&self) {
        if let Some(diagnostic) = &self.diagnostic {
            eprintln!("resourcefs: {diagnostic}");
        }
    }
}

impl fmt::Display for CliFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            self.diagnostic
                .as_deref()
                .unwrap_or("ResourceFS failed after reporting its diagnostic"),
        )
    }
}

impl std::error::Error for CliFailure {}

pub async fn run_cli() -> Result<CliOutcome, CliFailure> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let outcome = match error.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => CliOutcome::Success,
                _ => CliOutcome::UsageError,
            };
            error.print().map_err(CliFailure::internal)?;
            return Ok(outcome);
        }
    };
    match cli.command {
        Command::Serve(arguments) => serve(arguments).await,
        Command::Check(arguments) => check(arguments).await,
        Command::Schema => {
            write_schema().map_err(CliFailure::internal)?;
            Ok(CliOutcome::Success)
        }
    }
}

fn write_schema() -> io::Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    output.write_all(profile_schema_json().as_bytes())?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

async fn check(arguments: CheckArgs) -> Result<CliOutcome, CliFailure> {
    let output = profile::check(&arguments.config, arguments.probe)
        .await
        .map_err(CliFailure::configuration)?;
    let stdout = io::stdout();
    let mut destination = stdout.lock();
    destination
        .write_all(output.report().as_bytes())
        .map_err(CliFailure::internal)?;
    destination.write_all(b"\n").map_err(CliFailure::internal)?;
    destination.flush().map_err(CliFailure::internal)?;
    Ok(if output.ok() {
        CliOutcome::Success
    } else {
        CliOutcome::RequiredUnavailable
    })
}

fn bounded_diagnostic(message: &str, limit: usize) -> String {
    if message.len() <= limit {
        return message.to_owned();
    }
    let mut end = limit;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
}

async fn serve(arguments: ServeArgs) -> Result<CliOutcome, CliFailure> {
    let plan = match arguments.config {
        Some(config) => LaunchPlan::from_profile(&config).await,
        None => {
            let roots = arguments.roots.into_iter().map(LaunchRoot::from).collect();
            let primary_selector = arguments
                .primary_root
                .map(|primary| primary.0.as_str().to_owned());
            LaunchPlan::from_cli(roots, primary_selector).await
        }
    }
    .map_err(CliFailure::from_launch)?;
    server::serve(plan).await.map_err(|failure| {
        failure
            .diagnostic()
            .map_or_else(CliFailure::silent_internal, CliFailure::internal)
    })?;
    Ok(CliOutcome::Success)
}
