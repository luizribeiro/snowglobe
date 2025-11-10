//! Snowglobe core library
//!
//! This library contains the business logic for snowglobe, a NixOS cluster
//! orchestration tool. It handles Nix interactions, deployment orchestration,
//! and SSH-based deployments.
//!
//! The library is designed to be used by multiple frontends (CLI, TUI, agents)
//! and contains no terminal-specific I/O.

pub mod deploy;
pub mod error;
pub mod nix;
pub mod push;

// Re-export commonly used types
pub use deploy::{ActivationMode, DeployResult, Deployer};
pub use error::{Result, SnowError};
pub use nix::{DeployTarget, Host, HostMetadata, SnowContext, StorePath};
pub use push::{PushOptions, PushResults};

// Re-export specific implementations
pub use deploy::ssh::SshDeployer;

// Convenience re-exports of main functions
pub use nix::{build_host, get_host_metadata, list_hosts};
pub use push::push_parallel;
