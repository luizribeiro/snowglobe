//! Options for every ssh connection snow makes to a host

pub(crate) const CONNECT_TIMEOUT_SECS: u64 = 10;
const SERVER_ALIVE_INTERVAL_SECS: u64 = 15;
const SERVER_ALIVE_COUNT_MAX: u64 = 3;

/// Whether ssh gives up on a host that stops answering mid-command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keepalive {
    /// Drop the connection after ~45s of silence from the host. Without it, a
    /// host that drops off the network leaves ssh waiting until TCP gives up,
    /// which can take two hours.
    Probe,
    /// Keep waiting: activation restarts the host's network, and cutting the
    /// session could kill switch-to-configuration halfway through
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
}
