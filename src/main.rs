//! Tibber CLI entry point.

use clap::Parser;
use tibber_cli::Cli;

fn main() -> anyhow::Result<()> {
    // Load the .env file if it exists
    dotenvy::dotenv().ok();

    let cli = Cli::parse();
    tibber_cli::run(&cli)?;

    Ok(())
}
