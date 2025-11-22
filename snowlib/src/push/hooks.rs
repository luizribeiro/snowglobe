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

        let mut stage_hooks: Vec<&HookConfig> = hooks.iter().filter(|h| h.stage == stage).collect();
        stage_hooks.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.name.cmp(&b.name))
        });

        if stage_hooks.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for hook in stage_hooks {
            tracing::info!(
                target: "hooks",
                "Executing {:?} hook '{}' for {}: {}",
                stage,
                hook.name,
                hostname,
                hook.command
            );

            // Merge hook-specific environment variables with base environment
            let mut merged_env = env.clone();

            // Load environment files first (so explicit env vars can override)
            for env_file in &hook.environment_files {
                match Self::load_env_file(env_file) {
                    Ok(file_env) => {
                        merged_env.extend(file_env);
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "hooks",
                            "Failed to load environment file {:?} for hook '{}' on {}: {}",
                            env_file,
                            hook.name,
                            hostname,
                            e
                        );
                        if !hook.continue_on_failure {
                            return Err(crate::error::SnowError::Deploy(
                                crate::error::DeployError::Failed {
                                    host: hostname.to_string(),
                                    source: std::io::Error::other(format!(
                                        "Hook '{}' failed to load environment file {:?}: {}",
                                        hook.name, env_file, e
                                    )),
                                },
                            ));
                        }
                    }
                }
            }

            // Then merge explicit environment variables (these take precedence)
            merged_env.extend(hook.environment.clone());

            let result = match hook.location {
                HookLocation::Local => self.backend.execute_local(hook, &merged_env)?,
                HookLocation::Remote => {
                    let target = target.ok_or_else(|| {
                        crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                            host: hostname.to_string(),
                            source: std::io::Error::other(
                                "Remote hook execution requires target, but target is None",
                            ),
                        })
                    })?;
                    self.backend.execute_remote(target, hook, &merged_env)?
                }
            };

            if !result.success && !hook.continue_on_failure {
                tracing::error!(
                    target: "hooks",
                    "Hook '{}' ({:?} stage) failed for {} (exit code: {:?}): {}",
                    hook.name,
                    stage,
                    hostname,
                    result.exit_code,
                    result.stderr
                );

                return Err(crate::error::SnowError::Deploy(
                    crate::error::DeployError::CommandFailed {
                        host: hostname.to_string(),
                        command: format!(
                            "Hook '{}' ({:?} stage): {}",
                            hook.name, stage, hook.command
                        ),
                        exit_code: result.exit_code.unwrap_or(-1),
                        stderr: result.stderr.clone(),
                        stdout: result.stdout.clone(),
                    },
                ));
            } else if !result.success {
                tracing::warn!(
                    target: "hooks",
                    "Hook '{}' ({:?} stage) failed for {} but continuing (continueOnFailure=true): {}",
                    hook.name,
                    stage,
                    hostname,
                    result.stderr
                );
            } else {
                tracing::info!(
                    target: "hooks",
                    "Hook '{}' completed successfully for {} in {:?}",
                    hook.name,
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

    pub(crate) fn load_env_file(path: &std::path::Path) -> Result<HashMap<String, String>> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                host: "local".to_string(),
                source: std::io::Error::new(
                    e.kind(),
                    format!("Failed to read environment file {:?}: {}", path, e),
                ),
            })
        })?;

        let mut env_vars = HashMap::new();
        for (line_num, line) in contents.lines().enumerate() {
            let line = line.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                env_vars.insert(key.trim().to_string(), value.trim().to_string());
            } else {
                tracing::warn!(
                    target: "hooks",
                    "Skipping invalid line {} in {:?}: {}",
                    line_num + 1,
                    path,
                    line
                );
            }
        }

        Ok(env_vars)
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
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                "drain-node",
                HookStage::PreActivate,
                HookLocation::Local,
                "kubectl drain test-host --ignore-daemonsets --timeout=5m",
            )],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
        assert!(executed[0].3.contains("kubectl drain"));
    }

    #[test]
    fn test_executor_with_multiple_hooks() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    "drain-node",
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "kubectl drain test-host --ignore-daemonsets --timeout=5m",
                ),
                HookConfig::test_fixture(
                    "consul-maint",
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "consul maint -enable -reason='NixOS upgrade'",
                ),
                HookConfig::test_fixture(
                    "health-check",
                    HookStage::PostActivate,
                    HookLocation::Local,
                    "systemctl is-active nginx postgresql redis",
                ),
            ],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 2);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 2);
        // Hooks execute alphabetically by name: "consul-maint" < "drain-node"
        assert!(executed[0].3.contains("consul maint"));
        assert!(executed[1].3.contains("kubectl drain"));
    }

    #[test]
    fn test_executor_hook_failure_aborts() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                "test-hook",
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
        let mut hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "exit 1",
        );
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
                "drain-node",
                HookStage::PreActivate,
                HookLocation::Local,
                "kubectl drain test-host --ignore-daemonsets",
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
                "test-hook",
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
                "test-hook",
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
                "test-hook",
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
                    "test-hook",
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "kubectl drain ${SNOW_HOSTNAME}",
                ),
                HookConfig::test_fixture(
                    "test-hook",
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
                    "test-hook",
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "pre-activate",
                ),
                HookConfig::test_fixture(
                    "test-hook",
                    HookStage::PostActivate,
                    HookLocation::Local,
                    "post-activate",
                ),
                HookConfig::test_fixture(
                    "test-hook",
                    HookStage::PostReboot,
                    HookLocation::Local,
                    "post-reboot",
                ),
                HookConfig::test_fixture(
                    "test-hook",
                    HookStage::OnFailure,
                    HookLocation::Local,
                    "on-failure",
                ),
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
                "test-hook",
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
            Arc::new(MockHookBackend::new().with_local_failures(vec!["consul".to_string()]));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    "drain-node",
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "kubectl drain test-host --ignore-daemonsets",
                ),
                HookConfig::test_fixture(
                    "failing-check",
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "consul lock check/readiness",
                ),
                HookConfig::test_fixture(
                    "should-not-run",
                    HookStage::PreActivate,
                    HookLocation::Local,
                    "systemctl stop critical-service",
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

    // Integration tests for environment file loading in executor

    #[test]
    fn test_executor_with_environment_file() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_hook_env_integration.env");
        std::fs::write(&temp_file, "SECRET_KEY=secret123\nAPI_TOKEN=token456\n").unwrap();

        let mut hook = HookConfig::test_fixture(
            "env-test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo $SECRET_KEY $API_TOKEN",
        );
        hook.environment_files = vec![temp_file.clone()];

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert!(results[0].stdout.contains("secret123"));
        assert!(results[0].stdout.contains("token456"));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_executor_environment_file_overrides_precedence() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_hook_env_precedence.env");
        std::fs::write(&temp_file, "VAR=from_file\n").unwrap();

        let mut hook = HookConfig::test_fixture(
            "precedence-test",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo $VAR",
        );
        hook.environment_files = vec![temp_file.clone()];
        // Explicit env vars should override file values
        hook.environment
            .insert("VAR".to_string(), "from_explicit".to_string());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        // Explicit variable should win
        assert!(results[0].stdout.contains("from_explicit"));
        assert!(!results[0].stdout.contains("from_file"));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_executor_environment_file_not_found_fails_deployment() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hook = HookConfig::test_fixture(
            "missing-env-file",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo test",
        );
        hook.environment_files = vec![std::path::PathBuf::from("/nonexistent/file.env")];
        hook.continue_on_failure = false; // Should fail deployment

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let result = executor.execute_hooks_for_stage(
            "test-host",
            HookStage::PreActivate,
            None,
            &HashMap::new(),
        );

        assert!(result.is_err());
        // Error should indicate deployment failed for test-host
        let err = result.unwrap_err();
        assert!(err.to_string().contains("test-host"));
    }

    #[test]
    fn test_executor_environment_file_not_found_continues_with_flag() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hook = HookConfig::test_fixture(
            "missing-env-file-continue",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo test",
        );
        hook.environment_files = vec![std::path::PathBuf::from("/nonexistent/file.env")];
        hook.continue_on_failure = true; // Should continue even with missing file

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        // Hook should execute even though env file failed to load
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
    }

    #[test]
    fn test_executor_multiple_environment_files() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let temp_dir = std::env::temp_dir();
        let temp_file1 = temp_dir.join("test_hook_env_multi_1.env");
        let temp_file2 = temp_dir.join("test_hook_env_multi_2.env");

        std::fs::write(&temp_file1, "VAR1=value1\n").unwrap();
        std::fs::write(&temp_file2, "VAR2=value2\n").unwrap();

        let mut hook = HookConfig::test_fixture(
            "multi-env-file",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo $VAR1 $VAR2",
        );
        hook.environment_files = vec![temp_file1.clone(), temp_file2.clone()];

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert!(results[0].stdout.contains("value1"));
        assert!(results[0].stdout.contains("value2"));

        std::fs::remove_file(&temp_file1).ok();
        std::fs::remove_file(&temp_file2).ok();
    }

    #[test]
    fn test_load_env_file_valid() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_valid.env");
        std::fs::write(&temp_file, "KEY1=value1\nKEY2=value2\n").unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.get("KEY1"), Some(&"value1".to_string()));
        assert_eq!(result.get("KEY2"), Some(&"value2".to_string()));
        assert_eq!(result.len(), 2);

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_with_comments() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_comments.env");
        std::fs::write(
            &temp_file,
            "# This is a comment\nKEY=value\n# Another comment\n",
        )
        .unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result.get("KEY"), Some(&"value".to_string()));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_with_empty_lines() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_empty.env");
        std::fs::write(&temp_file, "KEY1=value1\n\n\nKEY2=value2\n\n").unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.get("KEY1"), Some(&"value1".to_string()));
        assert_eq!(result.get("KEY2"), Some(&"value2".to_string()));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_invalid_line_skipped() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_invalid.env");
        std::fs::write(
            &temp_file,
            "KEY=value\ninvalid_line_no_equals\nKEY2=value2\n",
        )
        .unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        // Invalid line should be skipped
        assert_eq!(result.len(), 2);
        assert_eq!(result.get("KEY"), Some(&"value".to_string()));
        assert_eq!(result.get("KEY2"), Some(&"value2".to_string()));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_not_found() {
        let result = HookExecutor::load_env_file(std::path::Path::new("/nonexistent/file.env"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_env_file_value_with_equals() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_equals.env");
        std::fs::write(&temp_file, "KEY=value=with=equals\n").unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.get("KEY"), Some(&"value=with=equals".to_string()));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_whitespace_trimmed() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_whitespace.env");
        std::fs::write(&temp_file, "  KEY1  =  value1  \n\tKEY2\t=\tvalue2\t\n").unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.get("KEY1"), Some(&"value1".to_string()));
        assert_eq!(result.get("KEY2"), Some(&"value2".to_string()));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_value_with_spaces() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_spaces.env");
        std::fs::write(&temp_file, "MESSAGE=Hello World\n").unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.get("MESSAGE"), Some(&"Hello World".to_string()));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_with_shell_injection_attempts() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_injection.env");

        std::fs::write(
            &temp_file,
            concat!(
                "CMD1=$(rm -rf /)\n",
                "CMD2=`cat /etc/passwd`\n",
                "CMD3=; rm -rf /\n",
                "CMD4=value && echo hacked\n",
                "CMD5=value | cat /etc/shadow\n",
                "CMD6='value'; echo malicious\n",
                "CMD7=\"value\"; echo injection\n"
            ),
        )
        .unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.get("CMD1"), Some(&"$(rm -rf /)".to_string()));
        assert_eq!(result.get("CMD2"), Some(&"`cat /etc/passwd`".to_string()));
        assert_eq!(result.get("CMD3"), Some(&"; rm -rf /".to_string()));
        assert_eq!(
            result.get("CMD4"),
            Some(&"value && echo hacked".to_string())
        );
        assert_eq!(
            result.get("CMD5"),
            Some(&"value | cat /etc/shadow".to_string())
        );
        assert_eq!(
            result.get("CMD6"),
            Some(&"'value'; echo malicious".to_string())
        );
        assert_eq!(
            result.get("CMD7"),
            Some(&"\"value\"; echo injection".to_string())
        );

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_with_unicode() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_unicode.env");

        std::fs::write(&temp_file, "GREETING=Hello 世界\nEMOJI=🚀\nARROW=→\n").unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.get("GREETING"), Some(&"Hello 世界".to_string()));
        assert_eq!(result.get("EMOJI"), Some(&"🚀".to_string()));
        assert_eq!(result.get("ARROW"), Some(&"→".to_string()));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_large_file() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_large.env");

        let mut content = String::new();
        for i in 0..10000 {
            content.push_str(&format!("KEY{}=value{}\n", i, i));
        }

        std::fs::write(&temp_file, &content).unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.len(), 10000);
        assert_eq!(result.get("KEY0"), Some(&"value0".to_string()));
        assert_eq!(result.get("KEY9999"), Some(&"value9999".to_string()));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_load_env_file_with_newlines_in_values() {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_env_newlines.env");

        std::fs::write(
            &temp_file,
            "KEY1=value with\nnewline should only get first part\nKEY2=second value\n",
        )
        .unwrap();

        let result = HookExecutor::load_env_file(&temp_file).unwrap();

        assert_eq!(result.get("KEY1"), Some(&"value with".to_string()));
        assert_eq!(result.get("KEY2"), Some(&"second value".to_string()));

        std::fs::remove_file(&temp_file).ok();
    }

    #[test]
    fn test_hook_execution_order_by_priority() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hook1 = HookConfig::test_fixture(
            "z-last-alphabetically",
            HookStage::PreActivate,
            HookLocation::Local,
            "etcdctl set /deploy/status preparing",
        );
        hook1.priority = 10; // Should execute first

        let mut hook2 = HookConfig::test_fixture(
            "a-first-alphabetically",
            HookStage::PreActivate,
            HookLocation::Local,
            "consul lock acquire deploy-lock",
        );
        hook2.priority = 50; // Should execute second

        let mut hook3 = HookConfig::test_fixture(
            "m-middle-alphabetically",
            HookStage::PreActivate,
            HookLocation::Local,
            "kubectl drain node --ignore-daemonsets",
        );
        hook3.priority = 100; // Should execute third

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook3, hook1, hook2]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 3);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 3);
        assert!(
            executed[0].3.contains("etcdctl"),
            "First hook should be priority 10"
        );
        assert!(
            executed[1].3.contains("consul"),
            "Second hook should be priority 50"
        );
        assert!(
            executed[2].3.contains("kubectl"),
            "Third hook should be priority 100"
        );
    }

    #[test]
    fn test_hook_execution_order_same_priority_sorted_by_name() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hook1 = HookConfig::test_fixture(
            "z-last",
            HookStage::PreActivate,
            HookLocation::Local,
            "systemctl stop z-service",
        );
        hook1.priority = 100;

        let mut hook2 = HookConfig::test_fixture(
            "a-first",
            HookStage::PreActivate,
            HookLocation::Local,
            "systemctl stop a-service",
        );
        hook2.priority = 100;

        let mut hook3 = HookConfig::test_fixture(
            "m-middle",
            HookStage::PreActivate,
            HookLocation::Local,
            "systemctl stop m-service",
        );
        hook3.priority = 100;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook1, hook3, hook2]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 3);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 3);
        assert!(
            executed[0].3.contains("a-service"),
            "First hook should be 'a-first' (alphabetical)"
        );
        assert!(
            executed[1].3.contains("m-service"),
            "Second hook should be 'm-middle' (alphabetical)"
        );
        assert!(
            executed[2].3.contains("z-service"),
            "Third hook should be 'z-last' (alphabetical)"
        );
    }

    #[test]
    fn test_hook_execution_order_negative_priorities() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hook1 = HookConfig::test_fixture(
            "normal-priority",
            HookStage::PreActivate,
            HookLocation::Local,
            "consul maint -enable",
        );
        hook1.priority = 100;

        let mut hook2 = HookConfig::test_fixture(
            "early-priority",
            HookStage::PreActivate,
            HookLocation::Local,
            "kubectl cordon node",
        );
        hook2.priority = -10; // Negative priority should execute first

        let mut hook3 = HookConfig::test_fixture(
            "late-priority",
            HookStage::PreActivate,
            HookLocation::Local,
            "curl -X POST https://webhook.site/notify",
        );
        hook3.priority = 200;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook1, hook2, hook3]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &HashMap::new())
            .unwrap();

        assert_eq!(results.len(), 3);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 3);
        assert!(
            executed[0].3.contains("kubectl cordon"),
            "First hook should be priority -10"
        );
        assert!(
            executed[1].3.contains("consul maint"),
            "Second hook should be priority 100"
        );
        assert!(
            executed[2].3.contains("curl"),
            "Third hook should be priority 200"
        );
    }

    #[test]
    fn test_environment_file_with_shell_injection_is_escaped() {
        use crate::backends::hook::SystemHookBackend;
        use crate::process::OutputMode;
        use std::sync::Arc;

        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_shell_injection.env");
        let marker_file = temp_dir.join("injection_test_marker");

        std::fs::remove_file(&marker_file).ok();

        std::fs::write(
            &temp_file,
            "MALICIOUS=$(rm -rf /tmp/test_injection_target)\n\
             INJECTION='; touch /tmp/injection_marker; echo 'pwned\n\
             BACKTICKS=`id`\n\
             COMMAND_SUB=$(whoami)\n",
        )
        .unwrap();

        let mut hook = HookConfig::test_fixture(
            "injection-test",
            HookStage::PreActivate,
            HookLocation::Local,
            &format!("test ! -f {} && echo $MALICIOUS && echo $INJECTION && echo $BACKTICKS && echo $COMMAND_SUB", marker_file.display()),
        );
        hook.environment_files = vec![temp_file.clone()];

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreActivate, None, &env)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(
            results[0].success,
            "Hook should succeed - injection should be escaped"
        );

        // Verify the malicious commands were NOT executed
        assert!(
            !marker_file.exists(),
            "Injection marker file should not exist - shell injection should be escaped"
        );

        // Verify the values were treated as literal strings
        let stdout = &results[0].stdout;
        assert!(
            stdout.contains("$(rm -rf /tmp/test_injection_target)")
                || stdout.contains("rm -rf /tmp/test_injection_target"),
            "Malicious command should appear as literal string in output, not be executed"
        );

        std::fs::remove_file(&temp_file).ok();
        std::fs::remove_file(&marker_file).ok();
    }
}
