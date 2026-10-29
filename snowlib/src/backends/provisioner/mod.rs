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
//!
//! - **sd-image**: Injects keys into SD card images (Raspberry Pi, etc.)
//!   using guestfish for rootless operation.
//!
//! - **raw**: Injects keys into generic raw disk images (VMs, cloud images)
//!   using guestfish for rootless operation.
//!
//! - **proxmox-vma**: Injects keys into Proxmox VMA backup images by extracting,
//!   modifying with guestfish, and repacking.

mod command;
mod guestfish;
mod manual;
#[cfg(any(debug_assertions, test, feature = "test-utils"))]
mod mock;
mod proxmox_vma;
mod raw;
mod sd_image;

#[cfg(any(debug_assertions, test, feature = "test-utils"))]
pub use command::MockCommandExecutor;
pub use command::{CommandExecutor, CommandOutput, SystemCommandExecutor};

pub use manual::ManualProvisioner;
#[cfg(any(debug_assertions, test, feature = "test-utils"))]
pub use mock::MockProvisioner;
pub use proxmox_vma::ProxmoxVmaProvisioner;
pub use raw::RawProvisioner;
pub use sd_image::SdImageProvisioner;

use crate::backends::deploy::DeployTarget;
use crate::bootstrap::HostKeys;
use crate::error::{ProvisionerError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Target type for deployment operations
#[derive(Debug, Clone)]
pub enum TargetType {
    /// Block device path (e.g., /dev/sdc)
    BlockDevice(PathBuf),
    /// Proxmox host (e.g., proxmox01.local or root@proxmox01.local:22)
    ProxmoxHost(DeployTarget),
}

impl TargetType {
    /// Parse a target string into a TargetType
    ///
    /// - Paths starting with /dev/ are treated as block devices
    /// - Everything else is treated as a Proxmox host (parsed as [user@]hostname[:port])
    pub fn parse(target: &str) -> Self {
        if target.starts_with("/dev/") {
            TargetType::BlockDevice(PathBuf::from(target))
        } else {
            TargetType::ProxmoxHost(DeployTarget::parse(target))
        }
    }
}

impl std::fmt::Display for TargetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetType::BlockDevice(path) => write!(f, "{}", path.display()),
            TargetType::ProxmoxHost(target) => write!(f, "{}", target.ssh_destination()),
        }
    }
}

/// Result of a deployment operation to a target
#[derive(Debug)]
pub enum ProvisioningResult {
    /// Deployment completed successfully
    Success {
        /// Message describing what was deployed
        message: String,
    },
    /// Deployment is not supported by this provisioner
    NotSupported {
        /// Explanation of why deployment isn't supported
        reason: String,
    },
}

/// Configuration for provisioner operations
#[derive(Debug, Clone, Default)]
pub struct ProvisionerConfig {
    /// Path where SSH keys should be injected in the image (default: /etc/ssh)
    pub ssh_key_path: Option<String>,
    /// Root filesystem label (e.g., "nixos", "NIXOS_SD")
    pub root_fs_label: Option<String>,
    /// Original root filesystem device path from NixOS config (for error messages)
    pub root_fs_device: Option<String>,
}

impl ProvisionerConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ssh_key_path(mut self, path: impl Into<String>) -> Self {
        self.ssh_key_path = Some(path.into());
        self
    }

    pub fn with_root_fs_label(mut self, label: impl Into<String>) -> Self {
        self.root_fs_label = Some(label.into());
        self
    }

    pub fn with_root_fs_device(mut self, device: impl Into<String>) -> Self {
        self.root_fs_device = Some(device.into());
        self
    }

    pub fn ssh_key_path(&self) -> &str {
        self.ssh_key_path.as_deref().unwrap_or("/etc/ssh")
    }

    pub fn root_fs_label(&self) -> Option<&str> {
        self.root_fs_label.as_deref()
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

    /// Validate that a target is supported by this provisioner
    ///
    /// Returns Ok(()) if the target is valid, or an error explaining why not.
    fn validate_target(&self, target: &TargetType) -> Result<()> {
        Err(ProvisionerError::DeploymentNotSupported {
            provisioner: self.name().to_string(),
            reason: format!("{} does not support deployment to {}", self.name(), target),
        }
        .into())
    }

    /// Deploy the provisioned image to a target
    ///
    /// This is called after inject_keys() succeeds. The image_path points to
    /// the provisioned image ready for deployment.
    fn deploy_to_target(
        &self,
        _image_path: &Path,
        target: &TargetType,
        _hostname: &str,
    ) -> Result<ProvisioningResult> {
        Ok(ProvisioningResult::NotSupported {
            reason: format!("{} does not support deployment to {}", self.name(), target),
        })
    }
}

/// Get a provisioner by name
pub fn get_provisioner(name: &str) -> Result<Box<dyn Provisioner>> {
    match name {
        "manual" => Ok(Box::new(ManualProvisioner::new())),
        "sd-image" => Ok(Box::new(SdImageProvisioner::new())),
        "raw" => Ok(Box::new(RawProvisioner::new())),
        "proxmox-vma" => Ok(Box::new(ProxmoxVmaProvisioner::new())),
        #[cfg(any(debug_assertions, test, feature = "test-utils"))]
        "mock" => Ok(Box::new(MockProvisioner::new("mock"))),
        _ => Err(crate::error::ProvisionerError::UnknownProvisioner {
            name: name.to_string(),
        }
        .into()),
    }
}

/// Validate a block device path
///
/// Checks that the path is in /dev/ and doesn't contain path traversal.
/// Does not check existence - the dd command will fail clearly if the device doesn't exist.
pub(super) fn validate_block_device(path: &Path, provisioner_name: &str) -> Result<()> {
    let path_str = path.to_string_lossy();

    // Must start with /dev/
    if !path_str.starts_with("/dev/") {
        return Err(ProvisionerError::InvalidTarget {
            provisioner: provisioner_name.to_string(),
            target: path.display().to_string(),
            reason: "Block device path must start with /dev/".to_string(),
        }
        .into());
    }

    // Reject path traversal attempts
    if path_str.contains("..") {
        return Err(ProvisionerError::InvalidTarget {
            provisioner: provisioner_name.to_string(),
            target: path.display().to_string(),
            reason: "Block device path must not contain '..'".to_string(),
        }
        .into());
    }

    Ok(())
}

/// Deploy a disk image to a block device using dd
pub(super) fn deploy_with_dd(
    image_path: &Path,
    device_path: &Path,
    provisioner_name: &str,
) -> Result<String> {
    let output = Command::new("sudo")
        .arg("dd")
        .arg(format!("if={}", image_path.display()))
        .arg(format!("of={}", device_path.display()))
        .arg("bs=4M")
        .arg("status=progress")
        .arg("conv=fsync")
        .output()
        .map_err(|e| ProvisionerError::DeploymentFailed {
            provisioner: provisioner_name.to_string(),
            reason: format!("Failed to execute dd: {}", e),
        })?;

    if !output.status.success() {
        return Err(ProvisionerError::DeploymentFailed {
            provisioner: provisioner_name.to_string(),
            reason: format!(
                "dd failed with exit code {}: {}",
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stderr)
            ),
        }
        .into());
    }

    Ok(format!("Image flashed to {}", device_path.display()))
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
    fn test_get_provisioner_sd_image() {
        let provisioner = get_provisioner("sd-image").unwrap();
        assert_eq!(provisioner.name(), "sd-image");
    }

    #[test]
    fn test_get_provisioner_raw() {
        let provisioner = get_provisioner("raw").unwrap();
        assert_eq!(provisioner.name(), "raw");
    }

    #[test]
    fn test_get_provisioner_proxmox_vma() {
        let provisioner = get_provisioner("proxmox-vma").unwrap();
        assert_eq!(provisioner.name(), "proxmox-vma");
    }

    #[test]
    fn test_get_provisioner_unknown() {
        let result = get_provisioner("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_target_type_parse_block_device() {
        let target = TargetType::parse("/dev/sdc");
        assert!(matches!(target, TargetType::BlockDevice(_)));
        assert_eq!(target.to_string(), "/dev/sdc");
    }

    #[test]
    fn test_target_type_parse_proxmox_host() {
        let target = TargetType::parse("proxmox01.local");
        assert!(matches!(target, TargetType::ProxmoxHost(_)));
        assert_eq!(target.to_string(), "proxmox01.local");
    }

    #[test]
    fn test_target_type_parse_proxmox_host_with_user() {
        let target = TargetType::parse("root@proxmox01.local");
        assert!(matches!(target, TargetType::ProxmoxHost(_)));
        assert_eq!(target.to_string(), "root@proxmox01.local");
    }

    #[test]
    fn test_target_type_parse_proxmox_host_with_port() {
        let target = TargetType::parse("admin@proxmox01.local:2222");
        if let TargetType::ProxmoxHost(deploy_target) = target {
            assert_eq!(deploy_target.user, Some("admin".to_string()));
            assert_eq!(deploy_target.hostname, "proxmox01.local");
            assert_eq!(deploy_target.port, Some(2222));
        } else {
            panic!("Expected ProxmoxHost");
        }
    }

    #[test]
    fn test_target_type_parse_proxmox_ip() {
        let target = TargetType::parse("192.168.1.50");
        assert!(matches!(target, TargetType::ProxmoxHost(_)));
        assert_eq!(target.to_string(), "192.168.1.50");
    }

    #[test]
    fn test_provisioning_result_success() {
        let result = ProvisioningResult::Success {
            message: "Deployed to /dev/sdc".to_string(),
        };
        match result {
            ProvisioningResult::Success { message } => {
                assert!(message.contains("/dev/sdc"));
            }
            _ => panic!("Expected Success"),
        }
    }

    #[test]
    fn test_provisioning_result_not_supported() {
        let result = ProvisioningResult::NotSupported {
            reason: "Manual provisioner doesn't support deployment".to_string(),
        };
        match result {
            ProvisioningResult::NotSupported { reason } => {
                assert!(reason.contains("Manual"));
            }
            _ => panic!("Expected NotSupported"),
        }
    }

    #[test]
    fn test_validate_block_device_accepts_valid_path() {
        let result = validate_block_device(Path::new("/dev/sdc"), "test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_block_device_rejects_non_dev_path() {
        let result = validate_block_device(Path::new("/tmp/disk.img"), "test");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_block_device_rejects_path_traversal() {
        let result = validate_block_device(Path::new("/dev/../etc/passwd"), "test");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains(".."));
    }
}
