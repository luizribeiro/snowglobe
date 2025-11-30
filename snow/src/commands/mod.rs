pub mod bootstrap;
pub mod build;
pub mod list;
pub mod push;
pub mod rebuild;
pub mod status;

use clap::Subcommand;

pub use bootstrap::BootstrapArgs;
pub use build::BuildArgs;
pub use list::ListArgs;
pub use push::PushArgs;
pub use rebuild::RebuildArgs;
pub use status::StatusArgs;

#[derive(Debug, Subcommand)]
pub enum Commands {
    Bootstrap(BootstrapArgs),
    Status(StatusArgs),
    List(ListArgs),
    Build(BuildArgs),
    Push(PushArgs),
    Rebuild(RebuildArgs),
}

impl Commands {
    pub async fn run(self, flake: &snowlib::Flake) -> snowlib::Result<()> {
        match self {
            Commands::Bootstrap(args) => args.run(flake).await,
            Commands::Status(args) => args.run(flake).await,
            Commands::List(args) => args.run(flake).await,
            Commands::Build(args) => args.run(flake).await,
            Commands::Push(args) => args.run(flake).await,
            Commands::Rebuild(args) => args.run(flake).await,
        }
    }
}
