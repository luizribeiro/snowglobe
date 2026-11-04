use super::{BootComponent, RebootBackend, RebootCheck, RebootDetectionConfig, RebootReason};
use crate::backends::deploy::DeployTarget;
use crate::error::{DeployError, Result};
use crate::nix::{StorePath, SystemType};
use crate::process::r#async::run_command;
use crate::process::OutputMode;
use async_trait::async_trait;
use std::fs;
use std::path::Path;

/// Local reboot detection backend for rebuilding the current host
///
/// Unlike SystemRebootBackend which operates over SSH,
/// LocalRebootBackend operates on the local machine directly.
pub struct LocalRebootBackend {
    config: RebootDetectionConfig,
}

impl LocalRebootBackend {
    /// Create a new backend with the given configuration
    pub fn new(config: RebootDetectionConfig) -> Self {
        Self { config }
    }

    /// Create a backend with default configuration
    pub fn with_defaults() -> Self {
        Self::new(RebootDetectionConfig::default())
    }

    /// Check if kernel or initrd changed (fast check)
    fn check_hard_requirements(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<Vec<RebootReason>> {
        let mut reasons = Vec::new();
        for component in BootComponent::checked(&self.config) {
            let booted_path = format!("/run/booted-system/{}", component.link());
            let new_path = format!("{}/{}", new_system.as_str(), component.link());
            if self.compare_symlinks(target, &booted_path, &new_path)? {
                continue;
            }
            let old_target = self.read_symlink_target(target, &booted_path)?;
            let new_target = self.read_symlink_target(target, &new_path)?;
            reasons.extend(component.reason(old_target, new_target));
        }

        Ok(reasons)
    }

    /// Read the target of a symlink on the local filesystem
    fn read_symlink_target(&self, target: &DeployTarget, symlink_path: &str) -> Result<String> {
        fs::canonicalize(symlink_path)
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|e| {
                DeployError::Failed {
                    host: target.hostname.clone(),
                    source: std::io::Error::new(
                        e.kind(),
                        format!("Failed to resolve path {}: {}", symlink_path, e),
                    ),
                }
                .into()
            })
    }

    /// Compare two symlinks on the local filesystem
    fn compare_symlinks(&self, target: &DeployTarget, path1: &str, path2: &str) -> Result<bool> {
        let path1_meta = Path::new(path1)
            .symlink_metadata()
            .map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::new(
                    e.kind(),
                    format!("Failed to read metadata for {}: {}", path1, e),
                ),
            })?;

        let path2_meta = Path::new(path2)
            .symlink_metadata()
            .map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::new(
                    e.kind(),
                    format!("Failed to read metadata for {}: {}", path2, e),
                ),
            })?;

        if !path1_meta.is_symlink() || !path2_meta.is_symlink() {
            return Ok(false);
        }

        let link1 = fs::read_link(path1).map_err(|e| DeployError::Failed {
            host: target.hostname.clone(),
            source: std::io::Error::new(
                e.kind(),
                format!("Failed to read symlink {}: {}", path1, e),
            ),
        })?;

        let link2 = fs::read_link(path2).map_err(|e| DeployError::Failed {
            host: target.hostname.clone(),
            source: std::io::Error::new(
                e.kind(),
                format!("Failed to read symlink {}: {}", path2, e),
            ),
        })?;

        Ok(link1 == link2)
    }

    /// Check if any critical services would be restarted
    async fn check_critical_services(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<Vec<RebootReason>> {
        let dry_activate_path = format!("{}/bin/switch-to-configuration", new_system.as_str());

        let result = run_command(
            "sudo",
            &[&dry_activate_path, "dry-activate"],
            OutputMode::Capture,
            None,
            None,
            &[],
        )
        .await
        .map_err(|e| DeployError::Failed {
            host: target.hostname.clone(),
            source: e,
        })?;

        if !result.status.success() {
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(format!(
                    "Running dry-activate failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                )),
            }
            .into());
        }

        let restart_list_path = Path::new("/run/nixos/dry-activation-restart-list");
        if !restart_list_path.exists() {
            return Ok(Vec::new());
        }

        let restart_list =
            fs::read_to_string(restart_list_path).map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::new(
                    e.kind(),
                    format!("Failed to read restart list: {}", e),
                ),
            })?;

        Ok(super::critical_restarts(
            &restart_list,
            &self.config.critical_services,
        ))
    }
}

impl Default for LocalRebootBackend {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[async_trait]
impl RebootBackend for LocalRebootBackend {
    async fn check_reboot_required(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<RebootCheck> {
        let system_type = SystemType::detect_current();

        match system_type {
            SystemType::NixOS => {
                let mut reasons = self.check_hard_requirements(target, new_system)?;
                // dry-activate is slow; skip it when a reboot is needed anyway
                if reasons.is_empty() && !self.config.critical_services.is_empty() {
                    reasons = self.check_critical_services(target, new_system).await?;
                }
                Ok(RebootCheck::from_reasons(reasons))
            }
            SystemType::Darwin => {
                tracing::info!(
                    "Reboot detection not implemented for Darwin - assuming no reboot required"
                );
                Ok(RebootCheck::not_required())
            }
        }
    }

    async fn reboot_and_wait(&self, _target: &DeployTarget) -> Result<()> {
        let system_type = SystemType::detect_current();

        let (program, args): (&str, &[&str]) = match system_type {
            SystemType::NixOS => ("sudo", &["systemctl", "reboot"]),
            SystemType::Darwin => ("sudo", &["shutdown", "-r", "now"]),
        };

        run_command(program, args, OutputMode::Capture, None, None, &[])
            .await
            .map_err(|e| DeployError::Failed {
                host: "localhost".to_string(),
                source: e,
            })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_compare_symlinks_same_target() {
        let backend = LocalRebootBackend::with_defaults();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };

        let result = backend.compare_symlinks(&target, "/proc/self", "/proc/self");
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_compare_symlinks_nonexistent() {
        let backend = LocalRebootBackend::with_defaults();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };

        let result =
            backend.compare_symlinks(&target, "/nonexistent/symlink1", "/nonexistent/symlink2");
        assert!(result.is_err());
    }

    #[test]
    fn test_compare_symlinks_not_a_symlink() {
        use std::io::Write;
        let backend = LocalRebootBackend::with_defaults();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let regular_file = temp_dir.path().join("regular_file");
        std::fs::File::create(&regular_file)
            .unwrap()
            .write_all(b"not a symlink")
            .unwrap();

        let result = backend.compare_symlinks(
            &target,
            regular_file.to_str().unwrap(),
            regular_file.to_str().unwrap(),
        );

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_default_backend() {
        let backend = LocalRebootBackend::default();
        assert!(backend.config.check_kernel);
        assert!(backend.config.check_initrd);
        assert!(!backend.config.critical_services.is_empty());
    }

    #[test]
    fn test_custom_config() {
        let config = RebootDetectionConfig {
            critical_services: vec!["test.service".to_string()],
            check_kernel: false,
            check_initrd: true,
            ..Default::default()
        };

        let backend = LocalRebootBackend::new(config);
        assert!(!backend.config.check_kernel);
        assert!(backend.config.check_initrd);
        assert_eq!(backend.config.critical_services.len(), 1);
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn test_check_reboot_required_darwin_always_returns_not_required() {
        let backend = LocalRebootBackend::with_defaults();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/fake-path".to_string());

        let result = backend.check_reboot_required(&target, &store_path).await;

        assert!(result.is_ok());
        let check = result.unwrap();
        assert!(!check.required);
        assert!(check.reason.is_none());
    }
}
