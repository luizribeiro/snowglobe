pub mod reboot;
pub mod ssh;

#[cfg(debug_assertions)]
pub mod mock;

use crate::error::Result;
use crate::nix::{DeployTarget, StorePath};

/// Activation mode for NixOS configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivationMode {
    /// Activate and make default (switch)
    Switch,
    /// Only set for next boot
    Boot,
    /// Test activation without making permanent
    Test,
    /// Dry run - show what would change
    DryActivate,
}

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

/// Trait for deployment implementations
///
/// Implementations must be Send + Sync to support parallel deployments
pub trait Deployer: Send + Sync {
    /// Copy a nix closure to the target host
    fn copy_closure(&self, target: &DeployTarget, store_path: &StorePath) -> Result<()>;

    /// Activate a configuration on the target host
    fn activate(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<()>;

    /// Deploy (copy + activate) in one operation
    fn deploy(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<DeployResult> {
        self.copy_closure(target, store_path)?;
        self.activate(target, store_path, mode)?;
        Ok(DeployResult::success(format!(
            "Deployed successfully to {}",
            target.hostname
        )))
    }
}
