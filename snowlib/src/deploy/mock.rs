use crate::deploy::{ActivationMode, Deployer};
use crate::error::Result;
use crate::nix::{DeployTarget, StorePath};
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
}
