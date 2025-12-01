//! Command execution abstraction for testable provisioner operations
//!
//! Provides a trait for executing shell commands that can be mocked in tests.

use crate::error::{ProvisionerError, Result};
use std::process::{Command, Output, Stdio};

/// Output from a command execution
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
}

impl CommandOutput {
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}

impl From<Output> for CommandOutput {
    fn from(output: Output) -> Self {
        Self {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.status.code(),
        }
    }
}

/// Trait for executing shell commands
///
/// Allows swapping between real command execution and mocked versions for testing.
pub trait CommandExecutor: Send + Sync {
    /// Execute a local command with the given arguments
    fn execute(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput>;

    /// Execute a local command with stdin piped
    fn execute_with_stdin(
        &self,
        cmd: &str,
        args: &[&str],
        stdin_data: &[u8],
    ) -> Result<CommandOutput>;

    /// Execute a command on a remote host via SSH
    fn execute_remote(&self, ssh_dest: &str, remote_cmd: &str) -> Result<CommandOutput>;

    /// Check if a command is available in PATH
    fn which(&self, cmd: &str) -> Option<std::path::PathBuf>;
}

/// Real command executor that runs actual shell commands
pub struct SystemCommandExecutor;

impl SystemCommandExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SystemCommandExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandExecutor for SystemCommandExecutor {
    fn execute(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        let output = Command::new(cmd).args(args).output().map_err(|e| {
            ProvisionerError::InjectionCommandFailed {
                provisioner: "system".to_string(),
                command: cmd.to_string(),
                stderr: format!("Failed to execute: {}", e),
            }
        })?;

        Ok(output.into())
    }

    fn execute_with_stdin(
        &self,
        cmd: &str,
        args: &[&str],
        stdin_data: &[u8],
    ) -> Result<CommandOutput> {
        use std::io::Write;

        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ProvisionerError::InjectionCommandFailed {
                provisioner: "system".to_string(),
                command: cmd.to_string(),
                stderr: format!("Failed to spawn: {}", e),
            })?;

        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(stdin_data)
                .map_err(|e| ProvisionerError::InjectionCommandFailed {
                    provisioner: "system".to_string(),
                    command: cmd.to_string(),
                    stderr: format!("Failed to write stdin: {}", e),
                })?;
        }

        let output =
            child
                .wait_with_output()
                .map_err(|e| ProvisionerError::InjectionCommandFailed {
                    provisioner: "system".to_string(),
                    command: cmd.to_string(),
                    stderr: format!("Failed to wait: {}", e),
                })?;

        Ok(output.into())
    }

    fn execute_remote(&self, ssh_dest: &str, remote_cmd: &str) -> Result<CommandOutput> {
        self.execute("ssh", &[ssh_dest, remote_cmd])
    }

    fn which(&self, cmd: &str) -> Option<std::path::PathBuf> {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths).find_map(|dir| {
                let full_path = dir.join(cmd);
                if full_path.is_file() {
                    Some(full_path)
                } else {
                    None
                }
            })
        })
    }
}

/// Mock command executor for testing
#[cfg(any(debug_assertions, test))]
pub struct MockCommandExecutor {
    /// Map of (command, args_pattern) -> CommandOutput
    responses: std::sync::Mutex<std::collections::HashMap<String, CommandOutput>>,
    /// Commands that were executed (for verification)
    executed: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    /// Commands available via which()
    available_commands: Vec<String>,
    /// Default response for unmatched commands
    default_success: bool,
}

#[cfg(any(debug_assertions, test))]
impl MockCommandExecutor {
    pub fn new() -> Self {
        Self {
            responses: std::sync::Mutex::new(std::collections::HashMap::new()),
            executed: std::sync::Mutex::new(Vec::new()),
            available_commands: vec!["sh".to_string()],
            default_success: true,
        }
    }

    /// Add a canned response for a command pattern
    pub fn with_response(self, pattern: &str, output: CommandOutput) -> Self {
        self.responses
            .lock()
            .unwrap()
            .insert(pattern.to_string(), output);
        self
    }

    /// Set which commands are available
    pub fn with_available_commands(mut self, cmds: Vec<String>) -> Self {
        self.available_commands = cmds;
        self
    }

    /// Set default behavior for unmatched commands
    pub fn with_default_success(mut self, success: bool) -> Self {
        self.default_success = success;
        self
    }

    /// Get list of executed commands for verification
    pub fn get_executed(&self) -> Vec<(String, Vec<String>)> {
        self.executed.lock().unwrap().clone()
    }

    /// Create a successful output
    pub fn success_output(stdout: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
            exit_code: Some(0),
        }
    }

    /// Create a failed output
    pub fn failure_output(stderr: &str) -> CommandOutput {
        CommandOutput {
            success: false,
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
            exit_code: Some(1),
        }
    }
}

#[cfg(any(debug_assertions, test))]
impl Default for MockCommandExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(debug_assertions, test))]
impl CommandExecutor for MockCommandExecutor {
    fn execute(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        let full_cmd = format!("{} {}", cmd, args.join(" "));
        self.executed.lock().unwrap().push((
            cmd.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));

        // Check for exact match first
        if let Some(response) = self.responses.lock().unwrap().get(&full_cmd) {
            return Ok(response.clone());
        }

        // Check for command-only match
        if let Some(response) = self.responses.lock().unwrap().get(cmd) {
            return Ok(response.clone());
        }

        // Check for pattern matches (contains)
        for (pattern, response) in self.responses.lock().unwrap().iter() {
            if full_cmd.contains(pattern) {
                return Ok(response.clone());
            }
        }

        // Default response
        Ok(CommandOutput {
            success: self.default_success,
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: Some(if self.default_success { 0 } else { 1 }),
        })
    }

    fn execute_with_stdin(
        &self,
        cmd: &str,
        args: &[&str],
        _stdin_data: &[u8],
    ) -> Result<CommandOutput> {
        // For mock, just delegate to execute
        self.execute(cmd, args)
    }

    fn execute_remote(&self, ssh_dest: &str, remote_cmd: &str) -> Result<CommandOutput> {
        self.execute("ssh", &[ssh_dest, remote_cmd])
    }

    fn which(&self, cmd: &str) -> Option<std::path::PathBuf> {
        if self.available_commands.contains(&cmd.to_string()) {
            Some(std::path::PathBuf::from(format!("/usr/bin/{}", cmd)))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_executor_which_finds_sh() {
        let executor = SystemCommandExecutor::new();
        assert!(executor.which("sh").is_some());
    }

    #[test]
    fn test_system_executor_which_missing_command() {
        let executor = SystemCommandExecutor::new();
        assert!(executor
            .which("definitely-not-a-real-command-12345")
            .is_none());
    }

    #[test]
    fn test_mock_executor_default_success() {
        let executor = MockCommandExecutor::new();
        let result = executor.execute("test", &["arg1"]).unwrap();
        assert!(result.success);
    }

    #[test]
    fn test_mock_executor_with_response() {
        let executor = MockCommandExecutor::new().with_response(
            "mycommand",
            MockCommandExecutor::success_output("hello world"),
        );

        let result = executor.execute("mycommand", &[]).unwrap();
        assert!(result.success);
        assert_eq!(result.stdout_str(), "hello world");
    }

    #[test]
    fn test_mock_executor_tracks_executed() {
        let executor = MockCommandExecutor::new();
        executor.execute("cmd1", &["a", "b"]).unwrap();
        executor.execute("cmd2", &["c"]).unwrap();

        let executed = executor.get_executed();
        assert_eq!(executed.len(), 2);
        assert_eq!(executed[0].0, "cmd1");
        assert_eq!(executed[0].1, vec!["a", "b"]);
    }

    #[test]
    fn test_mock_executor_which() {
        let executor =
            MockCommandExecutor::new().with_available_commands(vec!["guestfish".to_string()]);

        assert!(executor.which("guestfish").is_some());
        assert!(executor.which("missing").is_none());
    }

    #[test]
    fn test_command_output_from_std_output() {
        let std_output = Output {
            status: std::process::ExitStatus::default(),
            stdout: b"stdout content".to_vec(),
            stderr: b"stderr content".to_vec(),
        };

        let output: CommandOutput = std_output.into();
        assert_eq!(output.stdout_str(), "stdout content");
        assert_eq!(output.stderr_str(), "stderr content");
    }
}
