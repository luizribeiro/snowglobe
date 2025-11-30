//! SD Image provisioner for Raspberry Pi and similar devices
//!
//! Injects SSH keys into raw disk images using guestfish (rootless).

use super::{InjectionResult, Provisioner, ProvisionerConfig};
use crate::bootstrap::HostKeys;
use crate::error::{ProvisionerError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Provisioner for SD card images (Raspberry Pi, etc.)
///
/// Uses guestfish for rootless key injection into raw disk images.
/// Falls back to virt-copy-in if guestfish is not available.
pub struct SdImageProvisioner;

#[derive(Debug, Clone, Copy)]
enum InjectionTool {
    Guestfish,
    VirtCopyIn,
}

impl SdImageProvisioner {
    pub fn new() -> Self {
        Self
    }

    fn find_available_tool() -> Option<InjectionTool> {
        if which("guestfish").is_some() {
            return Some(InjectionTool::Guestfish);
        }
        if which("virt-copy-in").is_some() {
            return Some(InjectionTool::VirtCopyIn);
        }
        None
    }

    fn inject_with_guestfish(
        &self,
        image_path: &Path,
        host_keys: &HostKeys,
        ssh_path: &str,
    ) -> Result<()> {
        // guestfish commands are colon-separated
        // -i: auto-mount filesystems
        // --rw: read-write mode
        let private_key_dest = format!("{}/ssh_host_ed25519_key", ssh_path);
        let public_key_dest = format!("{}/ssh_host_ed25519_key.pub", ssh_path);

        let output = Command::new("guestfish")
            .arg("--rw")
            .arg("-a")
            .arg(image_path)
            .arg("-i")
            .arg("copy-in")
            .arg(&host_keys.ssh_private_key)
            .arg(ssh_path)
            .arg(":")
            .arg("copy-in")
            .arg(&host_keys.ssh_public_key)
            .arg(ssh_path)
            .arg(":")
            .arg("mv")
            .arg(format!(
                "{}/{}",
                ssh_path,
                host_keys
                    .ssh_private_key
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            ))
            .arg(&private_key_dest)
            .arg(":")
            .arg("mv")
            .arg(format!(
                "{}/{}",
                ssh_path,
                host_keys
                    .ssh_public_key
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            ))
            .arg(&public_key_dest)
            .arg(":")
            .arg("chmod")
            .arg("0600")
            .arg(&private_key_dest)
            .arg(":")
            .arg("chmod")
            .arg("0644")
            .arg(&public_key_dest)
            .output()
            .map_err(|e| ProvisionerError::InjectionCommandFailed {
                provisioner: "sd-image".to_string(),
                command: "guestfish".to_string(),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: "sd-image".to_string(),
                reason: String::from_utf8_lossy(&output.stderr).to_string(),
            }
            .into());
        }

        Ok(())
    }

    fn inject_with_virt_copy_in(
        &self,
        image_path: &Path,
        host_keys: &HostKeys,
        ssh_path: &str,
    ) -> Result<()> {
        // virt-copy-in copies files into an image
        // We need to copy both keys, then fix permissions with guestfish
        let output = Command::new("virt-copy-in")
            .arg("-a")
            .arg(image_path)
            .arg(&host_keys.ssh_private_key)
            .arg(&host_keys.ssh_public_key)
            .arg(ssh_path)
            .output()
            .map_err(|e| ProvisionerError::InjectionCommandFailed {
                provisioner: "sd-image".to_string(),
                command: "virt-copy-in".to_string(),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: "sd-image".to_string(),
                reason: String::from_utf8_lossy(&output.stderr).to_string(),
            }
            .into());
        }

        // Rename files to standard names and fix permissions
        let private_key_dest = format!("{}/ssh_host_ed25519_key", ssh_path);
        let public_key_dest = format!("{}/ssh_host_ed25519_key.pub", ssh_path);

        let output = Command::new("guestfish")
            .arg("--rw")
            .arg("-a")
            .arg(image_path)
            .arg("-i")
            .arg("mv")
            .arg(format!(
                "{}/{}",
                ssh_path,
                host_keys
                    .ssh_private_key
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            ))
            .arg(&private_key_dest)
            .arg(":")
            .arg("mv")
            .arg(format!(
                "{}/{}",
                ssh_path,
                host_keys
                    .ssh_public_key
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            ))
            .arg(&public_key_dest)
            .arg(":")
            .arg("chmod")
            .arg("0600")
            .arg(&private_key_dest)
            .arg(":")
            .arg("chmod")
            .arg("0644")
            .arg(&public_key_dest)
            .output()
            .map_err(|e| ProvisionerError::InjectionCommandFailed {
                provisioner: "sd-image".to_string(),
                command: "guestfish".to_string(),
                stderr: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: "sd-image".to_string(),
                reason: String::from_utf8_lossy(&output.stderr).to_string(),
            }
            .into());
        }

        Ok(())
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
        if Self::find_available_tool().is_none() {
            return Err(ProvisionerError::MissingDependency {
                provisioner: "sd-image".to_string(),
                tool: "guestfish or virt-copy-in".to_string(),
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
        let tool =
            Self::find_available_tool().ok_or_else(|| ProvisionerError::MissingDependency {
                provisioner: "sd-image".to_string(),
                tool: "guestfish or virt-copy-in".to_string(),
                install_hint: "Install libguestfs: nix-shell -p libguestfs".to_string(),
            })?;

        let ssh_path = config.ssh_key_path();

        match tool {
            InjectionTool::Guestfish => {
                self.inject_with_guestfish(image_path, host_keys, ssh_path)?;
            }
            InjectionTool::VirtCopyIn => {
                self.inject_with_virt_copy_in(image_path, host_keys, ssh_path)?;
            }
        }

        Ok(InjectionResult::Success {
            image_path: image_path.to_path_buf(),
        })
    }

    fn deployment_instructions(&self, output_path: &Path, hostname: &str) -> String {
        format!(
            r#"To deploy the SD image:
  sudo dd if={} of=/dev/sdX bs=4M status=progress

Replace /dev/sdX with your SD card device.

After inserting the SD card and booting:
  snow push {}"#,
            output_path.display(),
            hostname,
        )
    }
}

/// Check if a command is available in PATH
fn which(cmd: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full_path = dir.join(cmd);
            if full_path.is_file() {
                Some(full_path)
            } else {
                None
            }
        })
    })
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
    fn test_which_finds_existing_command() {
        // 'sh' should exist on all Unix systems
        assert!(which("sh").is_some());
    }

    #[test]
    fn test_which_returns_none_for_missing_command() {
        assert!(which("definitely-not-a-real-command-12345").is_none());
    }
}
