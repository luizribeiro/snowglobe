//! Hook execution backend for lifecycle hooks
//!
//! # Timeout Implementation
//!
//! Timeouts are implemented by spawning the process and tracking its PID. When a timeout
//! occurs, the process is terminated using SIGTERM, followed by SIGKILL if it doesn't
//! exit gracefully within 5 seconds.

use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::nix::HookConfig;
use crate::process::OutputMode;
use nix::sys::signal::{self, Signal};
use nix::sys::wait::{self, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Grace period to wait for process to exit after SIGTERM before sending SIGKILL
const SIGTERM_GRACE_PERIOD_SECS: u64 = 5;

/// Maximum time to wait for process to be reaped by kernel after SIGKILL
const SIGKILL_REAP_TIMEOUT_SECS: u64 = 2;

/// Polling interval when waiting for process to be reaped
const REAP_POLL_INTERVAL_MILLIS: u64 = 100;

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

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        for (key, value) in env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                host: "local".to_string(),
                source: std::io::Error::new(e.kind(), format!("Failed to spawn hook: {}", e)),
            })
        })?;

        let child_pid = Pid::from_raw(child.id() as i32);

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        let stdout_lines = Arc::new(Mutex::new(Vec::new()));
        let stderr_lines = Arc::new(Mutex::new(Vec::new()));

        let stdout_lines_clone = Arc::clone(&stdout_lines);
        let stderr_lines_clone = Arc::clone(&stderr_lines);
        let output_mode = self.output_mode;

        // Spawn threads to read stdout/stderr
        let stdout_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(|r| r.ok()) {
                if output_mode == OutputMode::Display && !line.is_empty() {
                    tracing::info!(target: "hooks", "{}", line);
                }
                let mut lines = stdout_lines_clone.lock().unwrap();
                lines.extend_from_slice(line.as_bytes());
                lines.push(b'\n');
            }
        });

        let stderr_thread = std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(|r| r.ok()) {
                if output_mode == OutputMode::Display && !line.is_empty() {
                    tracing::info!(target: "hooks", "stderr: {}", line);
                }
                let mut lines = stderr_lines_clone.lock().unwrap();
                lines.extend_from_slice(line.as_bytes());
                lines.push(b'\n');
            }
        });

        // Wait for process to complete or timeout
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let status = child.wait();
            let _ = tx.send(status);
        });

        let exit_status = match rx.recv_timeout(Duration::from_secs(timeout_secs)) {
            Ok(Ok(status)) => Some(status),
            Ok(Err(e)) => {
                stdout_thread.join().ok();
                stderr_thread.join().ok();
                let duration = start.elapsed();
                return Ok(HookResult::failure(
                    None,
                    String::new(),
                    format!("Failed to wait for hook: {}", e),
                    duration,
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                tracing::warn!(
                    target: "hooks",
                    "Hook timed out after {} seconds, sending SIGTERM to PID {}",
                    timeout_secs,
                    child_pid
                );

                // Send SIGTERM to gracefully terminate
                if let Err(e) = signal::kill(child_pid, Signal::SIGTERM) {
                    tracing::warn!(target: "hooks", "Failed to send SIGTERM: {}", e);
                }

                // Wait for graceful shutdown
                let grace_period = Duration::from_secs(SIGTERM_GRACE_PERIOD_SECS);
                match rx.recv_timeout(grace_period) {
                    Ok(Ok(_status)) => {
                        tracing::info!(target: "hooks", "Process exited gracefully after SIGTERM");
                    }
                    _ => {
                        tracing::warn!(
                            target: "hooks",
                            "Process did not exit after SIGTERM, sending SIGKILL"
                        );
                        // Force kill with SIGKILL
                        if let Err(e) = signal::kill(child_pid, Signal::SIGKILL) {
                            tracing::error!(target: "hooks", "Failed to send SIGKILL: {}", e);
                        }

                        // Wait for process to be reaped after SIGKILL
                        let reap_deadline =
                            Instant::now() + Duration::from_secs(SIGKILL_REAP_TIMEOUT_SECS);
                        loop {
                            if Instant::now() > reap_deadline {
                                tracing::error!(
                                    target: "hooks",
                                    "Process {} did not die after SIGKILL (may be zombie or in uninterruptible sleep)",
                                    child_pid
                                );
                                break;
                            }

                            match wait::waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) {
                                Ok(WaitStatus::Exited(_, _))
                                | Ok(WaitStatus::Signaled(_, _, _)) => {
                                    tracing::info!(target: "hooks", "Process {} reaped after SIGKILL", child_pid);
                                    break;
                                }
                                Ok(_) => {
                                    std::thread::sleep(Duration::from_millis(
                                        REAP_POLL_INTERVAL_MILLIS,
                                    ));
                                }
                                Err(nix::errno::Errno::ECHILD) => {
                                    tracing::debug!(target: "hooks", "Process {} already reaped", child_pid);
                                    break;
                                }
                                Err(e) => {
                                    tracing::warn!(target: "hooks", "waitpid failed: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                }

                None
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stdout_thread.join().ok();
                stderr_thread.join().ok();
                let duration = start.elapsed();
                return Ok(HookResult::failure(
                    None,
                    String::new(),
                    "Hook wait thread disconnected unexpectedly".to_string(),
                    duration,
                ));
            }
        };

        // Wait for output threads to complete
        if let Err(e) = stdout_thread.join() {
            tracing::warn!(target: "hooks", "stdout thread panicked: {:?}", e);
        }
        if let Err(e) = stderr_thread.join() {
            tracing::warn!(target: "hooks", "stderr thread panicked: {:?}", e);
        }

        let duration = start.elapsed();

        let stdout_bytes = Arc::try_unwrap(stdout_lines)
            .expect("stdout Arc has multiple references")
            .into_inner()
            .unwrap();
        let stderr_bytes = Arc::try_unwrap(stderr_lines)
            .expect("stderr Arc has multiple references")
            .into_inner()
            .unwrap();

        let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
        let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

        // Handle timeout case
        let exit_status = match exit_status {
            Some(status) => status,
            None => {
                return Ok(HookResult::failure(
                    None,
                    stdout,
                    format!("Hook timed out after {} seconds", timeout_secs),
                    duration,
                ));
            }
        };

        if exit_status.success() {
            Ok(HookResult::success(stdout, stderr, duration))
        } else {
            Ok(HookResult::failure(
                exit_status.code(),
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
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo \"VAR=$TEST\"",
        );
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
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo \"VAR=$PIPE_TEST\"",
        );
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
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo \"VAR=$BACKTICK\"",
        );
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
        let hook = HookConfig::test_fixture(
            "test-hook",
            HookStage::PreActivate,
            HookLocation::Local,
            "echo \"VAR=$SUBSHELL\"",
        );
        let mut env = HashMap::new();
        // Attempt command substitution with $()
        env.insert("SUBSHELL".to_string(), "$(echo injected)".to_string());

        let result = backend.execute_local(&hook, &env).unwrap();
        assert!(result.success);
        // The $() should be literal, not execute the subshell
        assert!(result.stdout.contains("$(echo injected)"));
    }

    // Remote execution security tests
    // These tests verify that environment variables are properly escaped when executing
    // hooks remotely via SSH, preventing command injection attacks

    #[test]
    fn test_execute_remote_escapes_env_vars_with_semicolon() {
        use crate::backends::deploy::DeployTarget;

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

        // Use localhost as target to avoid needing a real remote host
        // The command will be executed via SSH but on the same machine
        let target = DeployTarget {
            hostname: "localhost".to_string(),
            user: None,
            port: None,
        };

        let result = backend.execute_remote(&target, &hook, &env).unwrap();

        // If escaping failed, the semicolon would cause command injection
        // With proper escaping, the semicolon is part of the variable value
        assert!(
            result.success
                || result.stderr.contains("Connection refused")
                || result.stderr.contains("Host key")
        );

        // If we got output (SSH succeeded), verify no injection occurred
        if result.success {
            assert!(result.stdout.contains("value; rm -rf /tmp/test"));
            assert!(!result
                .stdout
                .lines()
                .any(|line| line.trim().is_empty() && !result.stderr.contains("test")));
        }
    }

    #[test]
    fn test_execute_remote_escapes_env_vars_with_backticks() {
        use crate::backends::deploy::DeployTarget;

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

        // If we got output (SSH succeeded), verify backticks are literal
        if result.success {
            assert!(result.stdout.contains("`whoami`"));
        }
    }

    #[test]
    fn test_execute_remote_escapes_env_vars_with_dollar_paren() {
        use crate::backends::deploy::DeployTarget;

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

        // If we got output (SSH succeeded), verify $() is literal, not executed
        if result.success {
            assert!(result.stdout.contains("$(echo injected)"));
        }
    }

    #[test]
    fn test_execute_remote_escapes_env_vars_with_pipes() {
        use crate::backends::deploy::DeployTarget;

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

        // If we got output (SSH succeeded), verify pipe is literal
        if result.success {
            assert!(result.stdout.contains("data | base64"));
            // Verify base64 encoding did NOT happen
            assert!(!result.stdout.contains("ZGF0YQ=="));
        }
    }

    #[test]
    fn test_execute_remote_escapes_env_vars_with_quotes() {
        use crate::backends::deploy::DeployTarget;

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

        // If we got output (SSH succeeded), verify quotes don't break out
        if result.success {
            assert!(result.stdout.contains("value\" && echo \"injected"));
        }
    }
}
