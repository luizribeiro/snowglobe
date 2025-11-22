//! Hook execution orchestration for deployment pipeline

use crate::backends::deploy::DeployTarget;
use crate::backends::hook::{HookBackend, HookResult};
use crate::error::Result;
use crate::nix::{HookConfig, HookLocation, HookStage};
use std::collections::HashMap;
use std::sync::Arc;

/// Orchestrates execution of lifecycle hooks during deployment
pub struct HookExecutor {
    backend: Arc<dyn HookBackend>,
    hooks_by_host: HashMap<String, Vec<HookConfig>>,
}

impl HookExecutor {
    /// Create a new hook executor with the given backend and hooks
    pub fn new(
        backend: Arc<dyn HookBackend>,
        hooks_by_host: HashMap<String, Vec<HookConfig>>,
    ) -> Self {
        Self {
            backend,
            hooks_by_host,
        }
    }

    /// Execute all hooks for a given hostname and stage
    ///
    /// Returns Ok(()) if all hooks succeed or have continue_on_failure=true
    /// Returns Err if any hook fails and has continue_on_failure=false
    pub fn execute_hooks_for_stage(
        &self,
        hostname: &str,
        stage: HookStage,
        target: Option<&DeployTarget>,
        env: &HashMap<String, String>,
    ) -> Result<Vec<HookResult>> {
        let hooks = match self.hooks_by_host.get(hostname) {
            Some(hooks) => hooks,
            None => return Ok(Vec::new()),
        };

        let stage_hooks: Vec<&HookConfig> = hooks.iter().filter(|h| h.stage == stage).collect();

        if stage_hooks.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for hook in stage_hooks {
            tracing::info!(
                target: "hooks",
                "Executing {:?} hook for {}: {}",
                stage,
                hostname,
                hook.command
            );

            let result = match hook.location {
                HookLocation::Local => self.backend.execute_local(hook, env)?,
                HookLocation::Remote => {
                    let target = target.ok_or_else(|| {
                        crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                            host: hostname.to_string(),
                            source: std::io::Error::other(
                                "Remote hook execution requires target, but target is None",
                            ),
                        })
                    })?;
                    self.backend.execute_remote(target, hook, env)?
                }
            };

            if !result.success && !hook.continue_on_failure {
                tracing::error!(
                    target: "hooks",
                    "Hook failed for {} (exit code: {:?}): {}",
                    hostname,
                    result.exit_code,
                    result.stderr
                );

                return Err(crate::error::SnowError::Deploy(
                    crate::error::DeployError::CommandFailed {
                        host: hostname.to_string(),
                        command: hook.command.clone(),
                        exit_code: result.exit_code.unwrap_or(-1),
                        stderr: result.stderr.clone(),
                        stdout: result.stdout.clone(),
                    },
                ));
            } else if !result.success {
                tracing::warn!(
                    target: "hooks",
                    "Hook failed for {} but continuing (continueOnFailure=true): {}",
                    hostname,
                    result.stderr
                );
            } else {
                tracing::info!(
                    target: "hooks",
                    "Hook completed successfully for {} in {:?}",
                    hostname,
                    result.duration
                );
            }

            results.push(result);
        }

        Ok(results)
    }

    /// Check if a host has any hooks configured for a given stage
    pub fn has_hooks_for_stage(&self, hostname: &str, stage: HookStage) -> bool {
        self.hooks_by_host
            .get(hostname)
            .map(|hooks| hooks.iter().any(|h| h.stage == stage))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::hook::SystemHookBackend;
    use crate::backends::mock::MockHookBackend;
    use crate::process::OutputMode;

    #[test]
    fn test_executor_with_no_hooks() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));
        let executor = HookExecutor::new(backend, HashMap::new());

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_executor_with_empty_hooks_array() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));
        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), Vec::new());
        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_executor_with_local_hook() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                HookStage::PreActivate,
                HookLocation::Local,
                "echo 'test'",
            )],
        );

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert!(results[0].stdout.contains("test"));
    }

    #[test]
    fn test_executor_with_multiple_hooks() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "echo 'hook1'",
                ),
                HookConfig::test_fixture(
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "echo 'hook2'",
                ),
                HookConfig::test_fixture(
                    HookStage::PostActivate,
                    HookLocation::Local,
                    "echo 'hook3'",
                ),
            ],
        );

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results[0].stdout.contains("hook1"));
        assert!(results[1].stdout.contains("hook2"));
    }

    #[test]
    fn test_executor_hook_failure_aborts() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                HookStage::PreActivate,
                HookLocation::Local,
                "exit 1",
            )],
        );

        let executor = HookExecutor::new(backend, hooks_by_host);

        let result = executor.execute_hooks_for_stage(
            "test-host",
            HookStage::PreActivate,
            None,
            &HashMap::new(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_executor_hook_failure_with_continue() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        let mut hook =
            HookConfig::test_fixture(HookStage::PreActivate, HookLocation::Local, "exit 1");
        hook.continue_on_failure = true;
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
    }

    #[test]
    fn test_has_hooks_for_stage() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                HookStage::PreActivate,
                HookLocation::Local,
                "echo 'test'",
            )],
        );

        let executor = HookExecutor::new(backend, hooks_by_host);

        assert!(executor.has_hooks_for_stage("test-host", HookStage::PreActivate));
        assert!(!executor.has_hooks_for_stage("test-host", HookStage::PostActivate));
        assert!(!executor.has_hooks_for_stage("other-host", HookStage::PreActivate));
    }

    // ========================================================================
    // Remote hook execution tests using MockHookBackend
    // ========================================================================

    #[test]
    fn test_executor_with_remote_hook_success() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                HookStage::PostActivate,
                HookLocation::Remote,
                "systemctl restart nginx",
            )],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);
        let target = DeployTarget {
            hostname: "test-host".to_string(),
            user: Some("deploy".to_string()),
            port: None,
        };

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PostActivate,
                Some(&target),
                &HashMap::new(),
            )
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].0, "test-host");
        assert_eq!(executed[0].1, HookStage::PostActivate);
        assert_eq!(executed[0].2, HookLocation::Remote);
        assert_eq!(executed[0].3, "systemctl restart nginx");
    }

    #[test]
    fn test_executor_with_remote_hook_failure() {
        let backend =
            Arc::new(MockHookBackend::new().with_remote_failures(vec!["test-host".to_string()]));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                HookStage::PostActivate,
                HookLocation::Remote,
                "systemctl restart nginx",
            )],
        );

        let executor = HookExecutor::new(backend, hooks_by_host);
        let target = DeployTarget {
            hostname: "test-host".to_string(),
            user: Some("deploy".to_string()),
            port: None,
        };

        let result = executor.execute_hooks_for_stage(
            "test-host",
            HookStage::PostActivate,
            Some(&target),
            &HashMap::new(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_executor_remote_hook_without_target() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                HookStage::PostActivate,
                HookLocation::Remote,
                "systemctl restart nginx",
            )],
        );

        let executor = HookExecutor::new(backend, hooks_by_host);

        let result = executor.execute_hooks_for_stage(
            "test-host",
            HookStage::PostActivate,
            None,
            &HashMap::new(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_executor_with_mixed_local_and_remote_hooks() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "kubectl drain ${SNOW_HOSTNAME}",
                ),
                HookConfig::test_fixture(
                    HookStage::PreActivate,
                    HookLocation::Remote,
                    "systemctl stop important-service",
                ),
            ],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);
        let target = DeployTarget {
            hostname: "test-host".to_string(),
            user: None,
            port: None,
        };

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, Some(&target), &env)
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results[0].success);
        assert!(results[1].success);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 2);
        assert_eq!(executed[0].2, HookLocation::Local);
        assert_eq!(executed[1].2, HookLocation::Remote);
    }

    #[test]
    fn test_executor_all_hook_stages() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "pre-activate",
                ),
                HookConfig::test_fixture(
                    HookStage::PostActivate,
                    HookLocation::Local,
                    "post-activate",
                ),
                HookConfig::test_fixture(HookStage::PostReboot, HookLocation::Local, "post-reboot"),
                HookConfig::test_fixture(HookStage::OnFailure, HookLocation::Local, "on-failure"),
            ],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();
        executor
            .execute_hooks_for_stage("test-host", HookStage::PostActivate, None, &HashMap::new())
            .unwrap();
        executor
            .execute_hooks_for_stage("test-host", HookStage::PostReboot, None, &HashMap::new())
            .unwrap();
        executor
            .execute_hooks_for_stage("test-host", HookStage::OnFailure, None, &HashMap::new())
            .unwrap();

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 4);
        assert_eq!(executed[0].1, HookStage::PreActivate);
        assert_eq!(executed[1].1, HookStage::PostActivate);
        assert_eq!(executed[2].1, HookStage::PostReboot);
        assert_eq!(executed[3].1, HookStage::OnFailure);
    }

    #[test]
    fn test_executor_environment_variables_passed() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                HookStage::PreActivate,
                HookLocation::Local,
                "echo $SNOW_HOSTNAME $SNOW_PHASE $SNOW_STORE_PATH",
            )],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());
        env.insert("SNOW_PHASE".to_string(), "production".to_string());
        env.insert("SNOW_STORE_PATH".to_string(), "/nix/store/test".to_string());

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &env)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].0, "test-host");
    }

    #[test]
    fn test_executor_multiple_hooks_first_failure_aborts_remaining() {
        let backend =
            Arc::new(MockHookBackend::new().with_local_failures(vec!["fail".to_string()]));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "echo 'hook1'",
                ),
                HookConfig::test_fixture(
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "fail command",
                ),
                HookConfig::test_fixture(
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "echo 'should not run'",
                ),
            ],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let result =
            executor.execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &env);

        assert!(result.is_err());

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 2);
    }
}
