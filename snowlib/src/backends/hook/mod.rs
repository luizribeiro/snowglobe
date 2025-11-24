mod system;

pub use system::SystemHookBackend;

use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::nix::HookConfig;
use std::collections::HashMap;
use std::time::Duration;

/// Result of executing a hook
#[derive(Debug, Clone)]
pub struct HookResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
}

impl HookResult {
    pub fn success(stdout: String, stderr: String, duration: Duration) -> Self {
        Self {
            success: true,
            exit_code: Some(0),
            stdout,
            stderr,
            duration,
        }
    }

    pub fn failure(
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        duration: Duration,
    ) -> Self {
        Self {
            success: false,
            exit_code,
            stdout,
            stderr,
            duration,
        }
    }
}

/// Backend for executing lifecycle hooks
pub trait HookBackend: Send + Sync {
    /// Execute a hook locally (on the machine running snow)
    fn execute_local(&self, hook: &HookConfig, env: &HashMap<String, String>)
        -> Result<HookResult>;

    /// Execute a hook remotely (on the target host via SSH)
    fn execute_remote(
        &self,
        target: &DeployTarget,
        hook: &HookConfig,
        env: &HashMap<String, String>,
    ) -> Result<HookResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_result_success() {
        let result =
            HookResult::success("output".to_string(), String::new(), Duration::from_secs(1));
        assert!(result.success);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "output");
    }

    #[test]
    fn test_hook_result_failure() {
        let result = HookResult::failure(
            Some(1),
            String::new(),
            "error".to_string(),
            Duration::from_secs(1),
        );
        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
        assert_eq!(result.stderr, "error");
    }
}
