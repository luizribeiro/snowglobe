//! Mock implementation of SOPS backend for testing

use super::SopsBackend;
use crate::error::{BootstrapError, Result};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Mock implementation of SopsBackend for testing
///
/// Allows configuring success/failure behavior and tracks all method calls.
#[derive(Debug, Clone)]
pub struct MockSopsBackend {
    /// Should check_dependencies fail?
    deps_fail: bool,
    /// Should sops_yaml_exists return true?
    sops_yaml_exists: bool,
    /// Hosts that already exist in the mock .sops.yaml
    existing_hosts: Vec<String>,
    /// Should add_host_key fail?
    add_key_fail: bool,
    /// Number of files to return from update_keys
    files_updated: usize,
    /// Should update_keys fail?
    update_keys_fail: bool,
    /// Track calls to add_host_key (hostname, age_key)
    add_key_calls: Arc<Mutex<Vec<(String, String)>>>,
    /// Track calls to update_keys (flake_dir)
    update_calls: Arc<Mutex<Vec<String>>>,
}

impl Default for MockSopsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockSopsBackend {
    pub fn new() -> Self {
        Self {
            deps_fail: false,
            sops_yaml_exists: true,
            existing_hosts: Vec::new(),
            add_key_fail: false,
            files_updated: 3,
            update_keys_fail: false,
            add_key_calls: Arc::new(Mutex::new(Vec::new())),
            update_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Configure check_dependencies to fail (sops not installed)
    pub fn with_sops_missing(mut self) -> Self {
        self.deps_fail = true;
        self
    }

    /// Configure sops_yaml_exists to return false
    pub fn with_no_sops_yaml(mut self) -> Self {
        self.sops_yaml_exists = false;
        self
    }

    /// Add a host to the list of existing hosts
    pub fn with_existing_host(mut self, hostname: &str) -> Self {
        self.existing_hosts.push(hostname.to_string());
        self
    }

    /// Configure add_host_key to fail
    pub fn with_add_key_failure(mut self) -> Self {
        self.add_key_fail = true;
        self
    }

    /// Configure the number of files updated by update_keys
    pub fn with_files_updated(mut self, count: usize) -> Self {
        self.files_updated = count;
        self
    }

    /// Configure update_keys to fail
    pub fn with_update_keys_failure(mut self) -> Self {
        self.update_keys_fail = true;
        self
    }

    /// Get all calls to add_host_key
    pub fn get_add_key_calls(&self) -> Vec<(String, String)> {
        self.add_key_calls.lock().unwrap().clone()
    }

    /// Get all calls to update_keys
    pub fn get_update_calls(&self) -> Vec<String> {
        self.update_calls.lock().unwrap().clone()
    }
}

impl SopsBackend for MockSopsBackend {
    fn check_dependencies(&self) -> Result<()> {
        if self.deps_fail {
            Err(BootstrapError::SopsMissing.into())
        } else {
            Ok(())
        }
    }

    fn sops_yaml_exists(&self, _flake_dir: &Path) -> bool {
        self.sops_yaml_exists
    }

    fn host_exists(&self, _flake_dir: &Path, hostname: &str) -> Result<bool> {
        Ok(self.existing_hosts.contains(&hostname.to_string()))
    }

    fn add_host_key(&self, _flake_dir: &Path, hostname: &str, age_key: &str) -> Result<()> {
        self.add_key_calls
            .lock()
            .unwrap()
            .push((hostname.to_string(), age_key.to_string()));

        if self.add_key_fail {
            Err(BootstrapError::SopsYamlWriteFailed {
                path: ".sops.yaml".to_string(),
                source: std::io::Error::other("mock write failure"),
            }
            .into())
        } else {
            Ok(())
        }
    }

    fn update_keys(&self, flake_dir: &Path) -> Result<usize> {
        self.update_calls
            .lock()
            .unwrap()
            .push(flake_dir.to_string_lossy().to_string());

        if self.update_keys_fail {
            Err(BootstrapError::SopsUpdateKeysFailed {
                stderr: "mock updatekeys failure".to_string(),
            }
            .into())
        } else {
            Ok(self.files_updated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_mock_sops_backend_default_success() {
        let backend = MockSopsBackend::new();
        assert!(backend.check_dependencies().is_ok());
        assert!(backend.sops_yaml_exists(Path::new("/tmp")));
        assert!(!backend.host_exists(Path::new("/tmp"), "heron").unwrap());
        assert!(backend
            .add_host_key(Path::new("/tmp"), "heron", "age1key")
            .is_ok());
        assert_eq!(backend.update_keys(Path::new("/tmp")).unwrap(), 3);
    }

    #[test]
    fn test_mock_sops_missing() {
        let backend = MockSopsBackend::new().with_sops_missing();
        let result = backend.check_dependencies();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("sops"));
    }

    #[test]
    fn test_mock_no_sops_yaml() {
        let backend = MockSopsBackend::new().with_no_sops_yaml();
        assert!(!backend.sops_yaml_exists(Path::new("/tmp")));
    }

    #[test]
    fn test_mock_existing_host() {
        let backend = MockSopsBackend::new().with_existing_host("heron");
        assert!(backend.host_exists(Path::new("/tmp"), "heron").unwrap());
        assert!(!backend.host_exists(Path::new("/tmp"), "osprey").unwrap());
    }

    #[test]
    fn test_mock_add_key_failure() {
        let backend = MockSopsBackend::new().with_add_key_failure();
        let result = backend.add_host_key(Path::new("/tmp"), "heron", "age1key");
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_update_keys_failure() {
        let backend = MockSopsBackend::new().with_update_keys_failure();
        let result = backend.update_keys(Path::new("/tmp"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("updatekeys"));
    }

    #[test]
    fn test_mock_tracks_add_key_calls() {
        let backend = MockSopsBackend::new();
        backend
            .add_host_key(Path::new("/tmp"), "heron", "age1heron")
            .unwrap();
        backend
            .add_host_key(Path::new("/tmp"), "osprey", "age1osprey")
            .unwrap();

        let calls = backend.get_add_key_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ("heron".to_string(), "age1heron".to_string()));
        assert_eq!(calls[1], ("osprey".to_string(), "age1osprey".to_string()));
    }

    #[test]
    fn test_mock_tracks_update_calls() {
        let backend = MockSopsBackend::new();
        backend.update_keys(Path::new("/tmp/flake1")).unwrap();
        backend.update_keys(Path::new("/tmp/flake2")).unwrap();

        let calls = backend.get_update_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], "/tmp/flake1");
        assert_eq!(calls[1], "/tmp/flake2");
    }

    #[test]
    fn test_mock_custom_files_updated() {
        let backend = MockSopsBackend::new().with_files_updated(10);
        assert_eq!(backend.update_keys(Path::new("/tmp")).unwrap(), 10);
    }

    #[test]
    fn test_update_sops_full_workflow() {
        let backend = MockSopsBackend::new().with_files_updated(5);
        let result = backend
            .update_sops(Path::new("/tmp"), "heron", "age1heron")
            .unwrap();

        assert_eq!(result.sops_yaml_path, PathBuf::from("/tmp/.sops.yaml"));
        assert_eq!(result.files_updated, 5);

        // Verify methods were called
        let add_calls = backend.get_add_key_calls();
        assert_eq!(add_calls.len(), 1);
        assert_eq!(add_calls[0], ("heron".to_string(), "age1heron".to_string()));

        let update_calls = backend.get_update_calls();
        assert_eq!(update_calls.len(), 1);
    }

    #[test]
    fn test_update_sops_fails_when_sops_yaml_missing() {
        let backend = MockSopsBackend::new().with_no_sops_yaml();
        let result = backend.update_sops(Path::new("/tmp"), "heron", "age1key");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No .sops.yaml"));
    }

    #[test]
    fn test_update_sops_fails_when_host_exists() {
        let backend = MockSopsBackend::new().with_existing_host("heron");
        let result = backend.update_sops(Path::new("/tmp"), "heron", "age1key");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }
}
