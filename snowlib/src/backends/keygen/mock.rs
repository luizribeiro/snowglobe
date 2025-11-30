use super::{KeyGenBackend, SshKeyPair};
use crate::error::{BootstrapError, Result};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Mock implementation of KeyGenBackend for testing
///
/// Allows configuring success/failure behavior and custom age key responses.
/// Tracks all method calls for assertions in tests.
#[derive(Debug, Clone)]
pub struct MockKeyGenBackend {
    /// Should check_dependencies fail?
    deps_fail: bool,
    /// Which tool should be reported as missing (if deps_fail is true)
    missing_tool: String,
    /// Should generate_ssh_keypair fail?
    keygen_fail: bool,
    /// Should derive_age_key fail?
    age_fail: bool,
    /// The age key to return from derive_age_key
    age_key: String,
    /// Track calls to generate_ssh_keypair (paths)
    keygen_calls: Arc<Mutex<Vec<String>>>,
    /// Track calls to derive_age_key (paths)
    age_calls: Arc<Mutex<Vec<String>>>,
}

impl Default for MockKeyGenBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockKeyGenBackend {
    /// Create a new MockKeyGenBackend with all operations succeeding
    pub fn new() -> Self {
        Self {
            deps_fail: false,
            missing_tool: String::new(),
            keygen_fail: false,
            age_fail: false,
            age_key: "age1mock0key0for0testing0only0000000000000000000000000000000".to_string(),
            keygen_calls: Arc::new(Mutex::new(Vec::new())),
            age_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Configure check_dependencies to fail with a specific missing tool
    pub fn with_missing_dependency(mut self, tool: &str) -> Self {
        self.deps_fail = true;
        self.missing_tool = tool.to_string();
        self
    }

    /// Configure generate_ssh_keypair to fail
    pub fn with_keygen_failure(mut self) -> Self {
        self.keygen_fail = true;
        self
    }

    /// Configure derive_age_key to fail
    pub fn with_age_failure(mut self) -> Self {
        self.age_fail = true;
        self
    }

    /// Configure the age key that will be returned by derive_age_key
    pub fn with_age_key(mut self, key: &str) -> Self {
        self.age_key = key.to_string();
        self
    }

    /// Get all paths where generate_ssh_keypair was called
    pub fn get_keygen_calls(&self) -> Vec<String> {
        self.keygen_calls.lock().unwrap().clone()
    }

    /// Get all paths passed to derive_age_key
    pub fn get_age_calls(&self) -> Vec<String> {
        self.age_calls.lock().unwrap().clone()
    }
}

impl KeyGenBackend for MockKeyGenBackend {
    fn check_dependencies(&self) -> Result<()> {
        if self.deps_fail {
            Err(BootstrapError::MissingDependency {
                tool: self.missing_tool.clone(),
            }
            .into())
        } else {
            Ok(())
        }
    }

    fn generate_ssh_keypair(&self, output_dir: &Path) -> Result<SshKeyPair> {
        self.keygen_calls
            .lock()
            .unwrap()
            .push(output_dir.to_string_lossy().to_string());

        if self.keygen_fail {
            Err(BootstrapError::KeyGenCommandFailed {
                stderr: "Mock keygen failure".to_string(),
            }
            .into())
        } else {
            let private_key = output_dir.join("ssh_host_ed25519_key");
            let public_key = output_dir.join("ssh_host_ed25519_key.pub");
            Ok(SshKeyPair::new(private_key, public_key))
        }
    }

    fn derive_age_key(&self, ssh_public_key: &Path) -> Result<String> {
        self.age_calls
            .lock()
            .unwrap()
            .push(ssh_public_key.to_string_lossy().to_string());

        if self.age_fail {
            Err(BootstrapError::AgeCommandFailed {
                stderr: "Mock age derivation failure".to_string(),
            }
            .into())
        } else {
            Ok(self.age_key.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_mock_keygen_backend_default_success() {
        let backend = MockKeyGenBackend::new();
        assert!(backend.check_dependencies().is_ok());

        let output_dir = PathBuf::from("/tmp/test");
        let result = backend.generate_ssh_keypair(&output_dir);
        assert!(result.is_ok());

        let pair = result.unwrap();
        assert_eq!(
            pair.private_key,
            PathBuf::from("/tmp/test/ssh_host_ed25519_key")
        );
        assert_eq!(
            pair.public_key,
            PathBuf::from("/tmp/test/ssh_host_ed25519_key.pub")
        );

        let public_key = PathBuf::from("/tmp/test/ssh_host_ed25519_key.pub");
        let age_key = backend.derive_age_key(&public_key).unwrap();
        assert!(age_key.starts_with("age1"));
    }

    #[test]
    fn test_mock_keygen_backend_missing_dependency() {
        let backend = MockKeyGenBackend::new().with_missing_dependency("ssh-to-age");
        let result = backend.check_dependencies();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("ssh-to-age"));
    }

    #[test]
    fn test_mock_keygen_backend_keygen_failure() {
        let backend = MockKeyGenBackend::new().with_keygen_failure();
        let output_dir = PathBuf::from("/tmp/test");
        let result = backend.generate_ssh_keypair(&output_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Mock keygen"));
    }

    #[test]
    fn test_mock_keygen_backend_age_failure() {
        let backend = MockKeyGenBackend::new().with_age_failure();
        let public_key = PathBuf::from("/tmp/test/key.pub");
        let result = backend.derive_age_key(&public_key);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Mock age"));
    }

    #[test]
    fn test_mock_keygen_backend_custom_age_key() {
        let custom_key = "age1custom0key0for0specific0test0000000000000000000000000";
        let backend = MockKeyGenBackend::new().with_age_key(custom_key);
        let public_key = PathBuf::from("/tmp/test/key.pub");
        let result = backend.derive_age_key(&public_key).unwrap();
        assert_eq!(result, custom_key);
    }

    #[test]
    fn test_mock_keygen_backend_tracks_calls() {
        let backend = MockKeyGenBackend::new();

        let output_dir1 = PathBuf::from("/tmp/host1");
        let output_dir2 = PathBuf::from("/tmp/host2");
        backend.generate_ssh_keypair(&output_dir1).unwrap();
        backend.generate_ssh_keypair(&output_dir2).unwrap();

        let calls = backend.get_keygen_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], "/tmp/host1");
        assert_eq!(calls[1], "/tmp/host2");

        let public_key = PathBuf::from("/tmp/host1/key.pub");
        backend.derive_age_key(&public_key).unwrap();

        let age_calls = backend.get_age_calls();
        assert_eq!(age_calls.len(), 1);
        assert_eq!(age_calls[0], "/tmp/host1/key.pub");
    }
}
