use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

/// A NixOS host from the flake
#[derive(Debug, Clone, Deserialize)]
pub struct Host {
    pub name: String,
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
}
