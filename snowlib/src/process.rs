//! Process execution utilities with streaming output
//!
//! Provides streaming command execution with real-time tracing support.
//! This module centralizes all subprocess execution logic to provide consistent
//! behavior across Nix operations, SSH deployment, and other command execution.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

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
}

impl<'a> StreamingExecutor<'a> {
    /// Create a new streaming executor for the given command
    pub fn new(command: &'a mut Command) -> Self {
        Self {
            command,
            output_mode: OutputMode::default(),
            working_dir: None,
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

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        let stdout_lines = Arc::new(Mutex::new(Vec::new()));
        let stderr_lines = Arc::new(Mutex::new(Vec::new()));

        let stdout_lines_clone = Arc::clone(&stdout_lines);
        let stderr_lines_clone = Arc::clone(&stderr_lines);

        let output_mode = self.output_mode;

        // Spawn thread to read stdout
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

        // Spawn thread to read stderr
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

        let status = child.wait()?;

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
        })
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
        assert_eq!(result.status.code(), Some(42));
    }
}
