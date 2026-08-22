use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match resourcefs_mcp::run_cli().await {
        Ok(outcome) => ExitCode::from(outcome.exit_code()),
        Err(failure) => {
            failure.report();
            ExitCode::from(failure.exit_code())
        }
    }
}
