use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use rebalance_app::config::Config;

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "rebalance",
    about = "Retirement portfolio rebalancer",
    version = "2.0.0"
)]
struct Cli {
    /// Path to config.toml
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Override the contribution amount from config
    #[arg(short = 'a', long)]
    contribution: Option<f64>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show current portfolio vs targets, drift warnings, and next contribution
    Status,

    /// Simulate future contributions and show a projection table
    Project {
        /// Number of months to project (2 contributions per month)
        #[arg(short, long)]
        months: usize,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let cli = Cli::parse();

    let cfg = Config::load(&cli.config)
        .with_context(|| format!("Failed to load config from {:?}", cli.config))?;

    match cli.command {
        Command::Status => {
            rebalance_app::status::run(&cfg, cli.contribution)?;
        }
        Command::Project { months } => {
            rebalance_app::project::run(&cfg, months, cli.contribution)?;
        }
    }

    Ok(())
}
