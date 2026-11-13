use clap::Parser;
use std::fs::File;
use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod backend_factory;
mod commands;
mod cursor;
mod fatal;
mod filter_helpers;
mod host_selection;
mod marks;
mod plural;
mod prompt;
mod signals;
mod terminal_warnings;

#[cfg(test)]
mod test_utils;

#[derive(Parser)]
#[command(name = "snow", version)]
#[command(about = "Deploy NixOS and nix-darwin hosts, in phases", long_about = None)]
struct Cli {
    /// Path to the flake directory or its flake.nix (defaults to searching up from the current directory)
    #[arg(short = 'f', long, global = true, env = "SNOW_FLAKE")]
    flake_path: Option<String>,

    #[command(subcommand)]
    command: commands::Commands,
}

fn default_env_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "snow=info,snowlib=info".into())
}

/// The log captures hook and nix output, so only its owner may read it
fn create_log_file() -> std::io::Result<(File, std::path::PathBuf)> {
    tempfile::Builder::new()
        .prefix("snow-")
        .suffix(".log")
        .tempfile()?
        .keep()
        .map_err(|error| error.error)
}

fn setup_logging(is_interactive: bool) -> Option<std::path::PathBuf> {
    if is_interactive {
        let (log_file, log_path) = create_log_file().ok()?;
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
            .with(terminal_warnings::layer(
                indicatif_layer.get_stderr_writer(),
            ))
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
async fn main() -> ExitCode {
    miette::set_panic_hook();

    let cli = Cli::parse();
    let is_interactive = std::io::stderr().is_terminal();
    let log_file = setup_logging(is_interactive);

    // Every block ends with one blank line, this one too, so whatever comes
    // next (even a fatal error) starts after a blank line
    if let Some(ref path) = log_file {
        eprintln!("Logs: {}\n", path.display());
    }

    if is_interactive {
        if let Err(error) = signals::exit_on_signals() {
            tracing::warn!("Ctrl-C may leave the cursor hidden: {error}");
        }
    }
    let _cursor = cursor::HiddenCursor::new(cursor::Cursor::stderr());

    match run(cli, log_file.as_deref()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", fatal::render(&error));
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli, log_file: Option<&std::path::Path>) -> snowlib::Result<()> {
    let flake = snowlib::Flake::new(cli.flake_path.as_deref())?;
    cli.command.run(&flake, log_file).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_files_get_distinct_paths() {
        let (_, first) = create_log_file().unwrap();
        let (_, second) = create_log_file().unwrap();
        std::fs::remove_file(&first).unwrap();
        std::fs::remove_file(&second).unwrap();

        assert_ne!(first, second);
        for path in [&first, &second] {
            let name = path.file_name().unwrap().to_str().unwrap();
            assert!(
                name.starts_with("snow-") && name.ends_with(".log"),
                "{name}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_log_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let (file, path) = create_log_file().unwrap();
        let mode = file.metadata().unwrap().permissions().mode();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(mode & 0o777, 0o600);
    }
}
