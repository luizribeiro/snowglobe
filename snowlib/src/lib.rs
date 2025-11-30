//! Snowglobe core library
//!
//! This library contains the business logic for snowglobe, a NixOS cluster
//! orchestration tool. It handles Nix interactions, deployment orchestration,
//! and SSH-based deployments.
//!
//! The library is designed to be used by multiple frontends (CLI, TUI, agents)
//! and contains no terminal-specific I/O.

pub mod backends;
pub mod bootstrap;
pub mod error;
pub mod hooks;
pub mod host;
pub mod nix;
pub mod observer;
pub mod process;
pub mod push;
pub mod reboot;

pub use backends::deploy::{
    DeployBackend, DeployResult, DeployTarget, LocalDeployBackend, SshDeployBackend,
};
pub use backends::hook::{HookBackend, SystemHookBackend};
pub use backends::keygen::{KeyGenBackend, SshKeyPair, SystemKeyGenBackend};
pub use backends::nix::{NixBackend, SystemNixBackend};
pub use backends::reboot::{LocalRebootBackend, RebootBackend, RebootCheck, SystemRebootBackend};
pub use error::{BootstrapError, Result};
pub use hooks::{HookConfig, HookLocation, HookStage};
pub use host::{get_hostname, Host, HostMetadata};
pub use nix::{ActivationMode, Flake, StorePath, SystemType};
pub use observer::PushObserver;
pub use process::OutputMode;
pub use push::{
    push, HookExecutor, HostDeploymentResult, PhaseResult, PipelineStep, PushConfiguration,
    PushContext, PushResults, PushStrategy, SkippedPhase,
};
pub use reboot::RebootPolicy;

#[cfg(debug_assertions)]
pub use backends::deploy::{DeploymentFixture, HostFixture, MockDeployBackend, OperationFixture};
#[cfg(debug_assertions)]
pub use backends::hook::MockHookBackend;
#[cfg(debug_assertions)]
pub use backends::keygen::MockKeyGenBackend;
#[cfg(debug_assertions)]
pub use backends::nix::MockNixBackend;
#[cfg(debug_assertions)]
pub use backends::reboot::MockRebootBackend;
