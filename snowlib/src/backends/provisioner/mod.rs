//! Provisioner backends for image key injection
//!
//! Provisioners handle injecting SSH keys into built images so bootstrapped
//! hosts can be accessed immediately after first boot.
//!
//! ## Available Provisioners
//!
//! - **manual**: Outputs instructions for manually copying keys. Use when:
//!   - Image format is unsupported by automated provisioners
//!   - You need full control over the injection process
//!   - Testing bootstrap without modifying images

mod manual;
#[cfg(any(debug_assertions, test))]
mod mock;

pub use manual::ManualProvisioner;
#[cfg(any(debug_assertions, test))]
pub use mock::MockProvisioner;

use crate::bootstrap::HostKeys;
use crate::error::Result;
use std::path::{Path, PathBuf};

/// Configuration for provisioner operations
#[derive(Debug, Clone, Default)]
pub struct ProvisionerConfig {
    /// Path where SSH keys should be injected in the image (default: /etc/ssh)
    pub ssh_key_path: Option<String>,
}

impl ProvisionerConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ssh_key_path(mut self, path: impl Into<String>) -> Self {
        self.ssh_key_path = Some(path.into());
        self
    }

    pub fn ssh_key_path(&self) -> &str {
        self.ssh_key_path.as_deref().unwrap_or("/etc/ssh")
    }
}

/// Result of a key injection operation
#[derive(Debug)]
pub enum InjectionResult {
    /// Keys were successfully injected into the image
    Success {
        /// Path to the modified image (may be same as input for in-place modification)
        image_path: PathBuf,
    },
    /// Provisioner requires manual key injection
    ManualRequired {
        /// Instructions for manual injection
        instructions: String,
    },
}

/// Backend trait for image provisioning operations
///
/// Provisioners handle injecting SSH keys into built images. Each implementation
/// supports a specific image format and toolchain.
pub trait Provisioner: Send + Sync {
    /// Human-readable name for this provisioner
    fn name(&self) -> &str;

    /// Check if required external tools are available
    fn check_dependencies(&self) -> Result<()>;

    /// Inject SSH keys into the built image
    ///
    /// Returns `InjectionResult::Success` with the path to the modified image,
    /// or `InjectionResult::ManualRequired` with instructions for manual injection.
    fn inject_keys(
        &self,
        image_path: &Path,
        host_keys: &HostKeys,
        config: &ProvisionerConfig,
    ) -> Result<InjectionResult>;

    /// Get instructions for manual deployment
    ///
    /// Returns human-readable instructions for deploying the image manually.
    fn deployment_instructions(&self, output_path: &Path, hostname: &str) -> String;
}

/// Get a provisioner by name
pub fn get_provisioner(name: &str) -> Result<Box<dyn Provisioner>> {
    match name {
        "manual" => Ok(Box::new(ManualProvisioner::new())),
        _ => Err(crate::error::ProvisionerError::UnknownProvisioner {
            name: name.to_string(),
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provisioner_config_defaults() {
        let config = ProvisionerConfig::new();
        assert_eq!(config.ssh_key_path(), "/etc/ssh");
    }

    #[test]
    fn test_provisioner_config_with_custom_path() {
        let config = ProvisionerConfig::new().with_ssh_key_path("/custom/ssh");
        assert_eq!(config.ssh_key_path(), "/custom/ssh");
    }

    #[test]
    fn test_get_provisioner_manual() {
        let provisioner = get_provisioner("manual").unwrap();
        assert_eq!(provisioner.name(), "manual");
    }

    #[test]
    fn test_get_provisioner_unknown() {
        let result = get_provisioner("nonexistent");
        assert!(result.is_err());
    }
}
