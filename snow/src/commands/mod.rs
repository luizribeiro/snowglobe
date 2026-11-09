#[cfg(feature = "unstable")]
pub mod bootstrap;
pub mod build;
pub mod deploy;
pub mod list;
#[cfg(feature = "unstable")]
pub mod plan;
pub mod rebuild;
#[cfg(feature = "unstable")]
pub mod status;

use clap::Subcommand;

#[cfg(feature = "unstable")]
pub use bootstrap::BootstrapArgs;
pub use build::BuildArgs;
pub use deploy::DeployArgs;
pub use list::ListArgs;
#[cfg(feature = "unstable")]
pub use plan::PlanArgs;
pub use rebuild::RebuildArgs;
#[cfg(feature = "unstable")]
pub use status::StatusArgs;

/// The `dry_run`, `switch`, `boot` and `test` flags, of which a command takes one at most
fn activation_modes() -> clap::ArgGroup {
    clap::ArgGroup::new("activation_mode").args(["dry_run", "switch", "boot", "test"])
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[cfg(feature = "unstable")]
    Bootstrap(BootstrapArgs),
    #[cfg(feature = "unstable")]
    Status(StatusArgs),
    List(ListArgs),
    Build(BuildArgs),
    #[cfg(feature = "unstable")]
    Plan(PlanArgs),
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
            #[cfg(feature = "unstable")]
            Commands::Bootstrap(args) => args.run(flake).await,
            #[cfg(feature = "unstable")]
            Commands::Status(args) => args.run(flake).await,
            Commands::List(args) => args.run(flake).await,
            Commands::Build(args) => args.run(flake).await,
            #[cfg(feature = "unstable")]
            Commands::Plan(args) => args.run(flake).await,
            Commands::Deploy(args) => args.run(flake, log_file).await,
            Commands::Rebuild(args) => args.run(flake).await,
        }
    }
}
