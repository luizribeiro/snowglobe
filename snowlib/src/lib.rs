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
pub use nix::ops::{NixOps, SystemNixOps};
pub use nix::{DeployTarget, Host, HostMetadata, RebootPolicy, SnowContext, StorePath};
pub use push::{BlockedHost, DeploymentMode, DeploymentPlan, PushOptions, PushResults};

// Re-export specific implementations
pub use deploy::reboot::{RebootCheck, RebootDetectionConfig, RebootDetector, RebootReason};
pub use deploy::ssh::SshDeployer;

#[cfg(debug_assertions)]
pub use deploy::mock::{
    BuildFixture, DeploymentFixture, HostFixture, MetadataFixture, MockDeployer, OperationFixture,
};

#[cfg(debug_assertions)]
pub use nix::ops::mock::MockNixOps;

// Convenience re-exports of main functions
pub use nix::{build_host, get_host_metadata, list_hosts};
pub use push::{create_deployment_plans, push_parallel};
