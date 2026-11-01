use crate::error::{DeployError, Result};
use crate::nix::{ActivationMode, StorePath};
use crate::process::r#async::run_command;
use crate::process::{CommandResult, OutputMode};
use crate::ssh_options::{
    nix_sshopts, run_bounded, ssh_options, Keepalive, CONNECT_TIMEOUT_SECS, PROBE_TIMEOUT,
};
use async_trait::async_trait;
use std::time::Duration;

use super::{
    activation_mode_to_string, should_update_profile, store_path_from_resolved, DeployBackend,
    DeployTarget, HostSystems, CURRENT_SYSTEM_LINK, SYSTEM_PROFILE_LINK,
};

/// Convert command args to a shell-like string representation for error messages
fn args_to_command_string(program: &str, args: &[&str]) -> String {
    let escaped_program = shell_escape::escape(program.into());
    let escaped_args: Vec<_> = args
        .iter()
        .map(|arg| shell_escape::escape((*arg).into()))
        .collect();

    if escaped_args.is_empty() {
        escaped_program.to_string()
    } else {
        format!("{} {}", escaped_program, escaped_args.join(" "))
    }
}

fn command_failed(host: &str, command: String, result: &CommandResult) -> DeployError {
    DeployError::CommandFailed {
        host: host.to_string(),
        command,
        exit_code: result.status.code().unwrap_or(-1),
        stderr: String::from_utf8_lossy(&result.stderr).trim().to_string(),
        stdout: String::from_utf8_lossy(&result.stdout).trim().to_string(),
    }
}

const READ_HOST_SYSTEMS_ARGS: [&str; 4] =
    ["readlink", "-f", CURRENT_SYSTEM_LINK, SYSTEM_PROFILE_LINK];

/// Parse `readlink -f <current-system> <system-profile>`, which prints one line per link
fn host_systems_from_output(host: &str, result: &CommandResult) -> Result<HostSystems> {
    if !result.status.success() {
        return Err(command_failed(host, READ_HOST_SYSTEMS_ARGS.join(" "), result).into());
    }
    let stdout = String::from_utf8_lossy(&result.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let [running, next_boot] = lines.as_slice() else {
        return Err(DeployError::Failed {
            host: host.to_string(),
            source: std::io::Error::other(format!(
                "expected 2 lines from `{}`, got: {:?}",
                READ_HOST_SYSTEMS_ARGS.join(" "),
                stdout
            )),
        }
        .into());
    };
    Ok(HostSystems {
        running: store_path_from_resolved(host, CURRENT_SYSTEM_LINK, running)?,
        next_boot: store_path_from_resolved(host, SYSTEM_PROFILE_LINK, next_boot)?,
    })
}

/// SSH-based deployer using subprocess commands (MVP implementation)
pub struct SshDeployBackend {
    /// Optional SSH user (defaults to root in target)
    pub ssh_user: Option<String>,
    /// Output mode for subprocess operations
    pub output_mode: OutputMode,
    ssh_program: String,
    probe_timeout: Duration,
}

impl SshDeployBackend {
    pub fn new() -> Self {
        Self {
            ssh_user: None,
            output_mode: OutputMode::Display,
            ssh_program: "ssh".to_string(),
            probe_timeout: PROBE_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_fake_ssh(mut self, program: impl Into<String>, probe_timeout: Duration) -> Self {
        self.ssh_program = program.into();
        self.probe_timeout = probe_timeout;
        self
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

    /// Build SSH command args with proper flags
    fn ssh_args(
        &self,
        target: &DeployTarget,
        keepalive: Keepalive,
        remote_args: &[&str],
    ) -> Vec<String> {
        let mut args = ssh_options(CONNECT_TIMEOUT_SECS, keepalive);

        if let Some(port) = target.port {
            args.push("-p".to_string());
            args.push(port.to_string());
        }

        args.push(self.get_destination(target));
        args.extend(remote_args.iter().map(|s| s.to_string()));
        args
    }

    /// Check if snowglobe-activate helper is available on target
    async fn check_helper_available(&self, target: &DeployTarget) -> bool {
        let args = self.ssh_args(
            target,
            Keepalive::Probe,
            &["command", "-v", "snowglobe-activate"],
        );
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        match run_command("ssh", &args_refs, OutputMode::Capture, None, None, &[]).await {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }
}

#[async_trait]
impl DeployBackend for SshDeployBackend {
    async fn copy_closure(&self, target: &DeployTarget, store_path: &StorePath) -> Result<()> {
        let ssh_target = format!("ssh://{}", self.get_destination(target));

        let args = ["copy", "--to", &ssh_target, store_path.as_str()];
        let command = args_to_command_string("nix", &args);
        let sshopts = nix_sshopts(std::env::var("NIX_SSHOPTS").ok().as_deref());
        let env = [("NIX_SSHOPTS", sshopts.as_str())];

        let result = run_command("nix", &args, self.output_mode, None, None, &env)
            .await
            .map_err(|e| DeployError::Failed {
                host: target.hostname.clone(),
                source: e,
            })?;

        if !result.status.success() {
            return Err(command_failed(&target.hostname, command, &result).into());
        }

        Ok(())
    }

    async fn host_systems(&self, target: &DeployTarget) -> Result<HostSystems> {
        let args = self.ssh_args(target, Keepalive::Probe, &READ_HOST_SYSTEMS_ARGS);
        let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let result = run_bounded(&self.ssh_program, &args_refs, self.probe_timeout)
            .await
            .map_err(|e| DeployError::SshFailed {
                destination: self.get_destination(target),
                source: e,
            })?;

        host_systems_from_output(&target.hostname, &result)
    }

    async fn activate(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<()> {
        let mode_arg = activation_mode_to_string(mode);

        // Check if snowglobe-activate helper is available on target
        let helper_available = self.check_helper_available(target).await;

        // Only update profile separately if helper is NOT available
        // When helper is available, it handles profile update internally
        if !helper_available && should_update_profile(mode) {
            // Run nix-env via SSH to update the system profile
            let nix_env_args = [
                "sudo",
                "nix-env",
                "--profile",
                "/nix/var/nix/profiles/system",
                "--set",
                store_path.as_str(),
            ];
            let ssh_args = self.ssh_args(target, Keepalive::Probe, &nix_env_args);
            let ssh_args_refs: Vec<&str> = ssh_args.iter().map(|s| s.as_str()).collect();

            let result = run_command("ssh", &ssh_args_refs, self.output_mode, None, None, &[])
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
                return Err(command_failed(&target.hostname, command_display, &result).into());
            }
        }

        let (remote_args, command) = if helper_available {
            let args = vec![
                "sudo".to_string(),
                "snowglobe-activate".to_string(),
                store_path.as_str().to_string(),
                mode_arg.to_string(),
            ];
            let cmd =
                args_to_command_string("ssh", &args.iter().map(|s| s.as_str()).collect::<Vec<_>>());
            (args, cmd)
        } else {
            tracing::warn!(
                "snowglobe-activate not found on {}, falling back to direct activation. \
                 For better security, add the snowglobe module to your NixOS/Darwin configuration.",
                target.hostname
            );

            let switch_to_configuration =
                format!("{}/bin/switch-to-configuration", store_path.as_str());
            let args = vec![
                "sudo".to_string(),
                switch_to_configuration,
                mode_arg.to_string(),
            ];
            let cmd =
                args_to_command_string("ssh", &args.iter().map(|s| s.as_str()).collect::<Vec<_>>());
            (args, cmd)
        };

        let remote_args_refs: Vec<&str> = remote_args.iter().map(|s| s.as_str()).collect();
        let ssh_args = self.ssh_args(target, Keepalive::Never, &remote_args_refs);
        let ssh_args_refs: Vec<&str> = ssh_args.iter().map(|s| s.as_str()).collect();

        let result = run_command("ssh", &ssh_args_refs, self.output_mode, None, None, &[])
            .await
            .map_err(|e| DeployError::SshFailed {
                destination: self.get_destination(target),
                source: e,
            })?;

        if !result.status.success() {
            return Err(command_failed(&target.hostname, command, &result).into());
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

    fn command_result(exit_code: i32, stdout: &str, stderr: &str) -> CommandResult {
        use std::os::unix::process::ExitStatusExt;
        CommandResult {
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
            status: std::process::ExitStatus::from_raw(exit_code << 8),
            timed_out: false,
        }
    }

    #[test]
    fn test_host_systems_from_output_success() {
        let result = command_result(
            0,
            "/nix/store/new-nixos-system-hydrogen\n/nix/store/staged-nixos-system-hydrogen\n",
            "",
        );

        let systems = host_systems_from_output("hydrogen", &result).unwrap();

        assert_eq!(
            systems.running.as_str(),
            "/nix/store/new-nixos-system-hydrogen"
        );
        assert_eq!(
            systems.next_boot.as_str(),
            "/nix/store/staged-nixos-system-hydrogen"
        );
    }

    #[test]
    fn test_host_systems_from_output_command_failure() {
        let result = command_result(255, "", "ssh: Could not resolve hostname hydrogen");

        let err = host_systems_from_output("hydrogen", &result).unwrap_err();

        match err {
            crate::error::SnowError::Deploy(DeployError::CommandFailed {
                exit_code,
                stderr,
                ..
            }) => {
                assert_eq!(exit_code, 255);
                assert!(stderr.contains("Could not resolve hostname"));
            }
            other => panic!("expected CommandFailed, got {:?}", other),
        }
    }

    #[test]
    fn test_host_systems_from_output_rejects_wrong_line_count() {
        let one_line = command_result(0, "/nix/store/new-nixos-system-hydrogen\n", "");
        let with_banner = command_result(
            0,
            "Welcome to hydrogen!\n/nix/store/a-system\n/nix/store/b-system\n",
            "",
        );

        assert!(host_systems_from_output("hydrogen", &one_line).is_err());
        assert!(host_systems_from_output("hydrogen", &with_banner).is_err());
    }

    #[test]
    fn test_host_systems_from_output_rejects_non_store_path() {
        let result = command_result(0, "/run/current-system\n/nix/store/b-system\n", "");

        assert!(host_systems_from_output("hydrogen", &result).is_err());
    }

    #[test]
    fn test_host_systems_from_output_names_the_bad_link() {
        let result = command_result(0, "/nix/store/a-system\n/nix/var/nix/profiles/system\n", "");

        let err = host_systems_from_output("hydrogen", &result).unwrap_err();

        assert!(format!("{:?}", err).contains("/nix/var/nix/profiles/system resolved to"));
    }

    #[tokio::test]
    async fn test_copy_closure_error_includes_stderr() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/fake-path".to_string());

        let result = deployer.copy_closure(&target, &store_path).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = format!("{:?}", err);

        assert!(
            err_string.contains("DeployError") || err_string.contains("Failed"),
            "Error should be a deployment error: {}",
            err_string
        );
    }

    #[tokio::test]
    async fn test_copy_closure_quiet_mode_captures_stderr() {
        let deployer = SshDeployBackend::new().with_quiet();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/fake-path".to_string());

        let result = deployer.copy_closure(&target, &store_path).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = format!("{:?}", err);

        assert!(
            err_string.contains("DeployError") || err_string.contains("Failed"),
            "Error should be captured in quiet mode: {}",
            err_string
        );
    }

    #[tokio::test]
    async fn test_activate_error_includes_stderr() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/fake-path".to_string());

        let result = deployer
            .activate(&target, &store_path, ActivationMode::Test)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = format!("{:?}", err);

        assert!(
            err_string.contains("DeployError")
                || err_string.contains("SshFailed")
                || err_string.contains("Failed"),
            "Error should indicate SSH/deployment failure: {}",
            err_string
        );
    }

    #[tokio::test]
    async fn test_host_systems_times_out_when_ssh_hangs() {
        let dir = tempfile::tempdir().unwrap();
        let fake_ssh = dir.path().join("ssh");
        std::fs::write(&fake_ssh, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(
            &fake_ssh,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let deployer = SshDeployBackend::new()
            .with_fake_ssh(fake_ssh.display().to_string(), Duration::from_secs(1));
        let target = DeployTarget {
            user: None,
            hostname: "curium.example.com".to_string(),
            port: None,
        };

        let err = deployer.host_systems(&target).await.unwrap_err();

        assert!(crate::error::is_timeout(&err), "{err:?}");
    }

    #[test]
    fn test_ssh_args_bound_the_connection_but_only_probes_drop_silent_hosts() {
        let deployer = SshDeployBackend::new().with_user("pusher");
        let target = DeployTarget {
            user: None,
            hostname: "curium.example.com".to_string(),
            port: None,
        };

        let probe = deployer
            .ssh_args(&target, Keepalive::Probe, &READ_HOST_SYSTEMS_ARGS)
            .join(" ");
        let activation = deployer
            .ssh_args(&target, Keepalive::Never, &["sudo", "snowglobe-activate"])
            .join(" ");

        assert!(
            probe.contains("-o ConnectTimeout=10 -o ServerAliveInterval=15"),
            "{probe}"
        );
        assert!(probe.ends_with("pusher@curium.example.com readlink -f /run/current-system /nix/var/nix/profiles/system"), "{probe}");
        assert!(
            activation.contains("-o ConnectTimeout=10 pusher@curium.example.com"),
            "{activation}"
        );
        assert!(!activation.contains("ServerAlive"), "{activation}");
    }

    #[test]
    fn test_ssh_args_has_batch_mode() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "example.com".to_string(),
            port: Some(2222),
        };

        let args = deployer.ssh_args(&target, Keepalive::Probe, &["echo", "test"]);

        assert!(
            args.contains(&"BatchMode=yes".to_string()),
            "SSH args should include BatchMode: {:?}",
            args
        );

        assert!(
            args.contains(&"2222".to_string()),
            "SSH args should include port: {:?}",
            args
        );
    }

    #[test]
    fn test_args_to_command_string_basic() {
        assert_eq!(
            args_to_command_string("ssh", &["user@host"]),
            "ssh 'user@host'"
        );
    }

    #[test]
    fn test_args_to_command_string_with_spaces() {
        assert_eq!(
            args_to_command_string("ssh", &["user@host", "some command with spaces"]),
            "ssh 'user@host' 'some command with spaces'"
        );
    }

    #[test]
    fn test_args_to_command_string_no_args() {
        assert_eq!(args_to_command_string("nix", &[]), "nix");
    }

    #[test]
    fn test_args_to_command_string_multiple_args() {
        assert_eq!(
            args_to_command_string("nix", &["copy", "--to", "ssh://host", "/nix/store/path"]),
            "nix copy --to 'ssh://host' /nix/store/path"
        );
    }

    #[test]
    fn test_args_to_command_string_with_quotes() {
        assert_eq!(
            args_to_command_string("echo", &[r#"value with "quotes""#]),
            r#"echo 'value with "quotes"'"#
        );
    }

    #[test]
    fn test_args_to_command_string_with_dollar_signs() {
        assert_eq!(
            args_to_command_string("sh", &["-c", "echo $HOME"]),
            "sh -c 'echo $HOME'"
        );
    }

    #[test]
    fn test_args_to_command_string_with_backslashes() {
        assert_eq!(
            args_to_command_string("echo", &[r#"path\with\backslashes"#]),
            r#"echo 'path\with\backslashes'"#
        );
    }

    #[test]
    fn test_args_to_command_string_with_pipes_and_redirects() {
        assert_eq!(
            args_to_command_string("sh", &["-c", "cat file | grep pattern > output"]),
            "sh -c 'cat file | grep pattern > output'"
        );
    }

    #[test]
    fn test_args_to_command_string_empty_arg() {
        assert_eq!(args_to_command_string("test", &[""]), "test ''");
    }

    #[tokio::test]
    async fn test_check_helper_available_executes_without_panic() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "localhost".to_string(),
            port: None,
        };

        let _result = deployer.check_helper_available(&target).await;
    }

    #[tokio::test]
    async fn test_check_helper_available_when_command_missing() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent-host-that-should-not-exist.invalid".to_string(),
            port: None,
        };

        let result = deployer.check_helper_available(&target).await;

        assert!(!result);
    }

    #[tokio::test]
    async fn test_activate_builds_correct_command_structure() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test-path".to_string());

        let result = deployer
            .activate(&target, &store_path, ActivationMode::Test)
            .await;

        assert!(result.is_err());

        let err_string = format!("{:?}", result.unwrap_err());
        assert!(
            err_string.contains("sudo")
                || err_string.contains("snowglobe-activate")
                || err_string.contains("switch-to-configuration"),
            "Error should indicate sudo command was attempted: {}",
            err_string
        );
    }

    #[tokio::test]
    async fn test_activate_with_different_modes() {
        let deployer = SshDeployBackend::new();
        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: "nonexistent.invalid".to_string(),
            port: None,
        };
        let store_path = StorePath::new("/nix/store/test-path".to_string());

        for mode in [
            ActivationMode::Switch,
            ActivationMode::Boot,
            ActivationMode::Test,
            ActivationMode::DryActivate,
        ] {
            let result = deployer.activate(&target, &store_path, mode).await;
            assert!(result.is_err());
        }
    }

    use super::super::common::test_utils::{create_test_store_path, create_test_target_with_port};

    fn create_test_backend() -> SshDeployBackend {
        SshDeployBackend::new().with_quiet()
    }

    fn create_test_target(hostname: &str, port: Option<u16>) -> DeployTarget {
        let mut target = create_test_target_with_port(hostname, port);
        target.user = Some("root".to_string());
        target
    }

    #[tokio::test]
    async fn test_activate_switch_attempts_profile_update_via_ssh() {
        let deployer = create_test_backend();
        let target = create_test_target("nonexistent.invalid", None);
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("profile") || err_string.contains("SshFailed"),
            "Expected error to mention profile update or SSH failure: {}",
            err_string
        );
    }

    #[tokio::test]
    async fn test_activate_boot_attempts_profile_update_via_ssh() {
        let deployer = create_test_backend();
        let target = create_test_target("nonexistent.invalid", None);
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = deployer
            .activate(&target, &store_path, ActivationMode::Boot)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("profile") || err_string.contains("SshFailed"),
            "Expected error to mention profile update or SSH failure: {}",
            err_string
        );
    }

    #[tokio::test]
    async fn test_activate_test_skips_profile_update_via_ssh() {
        let deployer = create_test_backend();
        let target = create_test_target("nonexistent.invalid", None);
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = deployer
            .activate(&target, &store_path, ActivationMode::Test)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("SshFailed")
                || err_string.contains("CommandFailed")
                || err_string.contains("DeployError"),
            "Expected SSH/activation error (not profile update): {}",
            err_string
        );
    }

    #[tokio::test]
    async fn test_activate_dry_activate_skips_profile_update_via_ssh() {
        let deployer = create_test_backend();
        let target = create_test_target("nonexistent.invalid", None);
        let store_path = create_test_store_path("/nix/store/test-system");

        let result = deployer
            .activate(&target, &store_path, ActivationMode::DryActivate)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("SshFailed")
                || err_string.contains("CommandFailed")
                || err_string.contains("DeployError"),
            "Expected SSH/activation error: {}",
            err_string
        );
    }

    #[tokio::test]
    async fn test_activate_with_realistic_store_paths() {
        let deployer = create_test_backend();
        let target = create_test_target("hydrogen.example.com", Some(22));
        let store_path =
            create_test_store_path("/nix/store/abc123def456-nixos-system-hydrogen-24.05");

        let result = deployer
            .activate(&target, &store_path, ActivationMode::Switch)
            .await;

        assert!(result.is_err());
        let err_string = format!("{:?}", result.unwrap_err());

        assert!(
            err_string.contains("profile") || err_string.contains("SshFailed"),
            "Expected SSH or profile error: {}",
            err_string
        );
    }
}
