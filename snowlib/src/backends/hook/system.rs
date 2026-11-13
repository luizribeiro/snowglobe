//! Hook execution backend for lifecycle hooks
//!
//! Uses async run_command for subprocess handling with timeout support.

use super::{HookBackend, HookResult};
use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::hooks::HookConfig;
use crate::process::r#async::run_command;
use crate::process::OutputMode;
use crate::shell;
use crate::ssh_options::{ssh_options, Keepalive, CONNECT_TIMEOUT_SECS};
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Real implementation of HookBackend that executes hooks via shell
pub struct SystemHookBackend {
    ssh_user: Option<String>,
    output_mode: OutputMode,
}

impl SystemHookBackend {
    pub fn new(ssh_user: Option<String>, output_mode: OutputMode) -> Self {
        Self {
            ssh_user,
            output_mode,
        }
    }

    async fn execute_command(
        &self,
        command: &str,
        env: &HashMap<String, String>,
        timeout_secs: u64,
    ) -> Result<HookResult> {
        let start = Instant::now();

        let env_pairs: Vec<(&str, &str)> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        let result = run_command(
            "sh",
            &["-c", command],
            self.output_mode,
            Some(Duration::from_secs(timeout_secs)),
            None,
            &env_pairs,
        )
        .await
        .map_err(|source| crate::error::IoError::HookStart { source })?;

        let duration = start.elapsed();
        let stdout = String::from_utf8_lossy(&result.stdout).to_string();
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();

        if result.timed_out {
            Ok(HookResult::failure(
                None,
                stdout,
                format!("Hook timed out after {} seconds", timeout_secs),
                duration,
            ))
        } else if result.status.success() {
            Ok(HookResult::success(stdout, stderr, duration))
        } else {
            Ok(HookResult::failure(
                result.status.code(),
                stdout,
                stderr,
                duration,
            ))
        }
    }

    /// Connects as `snow deploy` does, so hooks need no SSH key of their own. Hooks may
    /// restart the host's network and are already bounded by their timeout.
    fn ssh_shell_command(&self, target: &DeployTarget, remote_command: &str) -> String {
        let options = ssh_options(CONNECT_TIMEOUT_SECS, Keepalive::Never, target.port);
        let destination = target.ssh_destination_as(self.ssh_user.as_deref());
        shell::join(
            std::iter::once("ssh")
                .chain(options.iter().map(String::as_str))
                .chain([destination.as_str(), remote_command]),
        )
    }
}

#[async_trait]
impl HookBackend for SystemHookBackend {
    async fn execute_local(
        &self,
        hook: &HookConfig,
        env: &HashMap<String, String>,
    ) -> Result<HookResult> {
        self.execute_command(&hook.command, env, hook.timeout).await
    }

    async fn execute_remote(
        &self,
        target: &DeployTarget,
        hook: &HookConfig,
        env: &HashMap<String, String>,
    ) -> Result<HookResult> {
        let remote_command = build_remote_command(&hook.command, env);
        let ssh_command = self.ssh_shell_command(target, &remote_command);

        self.execute_command(&ssh_command, &HashMap::new(), hook.timeout)
            .await
    }
}

/// Build a command string with escaped environment variables for remote execution.
///
/// Uses the `env` utility to set variables, which is safer than shell variable assignment.
/// All values are escaped to prevent command injection.
fn build_remote_command(command: &str, env: &HashMap<String, String>) -> String {
    let sh_command = shell::join(["sh", "-c", command]);
    if env.is_empty() {
        sh_command
    } else {
        let env_args: Vec<String> = env
            .iter()
            .map(|(k, v)| format!("{}={}", shell::quote(k), shell::quote(v)))
            .collect();

        format!("env {} {sh_command}", env_args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookLocation, HookStage};

    #[tokio::test]
    async fn test_execute_local_success() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo 'test'",
        );
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("test"));
    }

    #[tokio::test]
    async fn test_execute_local_failure() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "exit 1",
        );
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
    }

    #[tokio::test]
    async fn test_execute_local_with_env() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo $TEST_VAR",
        );
        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "test_value".to_string());

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("test_value"));
    }

    #[tokio::test]
    async fn test_execute_local_timeout() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "sleep 10",
        );
        hook.timeout = 1; // 1 second timeout for a 10 second sleep
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(!result.success);
        assert!(result.stderr.contains("timed out"));
        assert!(result.duration.as_secs() >= 1);
        // Verify timeout mechanism works - without it, this would hang for 10 seconds
        assert!(result.duration.as_secs() < 5);
    }

    /// Prevents command injection via shell metacharacters in environment variables.
    #[tokio::test]
    async fn test_execute_local_escapes_env_vars_with_shell_metacharacters() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo \"VAR=$MALICIOUS\"",
        );
        let mut env = HashMap::new();
        env.insert(
            "MALICIOUS".to_string(),
            "value'; rm -rf /tmp/test; echo 'pwned".to_string(),
        );

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(result.success);
        assert!(result
            .stdout
            .contains("value'; rm -rf /tmp/test; echo 'pwned"));
        assert!(!result.stdout.lines().any(|line| line.trim() == "pwned"));
    }

    /// Prevents command injection via quote escaping in environment variables.
    #[tokio::test]
    async fn test_execute_local_escapes_env_vars_with_quotes() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo \"VAR=$TEST\"",
        );
        let mut env = HashMap::new();
        env.insert("TEST".to_string(), "value\" && echo \"injected".to_string());

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("value\" && echo \"injected"));
    }

    /// Prevents command injection via pipe operators in environment variables.
    #[tokio::test]
    async fn test_execute_local_escapes_env_vars_with_pipe() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo \"VAR=$PIPE_TEST\"",
        );
        let mut env = HashMap::new();
        env.insert("PIPE_TEST".to_string(), "data | base64".to_string());

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("data | base64"));
        assert!(!result.stdout.contains("ZGF0YQ=="));
    }

    /// Prevents command injection via backtick command substitution.
    #[tokio::test]
    async fn test_execute_local_escapes_env_vars_with_backticks() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo \"VAR=$BACKTICK\"",
        );
        let mut env = HashMap::new();
        env.insert("BACKTICK".to_string(), "`whoami`".to_string());

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("`whoami`"));
        let current_user = std::env::var("USER").unwrap_or_default();
        if !current_user.is_empty() {
            assert!(!result.stdout.trim().ends_with(&current_user));
        }
    }

    /// Prevents command injection via $() command substitution.
    #[tokio::test]
    async fn test_execute_local_escapes_env_vars_with_dollar_paren() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo \"VAR=$SUBSHELL\"",
        );
        let mut env = HashMap::new();
        env.insert("SUBSHELL".to_string(), "$(echo injected)".to_string());

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("$(echo injected)"));
    }

    /// Prevents command injection via backticks in environment variables.
    #[tokio::test]
    async fn test_escaping_prevents_backtick_injection() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut env = HashMap::new();
        env.insert("BACKTICK_TEST".to_string(), "`whoami`".to_string());

        // Build the command as it would be for remote execution, then run locally
        let command = build_remote_command("echo $BACKTICK_TEST", &env);
        let result = backend
            .execute_command(&command, &HashMap::new(), 10)
            .await
            .unwrap();

        assert!(result.success);
        // The backticks should be printed literally, not executed
        assert!(result.stdout.contains("`whoami`"));
    }

    /// Prevents command injection via $() substitution in environment variables.
    #[tokio::test]
    async fn test_escaping_prevents_dollar_paren_injection() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut env = HashMap::new();
        env.insert("SUBSHELL_TEST".to_string(), "$(echo injected)".to_string());

        let command = build_remote_command("echo $SUBSHELL_TEST", &env);
        let result = backend
            .execute_command(&command, &HashMap::new(), 10)
            .await
            .unwrap();

        assert!(result.success);
        // The $() should be printed literally, not executed
        assert!(result.stdout.contains("$(echo injected)"));
    }

    /// Prevents command injection via pipes in environment variables.
    #[tokio::test]
    async fn test_escaping_prevents_pipe_injection() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut env = HashMap::new();
        env.insert("PIPE_TEST".to_string(), "data | base64".to_string());

        let command = build_remote_command("echo $PIPE_TEST", &env);
        let result = backend
            .execute_command(&command, &HashMap::new(), 10)
            .await
            .unwrap();

        assert!(result.success);
        // The pipe should be printed literally, not executed
        assert!(result.stdout.contains("data | base64"));
        // If pipe was executed, we'd see base64 output
        assert!(!result.stdout.contains("ZGF0YQ=="));
    }

    /// Prevents command injection via quote escaping in environment variables.
    #[tokio::test]
    async fn test_escaping_prevents_quote_injection() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut env = HashMap::new();
        env.insert(
            "QUOTE_TEST".to_string(),
            "value\" && echo \"injected".to_string(),
        );

        let command = build_remote_command("echo $QUOTE_TEST", &env);
        let result = backend
            .execute_command(&command, &HashMap::new(), 10)
            .await
            .unwrap();

        assert!(result.success);
        // The quotes should be printed literally, not break out of the string
        assert!(result.stdout.contains("value\" && echo \"injected"));
    }

    #[tokio::test]
    async fn test_timeout_captures_partial_output_before_termination() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut hook = HookConfig::test_fixture(
            "partial-output",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo 'Starting...'; sleep 10; echo 'Should not appear'",
        );
        hook.timeout = 1;
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(!result.success, "Hook should timeout and fail");
        assert!(
            result.stdout.contains("Starting..."),
            "Output before timeout should be captured"
        );
        assert!(
            !result.stdout.contains("Should not appear"),
            "Output after timeout should not appear"
        );
        assert!(result.stderr.contains("timed out"));
    }

    #[tokio::test]
    async fn test_timeout_with_large_output() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut hook = HookConfig::test_fixture(
            "large-output",
            HookStage::PreDeploy,
            HookLocation::Local,
            "for i in {1..100}; do echo \"Line $i\"; done; sleep 10",
        );
        hook.timeout = 1;
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(!result.success, "Hook should timeout");

        // Verify we captured at least some of the output
        let line_count = result.stdout.lines().count();
        assert!(
            line_count > 0,
            "Should have captured some output before timeout"
        );
        assert!(
            line_count <= 100,
            "Should not have all 100 lines (timed out first)"
        );
    }

    #[tokio::test]
    async fn test_timeout_duration_accuracy() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut hook = HookConfig::test_fixture(
            "timeout-test",
            HookStage::PreDeploy,
            HookLocation::Local,
            "sleep 30",
        );
        hook.timeout = 2; // 2 second timeout
        let env = HashMap::new();

        let start = std::time::Instant::now();
        let result = backend.execute_local(&hook, &env).await.unwrap();
        let elapsed = start.elapsed();

        assert!(!result.success);
        assert!(result.stderr.contains("timed out"));

        // Timeout should happen around 2 seconds, allow some margin
        // Should be at least 2 seconds (the timeout)
        assert!(
            elapsed.as_secs() >= 2,
            "Should wait at least timeout duration"
        );
        // Should be less than 10 seconds (grace period + reap timeout)
        // SIGTERM (2s) + grace (5s) + reap (2s) = 9s max
        assert!(
            elapsed.as_secs() < 10,
            "Should not wait too long after timeout"
        );
    }

    #[test]
    fn test_ssh_shell_command_never_prompts_and_bounds_the_connection() {
        let backend = SystemHookBackend::new(Some("deploy".to_string()), OutputMode::Capture);
        let command = backend.ssh_shell_command(
            &DeployTarget::new("condor.example.com"),
            "kubectl uncordon condor",
        );

        assert_eq!(
            command,
            "ssh -o BatchMode=yes -o ConnectTimeout=10 -o ForwardAgent=no 'deploy@condor.example.com' 'kubectl uncordon condor'"
        );
    }

    #[test]
    fn test_ssh_shell_command_connects_as_the_targets_own_user() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let command = backend.ssh_shell_command(
            &DeployTarget::with_user("pusher", "condor.example.com"),
            "kubectl uncordon condor",
        );

        assert!(
            command.ends_with(" 'pusher@condor.example.com' 'kubectl uncordon condor'"),
            "{command}"
        );
    }

    #[test]
    fn test_ssh_shell_command_connects_to_the_hosts_port() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut target = DeployTarget::with_user("pusher", "condor.example.com");
        target.port = Some(2222);
        let command = backend.ssh_shell_command(&target, "kubectl uncordon condor");

        assert!(
            command.contains("-o ForwardAgent=no -p 2222 'pusher@condor.example.com'"),
            "{command}"
        );
    }

    #[test]
    fn test_ssh_shell_command_without_a_user_leaves_it_to_ssh_as_deploy_does() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let command = backend.ssh_shell_command(
            &DeployTarget::new("condor.example.com"),
            "kubectl uncordon condor",
        );

        assert!(
            command.ends_with(" condor.example.com 'kubectl uncordon condor'"),
            "{command}"
        );
    }

    #[tokio::test]
    async fn test_remote_command_format_timeout() {
        // Test that commands built for remote execution also timeout properly
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let env = HashMap::new();

        // Build command as it would be for remote execution
        let command = build_remote_command("sleep 30", &env);

        let start = std::time::Instant::now();
        let result = backend
            .execute_command(&command, &HashMap::new(), 1)
            .await
            .unwrap();
        let elapsed = start.elapsed();

        assert!(!result.success, "Command should timeout");
        assert!(
            elapsed.as_secs() < 10,
            "Should not wait too long after timeout"
        );
    }

    #[tokio::test]
    async fn test_quick_command_completes_before_timeout() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "quick-command",
            HookStage::PreDeploy,
            HookLocation::Local,
            "echo 'Quick command'",
        );
        // Uses default timeout from test_fixture (300 seconds)
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).await.unwrap();
        assert!(
            result.success,
            "Quick command should complete before timeout"
        );
        assert!(result.stdout.contains("Quick command"));
        assert!(
            result.duration.as_secs() < 1,
            "Should complete very quickly"
        );
    }
}
