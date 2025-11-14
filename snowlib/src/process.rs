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
/// use snowlib::process::StreamingExecutor;
///
/// let mut cmd = Command::new("nix");
/// cmd.args(["build", ".#something"]);
///
/// let result = StreamingExecutor::new(&mut cmd)
///     .quiet(false)
///     .execute()?;
///
/// if !result.status.success() {
///     eprintln!("Command failed!");
/// }
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct StreamingExecutor<'a> {
    command: &'a mut Command,
    quiet: bool,
    working_dir: Option<&'a Path>,
}

impl<'a> StreamingExecutor<'a> {
    /// Create a new streaming executor for the given command
    pub fn new(command: &'a mut Command) -> Self {
        Self {
            command,
            quiet: true,
            working_dir: None,
        }
    }

    /// Set whether to suppress tracing output (default: true)
    pub fn quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// Set the working directory for command execution
    pub fn working_dir(mut self, dir: &'a Path) -> Self {
        self.working_dir = Some(dir);
        self
    }

    /// Execute the command with streaming output
    ///
    /// Spawns the command and streams stdout/stderr line-by-line to tracing
    /// (if not in quiet mode) while also capturing all output for the result.
    ///
    /// Returns `Ok(CommandResult)` whether the command succeeds or fails.
    /// Only returns `Err` if there's an IO error spawning or reading from the process.
    pub fn execute(self) -> std::io::Result<CommandResult> {
        // Configure working directory if specified
        if let Some(dir) = self.working_dir {
            self.command.current_dir(dir);
        }

        // Spawn with piped streams
        let mut child = self
            .command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Take ownership of stdout/stderr
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        // Shared buffers for captured output
        let stdout_lines = Arc::new(Mutex::new(Vec::new()));
        let stderr_lines = Arc::new(Mutex::new(Vec::new()));

        // Clone references for threads
        let stdout_lines_clone = Arc::clone(&stdout_lines);
        let stderr_lines_clone = Arc::clone(&stderr_lines);

        let quiet = self.quiet;

        // Spawn thread to read stdout
        let stdout_thread = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                // Log to tracing if not quiet
                if !quiet && !line.is_empty() {
                    tracing::debug!(target: "command", "{}", line);
                }

                // Capture for result
                let mut lines = stdout_lines_clone.lock().unwrap();
                lines.extend_from_slice(line.as_bytes());
                lines.push(b'\n');
            }
        });

        // Spawn thread to read stderr
        let stderr_thread = thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                // Log to tracing if not quiet
                if !quiet && !line.is_empty() {
                    tracing::debug!(target: "command", "stderr: {}", line);
                }

                // Capture for result
                let mut lines = stderr_lines_clone.lock().unwrap();
                lines.extend_from_slice(line.as_bytes());
                lines.push(b'\n');
            }
        });

        // Wait for process to complete
        let status = child.wait()?;

        // Wait for reader threads to finish
        stdout_thread.join().expect("stdout thread panicked");
        stderr_thread.join().expect("stderr thread panicked");

        // Extract captured output
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
