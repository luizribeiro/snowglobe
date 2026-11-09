//! SSH deployment target configuration

/// SSH deployment target
///
/// Represents the connection information needed to deploy to a remote host via SSH.
#[derive(Debug, Clone)]
pub struct DeployTarget {
    /// Optional SSH user. If None, SSH will use default behavior (SSH config or current user)
    pub user: Option<String>,
    pub hostname: String,
    pub port: Option<u16>,
}

impl DeployTarget {
    /// Create a new DeployTarget with just a hostname
    pub fn new(hostname: impl Into<String>) -> Self {
        Self {
            user: None,
            hostname: hostname.into(),
            port: None,
        }
    }

    /// Create a new DeployTarget with user and hostname
    pub fn with_user(user: impl Into<String>, hostname: impl Into<String>) -> Self {
        Self {
            user: Some(user.into()),
            hostname: hostname.into(),
            port: None,
        }
    }

    /// Parse a target string in the format [user@]hostname[:port]
    pub fn parse(s: &str) -> Self {
        let (user_host, port) = if let Some((prefix, port_str)) = s.rsplit_once(':') {
            if let Ok(port) = port_str.parse::<u16>() {
                (prefix, Some(port))
            } else {
                (s, None)
            }
        } else {
            (s, None)
        };

        let (user, hostname) = if let Some((user, host)) = user_host.split_once('@') {
            (Some(user.to_string()), host.to_string())
        } else {
            (None, user_host.to_string())
        };

        Self {
            user,
            hostname,
            port,
        }
    }

    /// Get the SSH destination in the format [user@]hostname
    ///
    /// Returns just the hostname if no user is specified.
    pub fn ssh_destination(&self) -> String {
        self.ssh_destination_as(None)
    }

    /// The SSH destination as `user`, or as the target's own user without one
    pub fn ssh_destination_as(&self, user: Option<&str>) -> String {
        match user.or(self.user.as_deref()) {
            Some(user) => format!("{}@{}", user, self.hostname),
            None => self.hostname.clone(),
        }
    }

    /// Get the SSH URL in the format ssh://[user@]hostname[:port]
    ///
    /// Used for Nix copy operations and other SSH-based commands.
    pub fn ssh_url(&self) -> String {
        let user_part = self
            .user
            .as_ref()
            .map(|u| format!("{}@", u))
            .unwrap_or_default();
        if let Some(port) = self.port {
            format!("ssh://{}{}:{}", user_part, self.hostname, port)
        } else {
            format!("ssh://{}{}", user_part, self.hostname)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ssh_url_with_port() {
        let target = DeployTarget {
            user: Some("deploy".to_string()),
            hostname: "hedgehog.example.com".to_string(),
            port: Some(2222),
        };
        assert_eq!(target.ssh_url(), "ssh://deploy@hedgehog.example.com:2222");
    }

    #[test]
    fn test_ssh_destination_with_user() {
        let target = DeployTarget {
            user: Some("deploy".to_string()),
            hostname: "hedgehog.example.com".to_string(),
            port: None,
        };
        assert_eq!(target.ssh_destination(), "deploy@hedgehog.example.com");
    }

    #[test]
    fn test_ssh_destination_without_user() {
        let target = DeployTarget {
            user: None,
            hostname: "hedgehog.example.com".to_string(),
            port: None,
        };
        assert_eq!(target.ssh_destination(), "hedgehog.example.com");
    }

    #[test]
    fn test_ssh_destination_as_a_user_overrides_the_targets_own() {
        let target = DeployTarget::with_user("root", "hedgehog.example.com");
        assert_eq!(
            target.ssh_destination_as(Some("deploy")),
            "deploy@hedgehog.example.com"
        );
    }

    #[test]
    fn test_ssh_destination_as_no_user_keeps_the_targets_own() {
        let target = DeployTarget::with_user("root", "hedgehog.example.com");
        assert_eq!(target.ssh_destination_as(None), "root@hedgehog.example.com");
    }

    #[test]
    fn test_ssh_destination_as_no_user_leaves_it_to_ssh() {
        let target = DeployTarget::new("hedgehog.example.com");
        assert_eq!(target.ssh_destination_as(None), "hedgehog.example.com");
    }

    #[test]
    fn test_ssh_url_without_user() {
        let target = DeployTarget {
            user: None,
            hostname: "hedgehog.example.com".to_string(),
            port: None,
        };
        assert_eq!(target.ssh_url(), "ssh://hedgehog.example.com");
    }

    #[test]
    fn test_ssh_url_without_user_with_port() {
        let target = DeployTarget {
            user: None,
            hostname: "hedgehog.example.com".to_string(),
            port: Some(2222),
        };
        assert_eq!(target.ssh_url(), "ssh://hedgehog.example.com:2222");
    }

    #[test]
    fn test_parse_hostname_only() {
        let target = DeployTarget::parse("proxmox01.local");
        assert_eq!(target.user, None);
        assert_eq!(target.hostname, "proxmox01.local");
        assert_eq!(target.port, None);
    }

    #[test]
    fn test_parse_user_and_hostname() {
        let target = DeployTarget::parse("root@proxmox01.local");
        assert_eq!(target.user, Some("root".to_string()));
        assert_eq!(target.hostname, "proxmox01.local");
        assert_eq!(target.port, None);
    }

    #[test]
    fn test_parse_with_port() {
        let target = DeployTarget::parse("deploy@server.example.com:2222");
        assert_eq!(target.user, Some("deploy".to_string()));
        assert_eq!(target.hostname, "server.example.com");
        assert_eq!(target.port, Some(2222));
    }

    #[test]
    fn test_parse_hostname_with_port() {
        let target = DeployTarget::parse("server.example.com:22");
        assert_eq!(target.user, None);
        assert_eq!(target.hostname, "server.example.com");
        assert_eq!(target.port, Some(22));
    }

    #[test]
    fn test_parse_ip_address() {
        let target = DeployTarget::parse("192.168.1.50");
        assert_eq!(target.user, None);
        assert_eq!(target.hostname, "192.168.1.50");
        assert_eq!(target.port, None);
    }

    #[test]
    fn test_new_constructor() {
        let target = DeployTarget::new("myhost");
        assert_eq!(target.user, None);
        assert_eq!(target.hostname, "myhost");
        assert_eq!(target.port, None);
    }

    #[test]
    fn test_with_user_constructor() {
        let target = DeployTarget::with_user("admin", "myhost");
        assert_eq!(target.user, Some("admin".to_string()));
        assert_eq!(target.hostname, "myhost");
        assert_eq!(target.port, None);
    }
}
