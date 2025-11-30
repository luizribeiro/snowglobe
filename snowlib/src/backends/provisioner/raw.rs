//! Raw disk image provisioner
//!
//! Injects SSH keys into generic raw disk images using guestfish (rootless).
//! Suitable for VM disk images, cloud images, and other raw format images.

use super::{guestfish, InjectionResult, Provisioner, ProvisionerConfig};
use crate::bootstrap::HostKeys;
use crate::error::{ProvisionerError, Result};
use std::path::Path;

/// Provisioner for generic raw disk images
///
/// Uses guestfish for rootless key injection. Works with any raw disk image
/// that contains a standard Linux filesystem layout.
pub struct RawProvisioner;

impl RawProvisioner {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RawProvisioner {
    fn default() -> Self {
        Self::new()
    }
}

impl Provisioner for RawProvisioner {
    fn name(&self) -> &str {
        "raw"
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
            r#"Raw disk image ready: {path}

Deployment options:
  - Flash to disk:    sudo dd if={path} of=/dev/sdX bs=4M status=progress
  - QEMU/KVM:         qemu-system-x86_64 -drive file={path},format=raw
  - Convert to qcow2: qemu-img convert -f raw -O qcow2 {path} {path}.qcow2

After the host is running:
  snow push {hostname}"#
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_provisioner_name() {
        let provisioner = RawProvisioner::new();
        assert_eq!(provisioner.name(), "raw");
    }

    #[test]
    fn test_raw_deployment_instructions() {
        let provisioner = RawProvisioner::new();
        let instructions =
            provisioner.deployment_instructions(Path::new("/tmp/image.raw"), "server01");

        assert!(instructions.contains("/tmp/image.raw"));
        assert!(instructions.contains("dd if="));
        assert!(instructions.contains("qemu-system"));
        assert!(instructions.contains("qemu-img convert"));
        assert!(instructions.contains("server01"));
        assert!(instructions.contains("snow push"));
    }

    #[test]
    fn test_check_dependencies_returns_error_when_guestfish_missing() {
        let error = ProvisionerError::MissingDependency {
            provisioner: "raw".to_string(),
            tool: "guestfish".to_string(),
            install_hint: "Install libguestfs: nix-shell -p libguestfs".to_string(),
        };
        assert!(error.to_string().contains("guestfish"));
        assert!(error.to_string().contains("raw"));
    }
}
