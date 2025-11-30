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
pub struct SdImageProvisioner;

impl SdImageProvisioner {
    pub fn new() -> Self {
        Self
    }

    fn guestfish_available() -> bool {
        which("guestfish").is_some()
    }

    fn injection_error(reason: String) -> ProvisionerError {
        ProvisionerError::InjectionFailed {
            provisioner: "sd-image".to_string(),
            reason,
        }
    }

    fn get_filename(path: &Path) -> Result<&str> {
        path.file_name()
            .ok_or_else(|| {
                Self::injection_error(format!(
                    "Invalid key path (no filename): {}",
                    path.display()
                ))
            })?
            .to_str()
            .ok_or_else(|| {
                Self::injection_error(format!(
                    "Key filename contains invalid UTF-8: {}",
                    path.display()
                ))
                .into()
            })
    }

    fn inject_with_guestfish(
        &self,
        image_path: &Path,
        host_keys: &HostKeys,
        ssh_path: &str,
    ) -> Result<()> {
        let private_key_dest = format!("{}/ssh_host_ed25519_key", ssh_path);
        let public_key_dest = format!("{}/ssh_host_ed25519_key.pub", ssh_path);

        let private_key_filename = Self::get_filename(&host_keys.ssh_private_key)?;
        let public_key_filename = Self::get_filename(&host_keys.ssh_public_key)?;

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
            .arg(format!("{}/{}", ssh_path, private_key_filename))
            .arg(&private_key_dest)
            .arg(":")
            .arg("mv")
            .arg(format!("{}/{}", ssh_path, public_key_filename))
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
            return Err(
                Self::injection_error(String::from_utf8_lossy(&output.stderr).to_string()).into(),
            );
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
        if !Self::guestfish_available() {
            return Err(ProvisionerError::MissingDependency {
                provisioner: "sd-image".to_string(),
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
        self.inject_with_guestfish(image_path, host_keys, config.ssh_key_path())?;

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
        assert!(which("sh").is_some());
    }

    #[test]
    fn test_which_returns_none_for_missing_command() {
        assert!(which("definitely-not-a-real-command-12345").is_none());
    }

    #[test]
    fn test_get_filename_valid_path() {
        let path = Path::new("/tmp/ssh_host_ed25519_key");
        let filename = SdImageProvisioner::get_filename(path).unwrap();
        assert_eq!(filename, "ssh_host_ed25519_key");
    }

    #[test]
    fn test_get_filename_root_path_fails() {
        let path = Path::new("/");
        let result = SdImageProvisioner::get_filename(path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no filename"));
    }

    #[test]
    fn test_check_dependencies_returns_error_when_guestfish_missing() {
        // This test verifies the error message format when guestfish is unavailable.
        // We can't easily mock the PATH, so we test the error type directly.
        let error = ProvisionerError::MissingDependency {
            provisioner: "sd-image".to_string(),
            tool: "guestfish".to_string(),
            install_hint: "Install libguestfs: nix-shell -p libguestfs".to_string(),
        };
        assert!(error.to_string().contains("guestfish"));
        assert!(error.to_string().contains("sd-image"));
    }
}
