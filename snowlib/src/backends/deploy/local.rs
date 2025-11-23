use crate::error::{DeployError, Result};
use crate::nix::{ActivationMode, StorePath, SystemType};
use crate::process::{OutputMode, StreamingExecutor};
use std::process::Command;

use super::{
    activation_mode_to_string, execute_command, should_update_profile, update_system_profile,
    DeployBackend, DeployTarget,
};

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
        let system_type = SystemType::detect_current();

        // Update system profile for NixOS (creates generation)
        if system_type == SystemType::NixOS && should_update_profile(mode) {
            update_system_profile(store_path, &target.hostname, |args| {
                let mut cmd = Command::new("sudo");
                cmd.args(args);
                execute_command(&mut cmd, self.output_mode)
            })?;
        }

        let mut cmd = Command::new("sudo");

        match system_type {
            SystemType::NixOS => {
                let switch_to_configuration =
                    format!("{}/bin/switch-to-configuration", store_path.as_str());
                let mode_arg = activation_mode_to_string(mode);
                cmd.args([&switch_to_configuration, mode_arg]);
            }
            SystemType::Darwin => {
                let activate_script = format!("{}/activate", store_path.as_str());

                match mode {
                    ActivationMode::Switch => {
                        cmd.arg(&activate_script);
                    }
                    ActivationMode::Boot => {
                        return Err(DeployError::Failed {
                            host: target.hostname.clone(),
                            source: std::io::Error::other(
                                "Boot mode not supported on Darwin - use switch mode instead",
                            ),
                        }
                        .into());
                    }
                    ActivationMode::Test => {
                        tracing::warn!(
                            "Test mode not natively supported on Darwin - using switch instead"
                        );
                        cmd.arg(&activate_script);
                    }
                    ActivationMode::DryActivate => {
                        return Err(DeployError::Failed {
                            host: target.hostname.clone(),
                            source: std::io::Error::other(
                                "Dry-activate mode not supported on Darwin",
                            ),
                        }
                        .into());
                    }
                }
            }
        }

        let command_name = match system_type {
            SystemType::NixOS => "switch-to-configuration",
            SystemType::Darwin => "activate script",
        };

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(self.output_mode)
            .execute()
            .map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: std::io::Error::new(
                    e.kind(),
                    format!("Failed to execute {}: {}", command_name, e),
                ),
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

    #[test]
    #[cfg(target_os = "macos")]
    fn test_activate_darwin_boot_mode_errors() {
        let backend = LocalDeployBackend::new().with_quiet();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test-darwin-system".to_string());

        let result = backend.activate(&target, &store_path, ActivationMode::Boot);

        assert!(result.is_err());
        let err = result.unwrap_err();

        // Check both Display and Debug representations for the error message
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

    #[test]
    #[cfg(target_os = "macos")]
    fn test_activate_darwin_dry_activate_errors() {
        let backend = LocalDeployBackend::new().with_quiet();
        let target = DeployTarget {
            user: None,
            hostname: "localhost".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test-darwin-system".to_string());

        let result = backend.activate(&target, &store_path, ActivationMode::DryActivate);

        assert!(result.is_err());
        let err = result.unwrap_err();

        // Check both Display and Debug representations for the error message
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

    fn create_test_backend() -> LocalDeployBackend {
        LocalDeployBackend::new().with_quiet()
    }

    fn create_test_target(hostname: &str) -> DeployTarget {
        DeployTarget {
            user: None,
            hostname: hostname.to_string(),
            port: None,
        }
    }

    fn create_test_store_path(path: &str) -> StorePath {
        StorePath::new(path.to_string())
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_activate_nixos_uses_switch_to_configuration() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/nonexistent-nixos-system");

        // Use Test mode to skip profile update (ephemeral activation)
        let result = backend.activate(&target, &store_path, ActivationMode::Test);

        assert!(result.is_err());
        let err = result.unwrap_err();

        // Check both Display and Debug representations for the error message
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

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_activate_switch_attempts_profile_update_on_nixos() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = backend.activate(&target, &store_path, ActivationMode::Switch);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail during profile update step (before activation)
        assert!(
            err_string.contains("profile"),
            "Expected error to mention profile update: {}",
            err_string
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_activate_boot_attempts_profile_update_on_nixos() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = backend.activate(&target, &store_path, ActivationMode::Boot);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail during profile update step (before activation)
        assert!(
            err_string.contains("profile"),
            "Expected error to mention profile update: {}",
            err_string
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_activate_test_skips_profile_update_on_nixos() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-system");

        // Test mode should skip profile update and go straight to activation
        let result = backend.activate(&target, &store_path, ActivationMode::Test);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail at switch-to-configuration, not at nix-env
        assert!(
            err_string.contains("switch-to-configuration"),
            "Expected error to mention switch-to-configuration (not profile update): {}",
            err_string
        );

        // Should NOT have attempted profile update
        assert!(
            !err_string.contains("nix-env") && !err_string.contains("update system profile"),
            "Test mode should skip profile update: {}",
            err_string
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_activate_dry_activate_skips_profile_update_on_nixos() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-system");

        // DryActivate mode should skip profile update
        let result = backend.activate(&target, &store_path, ActivationMode::DryActivate);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail at switch-to-configuration, not at nix-env
        assert!(
            err_string.contains("switch-to-configuration"),
            "Expected error to mention switch-to-configuration: {}",
            err_string
        );

        // Should NOT have attempted profile update
        assert!(
            !err_string.contains("nix-env") && !err_string.contains("update system profile"),
            "DryActivate mode should skip profile update: {}",
            err_string
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_activate_with_realistic_store_paths() {
        let backend = create_test_backend();
        let target = create_test_target("hydrogen");
        let store_path =
            create_test_store_path("/nix/store/abc123def456-nixos-system-hydrogen-24.05");

        // Test that realistic store paths work correctly
        let result = backend.activate(&target, &store_path, ActivationMode::Switch);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail at profile update (path doesn't actually exist)
        assert!(
            err_string.contains("profile"),
            "Expected error to mention profile update: {}",
            err_string
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_activate_switch_skips_profile_on_darwin() {
        let backend = create_test_backend();
        let target = create_test_target("localhost");
        let store_path = create_test_store_path("/nix/store/test-darwin-system");

        // Darwin doesn't use system profiles, should skip profile update entirely
        let result = backend.activate(&target, &store_path, ActivationMode::Switch);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail at activate script, not at nix-env
        assert!(
            err_string.contains("activate") || err_string.contains("Failed to execute"),
            "Expected error about activation: {}",
            err_string
        );

        // Should NOT have attempted nix-env on Darwin
        assert!(
            !err_string.contains("nix-env") && !err_string.contains("profile"),
            "Darwin should not attempt profile update: {}",
            err_string
        );
    }
}
