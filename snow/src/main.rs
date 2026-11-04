use clap::Parser;
use miette::{IntoDiagnostic, Result};
use std::fs::File;
use std::io::IsTerminal;
use std::sync::Arc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod backend_factory;
mod commands;
mod error_summary;
mod filter_helpers;
mod host_selection;
mod marks;
mod plural;
mod prompt;

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

fn default_env_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "snow=info,snowlib=info".into())
}

fn setup_logging(is_interactive: bool) -> Option<std::path::PathBuf> {
    if is_interactive {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let log_path = std::path::PathBuf::from(format!("/tmp/snow-{}.log", timestamp));

        let log_file = File::create(&log_path).ok()?;
        let file_writer = Arc::new(log_file);

        let indicatif_layer =
            tracing_indicatif::IndicatifLayer::new().with_max_progress_bars(100, None);

        tracing_subscriber::registry()
            .with(default_env_filter())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(move || Arc::clone(&file_writer)),
            )
            .with(indicatif_layer)
            .init();

        Some(log_path)
    } else {
        let indicatif_layer =
            tracing_indicatif::IndicatifLayer::new().with_max_progress_bars(100, None);

        tracing_subscriber::registry()
            .with(default_env_filter())
            .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
            .with(indicatif_layer)
            .init();

        None
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    miette::set_panic_hook();

    let is_interactive = std::io::stderr().is_terminal();
    let log_file = setup_logging(is_interactive);

    if let Some(ref path) = log_file {
        eprintln!("Logs: {}", path.display());
    }

    let cli = Cli::parse();

    let flake = snowlib::Flake::new(cli.flake_path.as_deref()).into_diagnostic()?;

    cli.command
        .run(&flake, log_file.as_deref())
        .await
        .into_diagnostic()
}
