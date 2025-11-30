use super::{KeyGenBackend, SshKeyPair};
use crate::error::{BootstrapError, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// Real implementation of KeyGenBackend using ssh-keygen and ssh-to-age
pub struct SystemKeyGenBackend;

impl SystemKeyGenBackend {
    pub fn new() -> Self {
        Self
    }

    fn which(cmd: &str) -> bool {
        Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn path_to_str(path: &Path) -> Result<&str> {
        path.to_str().ok_or_else(|| {
            BootstrapError::InvalidPath {
                path: path.display().to_string(),
            }
            .into()
        })
    }
}

impl Default for SystemKeyGenBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyGenBackend for SystemKeyGenBackend {
    fn check_dependencies(&self) -> Result<()> {
        if !Self::which("ssh-keygen") {
            return Err(BootstrapError::MissingDependency {
                tool: "ssh-keygen".to_string(),
            }
            .into());
        }

        if !Self::which("ssh-to-age") {
            return Err(BootstrapError::MissingDependency {
                tool: "ssh-to-age".to_string(),
            }
            .into());
        }

        Ok(())
    }

    fn generate_ssh_keypair(&self, output_dir: &Path) -> Result<SshKeyPair> {
        let private_key_path = output_dir.join("ssh_host_ed25519_key");
        let public_key_path = output_dir.join("ssh_host_ed25519_key.pub");

        let private_key_str = Self::path_to_str(&private_key_path)?;

        let output = Command::new("ssh-keygen")
            .args([
                "-t",
                "ed25519",
                "-f",
                private_key_str,
                "-N",
                "", // Host keys must have empty passphrase for unattended boot
                "-C",
                "host key",
            ])
            .output()
            .map_err(|e| BootstrapError::KeyGenFailed { source: e })?;

        if !output.status.success() {
            return Err(BootstrapError::KeyGenCommandFailed {
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            }
            .into());
        }

        // Enforce secure permissions on private key (0600)
        let mut perms = fs::metadata(&private_key_path)
            .map_err(|e| BootstrapError::KeyGenFailed { source: e })?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&private_key_path, perms)
            .map_err(|e| BootstrapError::KeyGenFailed { source: e })?;

        Ok(SshKeyPair::new(private_key_path, public_key_path))
    }

    fn derive_age_key(&self, ssh_public_key: &Path) -> Result<String> {
        let public_key_str = Self::path_to_str(ssh_public_key)?;

        let output = Command::new("ssh-to-age")
            .args(["-i", public_key_str])
            .output()
            .map_err(|e| BootstrapError::AgeDerivationFailed { source: e })?;

        if !output.status.success() {
            return Err(BootstrapError::AgeCommandFailed {
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            }
            .into());
        }

        let age_key = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Validate age key format
        if !age_key.starts_with("age1") {
            return Err(BootstrapError::AgeCommandFailed {
                stderr: format!(
                    "Invalid age key format: expected 'age1...' but got '{}'",
                    age_key
                ),
            }
            .into());
        }

        Ok(age_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_backend() {
        let backend = SystemKeyGenBackend::new();
        // Zero-sized type
        assert!(std::mem::size_of_val(&backend) == 0);
    }

    #[test]
    fn test_which_finds_common_commands() {
        assert!(SystemKeyGenBackend::which("ls"));
        assert!(SystemKeyGenBackend::which("sh"));
    }

    #[test]
    fn test_which_returns_false_for_nonexistent() {
        assert!(!SystemKeyGenBackend::which(
            "definitely_not_a_real_command_xyz123"
        ));
    }

    #[test]
    fn test_path_to_str_valid_utf8() {
        let path = std::path::Path::new("/tmp/test/key");
        let result = SystemKeyGenBackend::path_to_str(path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "/tmp/test/key");
    }
}

/// Integration tests that require real ssh-keygen and ssh-to-age
#[cfg(test)]
mod integration_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    #[ignore] // Requires ssh-keygen installed
    fn test_generate_keypair_creates_valid_keys() {
        let backend = SystemKeyGenBackend::new();
        let temp_dir = TempDir::new().unwrap();

        let pair = backend.generate_ssh_keypair(temp_dir.path()).unwrap();

        // Verify files exist
        assert!(pair.private_key.exists(), "Private key should exist");
        assert!(pair.public_key.exists(), "Public key should exist");

        // Verify private key permissions are secure (0600)
        let metadata = fs::metadata(&pair.private_key).unwrap();
        let permissions = metadata.permissions();
        assert_eq!(
            permissions.mode() & 0o777,
            0o600,
            "Private key should have 0600 permissions"
        );

        // Verify public key contains "ssh-ed25519"
        let public_key_content = fs::read_to_string(&pair.public_key).unwrap();
        assert!(
            public_key_content.starts_with("ssh-ed25519 "),
            "Public key should be ed25519 format"
        );

        // Verify private key header
        let private_key_content = fs::read_to_string(&pair.private_key).unwrap();
        assert!(
            private_key_content.contains("OPENSSH PRIVATE KEY"),
            "Private key should have OpenSSH header"
        );
    }

    #[test]
    #[ignore] // Requires ssh-keygen and ssh-to-age installed
    fn test_derive_age_key_produces_valid_key() {
        let backend = SystemKeyGenBackend::new();
        let temp_dir = TempDir::new().unwrap();

        // Generate SSH key first
        let pair = backend.generate_ssh_keypair(temp_dir.path()).unwrap();

        // Derive age key
        let age_key = backend.derive_age_key(&pair.public_key).unwrap();

        // Verify age key format (should start with "age1")
        assert!(
            age_key.starts_with("age1"),
            "Age public key should start with 'age1', got: {}",
            age_key
        );

        // Verify key length (age public keys are typically ~62 chars)
        assert!(
            age_key.len() > 50 && age_key.len() < 80,
            "Age key length looks wrong: {}",
            age_key.len()
        );
    }

    #[test]
    #[ignore] // Requires ssh-keygen installed
    fn test_full_bootstrap_workflow() {
        let backend = SystemKeyGenBackend::new();

        // Skip if dependencies not available
        if backend.check_dependencies().is_err() {
            return;
        }

        let temp_dir = TempDir::new().unwrap();

        // Generate keypair
        let pair = backend.generate_ssh_keypair(temp_dir.path()).unwrap();

        // Derive age key
        let age_key = backend.derive_age_key(&pair.public_key).unwrap();

        // Verify the age key is usable (basic format check)
        assert!(age_key.starts_with("age1"));
        assert!(!age_key.contains('\n'));
        assert!(!age_key.contains(' '));
    }

    #[test]
    fn test_check_dependencies_does_not_panic() {
        let backend = SystemKeyGenBackend::new();
        // Should not panic, may return Ok or Err depending on system
        let _ = backend.check_dependencies();
    }
}
