//! Domain types for NixOS host management

use crate::error::Result;
use serde::{Deserialize, Serialize};

/// System type for a host configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemType {
    /// NixOS configuration
    NixOS,
    /// nix-darwin (macOS) configuration
    Darwin,
}

impl SystemType {
    /// Detect the current system type
    pub fn detect_current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Darwin
        } else {
            Self::NixOS
        }
    }

    /// Get the flake attribute name for this system type (nixosConfigurations or darwinConfigurations)
    pub fn flake_attr(&self) -> &'static str {
        match self {
            Self::NixOS => "nixosConfigurations",
            Self::Darwin => "darwinConfigurations",
        }
    }

    /// Get the system build path suffix for this system type
    pub fn build_path_suffix(&self) -> &'static str {
        match self {
            Self::NixOS => "config.system.build.toplevel",
            Self::Darwin => "config.system.build.toplevel",
        }
    }
}

/// A host from the flake (either NixOS or Darwin)
#[derive(Debug, Clone, Deserialize)]
pub struct Host {
    pub name: String,
}

/// Reboot policy for a host
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RebootPolicy {
    /// Never automatically reboot (use boot mode without reboot)
    Never,
    /// Always reboot after deployment (force reboot even without detection)
    Always,
    /// Automatically detect if reboot is needed (default)
    #[default]
    Auto,
}

impl std::str::FromStr for RebootPolicy {
    type Err = crate::error::SnowError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "never" => Ok(Self::Never),
            "always" => Ok(Self::Always),
            "auto" => Ok(Self::Auto),
            _ => Err(crate::error::NixError::EvalFailed {
                stderr: format!(
                    "Invalid reboot policy '{}'. Valid values are: 'auto', 'always', 'never'",
                    s
                ),
                source: std::io::Error::other("Invalid reboot policy"),
            }
            .into()),
        }
    }
}

/// Stage in the deployment pipeline where a hook should execute
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookStage {
    /// Before activation (e.g., drain Kubernetes node)
    PreActivate,
    /// After activation completes (e.g., uncordon Kubernetes node)
    PostActivate,
    /// After host reboots and comes back online
    PostReboot,
    /// When deployment or reboot fails
    OnFailure,
}

impl std::str::FromStr for HookStage {
    type Err = crate::error::SnowError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "pre-activate" => Ok(Self::PreActivate),
            "post-activate" => Ok(Self::PostActivate),
            "post-reboot" => Ok(Self::PostReboot),
            "on-failure" => Ok(Self::OnFailure),
            _ => Err(crate::error::NixError::EvalFailed {
                stderr: format!(
                    "Invalid hook stage '{}'. Valid values are: 'pre-activate', 'post-activate', 'post-reboot', 'on-failure'",
                    s
                ),
                source: std::io::Error::other("Invalid hook stage"),
            }
            .into()),
        }
    }
}

/// Location where a hook should execute
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookLocation {
    /// Execute on the machine running snow (e.g., kubectl commands)
    Local,
    /// Execute on the target host via SSH (e.g., host-specific validation)
    Remote,
}

impl std::str::FromStr for HookLocation {
    type Err = crate::error::SnowError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "local" => Ok(Self::Local),
            "remote" => Ok(Self::Remote),
            _ => Err(crate::error::NixError::EvalFailed {
                stderr: format!(
                    "Invalid hook location '{}'. Valid values are: 'local', 'remote'",
                    s
                ),
                source: std::io::Error::other("Invalid hook location"),
            }
            .into()),
        }
    }
}

/// Configuration for a lifecycle hook
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookConfig {
    /// Name of the hook (from attribute name)
    pub name: String,
    /// When the hook should execute
    pub stage: HookStage,
    /// Where the hook should execute (local or remote)
    pub location: HookLocation,
    /// Command to execute (Nix-interpolated string)
    pub command: String,
    /// Environment variables to set
    #[serde(default)]
    pub environment: std::collections::HashMap<String, String>,
    /// Paths to files containing environment variables
    #[serde(default)]
    pub environment_files: Vec<std::path::PathBuf>,
    /// Timeout in seconds
    #[serde(default = "default_hook_timeout")]
    pub timeout: u64,
    /// Whether to continue deployment if the hook fails
    #[serde(default)]
    pub continue_on_failure: bool,
}

fn default_hook_timeout() -> u64 {
    300 // 5 minutes
}

#[cfg(test)]
impl HookConfig {
    /// Create a hook config for testing with default timeout and failure behavior
    pub(crate) fn test_fixture(
        name: &str,
        stage: HookStage,
        location: HookLocation,
        command: &str,
    ) -> Self {
        Self {
            name: name.to_string(),
            stage,
            location,
            command: command.to_string(),
            environment: std::collections::HashMap::new(),
            environment_files: Vec::new(),
            timeout: 300,
            continue_on_failure: false,
        }
    }
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

/// Activation mode for NixOS configuration
///
/// Maps directly to switch-to-configuration subcommands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivationMode {
    /// Activate and make default (switch)
    Switch,
    /// Only set for next boot
    Boot,
    /// Test activation without making permanent
    Test,
    /// Dry run - show what would change
    DryActivate,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    #[cfg(target_os = "macos")]
    fn test_detect_current_on_darwin() {
        assert_eq!(SystemType::detect_current(), SystemType::Darwin);
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn test_detect_current_on_nixos() {
        assert_eq!(SystemType::detect_current(), SystemType::NixOS);
    }

    #[test]
    fn test_flake_attr_nixos() {
        assert_eq!(SystemType::NixOS.flake_attr(), "nixosConfigurations");
    }

    #[test]
    fn test_flake_attr_darwin() {
        assert_eq!(SystemType::Darwin.flake_attr(), "darwinConfigurations");
    }

    #[test]
    fn test_build_path_suffix_nixos() {
        assert_eq!(
            SystemType::NixOS.build_path_suffix(),
            "config.system.build.toplevel"
        );
    }

    #[test]
    fn test_build_path_suffix_darwin() {
        assert_eq!(
            SystemType::Darwin.build_path_suffix(),
            "config.system.build.toplevel"
        );
    }

    #[test]
    fn test_system_type_equality() {
        assert_eq!(SystemType::NixOS, SystemType::NixOS);
        assert_eq!(SystemType::Darwin, SystemType::Darwin);
        assert_ne!(SystemType::NixOS, SystemType::Darwin);
    }

    #[test]
    fn test_reboot_policy_from_str_valid() {
        assert!(matches!(
            RebootPolicy::from_str("auto").unwrap(),
            RebootPolicy::Auto
        ));
        assert!(matches!(
            RebootPolicy::from_str("never").unwrap(),
            RebootPolicy::Never
        ));
        assert!(matches!(
            RebootPolicy::from_str("always").unwrap(),
            RebootPolicy::Always
        ));
    }

    #[test]
    fn test_reboot_policy_from_str_invalid() {
        let result = RebootPolicy::from_str("nver");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Invalid reboot policy 'nver'"),
            "Error should mention invalid policy: {}",
            err_msg
        );
        assert!(
            err_msg.contains("'auto', 'always', 'never'"),
            "Error should list valid values: {}",
            err_msg
        );
    }

    #[test]
    fn test_hook_stage_from_str_valid() {
        assert!(matches!(
            HookStage::from_str("pre-activate").unwrap(),
            HookStage::PreActivate
        ));
        assert!(matches!(
            HookStage::from_str("post-activate").unwrap(),
            HookStage::PostActivate
        ));
        assert!(matches!(
            HookStage::from_str("post-reboot").unwrap(),
            HookStage::PostReboot
        ));
        assert!(matches!(
            HookStage::from_str("on-failure").unwrap(),
            HookStage::OnFailure
        ));
    }

    #[test]
    fn test_hook_stage_from_str_invalid() {
        let result = HookStage::from_str("invalid-stage");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid hook stage 'invalid-stage'"));
    }

    #[test]
    fn test_hook_location_from_str_valid() {
        assert!(matches!(
            HookLocation::from_str("local").unwrap(),
            HookLocation::Local
        ));
        assert!(matches!(
            HookLocation::from_str("remote").unwrap(),
            HookLocation::Remote
        ));
    }

    #[test]
    fn test_hook_location_from_str_invalid() {
        let result = HookLocation::from_str("invalid-location");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid hook location 'invalid-location'"));
    }

    #[test]
    fn test_hook_config_construction() {
        let mut hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "kubectl drain node1",
        );
        hook.timeout = 600;
        hook.continue_on_failure = true;
        assert_eq!(hook.stage, HookStage::PreActivate);
        assert_eq!(hook.location, HookLocation::Local);
        assert_eq!(hook.command, "kubectl drain node1");
        assert_eq!(hook.timeout, 600);
        assert!(hook.continue_on_failure);
    }

    #[test]
    fn test_hook_config_with_defaults() {
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PostActivate,
            HookLocation::Remote,
            "systemctl status",
        );
        assert_eq!(hook.stage, HookStage::PostActivate);
        assert_eq!(hook.location, HookLocation::Remote);
        assert_eq!(hook.command, "systemctl status");
        assert_eq!(hook.timeout, 300);
        assert!(!hook.continue_on_failure);
    }
}
