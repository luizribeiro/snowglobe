use crate::backends::deploy::DeployTarget;
use crate::hooks::HookConfig;
use crate::nix::SystemType;
use crate::reboot::RebootPolicy;
use serde::Deserialize;

/// A host from the flake (either NixOS or Darwin)
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
    pub reboot_policy: RebootPolicy,
    pub system_type: SystemType,
    /// Nix platform, e.g. "aarch64-linux"; None when the config sets none
    pub system: Option<String>,
    pub hooks: Vec<HookConfig>,
    /// Hosts sharing a group run their disruptive steps one at a time within a phase
    pub serial_group: Option<String>,
    /// None leaves the port to the deployer's ssh config
    pub ssh_port: Option<u16>,
    /// When it fails, its phase and the push still succeed
    pub optional: bool,
}

impl HostMetadata {
    pub(crate) fn deploy_target(&self, user: Option<String>) -> DeployTarget {
        DeployTarget {
            user,
            hostname: self.fqdn.clone(),
            port: self.ssh_port,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(ssh_port: Option<u16>) -> HostMetadata {
        HostMetadata {
            name: "boron".to_string(),
            phase: None,
            tags: vec![],
            fqdn: "boron.example.com".to_string(),
            reboot_policy: RebootPolicy::Auto,
            system_type: SystemType::NixOS,
            system: None,
            hooks: vec![],
            serial_group: None,
            ssh_port,
            optional: false,
        }
    }

    #[test]
    fn test_deploy_target_connects_to_the_hosts_ssh_port() {
        let target = host(Some(2222)).deploy_target(Some("deploy".to_string()));

        assert_eq!(target.ssh_destination(), "deploy@boron.example.com");
        assert_eq!(target.port, Some(2222));
    }

    #[test]
    fn test_deploy_target_without_ssh_port_leaves_it_to_ssh() {
        let target = host(None).deploy_target(None);

        assert_eq!(target.ssh_destination(), "boron.example.com");
        assert_eq!(target.port, None);
    }
}
