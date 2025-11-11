use crate::error::{DeployError, Result};
use std::process::Command;

use super::{ActivationMode, Deployer};
use crate::nix::{DeployTarget, StorePath};

/// SSH-based deployer using subprocess commands (MVP implementation)
pub struct SshDeployer {
    /// Optional SSH user (defaults to root in target)
    pub ssh_user: Option<String>,
    /// Quiet mode - suppress subprocess output
    pub quiet: bool,
}

impl SshDeployer {
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

impl Deployer for SshDeployer {
    fn copy_closure(&self, target: &DeployTarget, store_path: &StorePath) -> Result<()> {
        let ssh_target = format!("ssh://{}", self.get_destination(target));

        let mut cmd = Command::new("nix");
        cmd.args(["copy", "--to", &ssh_target, store_path.as_str()]);

        // Configure SSH to disable interactive prompts for nix copy
        cmd.env("NIX_SSHOPTS", "-o BatchMode=yes");

        let success = if self.quiet {
            // Quiet mode: capture output
            let output = cmd.output().map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: e,
            })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(DeployError::Failed {
                    host: target.hostname.clone(),
                    source: std::io::Error::other(format!("nix copy failed: {}", stderr)),
                }
                .into());
            }
            output.status.success()
        } else {
            // Verbose mode: show output
            let status = cmd.status().map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: e,
            })?;
            if !status.success() {
                return Err(DeployError::Failed {
                    host: target.hostname.clone(),
                    source: std::io::Error::other("nix copy failed (see output above)"),
                }
                .into());
            }
            status.success()
        };

        if !success {
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("nix copy failed"),
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

        let success = if self.quiet {
            // Quiet mode: capture output
            let output = cmd.output().map_err(|e| DeployError::SshFailed {
                destination: self.get_destination(target),
                source: e,
            })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(DeployError::Failed {
                    host: target.hostname.clone(),
                    source: std::io::Error::other(format!("Activation failed: {}", stderr)),
                }
                .into());
            }
            output.status.success()
        } else {
            // Verbose mode: show output
            let status = cmd.status().map_err(|e| DeployError::SshFailed {
                destination: self.get_destination(target),
                source: e,
            })?;
            if !status.success() {
                return Err(DeployError::Failed {
                    host: target.hostname.clone(),
                    source: std::io::Error::other("Activation failed (see output above)"),
                }
                .into());
            }
            status.success()
        };

        if !success {
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other("Activation failed"),
            }
            .into());
        }

        Ok(())
    }
}

impl Default for SshDeployer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_destination_default() {
        let deployer = SshDeployer::new();
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
        let deployer = SshDeployer::new().with_user("deploy");
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
}
