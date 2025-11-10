use clap::Parser;
use miette::{IntoDiagnostic, Result};

mod error;
mod nix;
mod deploy;
mod push;
mod commands;

#[derive(Parser)]
#[command(name = "snow")]
#[command(about = "NixOS cluster orchestration tool", long_about = None)]
struct Cli {
    /// Path to the flake directory
    ///
    /// Can also be set via FLAKE_PATH environment variable.
    /// Precedence: CLI arg > FLAKE_PATH env var > search from current directory
    #[arg(short = 'f', long, global = true, env = "FLAKE_PATH")]
    flake_path: Option<String>,

    #[command(subcommand)]
    command: commands::Commands,
}

fn main() -> Result<()> {
    miette::set_panic_hook();
    let cli = Cli::parse();

    // Ensure we're in a valid flake directory before running any commands
    nix::ensure_flake_root(cli.flake_path.as_deref()).into_diagnostic()?;

    cli.command.run().into_diagnostic()
}
