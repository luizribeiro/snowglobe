use crate::deploy::{ActivationMode, Deployer};
use crate::error::Result;
use crate::nix::{DeployTarget, StorePath};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::thread;
use std::time::Duration;

/// Mock deployer for testing
///
/// Can operate in two modes:
/// 1. Simple mode (no fixture): Always succeeds with configurable delays
/// 2. Fixture mode: Auto-generates realistic output, only fails hosts specified in fixture
#[derive(Debug, Clone, Default)]
pub struct MockDeployer {
    fixture: Option<DeploymentFixture>,
    /// List of hostnames that should fail when not using fixtures (simple mode)
    should_fail: Vec<String>,
}

impl MockDeployer {
    /// Create a new MockDeployer with default behavior (always succeeds)
    pub fn new() -> Self {
        Self::default()
    }

    /// Create from a DeploymentFixture (useful for tests)
    pub fn from_fixture(fixture: DeploymentFixture) -> Self {
        Self {
            fixture: Some(fixture),
            should_fail: vec![],
        }
    }

    /// Load a fixture from a JSON file
    pub fn from_file(path: &Path) -> Result<Self> {
        let fixture = DeploymentFixture::from_file(path)?;
        Ok(Self::from_fixture(fixture))
    }

    /// Set failures for simple mode (when not using fixtures)
    pub fn with_failures(mut self, hostnames: Vec<String>) -> Self {
        self.should_fail = hostnames;
        self
    }

    /// Generate realistic stdout for copy_closure operation
    fn generate_copy_closure_output(&self, hostname: &str) -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let num_paths = rng.gen_range(10..50);
        format!(
            "copying {} paths to ssh://root@{}...\n",
            num_paths, hostname
        )
    }

    /// Generate realistic stdout for activate operation
    fn generate_activate_output(&self, success: bool) -> String {
        if success {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let messages = [
                "activation successful\n",
                "reloading services...\nactivation complete\n",
                "updating system profile...\nactivation successful\n",
            ];
            messages[rng.gen_range(0..messages.len())].to_string()
        } else {
            "starting services...\n".to_string()
        }
    }

    /// Check if a host should fail and at which phase
    fn should_fail_at(&self, hostname: &str, phase: &str) -> (bool, Option<String>) {
        if let Some(fixture) = &self.fixture {
            // Try exact match first
            if let Some(failure_spec) = fixture.failures.get(hostname) {
                if failure_spec.phase == phase {
                    let error = failure_spec
                        .error
                        .clone()
                        .unwrap_or_else(|| "Operation failed".to_string());
                    return (true, Some(error));
                }
            }

            // Try hostname without domain (e.g., "nitrogen.example.com" -> "nitrogen")
            let short_hostname = hostname.split('.').next().unwrap_or(hostname);
            if let Some(failure_spec) = fixture.failures.get(short_hostname) {
                if failure_spec.phase == phase {
                    let error = failure_spec
                        .error
                        .clone()
                        .unwrap_or_else(|| "Operation failed".to_string());
                    return (true, Some(error));
                }
            }
        }
        (false, None)
    }
}

impl Deployer for MockDeployer {
    fn copy_closure(&self, target: &DeployTarget, _store_path: &StorePath) -> Result<()> {
        // Realistic delay (1-2 seconds)
        thread::sleep(Duration::from_millis(1000));

        // Generate realistic output
        print!("{}", self.generate_copy_closure_output(&target.hostname));

        // Check if this host should fail at copy_closure
        let (should_fail, error_msg) = self.should_fail_at(&target.hostname, "copy_closure");

        if should_fail {
            if let Some(error) = error_msg {
                eprintln!("error: {}", error);
            }
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Copy closure failed"),
            }
            .into());
        }

        // Simple mode fallback for backward compatibility
        if self.fixture.is_none() && self.should_fail.contains(&target.hostname) {
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Mock: copy_closure failed"),
            }
            .into());
        }

        Ok(())
    }

    fn activate(
        &self,
        target: &DeployTarget,
        _store_path: &StorePath,
        _mode: ActivationMode,
    ) -> Result<()> {
        // Check if this host should fail at activate
        let (should_fail, error_msg) = self.should_fail_at(&target.hostname, "activate");

        // Realistic delay (500ms-1s)
        thread::sleep(Duration::from_millis(500));

        // Generate realistic output
        print!("{}", self.generate_activate_output(!should_fail));

        if should_fail {
            if let Some(error) = error_msg {
                eprintln!("error: {}", error);
            }
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Activation failed"),
            }
            .into());
        }

        // Simple mode fallback for backward compatibility
        if self.fixture.is_none() && self.should_fail.contains(&target.hostname) {
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Mock: activation failed"),
            }
            .into());
        }

        Ok(())
    }
}

/// Simplified deployment failure specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureSpec {
    /// Which phase fails: "copy_closure" or "activate"
    pub phase: String,
    /// Error message to display
    #[serde(default)]
    pub error: Option<String>,
}

/// Fixture for a complete deployment scenario
///
/// Simplified format that only specifies failures.
/// All hosts succeed by default with auto-generated realistic output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentFixture {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Map of hostname to failure spec (only for hosts that should fail)
    #[serde(default)]
    pub failures: HashMap<String, FailureSpec>,
    /// Legacy format support - will be removed
    #[serde(default)]
    pub hosts: Vec<HostFixture>,
}

impl DeploymentFixture {
    /// Load a fixture from a JSON file
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| crate::error::IoError::CurrentDir { source: e })?;

        let fixture: DeploymentFixture = serde_json::from_str(&contents)
            .map_err(|e| crate::error::IoError::JsonSerialize { source: e })?;

        Ok(fixture)
    }

    /// Get fixture for a specific host by name
    pub fn get_host(&self, hostname: &str) -> Option<&HostFixture> {
        self.hosts.iter().find(|h| h.hostname == hostname)
    }

    /// Get metadata for a specific host
    pub fn get_metadata(&self, hostname: &str) -> Option<&MetadataFixture> {
        self.get_host(hostname)?.metadata.as_ref()
    }

    /// Get build fixture for a specific host
    pub fn get_build(&self, hostname: &str) -> Option<&BuildFixture> {
        self.get_host(hostname)?.build.as_ref()
    }
}

/// Fixture for a single host's deployment
///
/// Contains the expected behavior for metadata, build, copy_closure, and activate operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostFixture {
    pub hostname: String,
    #[serde(default)]
    pub metadata: Option<MetadataFixture>,
    #[serde(default)]
    pub build: Option<BuildFixture>,
    pub copy_closure: OperationFixture,
    pub activate: OperationFixture,
}

/// Fixture for host metadata
///
/// Mocks the result of get_host_metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataFixture {
    pub fqdn: String,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Fixture for build phase
///
/// Mocks the result of build_host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildFixture {
    pub success: bool,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    pub duration_ms: u64,
    #[serde(default)]
    pub store_path: Option<String>,
}

/// Fixture for a single deployment operation
///
/// Records the success/failure status, output, and timing for an operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationFixture {
    pub success: bool,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_deployer_success() {
        let deployer = MockDeployer::new();
        let target = DeployTarget {
            user: "root".to_string(),
            hostname: "test.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .is_ok());
    }

    #[test]
    fn test_mock_deployer_failure() {
        let deployer = MockDeployer::new().with_failures(vec!["fail.example.com".to_string()]);

        let target = DeployTarget {
            user: "root".to_string(),
            hostname: "fail.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_err());
    }

    #[test]
    fn test_fixture_deserialization() {
        let json = r#"{
            "name": "Test scenario",
            "hosts": [
                {
                    "hostname": "test.example.com",
                    "copy_closure": {
                        "success": true,
                        "stdout": "copying...",
                        "stderr": "",
                        "duration_ms": 100
                    },
                    "activate": {
                        "success": true,
                        "stdout": "activating...",
                        "stderr": "",
                        "duration_ms": 50
                    }
                }
            ]
        }"#;

        let fixture: DeploymentFixture = serde_json::from_str(json).unwrap();
        assert_eq!(fixture.name, "Test scenario");
        assert_eq!(fixture.hosts.len(), 1);
        assert_eq!(fixture.hosts[0].hostname, "test.example.com");
    }

    #[test]
    fn test_mock_deployer_with_fixture() {
        let fixture = DeploymentFixture {
            name: "Test".to_string(),
            description: None,
            failures: HashMap::new(),
            hosts: vec![HostFixture {
                hostname: "test.example.com".to_string(),
                metadata: None,
                build: None,
                copy_closure: OperationFixture {
                    success: true,
                    stdout: "copying...\n".to_string(),
                    stderr: "".to_string(),
                    duration_ms: 10,
                },
                activate: OperationFixture {
                    success: true,
                    stdout: "activating...\n".to_string(),
                    stderr: "".to_string(),
                    duration_ms: 5,
                },
            }],
        };

        let deployer = MockDeployer::from_fixture(fixture);
        let target = DeployTarget {
            user: "root".to_string(),
            hostname: "test.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .is_ok());
    }

    #[test]
    fn test_mock_deployer_fixture_fallback() {
        // When a host is not in the fixture, succeeds with auto-generated output
        let fixture = DeploymentFixture {
            name: "Test".to_string(),
            description: None,
            failures: HashMap::new(),
            hosts: vec![],
        };

        let deployer = MockDeployer::from_fixture(fixture);
        let target = DeployTarget {
            user: "root".to_string(),
            hostname: "missing.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        // Should succeed (falls back to simple mode)
        assert!(deployer.copy_closure(&target, &store_path).is_ok());
    }
}
