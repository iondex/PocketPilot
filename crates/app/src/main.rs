//! Run the CLI and report its already-localized failure with a process exit status.

mod cli;

mod doctor;

mod telemetry;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match cli::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod documentation_tests;
