//! Common deployment primitives shared between local and SSH backends

use crate::error::{DeployError, Result};
use crate::nix::{ActivationMode, StorePath};
use crate::process::OutputMode;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

pub fn execute_command(
    cmd: &mut Command,
    output_mode: OutputMode,
) -> std::io::Result<std::process::Output> {
    match output_mode {
        OutputMode::Capture => cmd.output(),
        OutputMode::Inherit => {
            cmd.stdout(Stdio::inherit());
            cmd.stderr(Stdio::inherit());
            let status = cmd.spawn()?.wait()?;
            Ok(std::process::Output {
                status,
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
        OutputMode::Display => {
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            let mut child = cmd.spawn()?;

            let mut stdout_data = Vec::new();
            let mut stderr_data = Vec::new();

            // Stream stdout
            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    let line = line?;
                    writeln!(std::io::stdout(), "{}", line)?;
                    stdout_data.extend(line.as_bytes());
                    stdout_data.push(b'\n');
                }
            }

            // Stream stderr
            if let Some(stderr) = child.stderr.take() {
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    let line = line?;
                    writeln!(std::io::stderr(), "{}", line)?;
                    stderr_data.extend(line.as_bytes());
                    stderr_data.push(b'\n');
                }
            }

            let status = child.wait()?;

            Ok(std::process::Output {
                status,
                stdout: stdout_data,
                stderr: stderr_data,
            })
        }
    }
}

pub fn activation_mode_to_string(mode: ActivationMode) -> &'static str {
    match mode {
        ActivationMode::Switch => "switch",
        ActivationMode::Boot => "boot",
        ActivationMode::Test => "test",
        ActivationMode::DryActivate => "dry-activate",
    }
}

/// Profile generations are required for rollbacks and appear in nixos-rebuild list-generations.
/// Ephemeral modes (Test/DryActivate) intentionally skip this to avoid polluting the generation list.
pub fn should_update_profile(mode: ActivationMode) -> bool {
    !matches!(mode, ActivationMode::Test | ActivationMode::DryActivate)
}

/// Update the NixOS system profile to create a new generation
///
/// Required before activation in persistent modes to ensure rollbacks work correctly
/// and the deployment appears in nixos-rebuild list-generations. Ephemeral modes
/// (Test/DryActivate) skip this to avoid polluting the generation history.
///
/// # Arguments
///
/// * `store_path` - The store path of the new system configuration
/// * `hostname` - The hostname for error messages
/// * `run_command` - Dependency injection to allow local (sudo) and remote (SSH) execution without code duplication
pub fn update_system_profile(
    store_path: &StorePath,
    hostname: &str,
    mut run_command: impl FnMut(&[&str]) -> std::io::Result<std::process::Output>,
) -> Result<()> {
    let args = &[
        "nix-env",
        "--profile",
        "/nix/var/nix/profiles/system",
        "--set",
        store_path.as_str(),
    ];

    let output = run_command(args).map_err(|e| DeployError::Failed {
        host: hostname.to_string(),
        source: std::io::Error::new(e.kind(), format!("Failed to update system profile: {}", e)),
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let exit_code = output.status.code().unwrap_or(-1);
        let command_display = format!(
            "sudo nix-env --profile /nix/var/nix/profiles/system --set {}",
            store_path.as_str()
        );

        return Err(DeployError::CommandFailed {
            host: hostname.to_string(),
            command: command_display,
            exit_code,
            stderr,
            stdout,
        }
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn test_activation_mode_to_string() {
        assert_eq!(activation_mode_to_string(ActivationMode::Switch), "switch");
        assert_eq!(activation_mode_to_string(ActivationMode::Boot), "boot");
        assert_eq!(activation_mode_to_string(ActivationMode::Test), "test");
        assert_eq!(
            activation_mode_to_string(ActivationMode::DryActivate),
            "dry-activate"
        );
    }

    #[test]
    fn test_execute_command_succeeds_with_zero_exit_code() {
        let mut cmd = Command::new("echo");
        cmd.arg("test");

        let result = execute_command(&mut cmd, OutputMode::Capture);

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "test");
    }

    #[test]
    fn test_execute_command_captures_output_in_capture_mode() {
        let mut cmd = Command::new("echo");
        cmd.arg("captured output");

        let result = execute_command(&mut cmd, OutputMode::Capture);

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
    }

    #[test]
    fn test_execute_command_returns_output_in_display_mode() {
        let mut cmd = Command::new("echo");
        cmd.arg("displayed output");

        // Display mode should still return output
        let result = execute_command(&mut cmd, OutputMode::Display);

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn test_execute_command_succeeds_on_nonzero_exit_without_error() {
        let mut cmd = Command::new("false");

        let result = execute_command(&mut cmd, OutputMode::Capture);

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(!output.status.success());
    }

    #[test]
    fn test_execute_command_returns_io_error_for_nonexistent_binary() {
        let mut cmd = Command::new("nonexistent-binary-that-should-not-exist");

        let result = execute_command(&mut cmd, OutputMode::Capture);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn test_execute_command_preserves_stdout_and_stderr() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c");
        cmd.arg("echo stdout_text && echo stderr_text >&2");

        let result = execute_command(&mut cmd, OutputMode::Capture);

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("stdout_text"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("stderr_text"));
    }

    #[test]
    fn test_execute_command_preserves_nonzero_exit_code() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c");
        cmd.arg("exit 42");

        let result = execute_command(&mut cmd, OutputMode::Capture);

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(!output.status.success());
        assert_eq!(output.status.code(), Some(42));
    }

    #[test]
    fn test_should_update_profile_for_persistent_modes() {
        assert!(should_update_profile(ActivationMode::Switch));
        assert!(should_update_profile(ActivationMode::Boot));
    }

    #[test]
    fn test_should_not_update_profile_for_ephemeral_modes() {
        assert!(!should_update_profile(ActivationMode::Test));
        assert!(!should_update_profile(ActivationMode::DryActivate));
    }

    #[test]
    fn test_update_system_profile_calls_nix_env_with_correct_args() {
        let store_path = StorePath::new("/nix/store/test-system".to_string());
        let hostname = "testhost";

        let result = update_system_profile(&store_path, hostname, |args| {
            assert_eq!(args[0], "nix-env");
            assert_eq!(args[1], "--profile");
            assert_eq!(args[2], "/nix/var/nix/profiles/system");
            assert_eq!(args[3], "--set");
            assert_eq!(args[4], "/nix/store/test-system");

            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(0),
                stdout: vec![],
                stderr: vec![],
            })
        });

        assert!(result.is_ok());
    }

    #[test]
    fn test_update_system_profile_returns_error_on_nonzero_exit() {
        let store_path = StorePath::new("/nix/store/test-system".to_string());
        let hostname = "testhost";

        let result = update_system_profile(&store_path, hostname, |_| {
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(256),
                stdout: vec![],
                stderr: b"error: permission denied".to_vec(),
            })
        });

        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("testhost"));
        assert!(err_msg.contains("permission denied"));

        if let crate::error::SnowError::Deploy(crate::error::DeployError::CommandFailed {
            stderr,
            command,
            ..
        }) = err
        {
            assert!(stderr.contains("permission denied"));
            assert!(command.contains("nix-env"));
            assert!(command.contains("/nix/store/test-system"));
        } else {
            panic!("Expected DeployError::CommandFailed, got: {:?}", err);
        }
    }

    #[test]
    fn test_update_system_profile_with_realistic_path() {
        let store_path =
            StorePath::new("/nix/store/abc123def456-nixos-system-hydrogen-24.05".to_string());
        let hostname = "hydrogen";

        let result = update_system_profile(&store_path, hostname, |args| {
            assert_eq!(args.len(), 5);
            assert_eq!(args[0], "nix-env");
            assert_eq!(args[1], "--profile");
            assert_eq!(args[2], "/nix/var/nix/profiles/system");
            assert_eq!(args[3], "--set");
            assert_eq!(
                args[4],
                "/nix/store/abc123def456-nixos-system-hydrogen-24.05"
            );

            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(0),
                stdout: vec![],
                stderr: vec![],
            })
        });

        assert!(result.is_ok());
    }

    #[test]
    fn test_update_system_profile_with_special_characters() {
        let store_path =
            StorePath::new("/nix/store/xyz789-nixos-system-test-host-123_special".to_string());
        let hostname = "test-host-123";

        let result = update_system_profile(&store_path, hostname, |args| {
            assert_eq!(args.len(), 5);
            assert_eq!(args[0], "nix-env");
            assert_eq!(
                args[4],
                "/nix/store/xyz789-nixos-system-test-host-123_special"
            );

            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(0),
                stdout: vec![],
                stderr: vec![],
            })
        });

        assert!(result.is_ok());
    }

    #[test]
    fn test_update_system_profile_command_called_once() {
        let store_path =
            StorePath::new("/nix/store/xyz789-nixos-system-testhost-24.05".to_string());
        let mut call_count = 0;

        let result = update_system_profile(&store_path, "host", |_args| {
            call_count += 1;
            assert_eq!(call_count, 1, "Command should only be called once");

            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(0),
                stdout: vec![],
                stderr: vec![],
            })
        });

        assert!(result.is_ok());
        assert_eq!(call_count, 1);
    }

    #[test]
    fn test_update_system_profile_io_error() {
        let store_path = StorePath::new("/nix/store/test-system".to_string());
        let hostname = "testhost";

        let result = update_system_profile(&store_path, hostname, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "SSH connection refused",
            ))
        });

        assert!(result.is_err());
        let err = result.unwrap_err();

        if let crate::error::SnowError::Deploy(crate::error::DeployError::Failed { host, source }) =
            err
        {
            assert_eq!(host, "testhost");
            let source_msg = source.to_string();
            assert!(source_msg.contains("Failed to update system profile"));
            assert!(source_msg.contains("SSH connection refused"));
        } else {
            panic!("Expected DeployError::Failed");
        }
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::super::DeployTarget;
    use crate::nix::StorePath;

    pub(crate) fn create_test_target(hostname: &str) -> DeployTarget {
        DeployTarget {
            user: None,
            hostname: hostname.to_string(),
            port: None,
        }
    }

    pub(crate) fn create_test_target_with_port(hostname: &str, port: Option<u16>) -> DeployTarget {
        DeployTarget {
            user: None,
            hostname: hostname.to_string(),
            port,
        }
    }

    pub(crate) fn create_test_store_path(path: &str) -> StorePath {
        StorePath::new(path.to_string())
    }
}
