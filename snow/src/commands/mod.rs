pub mod build;
pub mod list;
pub mod push;
pub mod rebuild;
pub mod status;

use clap::Subcommand;

pub use build::BuildArgs;
pub use list::ListArgs;
pub use push::PushArgs;
pub use rebuild::RebuildArgs;
pub use status::StatusArgs;

#[derive(Debug, Subcommand)]
pub enum Commands {
    Status(StatusArgs),
    List(ListArgs),
    Build(BuildArgs),
    Push(PushArgs),
    Rebuild(RebuildArgs),
}

impl Commands {
    pub async fn run(self, flake: &snowlib::Flake) -> snowlib::Result<()> {
        match self {
            Commands::Status(args) => args.run(flake),
            Commands::List(args) => args.run(flake),
            Commands::Build(args) => args.run(flake),
            Commands::Push(args) => args.run(flake),
            Commands::Rebuild(args) => args.run(flake).await,
        }
    }
}
