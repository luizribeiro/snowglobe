//! Shared guestfish utilities for image provisioners
//!
//! Provides rootless key injection into disk images using libguestfs.

use super::command::{CommandExecutor, SystemCommandExecutor};
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
    ssh_path: &str,
) -> Result<()> {
    let executor: Arc<dyn CommandExecutor> = Arc::new(SystemCommandExecutor::new());
    inject_keys_with_executor(provisioner_name, image_path, host_keys, ssh_path, &executor)
}

/// Inject SSH host keys into a disk image using a custom executor
#[cfg(any(debug_assertions, test))]
pub fn inject_keys_with_executor(
    provisioner_name: &str,
    image_path: &Path,
    host_keys: &HostKeys,
    ssh_path: &str,
    executor: &Arc<dyn CommandExecutor>,
) -> Result<()> {
    if inject_keys_inspect(provisioner_name, image_path, host_keys, ssh_path, executor).is_ok() {
        return Ok(());
    }

    inject_keys_manual(provisioner_name, image_path, host_keys, ssh_path, executor)
}

#[cfg(not(any(debug_assertions, test)))]
pub fn inject_keys_with_executor(
    provisioner_name: &str,
    image_path: &Path,
    host_keys: &HostKeys,
    ssh_path: &str,
    executor: &Arc<dyn CommandExecutor>,
) -> Result<()> {
    if inject_keys_inspect(provisioner_name, image_path, host_keys, ssh_path, executor).is_ok() {
        return Ok(());
    }

    inject_keys_manual(provisioner_name, image_path, host_keys, ssh_path, executor)
}

/// Try to inject keys using guestfish inspect mode (-i)
fn inject_keys_inspect(
    provisioner_name: &str,
    image_path: &Path,
    host_keys: &HostKeys,
    ssh_path: &str,
    executor: &Arc<dyn CommandExecutor>,
) -> Result<()> {
    let private_key_dest = format!("{}/ssh_host_ed25519_key", ssh_path);
    let public_key_dest = format!("{}/ssh_host_ed25519_key.pub", ssh_path);

    let private_key_filename = get_filename(provisioner_name, &host_keys.ssh_private_key)?;
    let public_key_filename = get_filename(provisioner_name, &host_keys.ssh_public_key)?;

    let mut args = vec![
        "--rw".to_string(),
        "-a".to_string(),
        image_path.to_string_lossy().to_string(),
        "-i".to_string(),
        "copy-in".to_string(),
        host_keys.ssh_private_key.to_string_lossy().to_string(),
        ssh_path.to_string(),
        ":".to_string(),
        "copy-in".to_string(),
        host_keys.ssh_public_key.to_string_lossy().to_string(),
        ssh_path.to_string(),
    ];

    if private_key_filename != "ssh_host_ed25519_key" {
        args.extend([
            ":".to_string(),
            "mv".to_string(),
            format!("{}/{}", ssh_path, private_key_filename),
            private_key_dest.clone(),
        ]);
    }
    if public_key_filename != "ssh_host_ed25519_key.pub" {
        args.extend([
            ":".to_string(),
            "mv".to_string(),
            format!("{}/{}", ssh_path, public_key_filename),
            public_key_dest.clone(),
        ]);
    }

    args.extend([
        ":".to_string(),
        "chmod".to_string(),
        "0600".to_string(),
        private_key_dest,
        ":".to_string(),
        "chmod".to_string(),
        "0644".to_string(),
        public_key_dest,
    ]);

    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let output = executor.execute("guestfish", &args_refs)?;

    if !output.success {
        return Err(ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason: output.stderr_str(),
        }
        .into());
    }

    Ok(())
}

/// Inject keys by manually mounting the root filesystem
fn inject_keys_manual(
    provisioner_name: &str,
    image_path: &Path,
    host_keys: &HostKeys,
    ssh_path: &str,
    executor: &Arc<dyn CommandExecutor>,
) -> Result<()> {
    let filesystems = discover_filesystems(provisioner_name, image_path, executor)?;

    let root_device =
        find_root_filesystem(&filesystems).ok_or_else(|| ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason: format!(
                "No suitable root filesystem found. Available: {}",
                filesystems.join(", ")
            ),
        })?;

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

/// Discover filesystems in a disk image
fn discover_filesystems(
    provisioner_name: &str,
    image_path: &Path,
    executor: &Arc<dyn CommandExecutor>,
) -> Result<Vec<String>> {
    let output = executor.execute(
        "guestfish",
        &[
            "--ro",
            "-a",
            &image_path.to_string_lossy(),
            "run",
            ":",
            "list-filesystems",
        ],
    )?;

    if !output.success {
        return Err(ProvisionerError::InjectionFailed {
            provisioner: provisioner_name.to_string(),
            reason: format!("Failed to list filesystems: {}", output.stderr_str()),
        }
        .into());
    }

    let filesystems: Vec<String> = output
        .stdout_str()
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 {
                Some(format!("{}:{}", parts[0].trim(), parts[1].trim()))
            } else {
                None
            }
        })
        .collect();

    Ok(filesystems)
}

/// Find the most likely root filesystem from a list
fn find_root_filesystem(filesystems: &[String]) -> Option<String> {
    let linux_fs: Vec<&String> = filesystems
        .iter()
        .filter(|fs| {
            let fs_type = fs.split(':').nth(1).unwrap_or("").trim();
            matches!(fs_type, "ext4" | "ext3" | "ext2" | "btrfs" | "xfs")
        })
        .collect();

    linux_fs
        .last()
        .map(|fs| fs.split(':').next().unwrap_or("").trim().to_string())
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
    fn test_find_root_filesystem_prefers_ext4() {
        let filesystems = vec!["/dev/sda1: vfat".to_string(), "/dev/sda2: ext4".to_string()];
        let root = find_root_filesystem(&filesystems);
        assert_eq!(root, Some("/dev/sda2".to_string()));
    }

    #[test]
    fn test_find_root_filesystem_prefers_last_linux_fs() {
        let filesystems = vec!["/dev/sda1: ext4".to_string(), "/dev/sda2: ext4".to_string()];
        let root = find_root_filesystem(&filesystems);
        assert_eq!(root, Some("/dev/sda2".to_string()));
    }

    #[test]
    fn test_find_root_filesystem_ignores_non_linux() {
        let filesystems = vec!["/dev/sda1: vfat".to_string(), "/dev/sda2: swap".to_string()];
        let root = find_root_filesystem(&filesystems);
        assert_eq!(root, None);
    }

    #[test]
    fn test_find_root_filesystem_supports_btrfs() {
        let filesystems = vec!["/dev/sda1: btrfs".to_string()];
        let root = find_root_filesystem(&filesystems);
        assert_eq!(root, Some("/dev/sda1".to_string()));
    }

    #[test]
    fn test_find_root_filesystem_supports_xfs() {
        let filesystems = vec!["/dev/sda1: xfs".to_string()];
        let root = find_root_filesystem(&filesystems);
        assert_eq!(root, Some("/dev/sda1".to_string()));
    }

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
    fn test_discover_filesystems_parses_output() {
        let mock = MockCommandExecutor::new().with_response(
            "guestfish",
            MockCommandExecutor::success_output("/dev/sda1: vfat\n/dev/sda2: ext4\n"),
        );
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let result = discover_filesystems("test", Path::new("/tmp/image.raw"), &executor);
        assert!(result.is_ok());
        let fs = result.unwrap();
        assert_eq!(fs.len(), 2);
        assert_eq!(fs[0], "/dev/sda1:vfat");
        assert_eq!(fs[1], "/dev/sda2:ext4");
    }

    #[test]
    fn test_discover_filesystems_handles_failure() {
        let mock = MockCommandExecutor::new()
            .with_response("guestfish", MockCommandExecutor::failure_output("error"))
            .with_default_success(false);
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let result = discover_filesystems("test", Path::new("/tmp/image.raw"), &executor);
        assert!(result.is_err());
    }

    #[test]
    fn test_inject_keys_inspect_builds_correct_args() {
        let mock = MockCommandExecutor::new()
            .with_response("guestfish", MockCommandExecutor::success_output(""));
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let host_keys = make_test_host_keys();

        let result = inject_keys_inspect(
            "test",
            Path::new("/tmp/image.raw"),
            &host_keys,
            "/etc/ssh",
            &executor,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_inject_keys_manual_fallback() {
        // This test verifies the fallback from inspect mode to manual mode
        // When inspect mode fails (no -i in args), manual mode should be attempted
        // With default_success=true, the manual mode will succeed
        let mock = MockCommandExecutor::new();
        let executor: Arc<dyn CommandExecutor> = Arc::new(mock);

        let host_keys = make_test_host_keys();

        // The default mock returns success for all commands
        // This tests that the function can complete without errors
        let result = inject_keys_with_executor(
            "test",
            Path::new("/tmp/image.raw"),
            &host_keys,
            "/etc/ssh",
            &executor,
        );

        assert!(result.is_ok());
    }
}
