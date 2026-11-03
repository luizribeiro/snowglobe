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
pub mod plan;
pub mod process;
pub mod push;
pub mod reboot;
mod ssh_options;

pub use backends::deploy::{DeployBackend, DeployTarget, LocalDeployBackend, SshDeployBackend};
pub use backends::hook::{HookBackend, SystemHookBackend};
pub use backends::keygen::{KeyGenBackend, SshKeyPair, SystemKeyGenBackend};
pub use backends::nix::{NixBackend, SystemNixBackend};
pub use backends::plan::{PlanBackend, SystemPlanBackend};
pub use backends::provisioner::{
    get_provisioner, InjectionResult, ManualProvisioner, Provisioner, ProvisionerConfig,
    ProvisioningResult, ProxmoxVmaProvisioner, RawProvisioner, SdImageProvisioner, TargetType,
};
pub use backends::reboot::{LocalRebootBackend, RebootBackend, RebootCheck, SystemRebootBackend};
pub use backends::sops::{SopsBackend, SopsUpdateResult, SystemSopsBackend};
pub use bootstrap::BootstrapConfig;
pub use error::{BootstrapError, ProvisionerError, Result};
pub use hooks::{HookConfig, HookLocation, HookStage};
pub use host::{get_hostname, Host, HostMetadata};
pub use nix::{ActivationMode, Flake, StorePath, SystemType};
pub use observer::{HostEvent, LogObserver, PushObserver};
pub use process::OutputMode;
pub use push::{
    push, HookExecutor, HostDeploymentResult, PhaseResult, PipelineStep, PushConfiguration,
    PushContext, PushResults, PushStrategy, SkippedPhase,
};
pub use reboot::RebootPolicy;

#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::deploy::{DeploymentFixture, HostFixture, MockDeployBackend, OperationFixture};
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::hook::MockHookBackend;
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::keygen::MockKeyGenBackend;
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::nix::MockNixBackend;
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::plan::{MockPlanBackend, MockPlanCall};
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::provisioner::MockProvisioner;
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::reboot::MockRebootBackend;
#[cfg(any(debug_assertions, feature = "test-utils"))]
pub use backends::sops::MockSopsBackend;
