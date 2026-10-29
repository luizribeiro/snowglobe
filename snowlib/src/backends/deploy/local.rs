use crate::error::{DeployError, Result};
use crate::nix::{ActivationMode, StorePath, SystemType};
use crate::process::r#async::run_command;
use crate::process::OutputMode;
use async_trait::async_trait;
use std::path::Path;

use super::{
    activation_mode_to_string, should_update_profile, store_path_from_resolved, DeployBackend,
    DeployTarget, HostSystems, CURRENT_SYSTEM_LINK, SYSTEM_PROFILE_LINK,
};

/// Local deployment backend for rebuilding the current host
///
/// Unlike SshDeployBackend which deploys to remote hosts over SSH,
/// LocalDeployBackend operates on the local machine directly.
pub struct LocalDeployBackend {
    /// Output mode for subprocess operations
    pub output_mode: OutputMode,
}

fn store_path_at(link: &Path, host: &str) -> Result<StorePath> {
    let resolved = std::fs::canonicalize(link).map_err(|e| DeployError::Failed {
        host: host.to_string(),
        source: e,
    })?;
    store_path_from_resolved(host, &link.to_string_lossy(), &resolved.to_string_lossy())
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

#[async_trait]
impl DeployBackend for LocalDeployBackend {
    async fn copy_closure(&self, _target: &DeployTarget, _store_path: &StorePath) -> Result<()> {
        Ok(())
    }

    async fn host_systems(&self, target: &DeployTarget) -> Result<HostSystems> {
        Ok(HostSystems {
            running: store_path_at(Path::new(CURRENT_SYSTEM_LINK), &target.hostname)?,
            next_boot: store_path_at(Path::new(SYSTEM_PROFILE_LINK), &target.hostname)?,
        })
    }

    async fn activate(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<()> {
        let system_type = SystemType::detect_current();

        // Validate mode is supported on Darwin before doing any work
        if system_type == SystemType::Darwin && mode != ActivationMode::Switch {
            let msg = match mode {
                ActivationMode::Boot => {
                    "Boot mode not supported on Darwin - use switch mode instead"
                }
                ActivationMode::Test => {
                    "Test mode not supported on Darwin - use switch mode instead"
                }
                ActivationMode::DryActivate => "Dry-activate mode not supported on Darwin",
                ActivationMode::Switch => unreachable!(),
            };
            return Err(DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::other(msg),
            }
            .into());
        }

        // Create system profile generation for rollback support (persistent modes only)
        if should_update_profile(mode) {
            let args = [
                "nix-env",
                "--profile",
                "/nix/var/nix/profiles/system",
                "--set",
                store_path.as_str(),
            ];

            let result = run_command("sudo", &args, self.output_mode, None, None, &[])
                .await
                .map_err(|e| DeployError::Failed {
                    host: target.hostname.clone(),
                    source: std::io::Error::new(
                        e.kind(),
                        format!("Failed to update system profile: {}", e),
                    ),
                })?;

            if !result.status.success() {
                let command_display = format!(
                    "sudo nix-env --profile /nix/var/nix/profiles/system --set {}",
                    store_path.as_str()
                );
                let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&result.stdout).trim().to_string();
                return Err(DeployError::CommandFailed {
                    host: target.hostname.clone(),
                    command: command_display,
                    exit_code: result.status.code().unwrap_or(-1),
                    stderr,
                    stdout,
                }
                .into());
            }
        }

        let (owned_args, command_name): (Vec<String>, &str) = match system_type {
            SystemType::NixOS => {
                let switch_to_configuration =
                    format!("{}/bin/switch-to-configuration", store_path.as_str());
                let mode_arg = activation_mode_to_string(mode);
                (
                    vec![switch_to_configuration, mode_arg.to_string()],
                    "switch-to-configuration",
                )
            }
            SystemType::Darwin => {
                // Only Switch mode reaches here - others rejected at top of function
                let activate_script = format!("{}/activate", store_path.as_str());
                (vec![activate_script], "activate script")
            }
        };

        let args: Vec<&str> = owned_args.iter().map(|s| s.as_str()).collect();
        let result = run_command("sudo", &args, self.output_mode, None, None, &[])
            .await
            .map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::new(
                    e.kind(),
                    format!("Failed to execute {}: {}", command_name, e),
                ),
            })?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let stderr_trimmed = stderr.trim();
            let error_msg = if stderr_trimmed.is_empty() {
                "Activation failed".to_string()
            } else {
                let lines: Vec<&str> = stderr_trimmed.lines().collect();
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
    fn test_store_path_at_missing_link_fails() {
        let dir = tempfile::tempdir().unwrap();

        let err = store_path_at(&dir.path().join("current-system"), "localhost").unwrap_err();

        assert!(matches!(
            err,
            crate::error::SnowError::Deploy(DeployError::Failed { .. })
        ));
    }

    #[test]
    fn test_store_path_at_rejects_link_outside_nix_store() {
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("system");
        std::fs::create_dir(&system).unwrap();
        let link = dir.path().join("current-system");
        std::os::unix::fs::symlink(&system, &link).unwrap();

        let err = store_path_at(&link, "localhost").unwrap_err();

        assert!(format!("{:?}", err).contains("not a Nix store path"));
    }

    #[tokio::test]
    async fn test_copy_closure_is_noop() {
        let backend = LocalDeployBackend::new();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/fake-path".to_string());

        let result = backend.copy_closure(&target, &store_path).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_activate_nonexistent_path() {
        let backend = LocalDeployBackend::new().with_quiet();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/nonexistent-path".to_string());

        let result = backend
            .activate(&target, &store_path, ActivationMode::Test)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = format!("{:?}", err);

        assert!(
            err_string.contains("DeployError") || err_string.contains("Failed"),
            "Error should indicate deployment failure: {}",
            err_string
        );
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn test_activate_darwin_boot_mode_errors() {
        let backend = LocalDeployBackend::new().with_quiet();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test-darwin-system".to_string());

        let result = backend
            .activate(&target, &store_path, ActivationMode::Boot)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();

        let err_display = err.to_string();
        let err_debug = format!("{:?}", err);

        let contains_boot_error = err_display.contains("Boot mode not supported on Darwin")
            || err_debug.contains("Boot mode not supported on Darwin");

        assert!(
            contains_boot_error,
            "Error should mention Boot mode not supported. Display: {}, Debug: {}",
            err_display, err_debug
        );
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn test_activate_darwin_dry_activate_errors() {
        let backend = LocalDeployBackend::new().with_quiet();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test-darwin-system".to_string());

        let result = backend
            .activate(&target, &store_path, ActivationMode::DryActivate)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();

        let err_display = err.to_string();
        let err_debug = format!("{:?}", err);

        let contains_dry_activate_error = err_display
            .contains("Dry-activate mode not supported on Darwin")
            || err_debug.contains("Dry-activate mode not supported on Darwin");

        assert!(
            contains_dry_activate_error,
            "Error should mention Dry-activate mode not supported. Display: {}, Debug: {}",
            err_display, err_debug
        );
    }

    use super::super::common::test_utils::{create_test_store_path, create_test_target};

    fn create_test_backend() -> LocalDeployBackend {
        LocalDeployBackend::new().with_quiet()
    }

    #[tokio::test]
    #[cfg(not(target_os = "macos"))]
    async fn test_activate_nixos_uses_switch_to_configuration() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/nonexistent-nixos-system");

        let result = backend
            .activate(&target, &store_path, ActivationMode::Test)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();

        let err_display = err.to_string();
        let err_debug = format!("{:?}", err);

        let contains_switch_to_config = err_display.contains("switch-to-configuration")
            || err_debug.contains("switch-to-configuration");

        assert!(
            contains_switch_to_config,
            "NixOS should try to use switch-to-configuration. Display: {}, Debug: {}",
            err_display, err_debug
        );
    }

    #[tokio::test]
    #[cfg(not(target_os = "macos"))]
    async fn test_activate_switch_attempts_profile_update_on_nixos() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = backend
            .activate(&target, &store_path, ActivationMode::Switch)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("profile"),
            "Expected error to mention profile update: {}",
            err_string
        );
    }

    #[tokio::test]
    #[cfg(not(target_os = "macos"))]
    async fn test_activate_boot_attempts_profile_update_on_nixos() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = backend
            .activate(&target, &store_path, ActivationMode::Boot)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("profile"),
            "Expected error to mention profile update: {}",
            err_string
        );
    }

    #[tokio::test]
    #[cfg(not(target_os = "macos"))]
    async fn test_activate_test_skips_profile_update_on_nixos() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = backend
            .activate(&target, &store_path, ActivationMode::Test)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("switch-to-configuration"),
            "Expected error to mention switch-to-configuration (not profile update): {}",
            err_string
        );

        assert!(
            !err_string.contains("nix-env") && !err_string.contains("update system profile"),
            "Test mode should skip profile update: {}",
            err_string
        );
    }

    #[tokio::test]
    #[cfg(not(target_os = "macos"))]
    async fn test_activate_dry_activate_skips_profile_update_on_nixos() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = backend
            .activate(&target, &store_path, ActivationMode::DryActivate)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("switch-to-configuration"),
            "Expected error to mention switch-to-configuration: {}",
            err_string
        );

        assert!(
            !err_string.contains("nix-env") && !err_string.contains("update system profile"),
            "DryActivate mode should skip profile update: {}",
            err_string
        );
    }

    #[tokio::test]
    #[cfg(not(target_os = "macos"))]
    async fn test_activate_with_realistic_store_paths() {
        let backend = create_test_backend();
        let target = create_test_target("hydrogen");
        let store_path =
            create_test_store_path("/nix/store/abc123def456-nixos-system-hydrogen-24.05");

        let result = backend
            .activate(&target, &store_path, ActivationMode::Switch)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("profile"),
            "Expected error to mention profile update: {}",
            err_string
        );
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn test_activate_switch_attempts_profile_update_on_darwin() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-darwin-system");

        let result = backend
            .activate(&target, &store_path, ActivationMode::Switch)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("profile"),
            "Darwin should attempt profile update: {}",
            err_string
        );
    }
}
