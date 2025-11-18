mod command;
mod flake;
mod hostname;
mod types;

pub(crate) use command::NixCommand;
pub use flake::Flake;
pub use hostname::get_hostname;
pub use types::{ActivationMode, Host, HostMetadata, RebootPolicy, StorePath, SystemType};
