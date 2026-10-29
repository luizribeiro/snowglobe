#[cfg(any(debug_assertions, test, feature = "test-utils"))]
mod mock;
mod system;

#[cfg(any(debug_assertions, test, feature = "test-utils"))]
pub use mock::MockKeyGenBackend;
pub use system::SystemKeyGenBackend;

use crate::error::Result;
use std::path::{Path, PathBuf};

/// SSH key pair paths generated during bootstrap
#[derive(Debug, Clone)]
pub struct SshKeyPair {
    pub private_key: PathBuf,
    pub public_key: PathBuf,
}

impl SshKeyPair {
    pub fn new(private_key: PathBuf, public_key: PathBuf) -> Self {
        Self {
            private_key,
            public_key,
        }
    }
}

/// Backend for generating SSH keys and deriving age keys
///
/// This trait abstracts key generation operations to enable testing
/// without actually running ssh-keygen or ssh-to-age.
pub trait KeyGenBackend: Send + Sync {
    /// Generate an ed25519 SSH key pair for a host
    ///
    /// Creates both private and public key files in the specified directory.
    /// The files will be named `ssh_host_ed25519_key` and `ssh_host_ed25519_key.pub`.
    fn generate_ssh_keypair(&self, output_dir: &Path) -> Result<SshKeyPair>;

    /// Derive an age public key from an SSH public key
    ///
    /// Uses ssh-to-age to convert the SSH public key to an age public key
    /// for use with SOPS or other age-based encryption tools.
    fn derive_age_key(&self, ssh_public_key: &Path) -> Result<String>;

    /// Check if required tools are available
    ///
    /// Returns Ok(()) if all required tools (ssh-keygen, ssh-to-age) are available,
    /// or an error describing what's missing.
    fn check_dependencies(&self) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssh_key_pair_new() {
        let pair = SshKeyPair::new(
            PathBuf::from("/tmp/ssh_host_ed25519_key"),
            PathBuf::from("/tmp/ssh_host_ed25519_key.pub"),
        );
        assert_eq!(pair.private_key, PathBuf::from("/tmp/ssh_host_ed25519_key"));
        assert_eq!(
            pair.public_key,
            PathBuf::from("/tmp/ssh_host_ed25519_key.pub")
        );
    }
}
