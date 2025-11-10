pub mod list;
pub mod push;
pub mod status;

use clap::Subcommand;

pub use list::ListArgs;
pub use push::PushArgs;
pub use status::StatusArgs;

#[derive(Debug, Subcommand)]
pub enum Commands {
    Status(StatusArgs),
    List(ListArgs),
    Push(PushArgs),
}

impl Commands {
    pub fn run(self, ctx: &snowlib::SnowContext) -> snowlib::Result<()> {
        match self {
            Commands::Status(args) => args.run(ctx),
            Commands::List(args) => args.run(ctx),
            Commands::Push(args) => args.run(ctx),
        }
    }
}
