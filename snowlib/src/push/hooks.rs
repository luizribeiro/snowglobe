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
    use crate::process::OutputMode;

    fn make_test_hook(stage: HookStage, location: HookLocation, command: &str) -> HookConfig {
        HookConfig {
            stage,
            location,
            command: command.to_string(),
            timeout: 300,
            continue_on_failure: false,
        }
    }

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
    fn test_executor_with_local_hook() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![make_test_hook(
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
                make_test_hook(HookStage::PreActivate, HookLocation::Local, "echo 'hook1'"),
                make_test_hook(HookStage::PreActivate, HookLocation::Local, "echo 'hook2'"),
                make_test_hook(HookStage::PostActivate, HookLocation::Local, "echo 'hook3'"),
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
            vec![make_test_hook(
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
        let mut hook = make_test_hook(HookStage::PreActivate, HookLocation::Local, "exit 1");
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
            vec![make_test_hook(
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
}
