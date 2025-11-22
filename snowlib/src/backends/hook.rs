//! Hook execution backend for lifecycle hooks
//!
//! # Timeout Implementation
//!
//! Timeouts are implemented using spawned threads and `recv_timeout()`. This approach
//! has limitations:
//! - Timed-out processes cannot be killed from safe Rust
//! - The spawned thread continues running until the command completes
//! - The orphaned shell process runs unmonitored in the background
//! - In pathological cases (many slow hooks), thread resources could be exhausted
//!
//! This is a known limitation. Alternative approaches (like tokio::time::timeout) would
//! require the entire call stack to be async, which is not currently the case.
//!
//! When a timeout occurs, a warning is logged and the hook is marked as failed, but the
//! underlying process continues executing until completion.

use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::nix::HookConfig;
use crate::process::{OutputMode, StreamingExecutor};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Global counter for orphaned threads from timed-out hooks
/// This helps track resource leaks and provides visibility into hook timeout frequency
static ORPHANED_HOOK_THREADS: AtomicUsize = AtomicUsize::new(0);

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

        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(command);

        for (key, value) in env {
            cmd.env(key, value);
        }

        // Execute command in a thread to enable timeout
        let (tx, rx) = mpsc::channel();
        let output_mode = self.output_mode;

        let handle = std::thread::spawn(move || {
            let result = StreamingExecutor::new(&mut cmd)
                .output_mode(output_mode)
                .execute();
            let _ = tx.send(result);
        });

        // Wait for completion or timeout
        let output = match rx.recv_timeout(Duration::from_secs(timeout_secs)) {
            Ok(Ok(output)) => Some(output),
            Ok(Err(e)) => {
                let duration = start.elapsed();
                let _ = handle.join();
                return Ok(HookResult::failure(
                    None,
                    String::new(),
                    format!("Failed to execute hook: {}", e),
                    duration,
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // WARNING: Thread continues running in background - cannot be killed from safe Rust
                // The spawned shell process will eventually complete but is no longer monitored
                let orphaned_count = ORPHANED_HOOK_THREADS.fetch_add(1, Ordering::Relaxed) + 1;

                tracing::warn!(
                    target: "hooks",
                    "Hook timed out after {} seconds - process continues in background (total orphaned: {})",
                    timeout_secs,
                    orphaned_count
                );

                None
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let duration = start.elapsed();
                let _ = handle.join();
                return Ok(HookResult::failure(
                    None,
                    String::new(),
                    "Hook execution thread disconnected unexpectedly".to_string(),
                    duration,
                ));
            }
        };

        // Only join the thread if we got output (meaning it completed)
        // On timeout, we skip joining to avoid blocking - the thread continues in background
        if output.is_some() {
            let _ = handle.join();
        }

        let duration = start.elapsed();

        // Handle timeout case
        let output = match output {
            Some(output) => output,
            None => {
                return Ok(HookResult::failure(
                    None,
                    String::new(),
                    format!("Hook timed out after {} seconds", timeout_secs),
                    duration,
                ));
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(HookResult::success(stdout, stderr, duration))
        } else {
            Ok(HookResult::failure(
                output.status.code(),
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

        // Build SSH command with environment variables
        let env_vars: Vec<String> = env
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    shell_escape::escape(k.into()),
                    shell_escape::escape(v.into())
                )
            })
            .collect();

        // Security: hook.command is from NixOS config (trusted, declarative source).
        // Environment variables are escaped because they come from runtime deployment context.
        let remote_command = if env_vars.is_empty() {
            hook.command.clone()
        } else {
            format!("{} {}", env_vars.join(" "), hook.command)
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
    use crate::nix::{HookLocation, HookStage};

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

    #[test]
    fn test_execute_local_success() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig {
            stage: HookStage::PreActivate,
            location: HookLocation::Local,
            command: "echo 'test'".to_string(),
            timeout: 300,
            continue_on_failure: false,
        };
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("test"));
    }

    #[test]
    fn test_execute_local_failure() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig {
            stage: HookStage::PreActivate,
            location: HookLocation::Local,
            command: "exit 1".to_string(),
            timeout: 300,
            continue_on_failure: false,
        };
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
    }

    #[test]
    fn test_execute_local_with_env() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig {
            stage: HookStage::PreActivate,
            location: HookLocation::Local,
            command: "echo $TEST_VAR".to_string(),
            timeout: 300,
            continue_on_failure: false,
        };
        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "test_value".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("test_value"));
    }

    #[test]
    fn test_execute_local_timeout() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig {
            stage: HookStage::PreActivate,
            location: HookLocation::Local,
            command: "sleep 10".to_string(),
            timeout: 1, // 1 second timeout for a 10 second sleep
            continue_on_failure: false,
        };
        let env = HashMap::new();

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(!result.success);
        assert!(result.stderr.contains("timed out"));
        assert!(result.duration.as_secs() >= 1);
        // Note: Duration may be slightly longer than timeout due to cleanup attempts
        // The important thing is that it returns (doesn't hang for 10 seconds)
        assert!(result.duration.as_secs() < 5);
    }

    #[test]
    fn test_execute_local_escapes_env_vars_with_shell_metacharacters() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig {
            stage: HookStage::PreActivate,
            location: HookLocation::Local,
            command: "echo \"VAR=$MALICIOUS\"".to_string(),
            timeout: 300,
            continue_on_failure: false,
        };
        let mut env = HashMap::new();
        // Attempt command injection via semicolon
        env.insert(
            "MALICIOUS".to_string(),
            "value'; rm -rf /tmp/test; echo 'pwned".to_string(),
        );

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        // The malicious command should be printed as a literal string, not executed
        assert!(result
            .stdout
            .contains("value'; rm -rf /tmp/test; echo 'pwned"));
        // Verify the injection did NOT execute by checking we don't see "pwned" on its own line
        assert!(!result.stdout.lines().any(|line| line.trim() == "pwned"));
    }

    #[test]
    fn test_execute_local_escapes_env_vars_with_quotes() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig {
            stage: HookStage::PreActivate,
            location: HookLocation::Local,
            command: "echo \"VAR=$TEST\"".to_string(),
            timeout: 300,
            continue_on_failure: false,
        };
        let mut env = HashMap::new();
        // Environment variable with quotes that could break out of string context
        env.insert("TEST".to_string(), "value\" && echo \"injected".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        // The quotes should be part of the value, not shell syntax
        assert!(result.stdout.contains("value\" && echo \"injected"));
    }

    #[test]
    fn test_execute_local_escapes_env_vars_with_pipe() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig {
            stage: HookStage::PreActivate,
            location: HookLocation::Local,
            command: "echo \"VAR=$PIPE_TEST\"".to_string(),
            timeout: 300,
            continue_on_failure: false,
        };
        let mut env = HashMap::new();
        // Attempt to pipe to another command
        env.insert("PIPE_TEST".to_string(), "data | base64".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        // The pipe should be literal, not execute base64
        assert!(result.stdout.contains("data | base64"));
        // Verify base64 encoding did NOT happen (output is not base64)
        assert!(!result.stdout.contains("ZGF0YQ=="));
    }

    #[test]
    fn test_execute_local_escapes_env_vars_with_backticks() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig {
            stage: HookStage::PreActivate,
            location: HookLocation::Local,
            command: "echo \"VAR=$BACKTICK\"".to_string(),
            timeout: 300,
            continue_on_failure: false,
        };
        let mut env = HashMap::new();
        // Attempt command substitution with backticks
        env.insert("BACKTICK".to_string(), "`whoami`".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        // Backticks should be literal, not execute whoami
        assert!(result.stdout.contains("`whoami`"));
        // The output should contain the literal backticks, not the result of whoami
        let current_user = std::env::var("USER").unwrap_or_default();
        if !current_user.is_empty() {
            // If backticks were executed, we'd see just the username
            // Since they're escaped, we see the literal "`whoami`"
            assert!(!result.stdout.trim().ends_with(&current_user));
        }
    }

    #[test]
    fn test_execute_local_escapes_env_vars_with_dollar_paren() {
        let backend = SystemHookBackend::new(None, OutputMode::Capture);
        let hook = HookConfig {
            stage: HookStage::PreActivate,
            location: HookLocation::Local,
            command: "echo \"VAR=$SUBSHELL\"".to_string(),
            timeout: 300,
            continue_on_failure: false,
        };
        let mut env = HashMap::new();
        // Attempt command substitution with $()
        env.insert("SUBSHELL".to_string(), "$(echo injected)".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        // The $() should be literal, not execute the subshell
        assert!(result.stdout.contains("$(echo injected)"));
    }
}
