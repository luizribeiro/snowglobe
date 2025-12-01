//! Shared guestfish utilities for image provisioners
//!
//! Provides rootless key injection into disk images using libguestfs.
//! Uses filesystem labels from NixOS config (`fileSystems."/".device`) to
//! reliably locate the root partition via guestfish's `findfs-label`.

use super::command::{CommandExecutor, SystemCommandExecutor};
use super::ProvisionerConfig;
use crate::bootstrap::HostKeys;
use crate::error::{ProvisionerError, Result};
use std::path::Path;
use std::sync::Arc;

/// Check if guestfish is available in PATH
pub fn is_available() -> bool {
    SystemCommandExecutor::new().which("guestfish").is_some()
}

/// Inject SSH host keys into a disk image using guestfish
///
/// Uses the system command executor. For testing, use `inject_keys_with_executor`.
pub fn inject_keys(
    provisioner_name: &str,
    image_path: &Path,
    host_keys: &HostKeys,
    config: &ProvisionerConfig,
) -> Result<()> {
    let executor: Arc<dyn CommandExecutor> = Arc::new(SystemCommandExecutor::new());
    inject_keys_with_executor(provisioner_name, image_path, host_keys, config, &executor)
}

/// Inject SSH host keys into a disk image using a custom executor
///
/// Requires a root filesystem label (from `fileSystems."/".device` in NixOS config).
/// All standard NixOS images use labels like "nixos" or "NIXOS_SD".
pub fn inject_keys_with_executor(
    provisioner_name: &str,
    image_path: &Path,
    host_keys: &HostKeys,
    config: &ProvisionerConfig,
    executor: &Arc<dyn CommandExecutor>,
) -> Result<()> {
    let label = config.root_fs_label().ok_or_else(|| {
        let reason = match &config.root_fs_device {
            Some(device) => format!(
                "Root filesystem device '{}' is not a label-based path. \
                 Change fileSystems.\"/\".device to use /dev/disk/by-label/<label> format.",
                device
            ),
            None => "No root filesystem device configured. \
                     Ensure fileSystems.\"/\".device is set in your NixOS configuration."
                .to_string(),
        };
        ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason,
        }
    })?;

    if label.is_empty() {
        return Err(ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason: "Root filesystem label is empty".to_string(),
        }
        .into());
    }

    let root_device = find_device_by_label(provisioner_name, image_path, label, executor)?;

    let ssh_path = config.ssh_key_path();
    let private_key_dest = format!("{}/ssh_host_ed25519_key", ssh_path);
    let public_key_dest = format!("{}/ssh_host_ed25519_key.pub", ssh_path);

    let private_key_filename = get_filename(provisioner_name, &host_keys.ssh_private_key)?;
    let public_key_filename = get_filename(provisioner_name, &host_keys.ssh_public_key)?;

    let private_mv = if private_key_filename != "ssh_host_ed25519_key" {
        format!(
            "mv {}/{} {}\n",
            ssh_path, private_key_filename, private_key_dest
        )
    } else {
        String::new()
    };
    let public_mv = if public_key_filename != "ssh_host_ed25519_key.pub" {
        format!(
            "mv {}/{} {}\n",
            ssh_path, public_key_filename, public_key_dest
        )
    } else {
        String::new()
    };

    let script = format!(
        r#"run
mount {root_device} /
-mkdir-p {ssh_path}
copy-in {private_key} {ssh_path}
copy-in {public_key} {ssh_path}
{private_mv}{public_mv}chmod 0600 {private_dest}
chmod 0644 {public_dest}
"#,
        root_device = root_device,
        ssh_path = ssh_path,
        private_key = host_keys.ssh_private_key.display(),
        public_key = host_keys.ssh_public_key.display(),
        private_mv = private_mv,
        public_mv = public_mv,
        private_dest = private_key_dest,
        public_dest = public_key_dest,
    );

    let output = executor.execute_with_stdin(
        "guestfish",
        &["--rw", "-a", &image_path.to_string_lossy()],
        script.as_bytes(),
    )?;

    if !output.success {
        return Err(ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason: output.stderr_str(),
        }
        .into());
    }

    Ok(())
}

/// Find a filesystem device by its label using guestfish
fn find_device_by_label(
    provisioner_name: &str,
    image_path: &Path,
    label: &str,
    executor: &Arc<dyn CommandExecutor>,
) -> Result<String> {
    let script = format!("run\nfindfs-label {}\n", label);

    let output = executor.execute_with_stdin(
        "guestfish",
        &["--ro", "-a", &image_path.to_string_lossy()],
        script.as_bytes(),
    )?;

    if !output.success {
        return Err(ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason: format!(
                "Failed to find filesystem with label '{}': {}",
                label,
                output.stderr_str()
            ),
        }
        .into());
    }

    let device = output.stdout_str().trim().to_string();
    if device.is_empty() {
        return Err(ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason: format!("No filesystem found with label '{}'", label),
        }
        .into());
    }

    Ok(device)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::provisioner::MockCommandExecutor;

    fn make_test_host_keys() -> HostKeys {
        HostKeys {
            hostname: "testhost".to_string(),
            ssh_private_key: std::path::PathBuf::from("/tmp/ssh_host_ed25519_key"),
            ssh_public_key: std::path::PathBuf::from("/tmp/ssh_host_ed25519_key.pub"),
            age_public_key: "age1test".to_string(),
            temp_dir_root: std::path::PathBuf::from("/tmp/snow-test"),
        }
    }

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
    fn test_inject_keys_with_label() {
        let mock = MockCommandExecutor::new().with_response(
            "guestfish",
            MockCommandExecutor::success_output("/dev/sda2\n"),
        );
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let host_keys = make_test_host_keys();
        let config = ProvisionerConfig::new().with_root_fs_label("nixos");

        let result = inject_keys_with_executor(
            "test",
            Path::new("/tmp/image.raw"),
            &host_keys,
            &config,
            &executor,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_inject_keys_without_device_fails() {
        let mock = MockCommandExecutor::new();
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let host_keys = make_test_host_keys();
        let config = ProvisionerConfig::new(); // No device at all

        let result = inject_keys_with_executor(
            "test",
            Path::new("/tmp/image.raw"),
            &host_keys,
            &config,
            &executor,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No root filesystem device configured"));
    }

    #[test]
    fn test_inject_keys_with_non_label_device_fails() {
        let mock = MockCommandExecutor::new();
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let host_keys = make_test_host_keys();
        let config = ProvisionerConfig::new().with_root_fs_device("/dev/sda2"); // Non-label path

        let result = inject_keys_with_executor(
            "test",
            Path::new("/tmp/image.raw"),
            &host_keys,
            &config,
            &executor,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("/dev/sda2"));
        assert!(err.contains("not a label-based path"));
    }

    #[test]
    fn test_inject_keys_with_empty_label_fails() {
        let mock = MockCommandExecutor::new();
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let host_keys = make_test_host_keys();
        let config = ProvisionerConfig::new().with_root_fs_label("");

        let result = inject_keys_with_executor(
            "test",
            Path::new("/tmp/image.raw"),
            &host_keys,
            &config,
            &executor,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("label is empty"));
    }

    #[test]
    fn test_find_device_by_label_success() {
        let mock = MockCommandExecutor::new().with_response(
            "guestfish",
            MockCommandExecutor::success_output("/dev/sda2\n"),
        );
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let result = find_device_by_label("test", Path::new("/tmp/image.raw"), "nixos", &executor);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "/dev/sda2");
    }

    #[test]
    fn test_find_device_by_label_not_found() {
        let mock = MockCommandExecutor::new()
            .with_response("guestfish", MockCommandExecutor::success_output(""));
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let result = find_device_by_label(
            "test",
            Path::new("/tmp/image.raw"),
            "nonexistent",
            &executor,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No filesystem found with label"));
    }

    #[test]
    fn test_find_device_by_label_command_failure() {
        let mock = MockCommandExecutor::new()
            .with_response(
                "guestfish",
                MockCommandExecutor::failure_output("findfs-label: no such label"),
            )
            .with_default_success(false);
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let result =
            find_device_by_label("test", Path::new("/tmp/image.raw"), "badlabel", &executor);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Failed to find filesystem with label"));
    }
}
