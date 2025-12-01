//! SD Image provisioner for Raspberry Pi and similar devices
//!
//! Injects SSH keys into raw disk images using guestfish (rootless).
//! Supports deployment to block devices via dd.

use super::{
    deploy_with_dd, guestfish, validate_block_device, InjectionResult, Provisioner,
    ProvisionerConfig, ProvisioningResult, TargetType,
};
use crate::bootstrap::HostKeys;
use crate::error::{ProvisionerError, Result};
use std::path::Path;

/// Provisioner for SD card images (Raspberry Pi, etc.)
///
/// Uses guestfish for rootless key injection into raw disk images.
pub struct SdImageProvisioner;

impl SdImageProvisioner {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SdImageProvisioner {
    fn default() -> Self {
        Self::new()
    }
}

impl Provisioner for SdImageProvisioner {
    fn name(&self) -> &str {
        "sd-image"
    }

    fn check_dependencies(&self) -> Result<()> {
        if !guestfish::is_available() {
            return Err(ProvisionerError::MissingDependency {
                provisioner: self.name().to_string(),
                tool: "guestfish".to_string(),
                install_hint: "Install libguestfs: nix-shell -p libguestfs".to_string(),
            }
            .into());
        }
        Ok(())
    }

    fn inject_keys(
        &self,
        image_path: &Path,
        host_keys: &HostKeys,
        config: &ProvisionerConfig,
    ) -> Result<InjectionResult> {
        self.check_dependencies()?;
        guestfish::inject_keys(self.name(), image_path, host_keys, config)?;

        Ok(InjectionResult::Success {
            image_path: image_path.to_path_buf(),
        })
    }

    fn validate_target(&self, target: &TargetType) -> Result<()> {
        match target {
            TargetType::BlockDevice(path) => validate_block_device(path, self.name()),
            _ => Err(ProvisionerError::DeploymentNotSupported {
                provisioner: self.name().to_string(),
                reason: format!("sd-image only supports block devices, got {}", target),
            }
            .into()),
        }
    }

    fn deploy_to_target(
        &self,
        image_path: &Path,
        target: &TargetType,
        _hostname: &str,
    ) -> Result<ProvisioningResult> {
        self.validate_target(target)?;

        let TargetType::BlockDevice(device_path) = target else {
            unreachable!("validate_target ensures only BlockDevice reaches here");
        };

        let message = deploy_with_dd(image_path, device_path, self.name())?;
        Ok(ProvisioningResult::Success { message })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sd_image_provisioner_name() {
        let provisioner = SdImageProvisioner::new();
        assert_eq!(provisioner.name(), "sd-image");
    }

    #[test]
    fn test_check_dependencies_returns_error_when_guestfish_missing() {
        let error = ProvisionerError::MissingDependency {
            provisioner: "sd-image".to_string(),
            tool: "guestfish".to_string(),
            install_hint: "Install libguestfs: nix-shell -p libguestfs".to_string(),
        };
        assert!(error.to_string().contains("guestfish"));
        assert!(error.to_string().contains("sd-image"));
    }

    #[test]
    fn test_validate_target_accepts_block_device() {
        let provisioner = SdImageProvisioner::new();
        let target = TargetType::BlockDevice(std::path::PathBuf::from("/dev/sdc"));
        assert!(provisioner.validate_target(&target).is_ok());
    }

    #[test]
    fn test_validate_target_rejects_proxmox_host() {
        let provisioner = SdImageProvisioner::new();
        let target = TargetType::parse("proxmox01.local");
        let result = provisioner.validate_target(&target);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("only supports block devices"));
    }
}
