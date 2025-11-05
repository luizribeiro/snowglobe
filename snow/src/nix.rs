use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

/// A NixOS host from the flake
#[derive(Debug, Clone, Deserialize)]
pub struct Host {
    pub name: String,
}

/// Metadata about a host for deployment
#[derive(Debug, Clone)]
pub struct HostMetadata {
    pub name: String,
    pub phase: Option<String>,
    pub tags: Vec<String>,
    pub fqdn: String,
}

/// Nix store path for a built configuration
#[derive(Debug, Clone)]
pub struct StorePath(String);

impl StorePath {
    pub fn new(path: String) -> Self {
        Self(path)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// SSH deployment target
#[derive(Debug, Clone)]
pub struct DeployTarget {
    pub user: String,
    pub hostname: String,
    pub port: Option<u16>,
}

impl DeployTarget {
    pub fn ssh_destination(&self) -> String {
        format!("{}@{}", self.user, self.hostname)
    }

    pub fn ssh_url(&self) -> String {
        if let Some(port) = self.port {
            format!("ssh://{}@{}:{}", self.user, self.hostname, port)
        } else {
            format!("ssh://{}@{}", self.user, self.hostname)
        }
    }
}

/// Parse host names from nix JSON output
fn parse_host_names(json: &[u8]) -> Result<Vec<Host>> {
    let host_names: Vec<String> = serde_json::from_slice(json)
        .context("Failed to parse nix output")?;

    let hosts = host_names
        .into_iter()
        .map(|name| Host { name })
        .collect();

    Ok(hosts)
}

/// List all nixosConfigurations from the current flake
pub fn list_hosts() -> Result<Vec<Host>> {
    let output = Command::new("nix")
        .args(&[
            "eval",
            ".#nixosConfigurations",
            "--apply",
            "builtins.attrNames",
            "--json",
        ])
        .output()
        .context("Failed to execute nix command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("nix eval failed: {}", stderr);
    }

    parse_host_names(&output.stdout)
}

/// Get metadata for a specific host
pub fn get_host_metadata(host: &str) -> Result<HostMetadata> {
    let config_path = format!(".#nixosConfigurations.{}.config", host);

    // Get phase (optional)
    let phase = {
        let output = Command::new("nix")
            .args(&["eval", &format!("{}.core.pushPhase", config_path), "--raw"])
            .output()
            .context("Failed to eval pushPhase")?;

        if output.status.success() {
            let phase_str = String::from_utf8(output.stdout)
                .context("pushPhase output not valid UTF-8")?;
            Some(phase_str)
        } else {
            None
        }
    };

    // Get tags
    let tags = {
        let output = Command::new("nix")
            .args(&["eval", &format!("{}.core.tags", config_path), "--json"])
            .output()
            .context("Failed to eval tags")?;

        if output.status.success() {
            serde_json::from_slice(&output.stdout)
                .context("Failed to parse tags")?
        } else {
            Vec::new()
        }
    };

    // Get hostname
    let hostname = {
        let output = Command::new("nix")
            .args(&["eval", &format!("{}.networking.hostName", config_path), "--raw"])
            .output()
            .context("Failed to eval hostName")?;

        if !output.status.success() {
            anyhow::bail!("Failed to get hostname for {}", host);
        }

        String::from_utf8(output.stdout)
            .context("hostname output not valid UTF-8")?
    };

    // Get domain
    let domain = {
        let output = Command::new("nix")
            .args(&["eval", &format!("{}.networking.domain", config_path), "--raw"])
            .output()
            .context("Failed to eval domain")?;

        if output.status.success() {
            String::from_utf8(output.stdout)
                .context("domain output not valid UTF-8")?
        } else {
            String::new()
        }
    };

    let fqdn = if domain.is_empty() {
        hostname.clone()
    } else {
        format!("{}.{}", hostname, domain)
    };

    Ok(HostMetadata {
        name: host.to_string(),
        phase,
        tags,
        fqdn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_names() {
        let json = br#"["hydrogen","nitrogen","oxygen"]"#;
        let hosts = parse_host_names(json).unwrap();

        assert_eq!(hosts.len(), 3);
        assert_eq!(hosts[0].name, "hydrogen");
        assert_eq!(hosts[1].name, "nitrogen");
        assert_eq!(hosts[2].name, "oxygen");
    }

    #[test]
    fn test_parse_empty_hosts() {
        let json = br#"[]"#;
        let hosts = parse_host_names(json).unwrap();
        assert_eq!(hosts.len(), 0);
    }

    #[test]
    fn test_parse_invalid_json() {
        let json = b"not valid json";
        let result = parse_host_names(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_deploy_target_ssh_url_with_port() {
        let target = DeployTarget {
            user: "deploy".to_string(),
            hostname: "hydrogen.example.com".to_string(),
            port: Some(2222),
        };
        assert_eq!(target.ssh_url(), "ssh://deploy@hydrogen.example.com:2222");
    }
}
