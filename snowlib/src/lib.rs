//! Snowglobe core library
//!
//! This library contains the business logic for snowglobe, a NixOS cluster
//! orchestration tool. It handles Nix interactions, deployment orchestration,
//! and SSH-based deployments.
//!
//! The library is designed to be used by multiple frontends (CLI, TUI, agents)
//! and contains no terminal-specific I/O.

pub mod backends;
pub mod deploy;
pub mod error;
pub mod hooks;
pub mod host;
pub mod nix;
pub mod observer;
pub mod process;
pub mod reboot;
mod shell;
mod ssh_options;
pub mod sudo;
#[cfg(test)]
mod test_support;

pub use backends::deploy::{
    activate_reporting_failed_units, check_mode_supported, DeployBackend, DeployTarget,
    LocalDeployBackend, SshDeployBackend,
};
pub use backends::hook::{HookBackend, SystemHookBackend};
pub use backends::nix::{NixBackend, SystemNixBackend};
pub use backends::reboot::{
    LocalRebootBackend, RebootBackend, RebootCheck, SystemRebootBackend,
    DEFAULT_REBOOT_MAX_WAIT_SECS,
};
pub use deploy::{
    deploy, phases, DeployConfiguration, DeployContext, DeployResults, DeployStrategy,
    HookExecutor, HostDeploymentResult, PhaseResult, PipelineStep, SkippedPhase,
};
pub use error::Result;
pub use hooks::{HookConfig, HookLocation, HookStage};
pub use host::{get_hostname, Host, HostMetadata};
pub use nix::{ActivationMode, Flake, StorePath, SystemType};
pub use observer::{DeployObserver, HostEvent, LogObserver};
pub use process::OutputMode;
pub use reboot::RebootPolicy;
pub use sudo::{Sudo, SudoPrompt, SudoTerminal};

#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::deploy::{DeploymentFixture, HostFixture, MockDeployBackend, OperationFixture};
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::hook::MockHookBackend;
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::nix::MockNixBackend;
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::reboot::MockRebootBackend;
