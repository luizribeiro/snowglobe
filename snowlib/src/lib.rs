//! Snowglobe core library
//!
//! This library contains the business logic for snowglobe, a NixOS cluster
//! orchestration tool. It handles Nix interactions, deployment orchestration,
//! and SSH-based deployments.
//!
//! The library is designed to be used by multiple frontends (CLI, TUI, agents)
//! and contains no terminal-specific I/O.

pub mod error;
pub mod nix;
pub mod deploy;
pub mod push;

// Re-export commonly used types
pub use error::{Result, SnowError};
pub use nix::{SnowContext, Host, HostMetadata, StorePath, DeployTarget};
pub use deploy::{Deployer, ActivationMode, DeployResult};
pub use push::{PushOptions, PushResults};

// Re-export specific implementations
pub use deploy::ssh::SshDeployer;

// Convenience re-exports of main functions
pub use nix::{list_hosts, get_host_metadata, build_host};
pub use push::push_parallel;
