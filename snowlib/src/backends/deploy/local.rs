use crate::error::{DeployError, Result};
use crate::nix::{ActivationMode, StorePath};
use crate::process::{OutputMode, StreamingExecutor};
use std::process::Command;

use super::{DeployBackend, DeployTarget};

/// Local deployment backend for rebuilding the current host
///
/// Unlike SshDeployBackend which deploys to remote hosts over SSH,
/// LocalDeployBackend operates on the local machine directly.
pub struct LocalDeployBackend {
    /// Output mode for subprocess operations
    pub output_mode: OutputMode,
}

impl LocalDeployBackend {
    pub fn new() -> Self {
        Self {
            output_mode: OutputMode::Inherit,
        }
    }

    pub fn with_output_mode(mut self, mode: OutputMode) -> Self {
        self.output_mode = mode;
        self
    }

    pub fn with_quiet(self) -> Self {
        self.with_output_mode(OutputMode::Capture)
    }
}

impl DeployBackend for LocalDeployBackend {
    fn copy_closure(&self, _target: &DeployTarget, _store_path: &StorePath) -> Result<()> {
        Ok(())
    }

    fn activate(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<()> {
        let switch_to_configuration =
            format!("{}/bin/switch-to-configuration", store_path.as_str());

        let mode_arg = match mode {
            ActivationMode::Switch => "switch",
            ActivationMode::Boot => "boot",
            ActivationMode::Test => "test",
            ActivationMode::DryActivate => "dry-activate",
        };

        let mut cmd = Command::new("sudo");
        cmd.args([&switch_to_configuration, mode_arg]);

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(self.output_mode)
            .execute()
            .map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: e,
            })?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
            let error_msg = if stderr.is_empty() {
                "Activation failed".to_string()
            } else {
                let lines: Vec<&str> = stderr.lines().collect();
                let relevant_lines = if lines.len() <= 3 {
                    lines.join("\n")
                } else {
                    lines[lines.len() - 3..].join("\n")
                };
                format!("Activation failed:\n{}", relevant_lines)
            };
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(error_msg),
            }
            .into());
        }

        Ok(())
    }
}

impl Default for LocalDeployBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_closure_is_noop() {
        let backend = LocalDeployBackend::new();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/fake-path".to_string());

        let result = backend.copy_closure(&target, &store_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_activate_nonexistent_path() {
        let backend = LocalDeployBackend::new().with_quiet();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/nonexistent-path".to_string());

        let result = backend.activate(&target, &store_path, ActivationMode::Test);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = format!("{:?}", err);

        assert!(
            err_string.contains("DeployError") || err_string.contains("Failed"),
            "Error should indicate deployment failure: {}",
            err_string
        );
    }
}
