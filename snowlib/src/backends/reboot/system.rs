use super::{RebootBackend, RebootCheck, RebootDetectionConfig, RebootReason};
use crate::backends::deploy::DeployTarget;
use crate::error::{DeployError, Result};
use crate::nix::StorePath;
use crate::process::r#async::run_command;
use crate::process::{CommandResult, OutputMode};
use async_trait::async_trait;
use std::time::Duration;

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

    /// Check if kernel or initrd changed (fast check)
    async fn check_hard_requirements(
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

                if !self
                    .compare_symlinks(target, &booted_path, &new_path)
                    .await?
                {
                    let old_target = self.read_symlink_target(target, &booted_path).await?;
                    let new_target = self.read_symlink_target(target, &new_path).await?;

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
    async fn read_symlink_target(
        &self,
        target: &DeployTarget,
        symlink_path: &str,
    ) -> Result<String> {
        let cmd = format!("readlink -f {}", shell_escape(symlink_path));
        let result = self.ssh_exec_full(target, &cmd).await?;

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
    async fn compare_symlinks(
        &self,
        target: &DeployTarget,
        path1: &str,
        path2: &str,
    ) -> Result<bool> {
        let cmd = format!(
            r#"[ "$(readlink {})" = "$(readlink {})" ]"#,
            shell_escape(path1),
            shell_escape(path2)
        );

        let status = self.ssh_exec(target, &cmd).await?;
        Ok(status.success())
    }

    /// Check if any critical services would be restarted
    async fn check_critical_services(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<Option<RebootReason>> {
        let dry_activate_cmd = format!(
            "sudo {}/bin/switch-to-configuration dry-activate",
            shell_escape(new_system.as_str())
        );

        self.ssh_exec(target, &dry_activate_cmd).await?;

        let check_file_cmd = "test -f /run/nixos/dry-activation-restart-list";
        let file_exists = self.ssh_exec(target, check_file_cmd).await?.success();

        if !file_exists {
            return Ok(None);
        }

        let restart_list = self
            .ssh_read_file(target, "/run/nixos/dry-activation-restart-list")
            .await?;

        for line in restart_list.lines() {
            let service = line.trim();
            if !service.is_empty() && self.config.critical_services.contains(&service.to_string()) {
                return Ok(Some(RebootReason::CriticalService(service.to_string())));
            }
        }

        Ok(None)
    }

    /// Build SSH command arguments
    fn build_ssh_args(&self, target: &DeployTarget, command: &str) -> Vec<String> {
        let mut args = vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            format!("ConnectTimeout={}", self.config.ssh_connect_timeout),
        ];

        if let Some(port) = target.port {
            args.push("-p".to_string());
            args.push(port.to_string());
        }

        args.push(self.get_destination(target));
        args.push(command.to_string());
        args
    }

    /// Execute an SSH command and return the status
    async fn ssh_exec(
        &self,
        target: &DeployTarget,
        command: &str,
    ) -> Result<std::process::ExitStatus> {
        Ok(self.ssh_exec_full(target, command).await?.status)
    }

    /// Execute an SSH command and return the full result
    async fn ssh_exec_full(&self, target: &DeployTarget, command: &str) -> Result<CommandResult> {
        let args = self.build_ssh_args(target, command);
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        run_command("ssh", &args_refs, OutputMode::Capture, None, None, &[])
            .await
            .map_err(|e| {
                DeployError::SshFailed {
                    destination: self.get_destination(target),
                    source: e,
                }
                .into()
            })
    }

    /// Read a file from the remote host via SSH
    async fn ssh_read_file(&self, target: &DeployTarget, remote_path: &str) -> Result<String> {
        let cmd = format!("cat {}", shell_escape(remote_path));
        let result = self.ssh_exec_full(target, &cmd).await?;

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

    /// Wait for a host to come back online by repeatedly trying SSH
    async fn wait_for_host(&self, target: &DeployTarget) -> Result<()> {
        let max_attempts = self.config.reboot_max_wait / self.config.reboot_retry_interval;
        let retry_interval = Duration::from_secs(self.config.reboot_retry_interval as u64);

        for attempt in 1..=max_attempts {
            let test_cmd = "true";
            let result = self.ssh_exec(target, test_cmd).await;

            if result.is_ok() && result.unwrap().success() {
                return Ok(());
            }

            if attempt < max_attempts {
                tokio::time::sleep(retry_interval).await;
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

#[async_trait]
impl RebootBackend for SystemRebootBackend {
    async fn check_reboot_required(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<RebootCheck> {
        // Phase 1: Check hard requirements (fast)
        if self.config.check_kernel || self.config.check_initrd {
            if let Some(reason) = self.check_hard_requirements(target, new_system).await? {
                return Ok(RebootCheck::required(reason));
            }
        }

        // Phase 2: Check critical services (detailed)
        if !self.config.critical_services.is_empty() {
            if let Some(reason) = self.check_critical_services(target, new_system).await? {
                return Ok(RebootCheck::required(reason));
            }
        }

        Ok(RebootCheck::not_required())
    }

    async fn reboot_and_wait(&self, target: &DeployTarget) -> Result<()> {
        let reboot_cmd = "sudo systemctl reboot";

        // We expect this to fail/disconnect as the host reboots
        let _ = self.ssh_exec(target, reboot_cmd).await;

        // Wait a bit for the reboot to initiate
        tokio::time::sleep(Duration::from_secs(5)).await;

        // Wait for host to come back
        self.wait_for_host(target).await?;

        Ok(())
    }
}

fn shell_escape(s: &str) -> String {
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

        assert_eq!(
            backend.get_destination(&target),
            "root@hydrogen.example.com"
        );
    }

    #[test]
    fn test_empty_critical_services_list() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: true,
            check_initrd: true,
            ..Default::default()
        };

        let backend = SystemRebootBackend::new(config.clone());

        assert!(backend.config.critical_services.is_empty());
        assert!(backend.config.check_kernel);
        assert!(backend.config.check_initrd);
    }

    #[test]
    fn test_config_with_no_checks_enabled() {
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
