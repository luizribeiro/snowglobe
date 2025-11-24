use super::{HookBackend, HookResult};
use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::hooks::{HookConfig, HookLocation, HookStage};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Track executed hooks: (hostname, stage, location, command)
type ExecutedHooks = Arc<Mutex<Vec<(String, HookStage, HookLocation, String)>>>;

/// Mock implementation of HookBackend for testing
///
/// Tracks all hook executions and allows configuring success/failure behavior
/// per hostname or command. This enables deterministic testing of hook execution
/// in the deployment pipeline without actually running shell commands.
#[derive(Debug, Clone, Default)]
pub struct MockHookBackend {
    /// Commands that should fail when executed locally
    local_failures: Vec<String>,
    /// Hostnames where remote hooks should fail
    remote_failures: Vec<String>,
    /// Track executed hooks: (hostname, stage, location, command)
    executed_hooks: ExecutedHooks,
}

impl MockHookBackend {
    /// Create a new MockHookBackend with default behavior (all hooks succeed)
    pub fn new() -> Self {
        Self {
            local_failures: Vec::new(),
            remote_failures: Vec::new(),
            executed_hooks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Configure local commands that should fail
    pub fn with_local_failures(mut self, commands: Vec<String>) -> Self {
        self.local_failures = commands;
        self
    }

    /// Configure hostnames where remote hooks should fail
    pub fn with_remote_failures(mut self, hostnames: Vec<String>) -> Self {
        self.remote_failures = hostnames;
        self
    }

    /// Get list of all executed hooks for assertions
    pub fn get_executed_hooks(&self) -> Vec<(String, HookStage, HookLocation, String)> {
        self.executed_hooks.lock().unwrap().clone()
    }

    /// Record a hook execution
    fn record_execution(
        &self,
        hostname: &str,
        stage: HookStage,
        location: HookLocation,
        command: &str,
    ) {
        self.executed_hooks.lock().unwrap().push((
            hostname.to_string(),
            stage,
            location,
            command.to_string(),
        ));
    }

    /// Check if a local command should fail
    fn should_fail_local(&self, command: &str) -> bool {
        self.local_failures.iter().any(|cmd| command.contains(cmd))
    }

    /// Check if a remote hook should fail for this hostname
    fn should_fail_remote(&self, hostname: &str) -> bool {
        self.remote_failures.contains(&hostname.to_string())
    }
}

impl HookBackend for MockHookBackend {
    fn execute_local(
        &self,
        hook: &HookConfig,
        env: &HashMap<String, String>,
    ) -> Result<HookResult> {
        let hostname = env
            .get("SNOW_HOSTNAME")
            .map(|s| s.as_str())
            .unwrap_or("unknown");

        self.record_execution(hostname, hook.stage, HookLocation::Local, &hook.command);

        if self.should_fail_local(&hook.command) {
            Ok(HookResult::failure(
                Some(1),
                String::new(),
                format!("Mock local hook failed: {}", hook.command),
                Duration::from_millis(10),
            ))
        } else {
            Ok(HookResult::success(
                format!("Mock local hook executed: {}", hook.command),
                String::new(),
                Duration::from_millis(10),
            ))
        }
    }

    fn execute_remote(
        &self,
        target: &DeployTarget,
        hook: &HookConfig,
        _env: &HashMap<String, String>,
    ) -> Result<HookResult> {
        self.record_execution(
            &target.hostname,
            hook.stage,
            HookLocation::Remote,
            &hook.command,
        );

        if self.should_fail_remote(&target.hostname) {
            Ok(HookResult::failure(
                Some(1),
                String::new(),
                format!(
                    "Mock remote hook failed on {}: {}",
                    target.hostname, hook.command
                ),
                Duration::from_millis(10),
            ))
        } else {
            Ok(HookResult::success(
                format!(
                    "Mock remote hook executed on {}: {}",
                    target.hostname, hook.command
                ),
                String::new(),
                Duration::from_millis(10),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_hook_backend_local_success() {
        let backend = MockHookBackend::new();
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "kubectl drain node",
        );
        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("kubectl drain node"));

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].0, "test-host");
        assert_eq!(executed[0].1, HookStage::PreActivate);
        assert_eq!(executed[0].2, HookLocation::Local);
        assert_eq!(executed[0].3, "kubectl drain node");
    }

    #[test]
    fn test_mock_hook_backend_local_failure() {
        let backend = MockHookBackend::new().with_local_failures(vec!["kubectl".to_string()]);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "kubectl drain node",
        );
        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
        assert!(result.stderr.contains("Mock local hook failed"));

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
    }

    #[test]
    fn test_mock_hook_backend_remote_success() {
        let backend = MockHookBackend::new();
        let target = DeployTarget {
            hostname: "remote-host".to_string(),
            user: Some("deploy".to_string()),
            port: None,
        };
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PostActivate,
            HookLocation::Remote,
            "systemctl restart nginx",
        );
        let env = HashMap::new();

        let result = backend.execute_remote(&target, &hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("remote-host"));
        assert!(result.stdout.contains("systemctl restart nginx"));

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].0, "remote-host");
        assert_eq!(executed[0].1, HookStage::PostActivate);
        assert_eq!(executed[0].2, HookLocation::Remote);
    }

    #[test]
    fn test_mock_hook_backend_remote_failure() {
        let backend = MockHookBackend::new().with_remote_failures(vec!["fail-host".to_string()]);
        let target = DeployTarget {
            hostname: "fail-host".to_string(),
            user: Some("deploy".to_string()),
            port: None,
        };
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PostActivate,
            HookLocation::Remote,
            "systemctl restart nginx",
        );
        let env = HashMap::new();

        let result = backend.execute_remote(&target, &hook, &env).unwrap();
        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
        assert!(result.stderr.contains("Mock remote hook failed"));

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
    }

    #[test]
    fn test_mock_hook_backend_tracks_multiple_executions() {
        let backend = MockHookBackend::new();
        let target1 = DeployTarget {
            hostname: "host1".to_string(),
            user: None,
            port: None,
        };
        let target2 = DeployTarget {
            hostname: "host2".to_string(),
            user: None,
            port: None,
        };

        let hook1 = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Remote,
            "pre-activate hook",
        );
        let hook2 = HookConfig::test_fixture(
            "test-hook",
            HookStage::PostActivate,
            HookLocation::Remote,
            "post-activate hook",
        );

        backend
            .execute_remote(&target1, &hook1, &HashMap::new())
            .unwrap();
        backend
            .execute_remote(&target2, &hook1, &HashMap::new())
            .unwrap();
        backend
            .execute_remote(&target1, &hook2, &HashMap::new())
            .unwrap();

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 3);
        assert_eq!(executed[0].0, "host1");
        assert_eq!(executed[0].1, HookStage::PreActivate);
        assert_eq!(executed[1].0, "host2");
        assert_eq!(executed[1].1, HookStage::PreActivate);
        assert_eq!(executed[2].0, "host1");
        assert_eq!(executed[2].1, HookStage::PostActivate);
    }
}
