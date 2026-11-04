pub mod bootstrap;
pub mod build;
pub mod list;
pub mod plan;
pub mod push;
pub mod rebuild;
pub mod status;

use clap::Subcommand;

pub use bootstrap::BootstrapArgs;
pub use build::BuildArgs;
pub use list::ListArgs;
pub use plan::PlanArgs;
pub use push::PushArgs;
pub use rebuild::RebuildArgs;
pub use status::StatusArgs;

/// The `dry_run`, `switch`, `boot` and `test` flags, of which a command takes one at most
fn activation_modes() -> clap::ArgGroup {
    clap::ArgGroup::new("activation_mode").args(["dry_run", "switch", "boot", "test"])
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Bootstrap(BootstrapArgs),
    Status(StatusArgs),
    List(ListArgs),
    Build(BuildArgs),
    Plan(PlanArgs),
    Push(PushArgs),
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
            Commands::Bootstrap(args) => args.run(flake).await,
            Commands::Status(args) => args.run(flake).await,
            Commands::List(args) => args.run(flake).await,
            Commands::Build(args) => args.run(flake).await,
            Commands::Plan(args) => args.run(flake).await,
            Commands::Push(args) => args.run(flake, log_file).await,
            Commands::Rebuild(args) => args.run(flake).await,
        }
    }
}
