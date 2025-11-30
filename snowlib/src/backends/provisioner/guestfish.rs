//! Shared guestfish utilities for image provisioners
//!
//! Provides rootless key injection into disk images using libguestfs.

use crate::bootstrap::HostKeys;
use crate::error::{ProvisionerError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Check if guestfish is available in PATH
pub fn is_available() -> bool {
    which("guestfish").is_some()
}

/// Inject SSH host keys into a disk image using guestfish
///
/// This function copies the SSH keypair into the image at the specified path,
/// renames them to the standard names, and sets appropriate permissions.
pub fn inject_keys(
    provisioner_name: &str,
    image_path: &Path,
    host_keys: &HostKeys,
    ssh_path: &str,
) -> Result<()> {
    let private_key_dest = format!("{}/ssh_host_ed25519_key", ssh_path);
    let public_key_dest = format!("{}/ssh_host_ed25519_key.pub", ssh_path);

    let private_key_filename = get_filename(provisioner_name, &host_keys.ssh_private_key)?;
    let public_key_filename = get_filename(provisioner_name, &host_keys.ssh_public_key)?;

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
            provisioner: provisioner_name.to_string(),
            command: "guestfish".to_string(),
            stderr: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason: String::from_utf8_lossy(&output.stderr).to_string(),
        }
        .into());
    }

    Ok(())
}

fn get_filename<'a>(provisioner_name: &str, path: &'a Path) -> Result<&'a str> {
    path.file_name()
        .ok_or_else(|| ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason: format!("Invalid key path (no filename): {}", path.display()),
        })?
        .to_str()
        .ok_or_else(|| {
            ProvisionerError::InjectionFailed {
                provisioner: provisioner_name.to_string(),
                reason: format!("Key filename contains invalid UTF-8: {}", path.display()),
            }
            .into()
        })
}

/// Check if a command is available in PATH
pub fn which(cmd: &str) -> Option<PathBuf> {
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
    fn test_get_filename_valid_path() {
        let path = Path::new("/tmp/ssh_host_ed25519_key");
        let filename = get_filename("test", path).unwrap();
        assert_eq!(filename, "ssh_host_ed25519_key");
    }

    #[test]
    fn test_get_filename_root_path_fails() {
        let path = Path::new("/");
        let result = get_filename("test", path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no filename"));
    }

    #[test]
    fn test_which_finds_existing_command() {
        assert!(which("sh").is_some());
    }

    #[test]
    fn test_which_returns_none_for_missing_command() {
        assert!(which("definitely-not-a-real-command-12345").is_none());
    }
}
