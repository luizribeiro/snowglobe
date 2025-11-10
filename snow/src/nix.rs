use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};
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

/// Find the flake root by searching up from the current directory
/// Returns the path to the directory containing flake.nix
pub fn find_flake_root() -> Result<PathBuf> {
    let current_dir = env::current_dir()
        .context("Failed to get current directory")?;

    find_flake_root_from(&current_dir)
}

/// Find the flake root by searching up from a given directory
fn find_flake_root_from(start: &Path) -> Result<PathBuf> {
    let mut current = start;

    loop {
        let flake_path = current.join("flake.nix");
        if flake_path.exists() {
            return Ok(current.to_path_buf());
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => {
                anyhow::bail!(
                    "Could not find flake.nix in {} or any parent directory.\n\
                     snowglobe requires a flake.nix with nixosConfigurations to operate.\n\
                     Please run this command from within a NixOS flake directory.",
                    start.display()
                )
            }
        }
    }
}

/// Ensure we're in a valid flake directory and change to the flake root
pub fn ensure_flake_root() -> Result<PathBuf> {
    let flake_root = find_flake_root()?;
    env::set_current_dir(&flake_root)
        .with_context(|| format!("Failed to change to flake root: {}", flake_root.display()))?;
    Ok(flake_root)
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

/// Build a host configuration and return the store path
pub fn build_host(host: &str) -> Result<StorePath> {
    let target = format!(".#nixosConfigurations.{}.config.system.build.toplevel", host);

    let output = Command::new("nix")
        .args(&["build", &target, "--no-link", "--print-out-paths"])
        .output()
        .context("Failed to execute nix build")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("nix build failed for {}: {}", host, stderr);
    }

    let path = String::from_utf8(output.stdout)
        .context("build output not valid UTF-8")?
        .trim()
        .to_string();

    Ok(StorePath::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    #[test]
    fn test_find_flake_root_in_current_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_path = temp_dir.path().join("flake.nix");
        fs::write(&flake_path, "# test flake").unwrap();

        let result = find_flake_root_from(temp_dir.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_find_flake_root_in_parent_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_path = temp_dir.path().join("flake.nix");
        fs::write(&flake_path, "# test flake").unwrap();

        let subdir = temp_dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();

        let result = find_flake_root_from(&subdir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_find_flake_root_not_found() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = find_flake_root_from(temp_dir.path());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Could not find flake.nix"));
    }

    #[test]
    fn test_find_flake_root_multiple_levels() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_path = temp_dir.path().join("flake.nix");
        fs::write(&flake_path, "# test flake").unwrap();

        let deep_dir = temp_dir.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep_dir).unwrap();

        let result = find_flake_root_from(&deep_dir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), temp_dir.path());
    }
}
