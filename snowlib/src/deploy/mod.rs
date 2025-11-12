//! Backward compatibility re-exports for deploy module
//!
//! This module is deprecated. Please use `crate::backends::deploy` instead.

// Re-export from backends for backward compatibility
pub use crate::backends::deploy::{
    ActivationMode, DeployBackend as Deployer, DeployResult, SshDeployBackend,
};
pub use crate::backends::reboot::{
    RebootBackend as RebootOps, RebootCheck, RebootDetectionConfig, RebootReason,
    SystemRebootBackend as RebootDetector,
};

// Legacy module structure for compatibility
#[cfg(debug_assertions)]
pub mod mock {
    pub use crate::backends::mock::{
        BuildFixture, DeploymentFixture, FailureSpec, HostFixture, MetadataFixture,
        MockDeployBackend as MockDeployer, MockRebootBackend as MockRebootOps, OperationFixture,
    };
}

pub mod reboot {
    pub use crate::backends::reboot::{
        RebootBackend as RebootOps, RebootCheck, RebootDetectionConfig, RebootReason,
        SystemRebootBackend as RebootDetector,
    };
}

pub mod ssh {
    pub use crate::backends::deploy::SshDeployBackend as SshDeployer;
}
