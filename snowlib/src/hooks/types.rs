use crate::error::Result;
use serde::{Deserialize, Serialize};

/// Stage in the deployment pipeline where a hook should execute
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

impl std::fmt::Display for HookStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreActivate => write!(f, "PreActivate"),
            Self::PostActivate => write!(f, "PostActivate"),
            Self::PostReboot => write!(f, "PostReboot"),
            Self::OnFailure => write!(f, "OnFailure"),
        }
    }
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
    /// Execution priority (lower values execute first, default: 100)
    /// Used to control hook execution order within a stage
    #[serde(default = "default_hook_priority")]
    pub priority: i32,
    /// Number of retry attempts (0 = no retries)
    #[serde(default)]
    pub retries: u32,
    /// Seconds to wait between retries
    #[serde(default = "default_retry_delay")]
    pub retry_delay: u64,
}

fn default_hook_timeout() -> u64 {
    300 // 5 minutes
}

fn default_hook_priority() -> i32 {
    100
}

fn default_retry_delay() -> u64 {
    5
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
            priority: 100,
            retries: 0,
            retry_delay: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

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
