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
pub mod push;

pub use backends::deploy::{
    ActivationMode, DeployBackend, DeployResult, DeployTarget, SshDeployBackend,
};
pub use backends::nix::{NixBackend, SystemNixBackend};
pub use backends::reboot::SystemRebootBackend;
pub use error::Result;
pub use nix::{Flake, StorePath};
pub use observer::PushObserver;
pub use push::{push, DeploymentMode, PushConfiguration, PushResults};

#[cfg(debug_assertions)]
pub use backends::mock::{
    DeploymentFixture, HostFixture, MockDeployBackend, MockNixBackend, OperationFixture,
};
