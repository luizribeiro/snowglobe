//! Common deployment primitives shared between local and SSH backends

use crate::error::{DeployError, Result};
use crate::nix::{ActivationMode, SystemType};
use crate::process::{CommandResult, OutputMode};
use crate::sudo::Sudo;

pub fn activation_mode_to_string(mode: ActivationMode) -> &'static str {
    match mode {
        ActivationMode::Switch => "switch",
        ActivationMode::Boot => "boot",
        ActivationMode::Test => "test",
        ActivationMode::DryActivate => "dry-activate",
    }
}

/// Fails, naming every Darwin host, unless all of `hosts` can activate in `mode`
pub fn check_mode_supported<'a>(
    mode: ActivationMode,
    hosts: impl IntoIterator<Item = (&'a str, SystemType)>,
) -> std::result::Result<(), DeployError> {
    if mode == ActivationMode::Switch {
        return Ok(());
    }
    let darwin_hosts: Vec<String> = hosts
        .into_iter()
        .filter(|(_, system_type)| *system_type == SystemType::Darwin)
        .map(|(name, _)| name.to_string())
        .collect();
    if darwin_hosts.is_empty() {
        return Ok(());
    }
    Err(DeployError::DarwinOnlySwitches {
        mode: activation_mode_to_string(mode),
        hosts: darwin_hosts,
    })
}

/// Profile generations are required for rollbacks and appear in nixos-rebuild list-generations.
/// Ephemeral modes (Test/DryActivate) intentionally skip this to avoid polluting the generation list.
pub fn should_update_profile(mode: ActivationMode) -> bool {
    !matches!(mode, ActivationMode::Test | ActivationMode::DryActivate)
}

/// Run over ssh to tell whether the host has snowglobe-activate, the only
/// command the snowglobe module lets deploy users sudo without a password
pub(crate) const HELPER_CHECK: [&str; 3] = ["command", "-v", "snowglobe-activate"];

/// `command -v`'s exit status when the command isn't installed. Anything
/// else non-zero (ssh failing, a non-POSIX login shell) is not an answer.
const COMMAND_NOT_FOUND: i32 = 1;

/// Whether `HELPER_CHECK`, run over ssh to `destination`, found the helper
///
/// A failed check is an error rather than "not installed": guessing wrong
/// would silently take the path without the helper.
pub(crate) fn helper_found(destination: &str, result: &CommandResult) -> Result<bool> {
    match result.status.code() {
        Some(0) => Ok(true),
        Some(COMMAND_NOT_FOUND) => Ok(false),
        _ => Err(DeployError::SshFailed {
            destination: destination.to_string(),
            source: std::io::Error::other(format!(
                "The ssh command exited with {}: {}",
                result.status,
                String::from_utf8_lossy(&result.stderr).trim()
            )),
        }
        .into()),
    }
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

/// What `program` printed, which must have exited 0
pub(crate) fn stdout_of(
    host: &str,
    program: &str,
    args: &[String],
    result: &CommandResult,
) -> Result<String> {
    if !result.status.success() {
        let command = std::iter::once(program.to_string()).chain(args.iter().cloned());
        return Err(command_failed(host, command.collect::<Vec<_>>().join(" "), result).into());
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

/// Fails unless `command` exits 0; `doing` says what it is for, as in
/// "update system profile"
pub(crate) async fn run_as_root(
    sudo: &Sudo,
    host: &str,
    command: &[&str],
    output_mode: OutputMode,
    doing: &str,
) -> crate::error::Result<CommandResult> {
    let result = sudo
        .run(command, output_mode)
        .await?
        .map_err(|e| DeployError::Failed {
            host: host.to_string(),
            source: std::io::Error::new(e.kind(), format!("Failed to {doing}: {e}")),
        })?;
    if !result.status.success() {
        let command = format!("sudo {}", command.join(" "));
        return Err(command_failed(host, command, &result).into());
    }
    Ok(result)
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
    fn test_check_mode_supported_names_every_darwin_host() {
        let hosts = [
            ("ferrum", SystemType::Darwin),
            ("boron", SystemType::NixOS),
            ("cobalt", SystemType::Darwin),
        ];

        let error = check_mode_supported(ActivationMode::Boot, hosts).unwrap_err();

        assert_eq!(
            error.to_string(),
            "nix-darwin can only switch, so ferrum, cobalt can't boot"
        );
    }

    #[test]
    fn test_check_mode_supported_rejects_every_mode_but_switch_on_darwin() {
        for mode in [
            ActivationMode::Boot,
            ActivationMode::Test,
            ActivationMode::DryActivate,
        ] {
            let result = check_mode_supported(mode, [("ferrum", SystemType::Darwin)]);

            assert!(
                matches!(result, Err(DeployError::DarwinOnlySwitches { .. })),
                "{mode:?}"
            );
        }
        assert!(
            check_mode_supported(ActivationMode::Switch, [("ferrum", SystemType::Darwin)]).is_ok()
        );
    }

    #[test]
    fn test_check_mode_supported_allows_any_mode_on_nixos() {
        for mode in [
            ActivationMode::Switch,
            ActivationMode::Boot,
            ActivationMode::Test,
            ActivationMode::DryActivate,
        ] {
            assert!(check_mode_supported(mode, [("boron", SystemType::NixOS)]).is_ok());
        }
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
