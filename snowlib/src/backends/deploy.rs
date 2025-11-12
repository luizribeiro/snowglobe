use crate::error::{DeployError, Result};
use crate::nix::{DeployTarget, StorePath};
use std::process::Command;

/// Activation mode for NixOS configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivationMode {
    /// Activate and make default (switch)
    Switch,
    /// Only set for next boot
    Boot,
    /// Test activation without making permanent
    Test,
    /// Dry run - show what would change
    DryActivate,
}

/// Result of a deployment operation
#[derive(Debug, Clone)]
pub struct DeployResult {
    pub success: bool,
    pub message: String,
}

impl DeployResult {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
        }
    }
}

/// Trait for deployment backend implementations
///
/// Implementations must be Send + Sync to support parallel deployments
pub trait DeployBackend: Send + Sync {
    /// Copy a nix closure to the target host
    fn copy_closure(&self, target: &DeployTarget, store_path: &StorePath) -> Result<()>;

    /// Activate a configuration on the target host
    fn activate(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<()>;

    /// Deploy (copy + activate) in one operation
    fn deploy(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<DeployResult> {
        self.copy_closure(target, store_path)?;
        self.activate(target, store_path, mode)?;
        Ok(DeployResult::success(format!(
            "Deployed successfully to {}",
            target.hostname
        )))
    }
}

/// SSH-based deployer using subprocess commands (MVP implementation)
pub struct SshDeployBackend {
    /// Optional SSH user (defaults to root in target)
    pub ssh_user: Option<String>,
    /// Quiet mode - suppress subprocess output
    pub quiet: bool,
}

impl SshDeployBackend {
    pub fn new() -> Self {
        Self {
            ssh_user: None,
            quiet: false,
        }
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.ssh_user = Some(user.into());
        self
    }

    pub fn with_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// Get the effective SSH destination
    fn get_destination(&self, target: &DeployTarget) -> String {
        if let Some(ref user) = self.ssh_user {
            format!("{}@{}", user, target.hostname)
        } else {
            target.ssh_destination()
        }
    }

    /// Build SSH command with proper flags
    fn ssh_command(&self, target: &DeployTarget, args: &[&str]) -> Command {
        let mut cmd = Command::new("ssh");

        // Disable interactive prompts - will error instead of prompting
        cmd.arg("-o").arg("BatchMode=yes");

        if let Some(port) = target.port {
            cmd.arg("-p").arg(port.to_string());
        }

        cmd.arg(self.get_destination(target));
        cmd.args(args);
        cmd
    }
}

impl DeployBackend for SshDeployBackend {
    fn copy_closure(&self, target: &DeployTarget, store_path: &StorePath) -> Result<()> {
        let ssh_target = format!("ssh://{}", self.get_destination(target));

        let mut cmd = Command::new("nix");
        cmd.args(["copy", "--to", &ssh_target, store_path.as_str()]);

        // Configure SSH to disable interactive prompts for nix copy
        cmd.env("NIX_SSHOPTS", "-o BatchMode=yes");

        // Always capture output to get detailed error messages
        let output = cmd.output().map_err(|e| DeployError::Failed {
            host: target.hostname.clone(),
            source: e,
        })?;

        // In verbose mode, print stdout/stderr for user to see
        if !self.quiet {
            if !output.stdout.is_empty() {
                print!("{}", String::from_utf8_lossy(&output.stdout));
            }
            if !output.stderr.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
            }
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let error_msg = if stderr.is_empty() {
                "nix copy failed".to_string()
            } else {
                // Include the last few lines of stderr for context (up to 3 lines)
                let lines: Vec<&str> = stderr.lines().collect();
                let relevant_lines = if lines.len() <= 3 {
                    lines.join("\n")
                } else {
                    lines[lines.len() - 3..].join("\n")
                };
                format!("nix copy failed:\n{}", relevant_lines)
            };
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(error_msg),
            }
            .into());
        }

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

        let mut cmd = self.ssh_command(target, &["sudo", &switch_to_configuration, mode_arg]);

        // Always capture output to get detailed error messages
        let output = cmd.output().map_err(|e| DeployError::SshFailed {
            destination: self.get_destination(target),
            source: e,
        })?;

        // In verbose mode, print stdout/stderr for user to see
        if !self.quiet {
            if !output.stdout.is_empty() {
                print!("{}", String::from_utf8_lossy(&output.stdout));
            }
            if !output.stderr.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
            }
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let error_msg = if stderr.is_empty() {
                "Activation failed".to_string()
            } else {
                // Include the last few lines of stderr for context (up to 3 lines)
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

impl Default for SshDeployBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_destination_default() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };

        assert_eq!(
            deployer.get_destination(&target),
            "root@hydrogen.example.com"
        );
    }

    #[test]
    fn test_get_destination_with_user() {
        let deployer = SshDeployBackend::new().with_user("deploy");
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };

        assert_eq!(
            deployer.get_destination(&target),
            "deploy@hydrogen.example.com"
        );
    }

    #[test]
    fn test_copy_closure_error_includes_stderr() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/fake-path".to_string());

        // This should fail and include stderr in the error
        let result = deployer.copy_closure(&target, &store_path);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = format!("{:?}", err);

        // The error should mention it's a deployment failure
        assert!(
            err_string.contains("DeployError") || err_string.contains("Failed"),
            "Error should be a deployment error: {}",
            err_string
        );
    }

    #[test]
    fn test_copy_closure_quiet_mode_captures_stderr() {
        let deployer = SshDeployBackend::new().with_quiet(true);
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/fake-path".to_string());

        // This should fail in quiet mode too
        let result = deployer.copy_closure(&target, &store_path);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = format!("{:?}", err);

        // Should contain error details even in quiet mode
        assert!(
            err_string.contains("DeployError") || err_string.contains("Failed"),
            "Error should be captured in quiet mode: {}",
            err_string
        );
    }

    #[test]
    fn test_activate_error_includes_stderr() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/fake-path".to_string());

        // This should fail with SSH error
        let result = deployer.activate(&target, &store_path, ActivationMode::Test);

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = format!("{:?}", err);

        // Should be a deployment or SSH error
        assert!(
            err_string.contains("DeployError")
                || err_string.contains("SshFailed")
                || err_string.contains("Failed"),
            "Error should indicate SSH/deployment failure: {}",
            err_string
        );
    }

    #[test]
    fn test_ssh_command_has_batch_mode() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "example.com".to_string(),
            port: Some(2222),
        };

        let cmd = deployer.ssh_command(&target, &["echo", "test"]);
        let cmd_string = format!("{:?}", cmd);

        // Should include BatchMode flag
        assert!(
            cmd_string.contains("BatchMode=yes"),
            "SSH command should include BatchMode: {}",
            cmd_string
        );

        // Should include port
        assert!(
            cmd_string.contains("2222"),
            "SSH command should include port: {}",
            cmd_string
        );
    }
}
