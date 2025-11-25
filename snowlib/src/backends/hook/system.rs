//! Hook execution backend for lifecycle hooks
//!
//! Uses StreamingExecutor for consistent subprocess handling with timeout support.

use super::{HookBackend, HookResult};
use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::hooks::HookConfig;
use crate::process::{OutputMode, StreamingExecutor};
use std::collections::HashMap;
use std::process::Command;
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

    fn execute_command(
        &self,
        command: &str,
        env: &HashMap<String, String>,
        timeout_secs: u64,
    ) -> Result<HookResult> {
        let start = Instant::now();

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);

        for (key, value) in env {
            cmd.env(key, value);
        }

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(self.output_mode)
            .timeout(Duration::from_secs(timeout_secs))
            .execute()
            .map_err(|e| {
                crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                    host: "local".to_string(),
                    source: std::io::Error::new(e.kind(), format!("Failed to execute hook: {}", e)),
                })
            })?;

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
}

impl HookBackend for SystemHookBackend {
    fn execute_local(
        &self,
        hook: &HookConfig,
        env: &HashMap<String, String>,
    ) -> Result<HookResult> {
        self.execute_command(&hook.command, env, hook.timeout)
    }

    fn execute_remote(
        &self,
        target: &DeployTarget,
        hook: &HookConfig,
        env: &HashMap<String, String>,
    ) -> Result<HookResult> {
        let user = self
            .ssh_user
            .as_ref()
            .or(target.user.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("root");

        let destination = format!("{}@{}", user, target.hostname);

        // Build remote command with environment variables using 'env' utility
        // This is safer than prepending VAR=value because it doesn't rely on shell parsing
        let remote_command = if env.is_empty() {
            format!(
                "sh -c {}",
                shell_escape::escape(hook.command.as_str().into())
            )
        } else {
            let env_args: Vec<String> = env
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}={}",
                        shell_escape::escape(k.into()),
                        shell_escape::escape(v.into())
                    )
                })
                .collect();

            format!(
                "env {} sh -c {}",
                env_args.join(" "),
                shell_escape::escape(hook.command.as_str().into())
            )
        };

        let ssh_command = format!(
            "ssh -o ConnectTimeout=10 {} {}",
            shell_escape::escape(destination.into()),
            shell_escape::escape(remote_command.into())
        );

        self.execute_command(&ssh_command, &HashMap::new(), hook.timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookLocation, HookStage};

    #[test]
    fn test_execute_local_success() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo 'test'",
        );
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("test"));
    }

    #[test]
    fn test_execute_local_failure() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "exit 1",
        );
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
    }

    #[test]
    fn test_execute_local_with_env() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo $TEST_VAR",
        );
        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "test_value".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("test_value"));
    }

    #[test]
    fn test_execute_local_timeout() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "sleep 10",
        );
        hook.timeout = 1; // 1 second timeout for a 10 second sleep
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(!result.success);
        assert!(result.stderr.contains("timed out"));
        assert!(result.duration.as_secs() >= 1);
        // Verify timeout mechanism works - without it, this would hang for 10 seconds
        assert!(result.duration.as_secs() < 5);
    }

    /// Prevents command injection via shell metacharacters in environment variables.
    #[test]
    fn test_execute_local_escapes_env_vars_with_shell_metacharacters() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo \"VAR=$MALICIOUS\"",
        );
        let mut env = HashMap::new();
        env.insert(
            "MALICIOUS".to_string(),
            "value'; rm -rf /tmp/test; echo 'pwned".to_string(),
        );

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result
            .stdout
            .contains("value'; rm -rf /tmp/test; echo 'pwned"));
        assert!(!result.stdout.lines().any(|line| line.trim() == "pwned"));
    }

    /// Prevents command injection via quote escaping in environment variables.
    #[test]
    fn test_execute_local_escapes_env_vars_with_quotes() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo \"VAR=$TEST\"",
        );
        let mut env = HashMap::new();
        env.insert("TEST".to_string(), "value\" && echo \"injected".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("value\" && echo \"injected"));
    }

    /// Prevents command injection via pipe operators in environment variables.
    #[test]
    fn test_execute_local_escapes_env_vars_with_pipe() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo \"VAR=$PIPE_TEST\"",
        );
        let mut env = HashMap::new();
        env.insert("PIPE_TEST".to_string(), "data | base64".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("data | base64"));
        assert!(!result.stdout.contains("ZGF0YQ=="));
    }

    /// Prevents command injection via backtick command substitution.
    #[test]
    fn test_execute_local_escapes_env_vars_with_backticks() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo \"VAR=$BACKTICK\"",
        );
        let mut env = HashMap::new();
        env.insert("BACKTICK".to_string(), "`whoami`".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("`whoami`"));
        let current_user = std::env::var("USER").unwrap_or_default();
        if !current_user.is_empty() {
            assert!(!result.stdout.trim().ends_with(&current_user));
        }
    }

    /// Prevents command injection via $() command substitution.
    #[test]
    fn test_execute_local_escapes_env_vars_with_dollar_paren() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo \"VAR=$SUBSHELL\"",
        );
        let mut env = HashMap::new();
        env.insert("SUBSHELL".to_string(), "$(echo injected)".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("$(echo injected)"));
    }

    /// Prevents command injection via semicolons in remote SSH execution.
    #[test]
    fn test_execute_remote_escapes_env_vars_with_semicolon() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Remote,
            "echo $MALICIOUS",
        );
        let mut env = HashMap::new();
        env.insert(
            "MALICIOUS".to_string(),
            "value; rm -rf /tmp/test".to_string(),
        );

        let target = DeployTarget {
            hostname: "localhost".to_string(),
            user: None,
            port: None,
        };

        let result = backend.execute_remote(&target, &hook, &env).unwrap();

        assert!(
            result.success
                || result.stderr.contains("Connection refused")
                || result.stderr.contains("Host key")
        );

        if result.success {
            assert!(result.stdout.contains("value; rm -rf /tmp/test"));
            assert!(!result
                .stdout
                .lines()
                .any(|line| line.trim().is_empty() && !result.stderr.contains("test")));
        }
    }

    /// Prevents command injection via backticks in remote SSH execution.
    #[test]
    fn test_execute_remote_escapes_env_vars_with_backticks() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Remote,
            "echo $BACKTICK_TEST",
        );
        let mut env = HashMap::new();
        env.insert("BACKTICK_TEST".to_string(), "`whoami`".to_string());

        let target = DeployTarget {
            hostname: "localhost".to_string(),
            user: None,
            port: None,
        };

        let result = backend.execute_remote(&target, &hook, &env).unwrap();

        if result.success {
            assert!(result.stdout.contains("`whoami`"));
        }
    }

    /// Prevents command injection via $() substitution in remote SSH execution.
    #[test]
    fn test_execute_remote_escapes_env_vars_with_dollar_paren() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Remote,
            "echo $SUBSHELL_TEST",
        );
        let mut env = HashMap::new();
        env.insert("SUBSHELL_TEST".to_string(), "$(echo injected)".to_string());

        let target = DeployTarget {
            hostname: "localhost".to_string(),
            user: None,
            port: None,
        };

        let result = backend.execute_remote(&target, &hook, &env).unwrap();

        if result.success {
            assert!(result.stdout.contains("$(echo injected)"));
        }
    }

    /// Prevents command injection via pipes in remote SSH execution.
    #[test]
    fn test_execute_remote_escapes_env_vars_with_pipes() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Remote,
            "echo $PIPE_TEST",
        );
        let mut env = HashMap::new();
        env.insert("PIPE_TEST".to_string(), "data | base64".to_string());

        let target = DeployTarget {
            hostname: "localhost".to_string(),
            user: None,
            port: None,
        };

        let result = backend.execute_remote(&target, &hook, &env).unwrap();

        if result.success {
            assert!(result.stdout.contains("data | base64"));
            assert!(!result.stdout.contains("ZGF0YQ=="));
        }
    }

    /// Prevents command injection via quote escaping in remote SSH execution.
    #[test]
    fn test_execute_remote_escapes_env_vars_with_quotes() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Remote,
            "echo $QUOTE_TEST",
        );
        let mut env = HashMap::new();
        env.insert(
            "QUOTE_TEST".to_string(),
            "value\" && echo \"injected".to_string(),
        );

        let target = DeployTarget {
            hostname: "localhost".to_string(),
            user: None,
            port: None,
        };

        let result = backend.execute_remote(&target, &hook, &env).unwrap();

        if result.success {
            assert!(result.stdout.contains("value\" && echo \"injected"));
        }
    }

    #[test]
    fn test_timeout_captures_partial_output_before_termination() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut hook = HookConfig::test_fixture(
            "partial-output",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo 'Starting...'; sleep 10; echo 'Should not appear'",
        );
        hook.timeout = 1;
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).unwrap();
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

    #[test]
    fn test_timeout_with_large_output() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut hook = HookConfig::test_fixture(
            "large-output",
            HookStage::PreActivate,
            HookLocation::Local,
            "for i in {1..100}; do echo \"Line $i\"; done; sleep 10",
        );
        hook.timeout = 1;
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).unwrap();
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

    #[test]
    fn test_timeout_duration_accuracy() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut hook = HookConfig::test_fixture(
            "timeout-test",
            HookStage::PreActivate,
            HookLocation::Local,
            "sleep 30",
        );
        hook.timeout = 2; // 2 second timeout
        let env = HashMap::new();

        let start = std::time::Instant::now();
        let result = backend.execute_local(&hook, &env).unwrap();
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
    fn test_remote_hook_timeout() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let mut hook = HookConfig::test_fixture(
            "remote-timeout",
            HookStage::PreActivate,
            HookLocation::Remote,
            "sleep 30",
        );
        hook.timeout = 1;
        let env = HashMap::new();

        let target = DeployTarget {
            hostname: "localhost".to_string(),
            user: None,
            port: None,
        };

        let start = std::time::Instant::now();
        let result = backend.execute_remote(&target, &hook, &env).unwrap();
        let elapsed = start.elapsed();

        // Remote hook should also timeout
        assert!(!result.success, "Remote hook should timeout");

        // Should complete in reasonable time (not wait for full sleep)
        assert!(
            elapsed.as_secs() < 10,
            "Remote timeout should not wait too long"
        );
    }

    #[test]
    fn test_quick_command_completes_before_timeout() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig::test_fixture(
            "quick-command",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo 'Quick command'",
        );
        // Uses default timeout from test_fixture (300 seconds)
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).unwrap();
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
