pub mod build;
pub mod deploy;
pub mod list;
pub mod rebuild;

use clap::Subcommand;

pub use build::BuildArgs;
pub use deploy::DeployArgs;
pub use list::ListArgs;
pub use rebuild::RebuildArgs;

/// The `dry_run`, `switch`, `boot` and `test` flags, of which a command takes one at most
fn activation_modes() -> clap::ArgGroup {
    clap::ArgGroup::new("activation_mode").args(["dry_run", "switch", "boot", "test"])
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    List(ListArgs),
    Build(BuildArgs),
    Deploy(DeployArgs),
    Rebuild(RebuildArgs),
}

impl Commands {
    /// `log_file` is where logs go; without one they go to stderr
    pub async fn run(
        self,
        flake: &snowlib::Flake,
        log_file: Option<&std::path::Path>,
    ) -> snowlib::Result<()> {
        match self {
            Commands::List(args) => args.run(flake).await,
            Commands::Build(args) => args.run(flake).await,
            Commands::Deploy(args) => args.run(flake, log_file).await,
            Commands::Rebuild(args) => args.run(flake).await,
        }
    }
}
