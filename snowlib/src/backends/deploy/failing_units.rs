//! Units failing on a NixOS host. switch-to-configuration exits 4 when
//! any unit is failing after a switch, whether or not the switch caused it:
//! it starts every active target again, which reruns units that were
//! already failing.

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
}
