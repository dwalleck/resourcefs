use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match resourcefs_mcp::run_cli().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("resourcefs: {error}");
            ExitCode::FAILURE
        }
    }
}
