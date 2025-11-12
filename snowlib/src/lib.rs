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

// Re-export commonly used types
pub use backends::deploy::{ActivationMode, DeployBackend, DeployResult, SshDeployBackend};
pub use backends::nix::{NixBackend, SystemNixBackend};
pub use backends::reboot::{
    RebootBackend, RebootCheck, RebootDetectionConfig, RebootReason, SystemRebootBackend,
};
pub use error::{Result, SnowError};
pub use nix::{DeployTarget, Host, HostMetadata, RebootPolicy, SnowContext, StorePath};
pub use observer::{CompositeObserver, NoopObserver, PushObserver};
pub use push::{BlockedHost, DeploymentMode, PushConfiguration, PushPlan, PushResults};

#[cfg(debug_assertions)]
pub use backends::mock::{
    BuildFixture, DeploymentFixture, HostFixture, MetadataFixture, MockDeployBackend,
    MockNixBackend, MockRebootBackend, OperationFixture,
};

// Convenience re-exports of main functions
pub use nix::{build_host, get_host_metadata, list_hosts};
pub use push::{create_deployment_plans, push};
