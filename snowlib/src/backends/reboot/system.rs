use super::{RebootBackend, RebootCheck, RebootDetectionConfig, RebootReason};
use crate::backends::deploy::DeployTarget;
use crate::error::{DeployError, Result};
use crate::nix::StorePath;
use crate::process::{CommandResult, OutputMode, StreamingExecutor};
use std::process::Command;

/// System reboot detection backend
pub struct SystemRebootBackend {
    config: RebootDetectionConfig,
    ssh_user: Option<String>,
}

impl SystemRebootBackend {
    /// Create a new backend with the given configuration
    pub fn new(config: RebootDetectionConfig) -> Self {
        Self {
            config,
            ssh_user: None,
        }
    }

    /// Create a backend with default configuration
    pub fn with_defaults() -> Self {
        Self::new(RebootDetectionConfig::default())
    }

    /// Set the SSH user for connections
    pub fn with_ssh_user(mut self, user: impl Into<String>) -> Self {
        self.ssh_user = Some(user.into());
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

    /// Check if a deployment requires a reboot
    ///
    /// This performs a two-phase check:
    /// 1. Compare kernel/initrd/init symlinks (fast)
    /// 2. Run dry-activate and check critical services (detailed)
    pub fn check_reboot_required(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<RebootCheck> {
        // Phase 1: Check hard requirements (fast)
        if self.config.check_kernel || self.config.check_initrd {
            if let Some(reason) = self.check_hard_requirements(target, new_system)? {
                return Ok(RebootCheck::required(reason));
            }
        }

        // Phase 2: Check critical services (detailed)
        if !self.config.critical_services.is_empty() {
            if let Some(reason) = self.check_critical_services(target, new_system)? {
                return Ok(RebootCheck::required(reason));
            }
        }

        // No reboot required
        Ok(RebootCheck::not_required())
    }

    /// Check if kernel or initrd changed (fast check)
    fn check_hard_requirements(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<Option<RebootReason>> {
        let checks = [
            (self.config.check_kernel, "kernel"),
            (self.config.check_initrd, "initrd"),
        ];

        for (enabled, component) in checks {
            if enabled {
                let booted_path = format!("/run/booted-system/{}", component);
                let new_path = format!("{}/{}", new_system.as_str(), component);

                if !self.compare_symlinks(target, &booted_path, &new_path)? {
                    let old_target = self.read_symlink_target(target, &booted_path)?;
                    let new_target = self.read_symlink_target(target, &new_path)?;

                    // Only report as changed if the canonical paths actually differ
                    if old_target != new_target {
                        let reason = match component {
                            "kernel" => RebootReason::KernelChanged {
                                old: old_target,
                                new: new_target,
                            },
                            "initrd" => RebootReason::InitrdChanged {
                                old: old_target,
                                new: new_target,
                            },
                            _ => unreachable!(),
                        };

                        return Ok(Some(reason));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Read the target of a symlink on the remote host
    ///
    /// Uses `readlink -f` to resolve all symlinks and get the canonical path.
    fn read_symlink_target(&self, target: &DeployTarget, symlink_path: &str) -> Result<String> {
        let cmd = format!("readlink -f {}", shell_escape(symlink_path));
        let result = self.ssh_exec_full(target, &cmd)?;

        if !result.status.success() {
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(format!(
                    "Failed to resolve path {}: {}",
                    symlink_path,
                    String::from_utf8_lossy(&result.stderr)
                )),
            }
            .into());
        }

        Ok(String::from_utf8_lossy(&result.stdout).trim().to_string())
    }

    /// Compare two symlinks on the remote host
    fn compare_symlinks(&self, target: &DeployTarget, path1: &str, path2: &str) -> Result<bool> {
        // Build SSH command: [ "$(readlink path1)" = "$(readlink path2)" ]
        let cmd = format!(
            r#"[ "$(readlink {})" = "$(readlink {})" ]"#,
            shell_escape(path1),
            shell_escape(path2)
        );

        let status = self.ssh_exec(target, &cmd)?;
        Ok(status.success())
    }

    /// Check if any critical services would be restarted
    ///
    /// # Implementation Note
    ///
    /// The `/run/nixos/dry-activation-restart-list` file is only created by NixOS
    /// when there are services that need to be restarted. If no services need
    /// restarting, the file won't exist. This is expected behavior and we must
    /// check for file existence before attempting to read it.
    ///
    /// This edge case was discovered when deploying configurations with no service
    /// changes - the file doesn't exist and attempting to unconditionally read it
    /// causes "No such file or directory" errors.
    fn check_critical_services(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<Option<RebootReason>> {
        // Run dry-activate on remote host
        let dry_activate_cmd = format!(
            "sudo {}/bin/switch-to-configuration dry-activate",
            shell_escape(new_system.as_str())
        );

        self.ssh_exec(target, &dry_activate_cmd)?;

        // CRITICAL: Check if the restart list file exists first
        // NixOS only creates this file if there are services to restart
        let check_file_cmd = "test -f /run/nixos/dry-activation-restart-list";
        let file_exists = self.ssh_exec(target, check_file_cmd)?.success();

        if !file_exists {
            // No restart list means no services would be restarted
            return Ok(None);
        }

        // Read the machine-readable restart list
        let restart_list = self.ssh_read_file(target, "/run/nixos/dry-activation-restart-list")?;

        // Check if any critical service is in the restart list
        for line in restart_list.lines() {
            let service = line.trim();
            if !service.is_empty() && self.config.critical_services.contains(&service.to_string()) {
                return Ok(Some(RebootReason::CriticalService(service.to_string())));
            }
        }

        Ok(None)
    }

    /// Build an SSH command with common configuration
    fn build_ssh_command(&self, target: &DeployTarget, command: &str) -> Command {
        let mut cmd = Command::new("ssh");

        // Disable interactive prompts - will error instead of prompting
        cmd.arg("-o").arg("BatchMode=yes");

        // Set connection timeout
        cmd.arg("-o").arg(format!(
            "ConnectTimeout={}",
            self.config.ssh_connect_timeout
        ));

        if let Some(port) = target.port {
            cmd.arg("-p").arg(port.to_string());
        }

        cmd.arg(self.get_destination(target)).arg(command);
        cmd
    }

    /// Execute an SSH command and return the status
    fn ssh_exec(&self, target: &DeployTarget, command: &str) -> Result<std::process::ExitStatus> {
        Ok(self.ssh_exec_full(target, command)?.status)
    }

    /// Execute an SSH command and return the full result
    fn ssh_exec_full(&self, target: &DeployTarget, command: &str) -> Result<CommandResult> {
        let mut cmd = self.build_ssh_command(target, command);
        StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Capture)
            .execute()
            .map_err(|e| {
                DeployError::SshFailed {
                    destination: self.get_destination(target),
                    source: e,
                }
                .into()
            })
    }

    /// Read a file from the remote host via SSH
    fn ssh_read_file(&self, target: &DeployTarget, remote_path: &str) -> Result<String> {
        let cmd = format!("cat {}", shell_escape(remote_path));
        let result = self.ssh_exec_full(target, &cmd)?;

        if !result.status.success() {
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(format!(
                    "Failed to read file {}: {}",
                    remote_path,
                    String::from_utf8_lossy(&result.stderr)
                )),
            }
            .into());
        }

        Ok(String::from_utf8_lossy(&result.stdout).into_owned())
    }

    /// Reboot a host and wait for it to come back online
    pub fn reboot_and_wait(&self, target: &DeployTarget) -> Result<()> {
        // Trigger reboot
        let reboot_cmd = "sudo systemctl reboot";

        // We expect this to fail/disconnect as the host reboots
        // Ignore the result
        let _ = self.ssh_exec(target, reboot_cmd);

        // Wait a bit for the reboot to initiate
        std::thread::sleep(std::time::Duration::from_secs(5));

        // Wait for host to come back
        self.wait_for_host(target)?;

        Ok(())
    }

    /// Wait for a host to come back online by repeatedly trying SSH
    fn wait_for_host(&self, target: &DeployTarget) -> Result<()> {
        let max_attempts = self.config.reboot_max_wait / self.config.reboot_retry_interval;
        let retry_interval =
            std::time::Duration::from_secs(self.config.reboot_retry_interval as u64);

        for attempt in 1..=max_attempts {
            // Try a simple SSH command
            let test_cmd = "true";
            let result = self.ssh_exec(target, test_cmd);

            if result.is_ok() && result.unwrap().success() {
                return Ok(());
            }

            if attempt < max_attempts {
                std::thread::sleep(retry_interval);
            }
        }

        Err(DeployError::Failed {
            host: target.hostname.clone(),
            source: std::io::Error::other(format!(
                "Host did not come back online after {} attempts ({} seconds)",
                max_attempts, self.config.reboot_max_wait
            )),
        }
        .into())
    }
}

impl Default for SystemRebootBackend {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl RebootBackend for SystemRebootBackend {
    fn check_reboot_required(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<RebootCheck> {
        self.check_reboot_required(target, new_system)
    }

    fn reboot_and_wait(&self, target: &DeployTarget) -> Result<()> {
        self.reboot_and_wait(target)
    }
}

fn shell_escape(s: &str) -> String {
    // Use single quotes and escape any single quotes in the string
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("simple"), "'simple'");
        assert_eq!(shell_escape("with'quote"), "'with'\\''quote'");
        assert_eq!(shell_escape("with spaces"), "'with spaces'");
        assert_eq!(shell_escape("/nix/store/path"), "'/nix/store/path'");
    }

    #[test]
    fn test_reboot_backend_with_ssh_user() {
        let backend = SystemRebootBackend::with_defaults().with_ssh_user("deploy");
        assert_eq!(backend.ssh_user, Some("deploy".to_string()));
    }

    #[test]
    fn test_get_destination_with_backend_user() {
        let backend = SystemRebootBackend::with_defaults().with_ssh_user("deploy");
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };

        // Backend's user should override target's user
        assert_eq!(
            backend.get_destination(&target),
            "deploy@hydrogen.example.com"
        );
    }

    #[test]
    fn test_get_destination_without_backend_user() {
        let backend = SystemRebootBackend::with_defaults();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };

        // Should fall back to target's ssh_destination()
        assert_eq!(
            backend.get_destination(&target),
            "root@hydrogen.example.com"
        );
    }

    #[test]
    fn test_empty_critical_services_list() {
        // Test that a config with empty critical services list is valid
        // This configuration represents the edge case where:
        // - dry-activate runs but produces no service restarts
        // - /run/nixos/dry-activation-restart-list won't be created
        // - We must check for file existence before reading
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: true,
            check_initrd: true,
            ..Default::default()
        };

        let backend = SystemRebootBackend::new(config.clone());

        // Verify the configuration is as expected
        assert!(backend.config.critical_services.is_empty());
        assert!(backend.config.check_kernel);
        assert!(backend.config.check_initrd);
    }

    #[test]
    fn test_config_with_no_checks_enabled() {
        // Test configuration where all reboot checks are disabled
        // This should never trigger reboot detection, simulating minimal deployments
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            ..Default::default()
        };

        let backend = SystemRebootBackend::new(config);

        assert!(backend.config.critical_services.is_empty());
        assert!(!backend.config.check_kernel);
        assert!(!backend.config.check_initrd);
    }
}
