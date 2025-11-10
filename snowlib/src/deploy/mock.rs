use crate::deploy::{ActivationMode, Deployer};
use crate::error::Result;
use crate::nix::{DeployTarget, StorePath};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::thread;
use std::time::Duration;

/// Simple mock deployer for testing
///
/// Always succeeds with configurable delays and messages.
/// Useful for basic testing and development iteration.
#[derive(Debug, Clone)]
pub struct MockDeployer {
    /// Milliseconds to sleep during operations (simulates network/work)
    pub delay_ms: u64,

    /// List of hostnames that should fail
    pub should_fail: Vec<String>,
}

impl Default for MockDeployer {
    fn default() -> Self {
        Self {
            delay_ms: 100,
            should_fail: vec![],
        }
    }
}

impl MockDeployer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_delay(mut self, ms: u64) -> Self {
        self.delay_ms = ms;
        self
    }

    pub fn with_failures(mut self, hostnames: Vec<String>) -> Self {
        self.should_fail = hostnames;
        self
    }
}

impl Deployer for MockDeployer {
    fn copy_closure(&self, target: &DeployTarget, _store_path: &StorePath) -> Result<()> {
        thread::sleep(Duration::from_millis(self.delay_ms));

        if self.should_fail.contains(&target.hostname) {
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
        thread::sleep(Duration::from_millis(self.delay_ms));

        if self.should_fail.contains(&target.hostname) {
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Mock: activation failed"),
            }
            .into());
        }

        Ok(())
    }
}

/// Fixture for a complete deployment scenario
///
/// Represents a recorded deployment scenario with multiple hosts.
/// Can be serialized to/from JSON for testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentFixture {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
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

/// Deployer that replays recorded deployment scenarios from JSON fixtures
///
/// Useful for testing complex deployment scenarios with realistic
/// output, timing, and error conditions.
#[derive(Debug, Clone)]
pub struct FixtureDeployer {
    fixtures: HashMap<String, HostFixture>,
}

impl FixtureDeployer {
    /// Load a fixture from a JSON file
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| crate::error::IoError::CurrentDir { source: e })?;

        let fixture: DeploymentFixture = serde_json::from_str(&contents)
            .map_err(|e| crate::error::IoError::JsonSerialize { source: e })?;

        Ok(Self::from_fixture(fixture))
    }

    /// Create from a DeploymentFixture directly (useful for tests)
    pub fn from_fixture(fixture: DeploymentFixture) -> Self {
        let mut fixtures: HashMap<String, HostFixture> = HashMap::new();

        for host in fixture.hosts {
            // Index by hostname
            fixtures.insert(host.hostname.clone(), host.clone());

            // Also index by FQDN if metadata is present
            if let Some(ref metadata) = host.metadata {
                fixtures.insert(metadata.fqdn.clone(), host);
            }
        }

        Self { fixtures }
    }

    /// Get fixture for a specific hostname
    fn get_fixture(&self, hostname: &str) -> Result<&HostFixture> {
        self.fixtures.get(hostname).ok_or_else(|| {
            crate::error::DeployError::Failed {
                host: hostname.to_string(),
                source: std::io::Error::other(format!("No fixture found for host: {}", hostname)),
            }
            .into()
        })
    }

    /// Simulate an operation with output and timing
    fn simulate_operation(&self, op: &OperationFixture) -> Result<()> {
        // Simulate work with delay
        thread::sleep(Duration::from_millis(op.duration_ms));

        // Print output (simulates real command output)
        if !op.stdout.is_empty() {
            print!("{}", op.stdout);
        }
        if !op.stderr.is_empty() {
            eprint!("{}", op.stderr);
        }

        if !op.success {
            return Err(crate::error::DeployError::Failed {
                host: "fixture".to_string(),
                source: std::io::Error::other("Operation failed (from fixture)"),
            }
            .into());
        }

        Ok(())
    }
}

impl Deployer for FixtureDeployer {
    fn copy_closure(&self, target: &DeployTarget, _store_path: &StorePath) -> Result<()> {
        let fixture = self.get_fixture(&target.hostname)?;
        self.simulate_operation(&fixture.copy_closure)
    }

    fn activate(
        &self,
        target: &DeployTarget,
        _store_path: &StorePath,
        _mode: ActivationMode,
    ) -> Result<()> {
        let fixture = self.get_fixture(&target.hostname)?;
        self.simulate_operation(&fixture.activate)
    }
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
    fn test_fixture_deployer_success() {
        let fixture = DeploymentFixture {
            name: "Test".to_string(),
            description: None,
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

        let deployer = FixtureDeployer::from_fixture(fixture);
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
    fn test_fixture_deployer_missing_host() {
        let fixture = DeploymentFixture {
            name: "Test".to_string(),
            description: None,
            hosts: vec![],
        };

        let deployer = FixtureDeployer::from_fixture(fixture);
        let target = DeployTarget {
            user: "root".to_string(),
            hostname: "missing.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).is_err());
    }
}
