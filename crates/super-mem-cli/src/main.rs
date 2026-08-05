//! `supermem` command-line entry point.

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = super_mem::Cli::parse();
    super_mem::run_sync(cli)
}
