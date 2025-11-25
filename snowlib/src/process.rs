//! Process execution utilities with async subprocess handling
//!
//! Provides async command execution with real-time tracing support using tokio.
//! This module centralizes all subprocess execution logic to provide consistent
//! behavior across Nix operations, SSH deployment, and other command execution.
//!
//! # Example
//!
//! ```no_run
//! use snowlib::process::{r#async::run_command, OutputMode};
//! use std::time::Duration;
//!
//! # async fn example() -> std::io::Result<()> {
//! let result = run_command(
//!     "nix",
//!     &["build", ".#something"],
//!     OutputMode::Display,
//!     Some(Duration::from_secs(300)),
//!     None,
//!     &[],
//! ).await?;
//!
//! if !result.status.success() {
//!     eprintln!("Command failed!");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Timeout Handling
//!
//! When a timeout is specified, the executor will:
//! 1. Wait for the process to complete within the timeout
//! 2. Send SIGTERM if timeout expires, allowing graceful shutdown
//! 3. Wait a 5-second grace period for the process to exit
//! 4. Send SIGKILL if the process hasn't exited
//! 5. Return partial captured output with a timeout indicator

use std::process::ExitStatus;
use std::time::Duration;

/// Grace period to wait for process to exit after SIGTERM before sending SIGKILL
const SIGTERM_GRACE_PERIOD: Duration = Duration::from_secs(5);

/// Controls how command output is handled during execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMode {
    /// Capture output but don't display it
    #[default]
    Capture,
    /// Display output via tracing at info level
    Display,
    /// Direct passthrough to terminal (inherit stdio)
    Inherit,
}

/// Result of executing a command
#[derive(Debug)]
pub struct CommandResult {
    /// Captured stdout as raw bytes
    pub stdout: Vec<u8>,
    /// Captured stderr as raw bytes
    pub stderr: Vec<u8>,
    /// Exit status of the process
    pub status: ExitStatus,
    /// Whether the command was terminated due to timeout
    pub timed_out: bool,
}

pub mod r#async {
    //! Async subprocess execution with tokio
    //!
    //! Provides async command execution with real-time tracing support.
    //! This eliminates manual thread spawning and provides cleaner timeout handling.

    use super::{CommandResult, OutputMode, SIGTERM_GRACE_PERIOD};
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;
    use std::path::Path;
    use std::process::ExitStatus;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    /// Execute a command asynchronously with streaming output support
    ///
    /// # Arguments
    /// * `program` - The program to execute
    /// * `args` - Arguments to pass to the program
    /// * `output_mode` - How to handle stdout/stderr
    /// * `timeout` - Optional timeout duration
    /// * `working_dir` - Optional working directory
    /// * `envs` - Environment variables to set
    ///
    /// # Example
    ///
    /// ```no_run
    /// use snowlib::process::r#async::run_command;
    /// use snowlib::process::OutputMode;
    /// use std::time::Duration;
    ///
    /// # async fn example() -> std::io::Result<()> {
    /// let result = run_command(
    ///     "echo",
    ///     &["hello"],
    ///     OutputMode::Capture,
    ///     Some(Duration::from_secs(10)),
    ///     None,
    ///     &[],
    /// ).await?;
    ///
    /// println!("Output: {}", String::from_utf8_lossy(&result.stdout));
    /// # Ok(())
    /// # }
    /// ```
    pub async fn run_command(
        program: &str,
        args: &[&str],
        output_mode: OutputMode,
        timeout: Option<Duration>,
        working_dir: Option<&Path>,
        envs: &[(&str, &str)],
    ) -> std::io::Result<CommandResult> {
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        for (key, value) in envs {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn()?;
        let child_pid = Pid::from_raw(child.id().expect("child has pid") as i32);

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        // Use shared buffers so we can access partial output on timeout
        let stdout_buf = Arc::new(Mutex::new(Vec::new()));
        let stderr_buf = Arc::new(Mutex::new(Vec::new()));

        let stdout_buf_clone = Arc::clone(&stdout_buf);
        let stderr_buf_clone = Arc::clone(&stderr_buf);

        let stdout_task = tokio::spawn(async move {
            stream_and_capture_shared(stdout, output_mode, false, stdout_buf_clone).await
        });
        let stderr_task = tokio::spawn(async move {
            stream_and_capture_shared(stderr, output_mode, true, stderr_buf_clone).await
        });

        let wait_task = async {
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            child.wait().await
        };

        match timeout {
            Some(duration) => {
                match tokio::time::timeout(duration, wait_task).await {
                    Ok(Ok(status)) => {
                        let stdout = stdout_buf.lock().await.clone();
                        let stderr = stderr_buf.lock().await.clone();
                        Ok(CommandResult {
                            stdout,
                            stderr,
                            status,
                            timed_out: false,
                        })
                    }
                    Ok(Err(e)) => Err(e),
                    Err(_elapsed) => {
                        // Timeout occurred - terminate the process
                        tracing::warn!(
                            "Process timed out after {:?}, sending SIGTERM to PID {}",
                            duration,
                            child_pid
                        );

                        let status = terminate_process(child, child_pid).await?;

                        // Get whatever output was captured before timeout
                        let stdout = stdout_buf.lock().await.clone();
                        let stderr = stderr_buf.lock().await.clone();

                        Ok(CommandResult {
                            stdout,
                            stderr,
                            status,
                            timed_out: true,
                        })
                    }
                }
            }
            None => {
                let status = wait_task.await?;
                let stdout = stdout_buf.lock().await.clone();
                let stderr = stderr_buf.lock().await.clone();
                Ok(CommandResult {
                    stdout,
                    stderr,
                    status,
                    timed_out: false,
                })
            }
        }
    }

    /// Stream output from an async reader to a shared buffer, optionally displaying via tracing
    async fn stream_and_capture_shared(
        reader: impl tokio::io::AsyncRead + Unpin,
        output_mode: OutputMode,
        is_stderr: bool,
        buffer: std::sync::Arc<tokio::sync::Mutex<Vec<u8>>>,
    ) {
        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            match buf_reader.read_line(&mut line).await {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let trimmed = line.trim_end_matches('\n');
                    match output_mode {
                        OutputMode::Display if !trimmed.is_empty() => {
                            if is_stderr {
                                tracing::info!("stderr: {}", trimmed);
                            } else {
                                tracing::info!("{}", trimmed);
                            }
                        }
                        OutputMode::Inherit if !trimmed.is_empty() => {
                            if is_stderr {
                                eprintln!("{}", trimmed);
                            } else {
                                println!("{}", trimmed);
                            }
                        }
                        _ => {}
                    }
                    // Append to shared buffer
                    buffer.lock().await.extend_from_slice(line.as_bytes());
                }
                Err(e) => {
                    tracing::warn!("Error reading output: {}", e);
                    break;
                }
            }
        }
    }

    /// Terminate a process gracefully, escalating to SIGKILL if needed.
    /// Returns the exit status of the terminated process.
    async fn terminate_process(
        mut child: tokio::process::Child,
        child_pid: Pid,
    ) -> std::io::Result<ExitStatus> {
        // Send SIGTERM
        if let Err(e) = signal::kill(child_pid, Signal::SIGTERM) {
            tracing::warn!("Failed to send SIGTERM: {}", e);
        }

        // Give process time to exit gracefully
        match tokio::time::timeout(SIGTERM_GRACE_PERIOD, child.wait()).await {
            Ok(Ok(status)) => {
                tracing::info!("Process exited gracefully after SIGTERM");
                Ok(status)
            }
            Ok(Err(e)) => {
                tracing::warn!("Error waiting for process after SIGTERM: {}", e);
                Err(e)
            }
            Err(_) => {
                // Timeout - escalate to SIGKILL
                tracing::warn!("Process did not exit after SIGTERM, sending SIGKILL");
                if let Err(e) = signal::kill(child_pid, Signal::SIGKILL) {
                    tracing::error!("Failed to send SIGKILL: {}", e);
                }
                // Wait for SIGKILL to take effect
                child.wait().await
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::Instant;

        #[tokio::test]
        async fn test_async_capture_mode() {
            let result = run_command("echo", &["hello"], OutputMode::Capture, None, None, &[])
                .await
                .unwrap();

            assert!(result.status.success());
            assert!(!result.timed_out);
            assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "hello");
        }

        #[tokio::test]
        async fn test_async_with_env() {
            let result = run_command(
                "sh",
                &["-c", "echo $TEST_VAR"],
                OutputMode::Capture,
                None,
                None,
                &[("TEST_VAR", "test_value")],
            )
            .await
            .unwrap();

            assert!(result.status.success());
            assert!(String::from_utf8_lossy(&result.stdout).contains("test_value"));
        }

        #[tokio::test]
        async fn test_async_stderr_captured() {
            let result = run_command(
                "sh",
                &["-c", "echo error >&2"],
                OutputMode::Capture,
                None,
                None,
                &[],
            )
            .await
            .unwrap();

            assert!(result.status.success());
            assert_eq!(String::from_utf8_lossy(&result.stderr).trim(), "error");
        }

        #[tokio::test]
        async fn test_async_nonzero_exit() {
            let result = run_command(
                "sh",
                &["-c", "exit 42"],
                OutputMode::Capture,
                None,
                None,
                &[],
            )
            .await
            .unwrap();

            assert!(!result.status.success());
            assert!(!result.timed_out);
            assert_eq!(result.status.code(), Some(42));
        }

        #[tokio::test]
        async fn test_async_completes_before_timeout() {
            let result = run_command(
                "echo",
                &["quick"],
                OutputMode::Capture,
                Some(Duration::from_secs(10)),
                None,
                &[],
            )
            .await
            .unwrap();

            assert!(result.status.success());
            assert!(!result.timed_out);
        }

        #[tokio::test]
        async fn test_async_timeout_terminates_process() {
            let start = Instant::now();
            let result = run_command(
                "sleep",
                &["30"],
                OutputMode::Capture,
                Some(Duration::from_secs(1)),
                None,
                &[],
            )
            .await
            .unwrap();
            let elapsed = start.elapsed();

            assert!(!result.status.success());
            assert!(result.timed_out);
            assert!(
                elapsed.as_secs() >= 1,
                "Should wait at least timeout duration"
            );
            assert!(
                elapsed.as_secs() < 10,
                "Should terminate quickly after timeout"
            );
        }

        #[tokio::test]
        async fn test_async_timeout_captures_partial_output() {
            let result = run_command(
                "sh",
                &[
                    "-c",
                    "echo 'Before timeout'; sleep 30; echo 'After timeout'",
                ],
                OutputMode::Capture,
                Some(Duration::from_secs(1)),
                None,
                &[],
            )
            .await
            .unwrap();

            assert!(result.timed_out);
            let stdout = String::from_utf8_lossy(&result.stdout);
            assert!(
                stdout.contains("Before timeout"),
                "Should capture output before timeout"
            );
            assert!(
                !stdout.contains("After timeout"),
                "Should not have output after timeout"
            );
        }
    }
}
