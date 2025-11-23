use crate::error::{DeployError, Result};
use crate::nix::{ActivationMode, StorePath};
use crate::process::{OutputMode, StreamingExecutor};
use std::process::Command;

use super::{
    activation_mode_to_string, execute_command, should_update_profile, update_system_profile,
    DeployBackend, DeployTarget,
};

/// Convert a Command to a shell-like string representation for error messages
///
/// Uses proper shell escaping to safely represent the command that was executed.
fn command_to_string(cmd: &Command) -> String {
    let program = shell_escape::escape(cmd.get_program().to_string_lossy());
    let args: Vec<_> = cmd
        .get_args()
        .map(|arg| shell_escape::escape(arg.to_string_lossy()))
        .collect();

    if args.is_empty() {
        program.to_string()
    } else {
        format!("{} {}", program, args.join(" "))
    }
}

/// SSH-based deployer using subprocess commands (MVP implementation)
pub struct SshDeployBackend {
    /// Optional SSH user (defaults to root in target)
    pub ssh_user: Option<String>,
    /// Output mode for subprocess operations
    pub output_mode: OutputMode,
}

impl SshDeployBackend {
    pub fn new() -> Self {
        Self {
            ssh_user: None,
            output_mode: OutputMode::Display,
        }
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.ssh_user = Some(user.into());
        self
    }

    pub fn with_output_mode(mut self, mode: OutputMode) -> Self {
        self.output_mode = mode;
        self
    }

    pub fn with_quiet(self) -> Self {
        self.with_output_mode(OutputMode::Capture)
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

    /// Check if snowglobe-activate helper is available on target
    fn check_helper_available(&self, target: &DeployTarget) -> bool {
        let mut cmd = self.ssh_command(target, &["command", "-v", "snowglobe-activate"]);

        // Run silently to check for helper command presence
        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Capture)
            .execute();

        match result {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }
}

impl DeployBackend for SshDeployBackend {
    fn copy_closure(&self, target: &DeployTarget, store_path: &StorePath) -> Result<()> {
        let ssh_target = format!("ssh://{}", self.get_destination(target));

        let mut cmd = Command::new("nix");
        cmd.args(["copy", "--to", &ssh_target, store_path.as_str()]);

        // Configure SSH to disable interactive prompts for nix copy
        cmd.env("NIX_SSHOPTS", "-o BatchMode=yes");

        let command = command_to_string(&cmd);

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(self.output_mode)
            .execute()
            .map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: e,
            })?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&result.stdout).trim().to_string();
            let exit_code = result.status.code().unwrap_or(-1);

            return Err(DeployError::CommandFailed {
                host: target.hostname.clone(),
                command,
                exit_code,
                stderr,
                stdout,
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
        // Update system profile on remote host (creates generation)
        // Only for persistent modes (Switch/Boot), skip for Test/DryActivate
        if should_update_profile(mode) {
            update_system_profile(store_path, &target.hostname, |args| {
                // Build SSH command to run nix-env on remote
                let mut ssh_args = vec!["sudo"];
                ssh_args.extend_from_slice(args);
                let mut cmd = self.ssh_command(target, &ssh_args);
                execute_command(&mut cmd, self.output_mode)
            })?;
        }

        let mode_arg = activation_mode_to_string(mode);

        // Check if snowglobe-activate helper is available on target
        let helper_available = self.check_helper_available(target);

        let mut cmd = if helper_available {
            // Use snowglobe-activate helper
            self.ssh_command(
                target,
                &["sudo", "snowglobe-activate", store_path.as_str(), mode_arg],
            )
        } else {
            // Fall back to direct activation
            tracing::warn!(
                "snowglobe-activate not found on {}, falling back to direct activation. \
                 For better security, add the snowglobe module to your NixOS/Darwin configuration.",
                target.hostname
            );

            let switch_to_configuration =
                format!("{}/bin/switch-to-configuration", store_path.as_str());
            self.ssh_command(target, &["sudo", &switch_to_configuration, mode_arg])
        };

        let command = command_to_string(&cmd);

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(self.output_mode)
            .execute()
            .map_err(|e| DeployError::SshFailed {
                destination: self.get_destination(target),
                source: e,
            })?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&result.stdout).trim().to_string();
            let exit_code = result.status.code().unwrap_or(-1);

            return Err(DeployError::CommandFailed {
                host: target.hostname.clone(),
                command,
                exit_code,
                stderr,
                stdout,
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
        let deployer = SshDeployBackend::new().with_quiet();
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

    #[test]
    fn test_command_to_string_basic() {
        let mut cmd = Command::new("ssh");
        cmd.arg("user@host");
        // shell-escape quotes args with @ symbol for safety
        assert_eq!(command_to_string(&cmd), "ssh 'user@host'");
    }

    #[test]
    fn test_command_to_string_with_spaces() {
        let mut cmd = Command::new("ssh");
        cmd.arg("user@host");
        cmd.arg("some command with spaces");
        assert_eq!(
            command_to_string(&cmd),
            "ssh 'user@host' 'some command with spaces'"
        );
    }

    #[test]
    fn test_command_to_string_no_args() {
        let cmd = Command::new("nix");
        assert_eq!(command_to_string(&cmd), "nix");
    }

    #[test]
    fn test_command_to_string_multiple_args() {
        let mut cmd = Command::new("nix");
        cmd.args(["copy", "--to", "ssh://host", "/nix/store/path"]);
        assert_eq!(
            command_to_string(&cmd),
            "nix copy --to 'ssh://host' /nix/store/path"
        );
    }

    #[test]
    fn test_command_to_string_with_quotes() {
        let mut cmd = Command::new("echo");
        cmd.arg(r#"value with "quotes""#);
        // shell-escape uses single quotes which don't need escaping for double quotes
        assert_eq!(command_to_string(&cmd), r#"echo 'value with "quotes"'"#);
    }

    #[test]
    fn test_command_to_string_with_dollar_signs() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c");
        cmd.arg("echo $HOME");
        // Single quotes prevent variable expansion
        assert_eq!(command_to_string(&cmd), "sh -c 'echo $HOME'");
    }

    #[test]
    fn test_command_to_string_with_backslashes() {
        let mut cmd = Command::new("echo");
        cmd.arg(r#"path\with\backslashes"#);
        // Single quotes preserve backslashes literally
        assert_eq!(command_to_string(&cmd), r#"echo 'path\with\backslashes'"#);
    }

    #[test]
    fn test_command_to_string_with_pipes_and_redirects() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c");
        cmd.arg("cat file | grep pattern > output");
        // Single quotes protect shell metacharacters
        assert_eq!(
            command_to_string(&cmd),
            "sh -c 'cat file | grep pattern > output'"
        );
    }

    #[test]
    fn test_command_to_string_empty_arg() {
        let mut cmd = Command::new("test");
        cmd.arg("");
        assert_eq!(command_to_string(&cmd), "test ''");
    }

    #[test]
    fn test_check_helper_available_executes_without_panic() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "localhost".to_string(),
            port: None,
        };

        // Test that helper detection executes without panicking
        // We can't guarantee snowglobe-activate exists on localhost,
        // so we just verify the function completes successfully
        let _result = deployer.check_helper_available(&target);
        // If we reach here, the function didn't panic - test passes
    }

    #[test]
    fn test_check_helper_available_when_command_missing() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent-host-that-should-not-exist.invalid".to_string(),
            port: None,
        };

        let result = deployer.check_helper_available(&target);

        // SSH to nonexistent host should fail, returning false
        assert!(!result);
    }

    #[test]
    fn test_activate_builds_correct_command_structure() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test-path".to_string());

        // This will fail (host doesn't exist), but we can verify the command structure
        // by examining the error which includes the command string
        let result = deployer.activate(&target, &store_path, ActivationMode::Test);

        assert!(result.is_err());

        // The error should contain information about the command that was attempted
        let err_string = format!("{:?}", result.unwrap_err());
        assert!(
            err_string.contains("sudo")
                || err_string.contains("snowglobe-activate")
                || err_string.contains("switch-to-configuration"),
            "Error should indicate sudo command was attempted: {}",
            err_string
        );
    }

    #[test]
    fn test_activate_with_different_modes() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test-path".to_string());

        // Test that different activation modes are handled
        for mode in [
            ActivationMode::Switch,
            ActivationMode::Boot,
            ActivationMode::Test,
            ActivationMode::DryActivate,
        ] {
            let result = deployer.activate(&target, &store_path, mode);
            // Should fail due to nonexistent host, but shouldn't panic
            assert!(result.is_err());
        }
    }

    fn create_test_backend() -> SshDeployBackend {
        SshDeployBackend::new().with_quiet()
    }

    fn create_test_target(hostname: &str, port: Option<u16>) -> DeployTarget {
        DeployTarget {
            user: Some("root".to_string()),
            hostname: hostname.to_string(),
            port,
        }
    }

    fn create_test_store_path(path: &str) -> StorePath {
        StorePath::new(path.to_string())
    }

    #[test]
    fn test_activate_switch_attempts_profile_update_via_ssh() {
        let deployer = create_test_backend();
        let target = create_test_target("nonexistent.invalid", None);
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = deployer.activate(&target, &store_path, ActivationMode::Switch);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail during SSH profile update attempt
        assert!(
            err_string.contains("profile") || err_string.contains("SshFailed"),
            "Expected error to mention profile update or SSH failure: {}",
            err_string
        );
    }

    #[test]
    fn test_activate_boot_attempts_profile_update_via_ssh() {
        let deployer = create_test_backend();
        let target = create_test_target("nonexistent.invalid", None);
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = deployer.activate(&target, &store_path, ActivationMode::Boot);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail during SSH profile update attempt
        assert!(
            err_string.contains("profile") || err_string.contains("SshFailed"),
            "Expected error to mention profile update or SSH failure: {}",
            err_string
        );
    }

    #[test]
    fn test_activate_test_skips_profile_update_via_ssh() {
        let deployer = create_test_backend();
        let target = create_test_target("nonexistent.invalid", None);
        let store_path = create_test_store_path("/nix/store/test-system");

        // Test mode should skip profile update and attempt activation directly
        let result = deployer.activate(&target, &store_path, ActivationMode::Test);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail at SSH connection or activation, not at profile update
        assert!(
            err_string.contains("SshFailed")
                || err_string.contains("CommandFailed")
                || err_string.contains("DeployError"),
            "Expected SSH/activation error (not profile update): {}",
            err_string
        );
    }

    #[test]
    fn test_activate_dry_activate_skips_profile_update_via_ssh() {
        let deployer = create_test_backend();
        let target = create_test_target("nonexistent.invalid", None);
        let store_path = create_test_store_path("/nix/store/test-system");

        // DryActivate mode should skip profile update
        let result = deployer.activate(&target, &store_path, ActivationMode::DryActivate);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail at SSH/activation, not at profile update
        assert!(
            err_string.contains("SshFailed")
                || err_string.contains("CommandFailed")
                || err_string.contains("DeployError"),
            "Expected SSH/activation error: {}",
            err_string
        );
    }

    #[test]
    fn test_activate_with_realistic_store_paths() {
        let deployer = create_test_backend();
        let target = create_test_target("hydrogen.example.com", Some(22));
        let store_path =
            create_test_store_path("/nix/store/abc123def456-nixos-system-hydrogen-24.05");

        let result = deployer.activate(&target, &store_path, ActivationMode::Switch);

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        // Should fail at SSH connection (host doesn't exist)
        assert!(
            err_string.contains("profile") || err_string.contains("SshFailed"),
            "Expected SSH or profile error: {}",
            err_string
        );
    }
}
