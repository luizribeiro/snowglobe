//! Snowglobe core library
//!
//! This library contains the business logic for snowglobe, a NixOS cluster
//! orchestration tool. It handles Nix interactions, deployment orchestration,
//! and SSH-based deployments.
//!
//! The library is designed to be used by multiple frontends (CLI, TUI, agents)
//! and contains no terminal-specific I/O.

pub mod backends;
pub mod error;
pub mod nix;
pub mod observer;
pub mod process;
pub mod push;

pub use backends::deploy::{
    DeployBackend, DeployResult, DeployTarget, LocalDeployBackend, SshDeployBackend,
};
pub use backends::local_reboot::LocalRebootBackend;
pub use backends::nix::{NixBackend, SystemNixBackend};
pub use backends::reboot::{RebootBackend, RebootCheck, SystemRebootBackend};
pub use error::Result;
pub use nix::{get_hostname, ActivationMode, Flake, StorePath};
pub use observer::PushObserver;
pub use push::{push, PushConfiguration, PushContext, PushResults, PushStrategy};

#[cfg(debug_assertions)]
pub use backends::mock::{
    DeploymentFixture, HostFixture, MockDeployBackend, MockNixBackend, OperationFixture,
};
