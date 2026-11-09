//! Hook execution orchestration for deployment pipeline

use crate::backends::deploy::DeployTarget;
use crate::backends::hook::{HookBackend, HookResult};
use crate::error::Result;
use crate::hooks::{HookConfig, HookLocation, HookStage};
use crate::observer::DeployObserver;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

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
    pub async fn execute_hooks_for_stage(
        &self,
        hostname: &str,
        stage: HookStage,
        target: Option<&DeployTarget>,
        env: &HashMap<String, String>,
        observer: Option<&dyn DeployObserver>,
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

        // Notify observer that hooks are starting for this stage
        if let Some(obs) = observer {
            obs.on_hooks_start(hostname, stage);
        }

        let mut results = Vec::new();
        let mut all_succeeded = true;

        for hook in stage_hooks {
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
                            "Failed to load environment file {:?} for hook '{}' on {}: {}",
                            env_file,
                            hook.name,
                            hostname,
                            e
                        );
                        if !hook.continue_on_failure {
                            return Err(e);
                        }
                    }
                }
            }

            // Then merge explicit environment variables (these take precedence)
            merged_env.extend(hook.environment.clone());

            // Notify observer that this hook is starting
            if let Some(obs) = observer {
                obs.on_hook_start(hostname, stage, &hook.name);
            }

            // Execute with retry logic
            let max_attempts = hook.retries + 1;
            let mut attempt = 1u32;

            let result = loop {
                tracing::info!(
                    "Executing {} hook '{}' for {} (attempt {}/{}): {}",
                    stage,
                    hook.name,
                    hostname,
                    attempt,
                    max_attempts,
                    hook.command
                );

                let result = match hook.location {
                    HookLocation::Local => self.backend.execute_local(hook, &merged_env).await?,
                    HookLocation::Remote => {
                        let target = target.ok_or_else(|| {
                            crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                                host: hostname.to_string(),
                                source: std::io::Error::other(
                                    "Remote hook execution requires target, but target is None",
                                ),
                            })
                        })?;
                        self.backend
                            .execute_remote(target, hook, &merged_env)
                            .await?
                    }
                };

                if result.success {
                    tracing::info!(
                        "Hook '{}' completed successfully for {} in {:?}",
                        hook.name,
                        hostname,
                        result.duration
                    );
                    break result;
                }

                if attempt >= max_attempts {
                    break result;
                }

                tracing::warn!(
                    "{}, retrying in {}s: {}",
                    hook_failure(hostname, hook, &result, attempt),
                    hook.retry_delay,
                    last_stderr_line(&result.stderr)
                );

                tokio::time::sleep(Duration::from_secs(hook.retry_delay)).await;
                attempt += 1;
            };

            // Notify observer that this hook completed
            if let Some(obs) = observer {
                obs.on_hook_complete(hostname, stage, &hook.name, result.success);
            }

            if !result.success {
                all_succeeded = false;
            }

            if !result.success && !hook.continue_on_failure {
                tracing::error!(
                    "{}: {}",
                    hook_failure(hostname, hook, &result, attempt),
                    result.stderr.trim()
                );

                // Notify observer that hooks stage failed
                if let Some(obs) = observer {
                    obs.on_hooks_complete(hostname, stage, false);
                }

                return Err(crate::error::SnowError::Deploy(
                    crate::error::DeployError::CommandFailed {
                        host: hostname.to_string(),
                        command: format!("Hook '{}' ({stage}): {}", hook.name, hook.command),
                        exit_code: result.exit_code.unwrap_or(-1),
                        stderr: result.stderr.clone(),
                        stdout: result.stdout.clone(),
                    },
                ));
            } else if !result.success {
                tracing::warn!(
                    "{}, continuing as it may fail: {}",
                    hook_failure(hostname, hook, &result, attempt),
                    last_stderr_line(&result.stderr)
                );
            }

            results.push(result);
        }

        // Notify observer that all hooks for this stage completed
        if let Some(obs) = observer {
            obs.on_hooks_complete(hostname, stage, all_succeeded);
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

    fn load_env_file(path: &std::path::Path) -> Result<HashMap<String, String>> {
        let contents =
            std::fs::read_to_string(path).map_err(|source| crate::error::IoError::HookEnvFile {
                path: path.to_path_buf(),
                source,
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

/// As in "radon: hook 'drain' (pre-deploy) failed with exit code 1 after 3 attempts"
fn hook_failure(hostname: &str, hook: &HookConfig, result: &HookResult, attempts: u32) -> String {
    let exit = match result.exit_code {
        Some(code) => format!("with exit code {code}"),
        None => "without an exit code".to_string(),
    };
    let tries = match attempts {
        1 => String::new(),
        _ => format!(" after {attempts} attempts"),
    };
    format!(
        "{hostname}: hook '{}' ({}) failed {exit}{tries}",
        hook.name, hook.stage
    )
}

/// A failed command's last words on stderr usually say why it failed
fn last_stderr_line(stderr: &str) -> &str {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("no output")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::hook::MockHookBackend;
    use crate::backends::hook::SystemHookBackend;
    use crate::process::OutputMode;

    fn failed_hook(exit_code: Option<i32>) -> HookResult {
        HookResult {
            success: false,
            exit_code,
            stdout: String::new(),
            stderr: "storage on radon is degraded\n".to_string(),
            duration: Duration::from_secs(2),
        }
    }

    fn drain_hook() -> HookConfig {
        HookConfig::test_fixture(
            "drain",
            HookStage::PreDeploy,
            HookLocation::Local,
            "kubectl drain radon",
        )
    }

    #[test]
    fn test_hook_failure_gives_the_stage_and_plain_exit_code() {
        assert_eq!(
            hook_failure("radon", &drain_hook(), &failed_hook(Some(1)), 1),
            "radon: hook 'drain' (pre-deploy) failed with exit code 1"
        );
    }

    #[test]
    fn test_hook_failure_counts_the_attempts() {
        assert_eq!(
            hook_failure("radon", &drain_hook(), &failed_hook(Some(3)), 4),
            "radon: hook 'drain' (pre-deploy) failed with exit code 3 after 4 attempts"
        );
    }

    #[test]
    fn test_hook_failure_of_a_killed_hook() {
        assert_eq!(
            hook_failure("radon", &drain_hook(), &failed_hook(None), 1),
            "radon: hook 'drain' (pre-deploy) failed without an exit code"
        );
    }

    #[test]
    fn test_last_stderr_line_skips_trailing_blank_lines() {
        let stderr = "node/radon cordoned\nerror: cannot evict pod storage-0\n\n  \n";

        assert_eq!(
            last_stderr_line(stderr),
            "error: cannot evict pod storage-0"
        );
    }

    #[test]
    fn test_last_stderr_line_of_a_silent_hook() {
        assert_eq!(last_stderr_line(""), "no output");
        assert_eq!(last_stderr_line("\n \n"), "no output");
    }

    #[tokio::test]
    async fn test_executor_with_no_hooks() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));
        let executor = HookExecutor::new(backend, HashMap::new());

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_executor_with_empty_hooks_array() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));
        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), Vec::new());
        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_executor_with_local_hook() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                "drain-node",
                HookStage::PreDeploy,
                HookLocation::Local,
                "kubectl drain test-host --ignore-daemonsets --timeout=5m",
            )],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
        assert!(executed[0].3.contains("kubectl drain"));
    }

    #[tokio::test]
    async fn test_executor_with_multiple_hooks() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    "drain-node",
                    HookStage::PreDeploy,
                    HookLocation::Local,
                    "kubectl drain test-host --ignore-daemonsets --timeout=5m",
                ),
                HookConfig::test_fixture(
                    "consul-maint",
                    HookStage::PreDeploy,
                    HookLocation::Local,
                    "consul maint -enable -reason='NixOS upgrade'",
                ),
                HookConfig::test_fixture(
                    "health-check",
                    HookStage::PostDeploy,
                    HookLocation::Local,
                    "systemctl is-active nginx postgresql redis",
                ),
            ],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 2);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 2);
        // Hooks execute alphabetically by name: "consul-maint" < "drain-node"
        assert!(executed[0].3.contains("consul maint"));
        assert!(executed[1].3.contains("kubectl drain"));
    }

    #[tokio::test]
    async fn test_executor_hook_failure_aborts() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                "test-hook",
                HookStage::PreDeploy,
                HookLocation::Local,
                "exit 1",
            )],
        );

        let executor = HookExecutor::new(backend, hooks_by_host);

        let result = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_executor_hook_failure_with_continue() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        let mut hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "exit 1",
        );
        hook.continue_on_failure = true;
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
    }

    #[tokio::test]
    async fn test_hook_failure_with_continue_returns_failure_in_results() {
        let backend =
            Arc::new(MockHookBackend::new().with_local_failures(vec!["exit 1".to_string()]));

        let mut hook = HookConfig::test_fixture(
            "failing-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "exit 1",
        );
        hook.continue_on_failure = true;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        // Should return results even though hook failed
        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
        assert_eq!(results[0].exit_code, Some(1));
    }

    #[test]
    fn test_has_hooks_for_stage() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                "drain-node",
                HookStage::PreDeploy,
                HookLocation::Local,
                "kubectl drain test-host --ignore-daemonsets",
            )],
        );

        let executor = HookExecutor::new(backend, hooks_by_host);

        assert!(executor.has_hooks_for_stage("test-host", HookStage::PreDeploy));
        assert!(!executor.has_hooks_for_stage("test-host", HookStage::PostDeploy));
        assert!(!executor.has_hooks_for_stage("other-host", HookStage::PreDeploy));
    }

    // ========================================================================
    // Remote hook execution tests using MockHookBackend
    // ========================================================================

    #[tokio::test]
    async fn test_executor_with_remote_hook_success() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                "test-hook",
                HookStage::PostDeploy,
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
                HookStage::PostDeploy,
                Some(&target),
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].0, "test-host");
        assert_eq!(executed[0].1, HookStage::PostDeploy);
        assert_eq!(executed[0].2, HookLocation::Remote);
        assert_eq!(executed[0].3, "systemctl restart nginx");
    }

    #[tokio::test]
    async fn test_executor_with_remote_hook_failure() {
        let backend =
            Arc::new(MockHookBackend::new().with_remote_failures(vec!["test-host".to_string()]));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                "test-hook",
                HookStage::PostDeploy,
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

        let result = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PostDeploy,
                Some(&target),
                &HashMap::new(),
                None,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_executor_remote_hook_without_target() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                "test-hook",
                HookStage::PostDeploy,
                HookLocation::Remote,
                "systemctl restart nginx",
            )],
        );

        let executor = HookExecutor::new(backend, hooks_by_host);

        let result = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PostDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_executor_with_mixed_local_and_remote_hooks() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    "test-hook",
                    HookStage::PreDeploy,
                    HookLocation::Local,
                    "kubectl drain ${SNOW_HOSTNAME}",
                ),
                HookConfig::test_fixture(
                    "test-hook",
                    HookStage::PreDeploy,
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
            .execute_hooks_for_stage("test-host", HookStage::PreDeploy, Some(&target), &env, None)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results[0].success);
        assert!(results[1].success);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 2);
        assert_eq!(executed[0].2, HookLocation::Local);
        assert_eq!(executed[1].2, HookLocation::Remote);
    }

    #[tokio::test]
    async fn test_executor_all_hook_stages() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    "test-hook",
                    HookStage::PreDeploy,
                    HookLocation::Local,
                    "pre-deploy",
                ),
                HookConfig::test_fixture(
                    "test-hook",
                    HookStage::PostDeploy,
                    HookLocation::Local,
                    "post-deploy",
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
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();
        executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PostDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();
        executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PostReboot,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();
        executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::OnFailure,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 4);
        assert_eq!(executed[0].1, HookStage::PreDeploy);
        assert_eq!(executed[1].1, HookStage::PostDeploy);
        assert_eq!(executed[2].1, HookStage::PostReboot);
        assert_eq!(executed[3].1, HookStage::OnFailure);
    }

    #[tokio::test]
    async fn test_executor_environment_variables_passed() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![HookConfig::test_fixture(
                "test-hook",
                HookStage::PreDeploy,
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
            .execute_hooks_for_stage("test-host", HookStage::PreDeploy, None, &env, None)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].0, "test-host");
    }

    #[tokio::test]
    async fn test_executor_multiple_hooks_first_failure_aborts_remaining() {
        let backend =
            Arc::new(MockHookBackend::new().with_local_failures(vec!["consul".to_string()]));

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert(
            "test-host".to_string(),
            vec![
                HookConfig::test_fixture(
                    "drain-node",
                    HookStage::PreDeploy,
                    HookLocation::Local,
                    "kubectl drain test-host --ignore-daemonsets",
                ),
                HookConfig::test_fixture(
                    "failing-check",
                    HookStage::PreDeploy,
                    HookLocation::Local,
                    "consul lock check/readiness",
                ),
                HookConfig::test_fixture(
                    "should-not-run",
                    HookStage::PreDeploy,
                    HookLocation::Local,
                    "systemctl stop critical-service",
                ),
            ],
        );

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let result = executor
            .execute_hooks_for_stage("test-host", HookStage::PreDeploy, None, &env, None)
            .await;

        assert!(result.is_err());

        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 2);
    }

    // Integration tests for environment file loading in executor

    #[tokio::test]
    async fn test_executor_with_environment_file() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_hook_env_integration.env");
        std::fs::write(&temp_file, "SECRET_KEY=secret123\nAPI_TOKEN=token456\n").unwrap();

        let mut hook = HookConfig::test_fixture(
            "env-test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo $SECRET_KEY $API_TOKEN",
        );
        hook.environment_files = vec![temp_file.clone()];

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert!(results[0].stdout.contains("secret123"));
        assert!(results[0].stdout.contains("token456"));

        std::fs::remove_file(&temp_file).ok();
    }

    #[tokio::test]
    async fn test_executor_environment_file_overrides_precedence() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_hook_env_precedence.env");
        std::fs::write(&temp_file, "VAR=from_file\n").unwrap();

        let mut hook = HookConfig::test_fixture(
            "precedence-test",
            HookStage::PreDeploy,
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
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        // Explicit variable should win
        assert!(results[0].stdout.contains("from_explicit"));
        assert!(!results[0].stdout.contains("from_file"));

        std::fs::remove_file(&temp_file).ok();
    }

    #[tokio::test]
    async fn test_executor_environment_file_not_found_fails_deployment() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hook = HookConfig::test_fixture(
            "missing-env-file",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo test",
        );
        hook.environment_files = vec![std::path::PathBuf::from("/nonexistent/file.env")];
        hook.continue_on_failure = false; // Should fail deployment

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let result = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await;

        let error = result.unwrap_err();
        assert!(matches!(
            &error,
            crate::error::SnowError::Io(crate::error::IoError::HookEnvFile { .. })
        ));
        assert!(miette::Diagnostic::help(&error)
            .unwrap()
            .to_string()
            .contains("agenix or sops-nix"));
    }

    #[tokio::test]
    async fn test_executor_environment_file_not_found_continues_with_flag() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let mut hook = HookConfig::test_fixture(
            "missing-env-file-continue",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo test",
        );
        hook.environment_files = vec![std::path::PathBuf::from("/nonexistent/file.env")];
        hook.continue_on_failure = true; // Should continue even with missing file

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        // Hook should execute even though env file failed to load
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
    }

    #[tokio::test]
    async fn test_executor_multiple_environment_files() {
        let backend = Arc::new(SystemHookBackend::new(None, OutputMode::Capture));

        let temp_dir = std::env::temp_dir();
        let temp_file1 = temp_dir.join("test_hook_env_multi_1.env");
        let temp_file2 = temp_dir.join("test_hook_env_multi_2.env");

        std::fs::write(&temp_file1, "VAR1=value1\n").unwrap();
        std::fs::write(&temp_file2, "VAR2=value2\n").unwrap();

        let mut hook = HookConfig::test_fixture(
            "multi-env-file",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo $VAR1 $VAR2",
        );
        hook.environment_files = vec![temp_file1.clone(), temp_file2.clone()];

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend, hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert!(results[0].stdout.contains("value1"));
        assert!(results[0].stdout.contains("value2"));

        std::fs::remove_file(&temp_file1).ok();
        std::fs::remove_file(&temp_file2).ok();
    }

    #[test]
    fn test_load_env_file_parsing() {
        struct TestCase {
            name: &'static str,
            content: &'static str,
            expected: Vec<(&'static str, &'static str)>,
        }

        let test_cases = [
            TestCase {
                name: "basic key-value pairs",
                content: "KEY1=value1\nKEY2=value2\n",
                expected: vec![("KEY1", "value1"), ("KEY2", "value2")],
            },
            TestCase {
                name: "comments are ignored",
                content: "# This is a comment\nKEY=value\n# Another comment\n",
                expected: vec![("KEY", "value")],
            },
            TestCase {
                name: "empty lines are ignored",
                content: "KEY1=value1\n\n\nKEY2=value2\n\n",
                expected: vec![("KEY1", "value1"), ("KEY2", "value2")],
            },
            TestCase {
                name: "invalid lines are skipped",
                content: "KEY=value\ninvalid_line_no_equals\nKEY2=value2\n",
                expected: vec![("KEY", "value"), ("KEY2", "value2")],
            },
            TestCase {
                name: "values can contain equals signs",
                content: "KEY=value=with=equals\n",
                expected: vec![("KEY", "value=with=equals")],
            },
            TestCase {
                name: "whitespace is trimmed from keys and values",
                content: "  KEY1  =  value1  \n\tKEY2\t=\tvalue2\t\n",
                expected: vec![("KEY1", "value1"), ("KEY2", "value2")],
            },
            TestCase {
                name: "values can contain spaces",
                content: "MESSAGE=Hello World\n",
                expected: vec![("MESSAGE", "Hello World")],
            },
        ];

        let temp_dir = std::env::temp_dir();

        for (idx, test_case) in test_cases.iter().enumerate() {
            let temp_file = temp_dir.join(format!("test_env_parsing_{}.env", idx));
            std::fs::write(&temp_file, test_case.content).unwrap();

            let result = HookExecutor::load_env_file(&temp_file).unwrap();

            assert_eq!(
                result.len(),
                test_case.expected.len(),
                "Failed case '{}': expected {} entries, got {}",
                test_case.name,
                test_case.expected.len(),
                result.len()
            );

            for (key, expected_value) in &test_case.expected {
                assert_eq!(
                    result.get(*key),
                    Some(&expected_value.to_string()),
                    "Failed case '{}': key '{}' mismatch",
                    test_case.name,
                    key
                );
            }

            std::fs::remove_file(&temp_file).ok();
        }
    }

    #[test]
    fn test_load_env_file_not_found() {
        let result = HookExecutor::load_env_file(std::path::Path::new("/nonexistent/file.env"));

        let error = result.unwrap_err();
        assert!(matches!(
            &error,
            crate::error::SnowError::Io(crate::error::IoError::HookEnvFile { path, .. })
                if path == std::path::Path::new("/nonexistent/file.env")
        ));
        assert_eq!(
            error.to_string(),
            "Failed to read hook environment file '/nonexistent/file.env'"
        );
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

    #[tokio::test]
    async fn test_hook_execution_order_by_priority() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hook1 = HookConfig::test_fixture(
            "z-last-alphabetically",
            HookStage::PreDeploy,
            HookLocation::Local,
            "etcdctl set /deploy/status preparing",
        );
        hook1.priority = 10; // Should execute first

        let mut hook2 = HookConfig::test_fixture(
            "a-first-alphabetically",
            HookStage::PreDeploy,
            HookLocation::Local,
            "consul lock acquire deploy-lock",
        );
        hook2.priority = 50; // Should execute second

        let mut hook3 = HookConfig::test_fixture(
            "m-middle-alphabetically",
            HookStage::PreDeploy,
            HookLocation::Local,
            "kubectl drain node --ignore-daemonsets",
        );
        hook3.priority = 100; // Should execute third

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook3, hook1, hook2]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
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

    #[tokio::test]
    async fn test_hook_execution_order_same_priority_sorted_by_name() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hook1 = HookConfig::test_fixture(
            "z-last",
            HookStage::PreDeploy,
            HookLocation::Local,
            "systemctl stop z-service",
        );
        hook1.priority = 100;

        let mut hook2 = HookConfig::test_fixture(
            "a-first",
            HookStage::PreDeploy,
            HookLocation::Local,
            "systemctl stop a-service",
        );
        hook2.priority = 100;

        let mut hook3 = HookConfig::test_fixture(
            "m-middle",
            HookStage::PreDeploy,
            HookLocation::Local,
            "systemctl stop m-service",
        );
        hook3.priority = 100;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook1, hook3, hook2]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
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

    #[tokio::test]
    async fn test_hook_execution_order_negative_priorities() {
        let backend = Arc::new(MockHookBackend::new());

        let mut hook1 = HookConfig::test_fixture(
            "normal-priority",
            HookStage::PreDeploy,
            HookLocation::Local,
            "consul maint -enable",
        );
        hook1.priority = 100;

        let mut hook2 = HookConfig::test_fixture(
            "early-priority",
            HookStage::PreDeploy,
            HookLocation::Local,
            "kubectl cordon node",
        );
        hook2.priority = -10; // Negative priority should execute first

        let mut hook3 = HookConfig::test_fixture(
            "late-priority",
            HookStage::PreDeploy,
            HookLocation::Local,
            "curl -X POST https://webhook.site/notify",
        );
        hook3.priority = 200;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook1, hook2, hook3]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PreDeploy,
                None,
                &HashMap::new(),
                None,
            )
            .await
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

    /// End-to-end integration test preventing shell injection via environment files.
    ///
    /// Attack vector: Environment files could be controlled by attackers (e.g., through
    /// compromised secrets management or malicious configuration). Shell metacharacters
    /// in these files must not be interpreted during hook execution.
    ///
    /// This integration test verifies the complete flow: loading environment files,
    /// passing variables through the executor, and executing hooks. It confirms that
    /// multiple types of shell injection attempts (command substitution, semicolons,
    /// backticks) are all treated as literal strings and not executed.
    #[tokio::test]
    async fn test_environment_variables_are_literal_strings_not_shell_interpreted() {
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
            HookStage::PreDeploy,
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
            .execute_hooks_for_stage("test-host", HookStage::PreDeploy, None, &env, None)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(
            results[0].success,
            "Hook should succeed - environment variables are literal strings"
        );

        assert!(
            !marker_file.exists(),
            "Environment variables should be literal strings, not shell-interpreted"
        );

        let stdout = &results[0].stdout;
        assert!(
            stdout.contains("$(rm -rf /tmp/test_injection_target)")
                || stdout.contains("rm -rf /tmp/test_injection_target"),
            "Malicious command should appear as literal string in output, not be executed"
        );

        std::fs::remove_file(&temp_file).ok();
        std::fs::remove_file(&marker_file).ok();
    }

    // ========================================================================
    // Retry functionality tests
    // ========================================================================

    #[tokio::test]
    async fn test_hook_retry_succeeds_after_initial_failure() {
        // Configure hook to fail twice, then succeed on third attempt
        let backend = Arc::new(MockHookBackend::new().with_fail_n_times("flaky-service", 2));

        let mut hook = HookConfig::test_fixture(
            "retry-test",
            HookStage::PostReboot,
            HookLocation::Local,
            "flaky-service check",
        );
        hook.retries = 3;
        hook.retry_delay = 0; // No delay for faster test

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PostReboot, None, &env, None)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success, "Hook should eventually succeed");

        // Verify the hook was attempted 3 times (2 failures + 1 success)
        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 3, "Hook should have been attempted 3 times");
    }

    #[tokio::test]
    async fn test_hook_retry_exhausted_all_attempts() {
        // Configure hook to always fail (fail more times than retries allow)
        let backend = Arc::new(MockHookBackend::new().with_fail_n_times("always-fails", 10));

        let mut hook = HookConfig::test_fixture(
            "retry-test",
            HookStage::PostReboot,
            HookLocation::Local,
            "always-fails check",
        );
        hook.retries = 2; // Will try 3 times total
        hook.retry_delay = 0;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let result = executor
            .execute_hooks_for_stage("test-host", HookStage::PostReboot, None, &env, None)
            .await;

        assert!(
            result.is_err(),
            "Hook should fail after all retries exhausted"
        );

        // Verify the hook was attempted exactly max_attempts times
        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 3, "Hook should have been attempted 3 times");
    }

    #[tokio::test]
    async fn test_hook_retry_zero_retries_behaves_as_before() {
        // Hook with retries=0 (default) should behave exactly as before
        let backend = Arc::new(MockHookBackend::new().with_fail_n_times("one-shot", 1));

        let hook = HookConfig::test_fixture(
            "no-retry",
            HookStage::PreDeploy,
            HookLocation::Local,
            "one-shot command",
        );
        // retries is 0 by default from test_fixture

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let result = executor
            .execute_hooks_for_stage("test-host", HookStage::PreDeploy, None, &env, None)
            .await;

        assert!(
            result.is_err(),
            "Hook with retries=0 should fail on first failure"
        );

        // Only one attempt should be made
        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 1, "Hook should only be attempted once");
    }

    #[tokio::test]
    async fn test_hook_retry_with_continue_on_failure() {
        // Hook that fails all retries but has continueOnFailure=true
        let backend = Arc::new(MockHookBackend::new().with_fail_n_times("optional", 10));

        let mut hook = HookConfig::test_fixture(
            "optional-hook",
            HookStage::PostDeploy,
            HookLocation::Local,
            "optional check",
        );
        hook.retries = 2;
        hook.retry_delay = 0;
        hook.continue_on_failure = true;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PostDeploy, None, &env, None)
            .await
            .unwrap();

        // Should return result even though hook failed
        assert_eq!(results.len(), 1);
        assert!(!results[0].success, "Hook should report failure");

        // Should have tried all attempts before continuing
        let executed = backend.get_executed_hooks();
        assert_eq!(executed.len(), 3, "Hook should have been attempted 3 times");
    }

    #[tokio::test]
    async fn test_hook_retry_first_attempt_succeeds() {
        // Hook that succeeds on first attempt should not retry
        let backend = Arc::new(MockHookBackend::new());

        let mut hook = HookConfig::test_fixture(
            "success-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "successful command",
        );
        hook.retries = 5;
        hook.retry_delay = 0;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let results = executor
            .execute_hooks_for_stage("test-host", HookStage::PreDeploy, None, &env, None)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success);

        // Should only be attempted once
        let executed = backend.get_executed_hooks();
        assert_eq!(
            executed.len(),
            1,
            "Hook should only be attempted once on success"
        );
    }

    #[tokio::test]
    async fn test_hook_retry_remote_succeeds_after_failures() {
        // Remote hook that fails twice then succeeds
        let backend = Arc::new(MockHookBackend::new().with_fail_n_times("remote-check", 2));

        let mut hook = HookConfig::test_fixture(
            "remote-retry",
            HookStage::PostReboot,
            HookLocation::Remote,
            "remote-check service",
        );
        hook.retries = 3;
        hook.retry_delay = 0;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);
        let target = DeployTarget {
            hostname: "test-host".to_string(),
            user: Some("deploy".to_string()),
            port: None,
        };

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let results = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PostReboot,
                Some(&target),
                &env,
                None,
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].success, "Remote hook should eventually succeed");

        let executed = backend.get_executed_hooks();
        assert_eq!(
            executed.len(),
            3,
            "Remote hook should have been attempted 3 times"
        );
        assert_eq!(executed[0].2, HookLocation::Remote);
    }

    #[tokio::test]
    async fn test_hook_retry_remote_exhausted_attempts() {
        // Remote hook that fails all retry attempts
        let backend = Arc::new(MockHookBackend::new().with_fail_n_times("always-fails-remote", 10));

        let mut hook = HookConfig::test_fixture(
            "remote-retry",
            HookStage::PostReboot,
            HookLocation::Remote,
            "always-fails-remote check",
        );
        hook.retries = 2;
        hook.retry_delay = 0;

        let mut hooks_by_host = HashMap::new();
        hooks_by_host.insert("test-host".to_string(), vec![hook]);

        let executor = HookExecutor::new(backend.clone(), hooks_by_host);
        let target = DeployTarget {
            hostname: "test-host".to_string(),
            user: Some("deploy".to_string()),
            port: None,
        };

        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), "test-host".to_string());

        let result = executor
            .execute_hooks_for_stage(
                "test-host",
                HookStage::PostReboot,
                Some(&target),
                &env,
                None,
            )
            .await;

        assert!(
            result.is_err(),
            "Remote hook should fail after all retries exhausted"
        );

        let executed = backend.get_executed_hooks();
        assert_eq!(
            executed.len(),
            3,
            "Remote hook should have been attempted 3 times"
        );
        assert_eq!(executed[0].2, HookLocation::Remote);
    }
}
