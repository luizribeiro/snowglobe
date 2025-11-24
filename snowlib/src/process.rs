//! Process execution utilities with streaming output
//!
//! Provides streaming command execution with real-time tracing support.
//! This module centralizes all subprocess execution logic to provide consistent
//! behavior across Nix operations, SSH deployment, and other command execution.
//!
//! # Timeout Handling
//!
//! When a timeout is specified, the executor will:
//! 1. Wait for the process to complete within the timeout
//! 2. Send SIGTERM if timeout expires, allowing graceful shutdown
//! 3. Wait a 5-second grace period for the process to exit
//! 4. Send SIGKILL if the process hasn't exited
//! 5. Return partial captured output with a timeout indicator

use nix::sys::signal::{self, Signal};
use nix::sys::wait::{self, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Grace period to wait for process to exit after SIGTERM before sending SIGKILL
const SIGTERM_GRACE_PERIOD: Duration = Duration::from_secs(5);

/// Maximum time to wait for process to be reaped by kernel after SIGKILL
const SIGKILL_REAP_TIMEOUT: Duration = Duration::from_secs(2);

/// Polling interval when waiting for process to be reaped
const REAP_POLL_INTERVAL: Duration = Duration::from_millis(100);

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

/// Builder for streaming command execution
///
/// Executes commands with real-time output streaming to tracing while
/// also capturing stdout/stderr for processing. Provides a fluent API
/// for configuring execution parameters.
///
/// # Example
///
/// ```no_run
/// use std::process::Command;
/// use snowlib::process::{StreamingExecutor, OutputMode};
///
/// let mut cmd = Command::new("nix");
/// cmd.args(["build", ".#something"]);
///
/// let result = StreamingExecutor::new(&mut cmd)
///     .output_mode(OutputMode::Display)
///     .execute()?;
///
/// if !result.status.success() {
///     eprintln!("Command failed!");
/// }
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct StreamingExecutor<'a> {
    command: &'a mut Command,
    output_mode: OutputMode,
    working_dir: Option<&'a Path>,
    timeout: Option<Duration>,
}

impl<'a> StreamingExecutor<'a> {
    /// Create a new streaming executor for the given command
    pub fn new(command: &'a mut Command) -> Self {
        Self {
            command,
            output_mode: OutputMode::default(),
            working_dir: None,
            timeout: None,
        }
    }

    /// Set the output mode for command execution
    pub fn output_mode(mut self, mode: OutputMode) -> Self {
        self.output_mode = mode;
        self
    }

    pub fn quiet(self) -> Self {
        self.output_mode(OutputMode::Capture)
    }

    /// Set the working directory for command execution
    pub fn working_dir(mut self, dir: &'a Path) -> Self {
        self.working_dir = Some(dir);
        self
    }

    /// Set an optional timeout for command execution
    ///
    /// If the command doesn't complete within the timeout:
    /// 1. SIGTERM is sent to allow graceful shutdown
    /// 2. After a 5-second grace period, SIGKILL is sent
    /// 3. Partial output is returned with `timed_out: true`
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    /// Execute the command with streaming output
    ///
    /// Behavior depends on the configured OutputMode:
    /// - Inherit: Capture and display output directly to terminal
    /// - Capture: Capture output but don't display
    /// - Display: Capture and stream to tracing at info level
    ///
    /// Returns `Ok(CommandResult)` whether the command succeeds or fails.
    /// Only returns `Err` if there's an IO error spawning or reading from the process.
    pub fn execute(self) -> std::io::Result<CommandResult> {
        if let Some(dir) = self.working_dir {
            self.command.current_dir(dir);
        }

        let mut child = self
            .command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let child_pid = Pid::from_raw(child.id() as i32);

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        let stdout_lines = Arc::new(Mutex::new(Vec::new()));
        let stderr_lines = Arc::new(Mutex::new(Vec::new()));

        let stdout_lines_clone = Arc::clone(&stdout_lines);
        let stderr_lines_clone = Arc::clone(&stderr_lines);

        let output_mode = self.output_mode;

        let stdout_thread = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                match output_mode {
                    OutputMode::Display if !line.is_empty() => {
                        tracing::info!(target: "command", "{}", line);
                    }
                    OutputMode::Inherit if !line.is_empty() => {
                        println!("{}", line);
                    }
                    _ => {}
                }

                let mut lines = stdout_lines_clone.lock().unwrap();
                lines.extend_from_slice(line.as_bytes());
                lines.push(b'\n');
            }
        });

        let stderr_thread = thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                match output_mode {
                    OutputMode::Display if !line.is_empty() => {
                        tracing::info!(target: "command", "stderr: {}", line);
                    }
                    OutputMode::Inherit if !line.is_empty() => {
                        eprintln!("{}", line);
                    }
                    _ => {}
                }

                let mut lines = stderr_lines_clone.lock().unwrap();
                lines.extend_from_slice(line.as_bytes());
                lines.push(b'\n');
            }
        });

        let (status, timed_out) = match self.timeout {
            Some(timeout) => Self::wait_with_timeout(child, child_pid, timeout),
            None => {
                let status = child.wait()?;
                (status, false)
            }
        };

        stdout_thread.join().expect("stdout thread panicked");
        stderr_thread.join().expect("stderr thread panicked");

        let stdout = Arc::try_unwrap(stdout_lines)
            .expect("stdout Arc has multiple references")
            .into_inner()
            .unwrap();
        let stderr = Arc::try_unwrap(stderr_lines)
            .expect("stderr Arc has multiple references")
            .into_inner()
            .unwrap();

        Ok(CommandResult {
            stdout,
            stderr,
            status,
            timed_out,
        })
    }

    /// Wait for the child process with timeout handling
    ///
    /// Returns (ExitStatus, timed_out). On timeout, the returned status will have
    /// a non-zero exit code (typically from SIGTERM or SIGKILL).
    fn wait_with_timeout(
        mut child: std::process::Child,
        child_pid: Pid,
        timeout: Duration,
    ) -> (ExitStatus, bool) {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let status = child.wait();
            let _ = tx.send(status);
        });

        match rx.recv_timeout(timeout) {
            Ok(Ok(status)) => (status, false),
            Ok(Err(e)) => {
                tracing::error!("Failed to wait for process: {}", e);
                (Self::fake_exit_status(), true)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                tracing::warn!(
                    "Process timed out after {:?}, sending SIGTERM to PID {}",
                    timeout,
                    child_pid
                );
                Self::terminate_process(child_pid, &rx)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => (Self::fake_exit_status(), true),
        }
    }

    /// Terminate a process with SIGTERM, escalating to SIGKILL if needed
    fn terminate_process(
        child_pid: Pid,
        rx: &mpsc::Receiver<std::io::Result<ExitStatus>>,
    ) -> (ExitStatus, bool) {
        if let Err(e) = signal::kill(child_pid, Signal::SIGTERM) {
            tracing::warn!("Failed to send SIGTERM: {}", e);
        }

        match rx.recv_timeout(SIGTERM_GRACE_PERIOD) {
            Ok(Ok(status)) => {
                tracing::info!("Process exited gracefully after SIGTERM");
                return (status, true);
            }
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return (Self::fake_exit_status(), true);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                tracing::warn!("Process did not exit after SIGTERM, sending SIGKILL");
            }
        }

        if let Err(e) = signal::kill(child_pid, Signal::SIGKILL) {
            tracing::error!("Failed to send SIGKILL: {}", e);
        }

        Self::wait_for_reap(child_pid);
        (Self::fake_exit_status(), true)
    }

    /// Wait for a killed process to be reaped by the kernel
    fn wait_for_reap(child_pid: Pid) {
        let reap_deadline = Instant::now() + SIGKILL_REAP_TIMEOUT;
        loop {
            if Instant::now() > reap_deadline {
                tracing::error!(
                    "Process {} did not die after SIGKILL (may be zombie or in uninterruptible sleep)",
                    child_pid
                );
                break;
            }

            match wait::waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(_, _)) | Ok(WaitStatus::Signaled(_, _, _)) => {
                    tracing::info!("Process {} reaped after SIGKILL", child_pid);
                    break;
                }
                Ok(_) => {
                    thread::sleep(REAP_POLL_INTERVAL);
                }
                Err(nix::errno::Errno::ECHILD) => {
                    tracing::debug!("Process {} already reaped", child_pid);
                    break;
                }
                Err(e) => {
                    tracing::warn!("waitpid failed: {}", e);
                    break;
                }
            }
        }
    }

    /// Create a fake exit status for timeout cases
    ///
    /// Uses Command to run "exit 124" which gives us a real ExitStatus.
    /// 124 is the conventional exit code for timeout (used by GNU timeout).
    fn fake_exit_status() -> ExitStatus {
        Command::new("sh")
            .args(["-c", "exit 124"])
            .status()
            .expect("failed to create fake exit status")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_mode_captures_stdout() {
        let mut cmd = Command::new("echo");
        cmd.arg("hello");

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Capture)
            .execute()
            .unwrap();

        assert!(result.status.success());
        assert!(!result.timed_out);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "hello");
    }

    #[test]
    fn test_inherit_mode_captures_stdout() {
        let mut cmd = Command::new("echo");
        cmd.arg("test output");

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Inherit)
            .execute()
            .unwrap();

        assert!(result.status.success());
        assert!(!result.timed_out);
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "test output",
            "Inherit mode must capture stdout for commands that need to parse output"
        );
    }

    #[test]
    fn test_display_mode_captures_stdout() {
        let mut cmd = Command::new("echo");
        cmd.arg("display test");

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Display)
            .execute()
            .unwrap();

        assert!(result.status.success());
        assert!(!result.timed_out);
        assert_eq!(
            String::from_utf8_lossy(&result.stdout).trim(),
            "display test"
        );
    }

    #[test]
    fn test_stderr_captured_in_all_modes() {
        for mode in [
            OutputMode::Capture,
            OutputMode::Display,
            OutputMode::Inherit,
        ] {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", "echo error >&2"]);

            let result = StreamingExecutor::new(&mut cmd)
                .output_mode(mode)
                .execute()
                .unwrap();

            assert!(result.status.success());
            assert!(!result.timed_out);
            assert_eq!(
                String::from_utf8_lossy(&result.stderr).trim(),
                "error",
                "{:?} mode must capture stderr",
                mode
            );
        }
    }

    #[test]
    fn test_nonzero_exit_still_returns_ok() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 42"]);

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Capture)
            .execute()
            .unwrap();

        assert!(!result.status.success());
        assert!(!result.timed_out);
        assert_eq!(result.status.code(), Some(42));
    }

    #[test]
    fn test_command_completes_before_timeout() {
        let mut cmd = Command::new("echo");
        cmd.arg("quick");

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Capture)
            .timeout(Duration::from_secs(10))
            .execute()
            .unwrap();

        assert!(result.status.success());
        assert!(!result.timed_out);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "quick");
    }

    #[test]
    fn test_timeout_triggers_and_terminates_process() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");

        let start = Instant::now();
        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Capture)
            .timeout(Duration::from_secs(1))
            .execute()
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
            "Should terminate quickly after timeout, not wait for full sleep"
        );
    }

    #[test]
    fn test_timeout_captures_partial_output() {
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "echo 'Before timeout'; sleep 30; echo 'After timeout'",
        ]);

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Capture)
            .timeout(Duration::from_secs(1))
            .execute()
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

    #[test]
    fn test_timeout_with_large_output_before_hang() {
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "for i in $(seq 1 50); do echo \"Line $i\"; done; sleep 30",
        ]);

        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Capture)
            .timeout(Duration::from_secs(1))
            .execute()
            .unwrap();

        assert!(result.timed_out);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let line_count = stdout.lines().count();
        assert!(
            line_count > 0,
            "Should have captured some output before timeout"
        );
    }

    #[test]
    fn test_timeout_duration_accuracy() {
        let mut cmd = Command::new("sleep");
        cmd.arg("60");

        let start = Instant::now();
        let result = StreamingExecutor::new(&mut cmd)
            .output_mode(OutputMode::Capture)
            .timeout(Duration::from_secs(2))
            .execute()
            .unwrap();
        let elapsed = start.elapsed();

        assert!(result.timed_out);
        assert!(
            elapsed.as_secs() >= 2,
            "Should wait at least the timeout duration"
        );
        // Allow for SIGTERM grace period (5s) + SIGKILL reap timeout (2s) = 9s max
        assert!(
            elapsed.as_secs() < 10,
            "Should not wait too long after timeout"
        );
    }
}
