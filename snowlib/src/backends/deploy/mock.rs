use super::{should_update_profile, DeployBackend, DeployTarget, HostSystems};
use crate::error::Result;
use crate::nix::{ActivationMode, StorePath};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

// ============================================================================
// MockDeployBackend
// ============================================================================

/// Mock deployer for testing
///
/// Can operate in two modes:
/// 1. Simple mode (no fixture): Always succeeds with configurable delays
/// 2. Fixture mode: Auto-generates realistic output, only fails hosts specified in fixture
#[derive(Debug, Clone)]
pub struct MockDeployBackend {
    fixture: Option<DeploymentFixture>,
    /// List of hostnames that should fail when not using fixtures (simple mode) - both copy and activate
    should_fail: Vec<String>,
    /// List of hostnames that should fail during copy only
    copy_failures: Vec<String>,
    /// List of hostnames that should fail during activate only
    activate_failures: Vec<String>,
    /// Track which hosts had profile updates (hostname, mode)
    profile_updates: Arc<Mutex<Vec<(String, ActivationMode)>>>,
    copies: Arc<Mutex<Vec<String>>>,
    agent_hangs: bool,
    /// Systems per target hostname; hosts not listed run a system no build matches
    host_systems: HashMap<String, HostSystems>,
    host_systems_failures: Vec<String>,
    host_systems_timeouts: Vec<String>,
}

impl Default for MockDeployBackend {
    fn default() -> Self {
        Self {
            fixture: None,
            should_fail: Vec::new(),
            copy_failures: Vec::new(),
            activate_failures: Vec::new(),
            profile_updates: Arc::new(Mutex::new(Vec::new())),
            copies: Arc::new(Mutex::new(Vec::new())),
            agent_hangs: false,
            host_systems: HashMap::new(),
            host_systems_failures: Vec::new(),
            host_systems_timeouts: Vec::new(),
        }
    }
}

impl MockDeployBackend {
    /// Create a new MockDeployBackend with default behavior (always succeeds)
    pub fn new() -> Self {
        Self::default()
    }

    /// Create from a DeploymentFixture (useful for tests)
    pub fn from_fixture(fixture: DeploymentFixture) -> Self {
        Self {
            fixture: Some(fixture),
            ..Self::default()
        }
    }

    /// Load a fixture from a JSON file
    pub fn from_file(path: &Path) -> Result<Self> {
        let fixture = DeploymentFixture::from_file(path)?;
        Ok(Self::from_fixture(fixture))
    }

    /// Get a list of hosts and modes where profile updates were called
    pub fn get_profile_updates(&self) -> Vec<(String, ActivationMode)> {
        self.profile_updates.lock().unwrap().clone()
    }

    /// Check if profile update was called for a specific host
    pub fn profile_update_called_for(&self, hostname: &str) -> bool {
        self.profile_updates
            .lock()
            .unwrap()
            .iter()
            .any(|(h, _)| h == hostname)
    }

    /// How many copies to the target `hostname` were attempted
    pub fn copy_count(&self, hostname: &str) -> usize {
        self.copies
            .lock()
            .unwrap()
            .iter()
            .filter(|h| *h == hostname)
            .count()
    }

    /// Fails preflight the way a hung ssh-agent does
    pub fn with_hung_agent(mut self) -> Self {
        self.agent_hangs = true;
        self
    }

    /// Set failures for simple mode (when not using fixtures) - fails during both copy and activate
    pub fn with_failures(mut self, hostnames: Vec<String>) -> Self {
        self.should_fail = hostnames;
        self
    }

    /// Set copy failures for simple mode (when not using fixtures)
    pub fn with_copy_failures(mut self, hostnames: Vec<String>) -> Self {
        self.copy_failures = hostnames;
        self
    }

    /// Set the running and boot-next systems of each target hostname
    pub fn with_host_systems(mut self, systems: HashMap<String, HostSystems>) -> Self {
        self.host_systems = systems;
        self
    }

    /// Hosts whose installed systems can't be read
    pub fn with_host_systems_failures(mut self, hostnames: Vec<String>) -> Self {
        self.host_systems_failures = hostnames;
        self
    }

    /// Hosts whose installed-systems probe times out, as with a hung ssh-agent
    pub fn with_host_systems_timeouts(mut self, hostnames: Vec<String>) -> Self {
        self.host_systems_timeouts = hostnames;
        self
    }

    /// Set activate failures for simple mode (when not using fixtures)
    pub fn with_activate_failures(mut self, hostnames: Vec<String>) -> Self {
        self.activate_failures = hostnames;
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

#[async_trait]
impl DeployBackend for MockDeployBackend {
    async fn preflight(&self) -> Result<()> {
        if self.agent_hangs {
            return Err(crate::error::DeployError::SshAgentNotResponding {
                seconds: crate::ssh_options::AGENT_CHECK_TIMEOUT.as_secs(),
            }
            .into());
        }
        Ok(())
    }

    async fn copy_closure(&self, target: &DeployTarget, _store_path: &StorePath) -> Result<()> {
        if !cfg!(debug_assertions) {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }

        print!("{}", self.generate_copy_closure_output(&target.hostname));
        self.copies.lock().unwrap().push(target.hostname.clone());

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

        if self.copy_failures.contains(&target.hostname) {
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Mock: copy_closure failed"),
            }
            .into());
        }

        Ok(())
    }

    async fn host_systems(&self, target: &DeployTarget) -> Result<HostSystems> {
        if self.host_systems_timeouts.contains(&target.hostname) {
            return Err(crate::error::DeployError::SshFailed {
                destination: target.hostname.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "ssh did not finish within 60s",
                ),
            }
            .into());
        }
        if self.host_systems_failures.contains(&target.hostname) {
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Mock: host_systems failed"),
            }
            .into());
        }
        Ok(self
            .host_systems
            .get(&target.hostname)
            .cloned()
            .unwrap_or_else(|| {
                HostSystems::both(StorePath::new(format!(
                    "/nix/store/mock-running-{}",
                    target.hostname
                )))
            }))
    }

    async fn activate(
        &self,
        target: &DeployTarget,
        _store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<()> {
        // Track profile updates for modes that should update profile
        if should_update_profile(mode) {
            self.profile_updates
                .lock()
                .unwrap()
                .push((target.hostname.clone(), mode));
        }

        let (should_fail, error_msg) = self.should_fail_at(&target.hostname, "activate");

        if !cfg!(debug_assertions) {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        print!("{}", self.generate_activate_output(!should_fail));

        if should_fail {
            let error_text = if let Some(error) = error_msg.as_ref() {
                eprintln!("error: {}", error);
                error.clone()
            } else {
                "Activation failed".to_string()
            };
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(error_text),
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

        if self.activate_failures.contains(&target.hostname) {
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Mock: activation failed"),
            }
            .into());
        }

        Ok(())
    }
}

// ============================================================================
// Fixture Types (shared across all mocks)
// ============================================================================

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

/// Fixture for build step
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

    fn target(hostname: &str) -> DeployTarget {
        DeployTarget {
            user: None,
            hostname: hostname.to_string(),
            port: None,
        }
    }

    #[tokio::test]
    async fn test_host_systems_returns_configured_systems() {
        let systems = HostSystems {
            running: StorePath::new("/nix/store/abc-nixos-system-hydrogen".to_string()),
            next_boot: StorePath::new("/nix/store/def-nixos-system-hydrogen".to_string()),
        };
        let deployer = MockDeployBackend::new().with_host_systems(HashMap::from([(
            "hydrogen.example.com".to_string(),
            systems.clone(),
        )]));

        let reported = deployer
            .host_systems(&target("hydrogen.example.com"))
            .await
            .unwrap();

        assert_eq!(reported, systems);
    }

    #[tokio::test]
    async fn test_host_systems_default_to_a_system_no_build_produces() {
        use crate::backends::nix::{MockNixBackend, NixBackend};
        use crate::nix::{Flake, SystemType};

        let built = MockNixBackend::new()
            .build_host(
                &Flake::test_context(),
                "oxygen.example.com",
                SystemType::NixOS,
                crate::process::OutputMode::Capture,
            )
            .await
            .unwrap();
        let systems = MockDeployBackend::new()
            .host_systems(&target("oxygen.example.com"))
            .await
            .unwrap();

        assert!(!systems.are(&built));
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_success() {
        let deployer = MockDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "test.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_failure() {
        let deployer = MockDeployBackend::new().with_failures(vec!["fail.example.com".to_string()]);

        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "fail.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_err());
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

    #[tokio::test]
    async fn test_mock_deploy_backend_with_fixture() {
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

        let deployer = MockDeployBackend::from_fixture(fixture);
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "test.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_ok());
        assert!(deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_mock_deploy_backend_fixture_fallback() {
        let fixture = DeploymentFixture {
            name: "Test".to_string(),
            description: None,
            failures: HashMap::new(),
            hosts: vec![],
        };

        let deployer = MockDeployBackend::from_fixture(fixture);
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "missing.example.com".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test".to_string());

        assert!(deployer.copy_closure(&target, &store_path).await.is_ok());
    }
}
