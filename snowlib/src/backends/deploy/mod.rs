mod common;
mod local;
#[cfg(any(debug_assertions, test))]
mod mock;
mod ssh;
mod target;

pub use common::*;
pub use local::LocalDeployBackend;
#[cfg(any(debug_assertions, test))]
pub use mock::{
    BuildFixture, DeploymentFixture, FailureSpec, HostFixture, MetadataFixture, MockDeployBackend,
    OperationFixture,
};
pub use ssh::SshDeployBackend;
pub use target::DeployTarget;

use crate::error::Result;
use crate::nix::{ActivationMode, StorePath};
use async_trait::async_trait;

/// Result of a deployment operation
#[derive(Debug, Clone)]
pub struct DeployResult {
    pub success: bool,
    pub message: String,
}

impl DeployResult {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
        }
    }
}

/// Trait for deployment backend implementations
///
/// Implementations must be Send + Sync to support parallel deployments
#[async_trait]
pub trait DeployBackend: Send + Sync {
    /// Copy a nix closure to the target host
    async fn copy_closure(&self, target: &DeployTarget, store_path: &StorePath) -> Result<()>;

    /// Activate a configuration on the target host
    async fn activate(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<()>;

    /// Deploy (copy + activate) in one operation
    async fn deploy(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<DeployResult> {
        self.copy_closure(target, store_path).await?;
        self.activate(target, store_path, mode).await?;
        Ok(DeployResult::success(format!(
            "Deployed successfully to {}",
            target.hostname
        )))
    }
}
