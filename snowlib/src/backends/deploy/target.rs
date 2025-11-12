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
    /// Get the SSH destination in the format [user@]hostname
    ///
    /// Returns just the hostname if no user is specified.
    pub fn ssh_destination(&self) -> String {
        if let Some(ref user) = self.user {
            format!("{}@{}", user, self.hostname)
        } else {
            self.hostname.clone()
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
            hostname: "hydrogen.example.com".to_string(),
            port: Some(2222),
        };
        assert_eq!(target.ssh_url(), "ssh://deploy@hydrogen.example.com:2222");
    }

    #[test]
    fn test_ssh_destination_with_user() {
        let target = DeployTarget {
            user: Some("deploy".to_string()),
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        assert_eq!(target.ssh_destination(), "deploy@hydrogen.example.com");
    }

    #[test]
    fn test_ssh_destination_without_user() {
        let target = DeployTarget {
            user: None,
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        assert_eq!(target.ssh_destination(), "hydrogen.example.com");
    }

    #[test]
    fn test_ssh_url_without_user() {
        let target = DeployTarget {
            user: None,
            hostname: "hydrogen.example.com".to_string(),
            port: None,
        };
        assert_eq!(target.ssh_url(), "ssh://hydrogen.example.com");
    }

    #[test]
    fn test_ssh_url_without_user_with_port() {
        let target = DeployTarget {
            user: None,
            hostname: "hydrogen.example.com".to_string(),
            port: Some(2222),
        };
        assert_eq!(target.ssh_url(), "ssh://hydrogen.example.com:2222");
    }
}
