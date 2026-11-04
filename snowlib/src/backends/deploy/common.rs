//! Common deployment primitives shared between local and SSH backends

use crate::error::DeployError;
use crate::nix::ActivationMode;
use crate::process::CommandResult;

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

pub(crate) fn command_failed(host: &str, command: String, result: &CommandResult) -> DeployError {
    DeployError::CommandFailed {
        host: host.to_string(),
        command,
        exit_code: result.status.code().unwrap_or(-1),
        stderr: String::from_utf8_lossy(&result.stderr).trim().to_string(),
        stdout: String::from_utf8_lossy(&result.stdout).trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_should_update_profile_for_persistent_modes() {
        assert!(should_update_profile(ActivationMode::Switch));
        assert!(should_update_profile(ActivationMode::Boot));
    }

    #[test]
    fn test_command_failed_keeps_all_of_stderr() {
        let stderr: Vec<String> = (1..=12)
            .map(|n| format!("warning: unit storage-agent-{n}.service failed"))
            .collect();
        let result = test_utils::command_result(4, "setting up /etc...\n", &stderr.join("\n"));

        let error = command_failed(
            "cerium",
            "sudo switch-to-configuration switch".into(),
            &result,
        );

        let DeployError::CommandFailed {
            exit_code,
            stderr: kept,
            stdout,
            ..
        } = error
        else {
            panic!("expected CommandFailed, got {error:?}");
        };
        assert_eq!(exit_code, 4);
        assert_eq!(kept, stderr.join("\n"));
        assert_eq!(stdout, "setting up /etc...");
    }

    #[test]
    fn test_should_not_update_profile_for_ephemeral_modes() {
        assert!(!should_update_profile(ActivationMode::Test));
        assert!(!should_update_profile(ActivationMode::DryActivate));
    }
}

#[cfg(test)]
pub(crate) mod test_utils {
    use super::super::DeployTarget;
    use crate::nix::StorePath;
    use crate::process::CommandResult;

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

    pub(crate) fn command_result(exit_code: i32, stdout: &str, stderr: &str) -> CommandResult {
        use std::os::unix::process::ExitStatusExt;
        CommandResult {
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
            status: std::process::ExitStatus::from_raw(exit_code << 8),
            timed_out: false,
        }
    }
}
