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
    pub hooks: Vec<HookConfig>,
}
