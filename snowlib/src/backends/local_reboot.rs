use crate::backends::deploy::DeployTarget;
use crate::backends::reboot::{RebootBackend, RebootCheck, RebootDetectionConfig, RebootReason};
use crate::error::{DeployError, Result};
use crate::nix::StorePath;
use std::fs;
use std::path::Path;
use std::process::Command;

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

    /// Check if a deployment requires a reboot (local version)
    ///
    /// This performs a two-phase check:
    /// 1. Compare kernel/initrd symlinks (fast)
    /// 2. Run dry-activate and check critical services (detailed)
    fn check_reboot_required_impl(
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

    /// Read the target of a symlink on the local filesystem
    ///
    /// Uses canonicalize to resolve all symlinks in the path and get the real file.
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
    fn check_critical_services(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<Option<RebootReason>> {
        let dry_activate_path = format!("{}/bin/switch-to-configuration", new_system.as_str());

        let mut cmd = Command::new("sudo");
        cmd.args([&dry_activate_path, "dry-activate"]);

        let output = cmd.output().map_err(|e| DeployError::Failed {
            host: target.hostname.clone(),
            source: e,
        })?;

        if !output.status.success() {
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(format!(
                    "dry-activate failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )),
            }
            .into());
        }

        let restart_list_path = Path::new("/run/nixos/dry-activation-restart-list");
        if !restart_list_path.exists() {
            return Ok(None);
        }

        let restart_list =
            fs::read_to_string(restart_list_path).map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::new(
                    e.kind(),
                    format!("Failed to read restart list: {}", e),
                ),
            })?;

        for line in restart_list.lines() {
            let service = line.trim();
            if !service.is_empty() && self.config.critical_services.contains(&service.to_string()) {
                return Ok(Some(RebootReason::CriticalService(service.to_string())));
            }
        }

        Ok(None)
    }

    /// Reboot the local host without waiting
    fn reboot_and_wait_impl(&self, _target: &DeployTarget) -> Result<()> {
        let mut cmd = Command::new("sudo");
        cmd.args(["systemctl", "reboot"]);

        cmd.spawn().map_err(|e| DeployError::Failed {
            host: "localhost".to_string(),
            source: e,
        })?;

        Ok(())
    }
}

impl Default for LocalRebootBackend {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl RebootBackend for LocalRebootBackend {
    fn check_reboot_required(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<RebootCheck> {
        self.check_reboot_required_impl(target, new_system)
    }

    fn reboot_and_wait(&self, target: &DeployTarget) -> Result<()> {
        self.reboot_and_wait_impl(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_symlinks_same_target() {
        let backend = LocalRebootBackend::with_defaults();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };

        // Test with the same path (should be equal)
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
}
