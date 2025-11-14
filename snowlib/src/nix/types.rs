//! Domain types for NixOS host management

use crate::error::Result;
use serde::Deserialize;

/// A NixOS host from the flake
#[derive(Debug, Clone, Deserialize)]
pub struct Host {
    pub name: String,
}

/// Reboot policy for a host
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebootPolicy {
    /// Never automatically reboot (use boot mode without reboot)
    Never,
    /// Always reboot after deployment (force reboot even without detection)
    Always,
    /// Automatically detect if reboot is needed (default)
    Auto,
}

impl Default for RebootPolicy {
    fn default() -> Self {
        Self::Auto
    }
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

/// Metadata about a host for deployment
#[derive(Debug, Clone)]
pub struct HostMetadata {
    pub name: String,
    pub phase: Option<String>,
    pub tags: Vec<String>,
    pub fqdn: String,
    pub reboot_policy: RebootPolicy,
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
}
