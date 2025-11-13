use clap::Parser;
use miette::{IntoDiagnostic, Result};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod backend_factory;
mod commands;

#[cfg(test)]
mod test_utils;

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

    // Set up tracing with indicatif integration for progress bars
    let indicatif_layer = tracing_indicatif::IndicatifLayer::new();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "snow=info,snowlib=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
        .with(indicatif_layer)
        .init();

    let cli = Cli::parse();

    // Create flake context
    let flake = snowlib::Flake::new(cli.flake_path.as_deref()).into_diagnostic()?;

    cli.command.run(&flake).into_diagnostic()
}
