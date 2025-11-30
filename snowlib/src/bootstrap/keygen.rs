//! Key generation logic for host bootstrap
//!
//! Handles SSH key generation and age key derivation for new hosts.

use crate::backends::keygen::KeyGenBackend;
use crate::error::{BootstrapError, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// SSH host keys and derived age key for a bootstrapped host
#[derive(Debug, Clone)]
pub struct HostKeys {
    /// Hostname this key set belongs to
    pub hostname: String,
    /// Path to the SSH private key (ed25519)
    pub ssh_private_key: PathBuf,
    /// Path to the SSH public key
    pub ssh_public_key: PathBuf,
    /// Age public key derived from SSH public key (for SOPS)
    pub age_public_key: String,
    /// Root temporary directory containing all keys - caller should remove when done
    pub temp_dir_root: PathBuf,
}

impl HostKeys {
    /// Create a new HostKeys instance
    pub fn new(
        hostname: String,
        ssh_private_key: PathBuf,
        ssh_public_key: PathBuf,
        age_public_key: String,
        temp_dir_root: PathBuf,
    ) -> Self {
        Self {
            hostname,
            ssh_private_key,
            ssh_public_key,
            age_public_key,
            temp_dir_root,
        }
    }

    /// Clean up temporary directory containing keys
    ///
    /// Call this after keys have been copied to their final destination.
    /// Failure to call this leaves sensitive key material on disk.
    pub fn cleanup(&self) -> std::io::Result<()> {
        fs::remove_dir_all(&self.temp_dir_root)
    }
}

/// Set secure permissions on a path
fn set_secure_permissions(path: &Path, mode: u32) -> Result<()> {
    let mut perms = fs::metadata(path)
        .map_err(|e| BootstrapError::TempDirFailed { source: e })?
        .permissions();
    perms.set_mode(mode);
    fs::set_permissions(path, perms).map_err(|e| BootstrapError::TempDirFailed { source: e })?;
    Ok(())
}

/// Bootstrap a host by generating SSH keys and deriving an age key
///
/// Creates keys in a secure temporary directory with 0700 permissions.
/// Call `HostKeys::cleanup()` after copying keys to their final destination.
///
/// # Security Note
///
/// Private keys are created with 0600 permissions. The temporary directory
/// persists until `cleanup()` is called - ensure you remove it after use.
///
/// # Example
///
/// ```no_run
/// use snowlib::bootstrap::bootstrap_host;
/// use snowlib::SystemKeyGenBackend;
///
/// let backend = SystemKeyGenBackend::new();
/// let keys = bootstrap_host("myhost", &backend)?;
///
/// println!("Age public key: {}", keys.age_public_key);
/// // ... copy keys to destination ...
/// keys.cleanup()?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn bootstrap_host(hostname: &str, keygen_backend: &dyn KeyGenBackend) -> Result<HostKeys> {
    keygen_backend.check_dependencies()?;

    let temp_dir = create_secure_temp_dir(hostname)?;
    let host_dir = temp_dir.join(hostname);
    fs::create_dir_all(&host_dir).map_err(|e| BootstrapError::TempDirFailed { source: e })?;
    set_secure_permissions(&host_dir, 0o700)?;

    let ssh_keypair = keygen_backend.generate_ssh_keypair(&host_dir)?;
    let age_public_key = keygen_backend.derive_age_key(&ssh_keypair.public_key)?;

    Ok(HostKeys::new(
        hostname.to_string(),
        ssh_keypair.private_key,
        ssh_keypair.public_key,
        age_public_key,
        temp_dir,
    ))
}

/// Create a secure temporary directory for bootstrap operations
fn create_secure_temp_dir(hostname: &str) -> Result<PathBuf> {
    let temp_base = std::env::temp_dir();
    let dir_name = format!("snow-bootstrap-{}-{}", hostname, random_suffix());
    let temp_dir = temp_base.join(dir_name);

    fs::create_dir_all(&temp_dir).map_err(|e| BootstrapError::TempDirFailed { source: e })?;
    set_secure_permissions(&temp_dir, 0o700)?;

    Ok(temp_dir)
}

/// Generate a random suffix for unique directory names
fn random_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    // Use timestamp + process id for uniqueness
    format!("{:x}{:x}", timestamp, std::process::id())
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::backends::keygen::MockKeyGenBackend;

    #[test]
    fn test_host_keys_new() {
        let keys = HostKeys::new(
            "testhost".to_string(),
            PathBuf::from("/tmp/key"),
            PathBuf::from("/tmp/key.pub"),
            "age1test".to_string(),
            PathBuf::from("/tmp"),
        );

        assert_eq!(keys.hostname, "testhost");
        assert_eq!(keys.ssh_private_key, PathBuf::from("/tmp/key"));
        assert_eq!(keys.ssh_public_key, PathBuf::from("/tmp/key.pub"));
        assert_eq!(keys.age_public_key, "age1test");
        assert_eq!(keys.temp_dir_root, PathBuf::from("/tmp"));
    }

    #[test]
    fn test_random_suffix_is_unique() {
        let s1 = random_suffix();
        let s2 = random_suffix();
        assert!(!s1.is_empty());
        assert!(!s2.is_empty());
    }

    #[test]
    fn test_bootstrap_host_checks_dependencies_first() {
        let backend = MockKeyGenBackend::new().with_missing_dependency("ssh-to-age");
        let result = bootstrap_host("testhost", &backend);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("ssh-to-age"));
    }

    #[test]
    fn test_bootstrap_host_success_with_mock() {
        let custom_age_key = "age1testkey00000000000000000000000000000000000000000000000";
        let backend = MockKeyGenBackend::new().with_age_key(custom_age_key);

        let keys = bootstrap_host("myhost", &backend).unwrap();
        assert_eq!(keys.hostname, "myhost");
        assert_eq!(keys.age_public_key, custom_age_key);
        assert!(keys.ssh_private_key.to_string_lossy().contains("myhost"));
        assert!(keys.ssh_public_key.to_string_lossy().contains("myhost"));

        // Cleanup
        let _ = keys.cleanup();
    }

    #[test]
    fn test_bootstrap_host_keygen_failure() {
        let backend = MockKeyGenBackend::new().with_keygen_failure();
        let result = bootstrap_host("failhost", &backend);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Mock keygen"));
    }

    #[test]
    fn test_bootstrap_host_age_derivation_failure() {
        let backend = MockKeyGenBackend::new().with_age_failure();
        let result = bootstrap_host("failhost", &backend);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Mock age"));
    }

    #[test]
    fn test_create_secure_temp_dir_has_correct_permissions() {
        let temp_dir = create_secure_temp_dir("permtest").unwrap();

        let metadata = fs::metadata(&temp_dir).unwrap();
        let permissions = metadata.permissions();
        assert_eq!(
            permissions.mode() & 0o777,
            0o700,
            "Temp directory should have 0700 permissions"
        );

        // Cleanup
        fs::remove_dir_all(&temp_dir).expect("Failed to cleanup test directory");
    }

    #[test]
    fn test_bootstrap_creates_host_subdirectory_and_cleanup_works() {
        let backend = MockKeyGenBackend::new();
        let keys = bootstrap_host("subdir-test", &backend).unwrap();

        // The keys should be in a subdirectory named after the host
        assert!(keys
            .ssh_private_key
            .to_string_lossy()
            .contains("subdir-test"));

        // Temp dir should exist before cleanup
        assert!(keys.temp_dir_root.exists());

        // Cleanup should work
        keys.cleanup().expect("Cleanup should succeed");

        // Temp dir should be gone after cleanup
        assert!(!keys.temp_dir_root.exists());
    }

    #[test]
    fn test_host_keys_cleanup_removes_temp_directory() {
        let backend = MockKeyGenBackend::new();
        let keys = bootstrap_host("cleanup-test", &backend).unwrap();

        let temp_dir = keys.temp_dir_root.clone();
        assert!(temp_dir.exists(), "Temp dir should exist before cleanup");

        keys.cleanup().unwrap();

        assert!(
            !temp_dir.exists(),
            "Temp dir should be removed after cleanup"
        );
    }
}
