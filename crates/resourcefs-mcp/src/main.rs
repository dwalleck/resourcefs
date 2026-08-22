use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match resourcefs_mcp::run_cli().await {
        Ok(outcome) => ExitCode::from(outcome.exit_code()),
        Err(error) => {
            eprintln!("resourcefs: {error}");
            ExitCode::FAILURE
        }
    }
}
