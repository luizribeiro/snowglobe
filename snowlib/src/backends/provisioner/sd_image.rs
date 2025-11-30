//! SD Image provisioner for Raspberry Pi and similar devices
//!
//! Injects SSH keys into raw disk images using guestfish (rootless).

use super::{guestfish, InjectionResult, Provisioner, ProvisionerConfig};
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
        guestfish::inject_keys(self.name(), image_path, host_keys, config.ssh_key_path())?;

        Ok(InjectionResult::Success {
            image_path: image_path.to_path_buf(),
        })
    }

    fn deployment_instructions(&self, output_path: &Path, hostname: &str) -> String {
        let path = output_path.display();
        format!(
            r#"To deploy the SD image:
  sudo dd if={path} of=/dev/sdX bs=4M status=progress

Replace /dev/sdX with your SD card device.

After inserting the SD card and booting:
  snow push {hostname}"#
        )
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
    fn test_sd_image_deployment_instructions() {
        let provisioner = SdImageProvisioner::new();
        let instructions = provisioner.deployment_instructions(Path::new("/tmp/image.img"), "rpi4");

        assert!(instructions.contains("/tmp/image.img"));
        assert!(instructions.contains("dd if="));
        assert!(instructions.contains("rpi4"));
        assert!(instructions.contains("snow push"));
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
}
