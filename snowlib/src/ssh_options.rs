//! Options for every ssh connection snow makes to a host

use std::time::Duration;

use crate::process::r#async::run_command;
use crate::process::{CommandResult, OutputMode};

pub(crate) const CONNECT_TIMEOUT_SECS: u64 = 10;

/// How long a quick read-only command over ssh may take, auth included.
/// Keepalives can't catch a hung ssh-agent: it stalls before the session exists.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const SERVER_ALIVE_INTERVAL_SECS: u64 = 15;
const SERVER_ALIVE_COUNT_MAX: u64 = 3;

/// Whether ssh gives up on a host that stops answering mid-command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keepalive {
    /// Drop the connection after ~45s of silence from the host. Without it, a
    /// host that drops off the network leaves ssh waiting until TCP gives up,
    /// which can take two hours.
    Probe,
    /// Keep waiting, for commands that may restart the host's network
    /// (activation, hooks): cutting the session could kill them halfway
    Never,
}

pub(crate) fn ssh_options(connect_timeout_secs: u64, keepalive: Keepalive) -> Vec<String> {
    let mut options = vec![
        "BatchMode=yes".to_string(),
        format!("ConnectTimeout={connect_timeout_secs}"),
    ];
    if keepalive == Keepalive::Probe {
        options.push(format!("ServerAliveInterval={SERVER_ALIVE_INTERVAL_SECS}"));
        options.push(format!("ServerAliveCountMax={SERVER_ALIVE_COUNT_MAX}"));
    }
    options
        .into_iter()
        .flat_map(|option| ["-o".to_string(), option])
        .collect()
}

/// `NIX_SSHOPTS` for `nix copy`, after the user's own. An interrupted copy is
/// safe to retry, so it uses probe keepalives.
pub(crate) fn nix_sshopts(existing: Option<&str>) -> String {
    let ours = ssh_options(CONNECT_TIMEOUT_SECS, Keepalive::Probe).join(" ");
    match existing.map(str::trim).filter(|s| !s.is_empty()) {
        Some(existing) => format!("{existing} {ours}"),
        None => ours,
    }
}

/// Runs a captured command, failing with `TimedOut` if it outlives `timeout`
pub(crate) async fn run_bounded(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> std::io::Result<CommandResult> {
    let result = run_command(program, args, OutputMode::Capture, Some(timeout), None, &[]).await?;
    if result.timed_out {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("{program} did not finish within {}s", timeout.as_secs()),
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probes_give_up_on_a_silent_host() {
        let options = ssh_options(5, Keepalive::Probe).join(" ");

        assert_eq!(
            options,
            "-o BatchMode=yes -o ConnectTimeout=5 -o ServerAliveInterval=15 -o ServerAliveCountMax=3"
        );
    }

    #[test]
    fn test_activation_keeps_waiting_on_a_silent_host() {
        let options = ssh_options(10, Keepalive::Never).join(" ");

        assert_eq!(options, "-o BatchMode=yes -o ConnectTimeout=10");
    }

    #[test]
    fn test_nix_sshopts_keeps_the_users_options() {
        let sshopts = nix_sshopts(Some("-i /home/luiz/.ssh/deploy -p 2222"));

        assert!(sshopts.starts_with("-i /home/luiz/.ssh/deploy -p 2222 -o BatchMode=yes"));
        assert!(sshopts.ends_with("-o ServerAliveCountMax=3"));
        assert_eq!(nix_sshopts(Some("  ")), nix_sshopts(None));
    }

    #[tokio::test]
    async fn test_run_bounded_fails_when_the_command_hangs() {
        let err = run_bounded("sleep", &["30"], Duration::from_secs(1))
            .await
            .unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(err.to_string(), "sleep did not finish within 1s");
    }

    #[tokio::test]
    async fn test_run_bounded_returns_output_and_status() {
        let result = run_bounded("sh", &["-c", "echo boot-2; exit 3"], Duration::from_secs(5))
            .await
            .unwrap();

        assert_eq!(result.stdout, b"boot-2\n");
        assert_eq!(result.status.code(), Some(3));
    }
}
