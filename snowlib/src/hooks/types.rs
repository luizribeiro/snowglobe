use crate::error::Result;
use serde::{Deserialize, Serialize};

/// Stage in the deployment pipeline where a hook should execute
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookStage {
    /// Before activation of a host that changes (e.g., drain Kubernetes node)
    PreDeploy,
    /// Last, once the host runs the new system, after any reboot
    /// (e.g., uncordon Kubernetes node)
    PostDeploy,
    /// After host reboots and comes back online
    PostReboot,
    /// When deployment or reboot fails
    OnFailure,
}

impl HookStage {
    pub const ALL: [HookStage; 4] = [
        Self::PreDeploy,
        Self::PostDeploy,
        Self::PostReboot,
        Self::OnFailure,
    ];

    /// Attribute name under `snowglobe.hooks` in the NixOS/nix-darwin module
    pub fn nix_name(self) -> &'static str {
        match self {
            Self::PreDeploy => "pre-deploy",
            Self::PostDeploy => "post-deploy",
            Self::PostReboot => "post-reboot",
            Self::OnFailure => "on-failure",
        }
    }
}

impl std::fmt::Display for HookStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.nix_name())
    }
}

impl std::str::FromStr for HookStage {
    type Err = crate::error::SnowError;

    fn from_str(s: &str) -> Result<Self> {
        match Self::ALL.into_iter().find(|stage| stage.nix_name() == s) {
            Some(stage) => Ok(stage),
            None => Err(crate::error::NixError::EvalFailed {
                stderr: format!(
                    "Invalid hook stage '{}'. Valid values are: {}",
                    s,
                    Self::ALL
                        .map(|stage| format!("'{}'", stage.nix_name()))
                        .join(", ")
                ),
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
            HookStage::from_str("pre-deploy").unwrap(),
            HookStage::PreDeploy
        ));
        assert!(matches!(
            HookStage::from_str("post-deploy").unwrap(),
            HookStage::PostDeploy
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
    fn test_stages_display_in_kebab_case() {
        let shown = HookStage::ALL.map(|stage| stage.to_string());

        assert_eq!(
            shown,
            ["pre-deploy", "post-deploy", "post-reboot", "on-failure"]
        );
    }

    #[test]
    fn test_nix_name_matches_serde_name() {
        for stage in HookStage::ALL {
            assert_eq!(
                serde_json::to_value(stage).unwrap(),
                serde_json::Value::String(stage.nix_name().to_string())
            );
        }
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
            HookStage::PreDeploy,
            HookLocation::Local,
            "kubectl drain node1",
        );
        hook.timeout = 600;
        hook.continue_on_failure = true;
        assert_eq!(hook.stage, HookStage::PreDeploy);
        assert_eq!(hook.location, HookLocation::Local);
        assert_eq!(hook.command, "kubectl drain node1");
        assert_eq!(hook.timeout, 600);
        assert!(hook.continue_on_failure);
    }

    #[test]
    fn test_hook_config_with_defaults() {
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PostDeploy,
            HookLocation::Remote,
            "systemctl status",
        );
        assert_eq!(hook.stage, HookStage::PostDeploy);
        assert_eq!(hook.location, HookLocation::Remote);
        assert_eq!(hook.command, "systemctl status");
        assert_eq!(hook.timeout, 300);
        assert!(!hook.continue_on_failure);
    }
}
