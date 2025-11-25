use super::{RebootBackend, RebootCheck, RebootReason};
use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::nix::StorePath;
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;

/// Mock reboot operations for testing
///
/// Simulates reboot detection and execution without requiring SSH connections.
/// Allows configuring which hosts require reboot and which hosts fail during reboot.
#[derive(Debug, Clone, Default)]
pub struct MockRebootBackend {
    /// Hosts that require reboot (hostname -> reason)
    reboot_required: HashMap<String, RebootReason>,
    /// Hosts that should fail during reboot execution
    reboot_failures: Vec<String>,
}

impl MockRebootBackend {
    /// Create a new MockRebootBackend with no reboot requirements
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure hosts that require reboot
    pub fn with_reboot_required(mut self, hosts: HashMap<String, RebootReason>) -> Self {
        self.reboot_required = hosts;
        self
    }

    /// Configure hosts that should fail during reboot
    pub fn with_reboot_failures(mut self, hostnames: Vec<String>) -> Self {
        self.reboot_failures = hostnames;
        self
    }

    /// Check if a hostname requires reboot (tries both full and short hostname)
    fn check_hostname(&self, hostname: &str) -> Option<RebootReason> {
        // Try exact match first
        if let Some(reason) = self.reboot_required.get(hostname) {
            return Some(reason.clone());
        }

        // Try short hostname
        let short = hostname.split('.').next().unwrap_or(hostname);
        self.reboot_required.get(short).cloned()
    }

    /// Check if a hostname should fail reboot
    fn should_fail_reboot(&self, hostname: &str) -> bool {
        self.reboot_failures.contains(&hostname.to_string()) || {
            let short = hostname.split('.').next().unwrap_or(hostname);
            self.reboot_failures.contains(&short.to_string())
        }
    }
}

#[async_trait]
impl RebootBackend for MockRebootBackend {
    async fn check_reboot_required(
        &self,
        target: &DeployTarget,
        _new_system: &StorePath,
    ) -> Result<RebootCheck> {
        if let Some(reason) = self.check_hostname(&target.hostname) {
            Ok(RebootCheck::required(reason))
        } else {
            Ok(RebootCheck::not_required())
        }
    }

    async fn reboot_and_wait(&self, target: &DeployTarget) -> Result<()> {
        if !cfg!(debug_assertions) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        if self.should_fail_reboot(&target.hostname) {
            return Err(crate::error::DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Mock: reboot failed"),
            }
            .into());
        }

        Ok(())
    }
}
