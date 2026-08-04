//! `supermem` command-line entry point.

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = super_mem::Cli::parse();
    super_mem::run(cli).await
}
