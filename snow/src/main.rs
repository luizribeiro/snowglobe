use clap::{Parser, Subcommand};
use anyhow::Result;

mod nix;
mod deploy;
mod push;
mod commands;

#[derive(Parser)]
#[command(name = "snow")]
#[command(about = "NixOS cluster orchestration tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show cluster status
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Deploy configurations to hosts
    Push {
        /// Hosts to push (one or more)
        hosts: Vec<String>,

        /// Dry run - show what would change without activating
        #[arg(long)]
        dry_run: bool,

        /// Only set for next boot (don't activate now)
        #[arg(long)]
        boot: bool,

        /// Maximum number of hosts to push in parallel
        #[arg(long, default_value = "3")]
        concurrency: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Status { json } => commands::status::run(json),
        Commands::Push {
            hosts,
            dry_run,
            boot,
            concurrency,
        } => commands::push::run(hosts, dry_run, boot, concurrency),
    }
}
