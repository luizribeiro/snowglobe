//! Options for every ssh connection snow makes to a host

use std::time::Duration;

use crate::error::DeployError;

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

/// Options for every ssh connection snow makes, interactive ones included.
/// Without a `port`, ssh connects to the one the user's ssh config gives the
/// host.
///
/// A host could be compromised; with the deployer's agent forwarded, it
/// could sign in to every other host as the deployer.
pub(crate) fn connection_options(port: Option<u16>) -> Vec<String> {
    let mut args = vec!["-o".to_string(), "ForwardAgent=no".to_string()];
    if let Some(port) = port {
        args.extend(["-p".to_string(), port.to_string()]);
    }
    args
}

/// `connection_options` for a command that must never prompt
pub(crate) fn ssh_options(
    connect_timeout_secs: u64,
    keepalive: Keepalive,
    port: Option<u16>,
) -> Vec<String> {
    let mut options = vec![
        "BatchMode=yes".to_string(),
        format!("ConnectTimeout={connect_timeout_secs}"),
    ];
    if keepalive == Keepalive::Probe {
        options.push(format!("ServerAliveInterval={SERVER_ALIVE_INTERVAL_SECS}"));
        options.push(format!("ServerAliveCountMax={SERVER_ALIVE_COUNT_MAX}"));
    }
    let mut args: Vec<String> = options
        .into_iter()
        .flat_map(|option| ["-o".to_string(), option])
        .collect();
    args.extend(connection_options(port));
    args
}

/// `NIX_SSHOPTS` for `nix copy`, after the user's own. ssh keeps the first
/// value it sees for an option, so the user's win. An interrupted copy is
/// safe to retry, so it uses probe keepalives.
pub(crate) fn nix_sshopts(existing: Option<&str>, port: Option<u16>) -> String {
    let ours = ssh_options(CONNECT_TIMEOUT_SECS, Keepalive::Probe, port).join(" ");
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
            format!(
                "The {program} command did not finish within {}s",
                timeout.as_secs()
            ),
        ));
    }
    Ok(result)
}

// Agents backed by a hardware key or an app may take a few seconds to list
pub(crate) const AGENT_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// ssh-add's exit status when it can't connect to the agent
const AGENT_UNREACHABLE: i32 = 2;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AgentStatus {
    Responsive,
    /// ssh still works with key files, so this only deserves a warning
    Unreachable {
        reason: String,
    },
    /// ssh-add couldn't run or died; nothing is known about the agent
    Unchecked {
        reason: String,
    },
}

/// Lists the agent's keys with `ssh_add`; an agent that hangs is an error,
/// since every ssh call would hang on it too
pub(crate) async fn ssh_agent_status(
    ssh_add: &str,
    args: &[&str],
    timeout: Duration,
) -> std::result::Result<AgentStatus, DeployError> {
    let result = match run_bounded(ssh_add, args, timeout).await {
        Ok(result) => result,
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
            return Err(DeployError::SshAgentNotResponding {
                seconds: timeout.as_secs(),
            })
        }
        Err(e) => {
            return Ok(AgentStatus::Unchecked {
                reason: e.to_string(),
            })
        }
    };
    let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
    Ok(match result.status.code() {
        Some(0 | 1) => AgentStatus::Responsive,
        Some(AGENT_UNREACHABLE) => AgentStatus::Unreachable { reason: stderr },
        _ => AgentStatus::Unchecked {
            reason: format!(
                "The ssh-add command exited with {}: {stderr}",
                result.status
            ),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probes_give_up_on_a_silent_host() {
        let options = ssh_options(5, Keepalive::Probe, None).join(" ");

        assert_eq!(
            options,
            "-o BatchMode=yes -o ConnectTimeout=5 -o ServerAliveInterval=15 -o ServerAliveCountMax=3 -o ForwardAgent=no"
        );
    }

    #[test]
    fn test_connection_options_never_forward_the_agent() {
        assert_eq!(connection_options(None).join(" "), "-o ForwardAgent=no");
        assert_eq!(
            connection_options(Some(2222)).join(" "),
            "-o ForwardAgent=no -p 2222"
        );
    }

    #[test]
    fn test_activation_keeps_waiting_on_a_silent_host() {
        let options = ssh_options(10, Keepalive::Never, None).join(" ");

        assert_eq!(
            options,
            "-o BatchMode=yes -o ConnectTimeout=10 -o ForwardAgent=no"
        );
    }

    #[test]
    fn test_nix_sshopts_keeps_the_users_options() {
        let sshopts = nix_sshopts(Some("-i /home/luiz/.ssh/deploy"), None);

        assert!(sshopts.starts_with("-i /home/luiz/.ssh/deploy -o BatchMode=yes"));
        assert_eq!(sshopts.matches("-o ForwardAgent=no").count(), 1);
        assert!(sshopts.ends_with("-o ServerAliveCountMax=3 -o ForwardAgent=no"));
        assert_eq!(nix_sshopts(Some("  "), None), nix_sshopts(None, None));
    }

    #[test]
    fn test_ssh_options_connect_to_the_hosts_port() {
        let options = ssh_options(10, Keepalive::Never, Some(2222)).join(" ");

        assert_eq!(
            options,
            "-o BatchMode=yes -o ConnectTimeout=10 -o ForwardAgent=no -p 2222"
        );
    }

    #[test]
    fn test_nix_sshopts_copies_to_the_hosts_port() {
        let sshopts = nix_sshopts(Some("-i /home/luiz/.ssh/deploy"), Some(2222));

        assert!(sshopts.starts_with("-i /home/luiz/.ssh/deploy -o BatchMode=yes"));
        assert!(sshopts.ends_with("-o ForwardAgent=no -p 2222"), "{sshopts}");
    }

    #[test]
    fn test_nix_sshopts_puts_the_users_port_first_so_it_wins() {
        let sshopts = nix_sshopts(Some("-p 2200"), Some(2222));

        assert!(sshopts.starts_with("-p 2200 -o BatchMode=yes"), "{sshopts}");
    }

    #[tokio::test]
    async fn test_run_bounded_fails_when_the_command_hangs() {
        let err = run_bounded("sleep", &["30"], Duration::from_secs(1))
            .await
            .unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(
            err.to_string(),
            "The sleep command did not finish within 1s"
        );
    }

    #[tokio::test]
    async fn test_run_bounded_returns_output_and_status() {
        let result = run_bounded("sh", &["-c", "echo boot-2; exit 3"], Duration::from_secs(5))
            .await
            .unwrap();

        assert_eq!(result.stdout, b"boot-2\n");
        assert_eq!(result.status.code(), Some(3));
    }

    async fn status_from_fake_ssh_add(
        script: &str,
    ) -> std::result::Result<AgentStatus, DeployError> {
        ssh_agent_status("sh", &["-c", script], Duration::from_secs(1)).await
    }

    #[tokio::test]
    async fn test_hung_agent_fails_the_check() {
        let result = status_from_fake_ssh_add("sleep 10").await;

        assert!(matches!(
            result,
            Err(DeployError::SshAgentNotResponding { seconds: 1 })
        ));
    }

    #[tokio::test]
    async fn test_agent_with_or_without_keys_is_responsive() {
        let with_keys =
            status_from_fake_ssh_add("echo '256 SHA256:Xq0x luiz@sodium (ED25519)'; exit 0").await;
        let without_keys =
            status_from_fake_ssh_add("echo 'The agent has no identities.'; exit 1").await;

        assert_eq!(with_keys.unwrap(), AgentStatus::Responsive);
        assert_eq!(without_keys.unwrap(), AgentStatus::Responsive);
    }

    #[tokio::test]
    async fn test_agent_that_cannot_be_reached_says_why() {
        let status = status_from_fake_ssh_add(
            "echo 'Error connecting to agent: No such file or directory' >&2; exit 2",
        )
        .await;

        assert_eq!(
            status.unwrap(),
            AgentStatus::Unreachable {
                reason: "Error connecting to agent: No such file or directory".to_string()
            }
        );
    }

    #[tokio::test]
    async fn test_ssh_add_that_cannot_run_or_dies_leaves_the_agent_unchecked() {
        let missing =
            ssh_agent_status("/nonexistent/ssh-add", &["-l"], Duration::from_secs(1)).await;
        let killed = status_from_fake_ssh_add("kill -9 $$").await;

        assert!(
            matches!(missing, Ok(AgentStatus::Unchecked { .. })),
            "{missing:?}"
        );
        assert!(
            matches!(killed, Ok(AgentStatus::Unchecked { .. })),
            "{killed:?}"
        );
    }
}
