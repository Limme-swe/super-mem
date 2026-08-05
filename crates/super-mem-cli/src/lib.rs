//! Command-line and harness integration surface for Super Mem.

mod app;
mod cli;
mod hook;
mod mcp;
mod scope;

pub use app::{run, run_sync};
pub use cli::Cli;

/// Runs the parsed command-line application.
///
/// # Errors
///
/// Returns an error when argument parsing or the selected command fails.
pub async fn run_from<I, T>(arguments: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    use clap::Parser;
    run(Cli::parse_from(arguments)).await
}
