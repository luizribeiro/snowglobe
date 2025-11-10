pub mod status;
pub mod push;
pub mod list;

use clap::Subcommand;

pub use status::StatusArgs;
pub use list::ListArgs;
pub use push::PushArgs;

#[derive(Debug, Subcommand)]
pub enum Commands {
    Status(StatusArgs),
    List(ListArgs),
    Push(PushArgs),
}

impl Commands {
    pub fn run(self, ctx: &crate::nix::SnowContext) -> crate::error::Result<()> {
        match self {
            Commands::Status(args) => args.run(ctx),
            Commands::List(args) => args.run(ctx),
            Commands::Push(args) => args.run(ctx),
        }
    }
}
