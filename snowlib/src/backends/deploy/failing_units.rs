//! Units failing on a NixOS host. switch-to-configuration exits 4 when
//! any unit is failing after a switch, whether or not the switch caused it:
//! it starts every active target again, which reruns units that were
//! already failing.

use crate::error::{DeployError, SnowError};
use std::collections::HashMap;

/// `systemctl` arguments that list what switch-to-configuration counts as
/// failed: units in the failed state, and services waiting to restart
/// after a failure
pub(crate) const FAILING_UNITS_ARGS: [&str; 4] = [
    "show",
    "--state=failed,auto-restart",
    "--property=Id,ActiveState,SubState,ExecMainStatus",
    "*",
];

pub(crate) fn parse_failing_units(output: &str) -> Vec<String> {
    output
        .split("\n\n")
        .filter_map(|block| {
            let properties: HashMap<&str, &str> = block
                .lines()
                .filter_map(|line| line.split_once('='))
                .collect();
            let unit = *properties.get("Id")?;
            let failed = properties.get("ActiveState") == Some(&"failed");
            let restarting_after_failure = unit.ends_with(".service")
                && properties.get("SubState") == Some(&"auto-restart")
                && properties.get("ExecMainStatus").is_some_and(|s| *s != "0");
            (failed || restarting_after_failure).then(|| unit.to_string())
        })
        .collect()
}

/// The units a switch-to-configuration that exited 4 said were failing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitsFailed {
    pub units: Vec<String>,
    /// Whether failing units are the only reason it gave for exiting 4
    pub only_units: bool,
}

const FAILED_UNITS_WARNING: &str = "warning: the following units failed: ";

/// How switch-to-configuration starts the lines of its other reasons to
/// exit 4: a job it couldn't submit or that failed, as in "Failed to
/// restart nginx.service", its activation script failing, or a user whose
/// units it couldn't switch. A unit whose reload failed stays active, so it
/// isn't among the failed units.
const OTHER_REASONS_TO_EXIT_4: [&str; 8] = [
    "Failed to start ",
    "Failed to stop ",
    "Failed to restart ",
    "Failed to reload ",
    "Failed to run activate script",
    "Unable to list users with logind: ",
    "warning: failed to stat ",
    "warning: user activation for ",
];

/// What `error` says failed, if it's switch-to-configuration exiting 4
/// with units failing. Its own reports all go to stderr; stdout holds the
/// activation script's output and `systemctl status` of the failed units,
/// whose journal lines read like failures of their own.
pub fn units_failed(error: &SnowError) -> Option<UnitsFailed> {
    let SnowError::Deploy(DeployError::CommandFailed {
        exit_code: 4,
        stderr,
        ..
    }) = error
    else {
        return None;
    };
    let mut units: Vec<String> = Vec::new();
    let mut only_units = true;
    for line in stderr.lines().map(str::trim) {
        if let Some(failed) = line.strip_prefix(FAILED_UNITS_WARNING) {
            units.extend(failed.split(", ").map(|unit| unit.trim().to_string()));
        } else if OTHER_REASONS_TO_EXIT_4
            .iter()
            .any(|start| line.starts_with(start))
        {
            only_units = false;
        }
    }
    (!units.is_empty()).then_some(UnitsFailed { units, only_units })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BORON_SHOW: &str = "\
Id=stale-job.service
ActiveState=failed
SubState=failed
ExecMainStatus=1

Id=unifi.service
ActiveState=activating
SubState=auto-restart
ExecMainStatus=1

Id=backup.service
ActiveState=activating
SubState=auto-restart
ExecMainStatus=0

Id=mnt-data.mount
ActiveState=failed
SubState=failed
ExecMainStatus=0
";

    #[test]
    fn test_parse_failing_units_keeps_failed_and_crash_looping_units() {
        assert_eq!(
            parse_failing_units(BORON_SHOW),
            ["stale-job.service", "unifi.service", "mnt-data.mount"]
        );
    }

    #[test]
    fn test_parse_failing_units_counts_restarts_only_of_services_that_failed() {
        let show = "\
Id=logrotate.timer
ActiveState=activating
SubState=auto-restart
ExecMainStatus=1

Id=nginx.service
ActiveState=activating
SubState=auto-restart

ActiveState=failed
";

        assert!(parse_failing_units(show).is_empty());
    }

    #[test]
    fn test_parse_failing_units_of_a_healthy_host_is_empty() {
        assert!(parse_failing_units("").is_empty());
    }

    /// stderr of boron's switch, as a VM test recorded it
    const BORON_SWITCH: &str = "\
Checking switch inhibitors... done
activating the configuration...
reloading user units for root...
restarting the following user units: nixos-activation.service
restarting sysinit-reactivation.target
the following new units were started: sysinit-reactivation.target, systemd-tmpfiles-resetup.service
warning: the following units failed: stale-job.service, unifi.service";

    fn switch_failed(exit_code: i32, stderr: &str) -> SnowError {
        switch_failed_printing(exit_code, stderr, "setting up /etc...")
    }

    fn switch_failed_printing(exit_code: i32, stderr: &str, stdout: &str) -> SnowError {
        DeployError::CommandFailed {
            host: "boron".to_string(),
            command: "ssh sudo snowglobe-activate /nix/store/7y4n-nixos-system-boron switch"
                .to_string(),
            exit_code,
            stderr: stderr.to_string(),
            stdout: stdout.to_string(),
        }
        .into()
    }

    fn only_units(units: &[&str]) -> Option<UnitsFailed> {
        Some(UnitsFailed {
            units: units.iter().map(|unit| unit.to_string()).collect(),
            only_units: true,
        })
    }

    #[test]
    fn test_units_failed_lists_the_units_a_switch_reported() {
        assert_eq!(
            units_failed(&switch_failed(4, BORON_SWITCH)),
            only_units(&["stale-job.service", "unifi.service"])
        );
    }

    #[test]
    fn test_units_failed_gathers_every_failed_units_line() {
        let stderr = format!("warning: the following units failed: nginx.service\n{BORON_SWITCH}");

        assert_eq!(
            units_failed(&switch_failed(4, &stderr)),
            only_units(&["nginx.service", "stale-job.service", "unifi.service"])
        );
    }

    #[test]
    fn test_units_failed_lets_other_output_through() {
        let stderr = format!(
            "Warning: Permanently added 'boron.example.com' (ED25519) to the list of known hosts.\n\
             Failed to add the host to the list of known hosts (/home/luiz/.ssh/known_hosts).\n\
             sops-install-secrets: Imported /etc/ssh/ssh_host_ed25519_key as age key with fingerprint age1ql3z7hjy54pw3hyww5ayyfg7zqgvc7w3j2elw8zmrj2kg5sfn9aqmcac8p\n\
             Job for nginx.service failed because the control process exited with error code.\n\
             {BORON_SWITCH}"
        );

        assert_eq!(
            units_failed(&switch_failed(4, &stderr)),
            only_units(&["stale-job.service", "unifi.service"])
        );
    }

    #[test]
    fn test_units_failed_notes_the_other_reasons_to_exit_4() {
        for reason in [
            "Failed to restart unifi.service",
            "Failed to stop nginx.service",
            "Failed to reload dbus.service: Unit dbus.service not found.",
            "Failed to start nginx.service: Transaction is destructive.",
            "Failed to restart sysinit-reactivation.target: Connection timed out",
            "Failed to run activate script",
            "Failed to restart user unit pipewire.service: Unit pipewire.service not found.",
            "Unable to list users with logind: org.freedesktop.DBus.Error.ServiceUnknown",
            "warning: failed to stat /run/user/1000 for user luiz; skipping: Permission denied",
            "warning: user activation for luiz failed",
        ] {
            let stderr = format!("{reason}\n{BORON_SWITCH}");

            assert_eq!(
                units_failed(&switch_failed(4, &stderr)),
                Some(UnitsFailed {
                    units: vec!["stale-job.service".to_string(), "unifi.service".to_string()],
                    only_units: false,
                }),
                "{reason}"
            );
        }
    }

    #[test]
    fn test_units_failed_ignores_failures_in_the_status_on_stdout() {
        let status = "\
setting up /etc...
× stale-job.service
     Active: failed (Result: exit-code) since Sun 2026-11-08 22:43:02 UTC; 377ms ago
Nov 08 22:43:02 boron systemd[1]: stale-job.service: Failed with result 'exit-code'.
Nov 08 22:43:02 boron systemd[1]: Failed to start stale-job.service.";
        let failed = switch_failed_printing(
            4,
            "warning: the following units failed: stale-job.service",
            status,
        );

        assert_eq!(units_failed(&failed), only_units(&["stale-job.service"]));
    }

    #[test]
    fn test_units_failed_is_none_without_failed_units() {
        let user_activation = "warning: user activation for luiz failed";
        let unreachable = DeployError::Failed {
            host: "boron".to_string(),
            source: std::io::Error::other("Connection refused"),
        };

        assert_eq!(units_failed(&switch_failed(4, user_activation)), None);
        assert_eq!(units_failed(&switch_failed(1, BORON_SWITCH)), None);
        assert_eq!(units_failed(&unreachable.into()), None);
    }
}
