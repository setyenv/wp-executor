use std::process::ExitCode;
use wp_executor::cli;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    match cli::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::from(1)
        }
    }
}
