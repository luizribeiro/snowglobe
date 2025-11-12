//! Internal Nix command builder
//!
//! This module provides a builder pattern for constructing and executing Nix CLI
//! commands with proper error handling and output management.

use crate::error::{NixError, Result};
use crate::nix::Flake;
use std::process::Command;

/// Builder for constructing and executing Nix commands
///
/// Provides a fluent API for building nix command invocations with
/// proper error handling. Initialized with a Flake and allows
/// chaining operations.
///
/// This is an internal implementation detail and should not be used outside
/// the nix module. Use the public functions like `list_hosts()` instead.
pub(crate) struct NixCommand {
    cmd: Command,
    ctx: Flake,
    quiet: bool,
}

impl NixCommand {
    /// Create a new nix command builder with a context (private)
    fn new(ctx: &Flake) -> Self {
        Self {
            cmd: Command::new("nix"),
            ctx: ctx.clone(),
            quiet: true, // Default to quiet mode
        }
    }

    /// Start building a `nix eval` command for a flake attribute
    pub(crate) fn eval(ctx: &Flake, attr: impl AsRef<str>) -> Self {
        let mut builder = Self::new(ctx);
        builder.cmd.args(["eval", attr.as_ref()]);
        builder
    }

    /// Start building a `nix build` command
    pub(crate) fn build(ctx: &Flake, target: impl AsRef<str>) -> Self {
        let mut builder = Self::new(ctx);
        builder.cmd.args(["build", target.as_ref()]);
        builder
    }

    /// Add --json flag to output JSON
    pub(crate) fn json(mut self) -> Self {
        self.cmd.arg("--json");
        self
    }

    /// Add --apply flag with an expression
    pub(crate) fn apply(mut self, expr: impl AsRef<str>) -> Self {
        self.cmd.args(["--apply", expr.as_ref()]);
        self
    }

    /// Add --no-link flag (for build commands)
    pub(crate) fn no_link(mut self) -> Self {
        self.cmd.arg("--no-link");
        self
    }

    /// Add --print-out-paths flag (for build commands)
    pub(crate) fn print_out_paths(mut self) -> Self {
        self.cmd.arg("--print-out-paths");
        self
    }

    /// Set quiet mode (suppress output)
    pub(crate) fn quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// Execute the command and return stdout on success
    pub(crate) fn run(mut self) -> Result<Vec<u8>> {
        if self.quiet {
            // Quiet mode: capture all output
            let output = self
                .cmd
                .current_dir(self.ctx.flake_root())
                .output()
                .map_err(|e| NixError::CommandFailed { source: e })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(NixError::EvalFailed {
                    stderr,
                    source: std::io::Error::other("nix command exited with non-zero status"),
                }
                .into());
            }

            Ok(output.stdout)
        } else {
            // Verbose mode: show output in real-time
            use std::io::Read;
            use std::process::Stdio;

            let mut child = self
                .cmd
                .current_dir(self.ctx.flake_root())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| NixError::CommandFailed { source: e })?;

            // Read stdout while showing it in real-time
            let mut stdout = Vec::new();
            if let Some(mut child_stdout) = child.stdout.take() {
                let mut buffer = [0u8; 4096];
                loop {
                    match child_stdout.read(&mut buffer) {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            // Write to our stdout
                            use std::io::Write;
                            std::io::stdout().write_all(&buffer[..n]).ok();
                            // Store for return value
                            stdout.extend_from_slice(&buffer[..n]);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            return Err(NixError::CommandFailed { source: e }.into());
                        }
                    }
                }
            }

            let status = child
                .wait()
                .map_err(|e| NixError::CommandFailed { source: e })?;

            if !status.success() {
                return Err(NixError::EvalFailed {
                    stderr: "nix command failed (see output above)".to_string(),
                    source: std::io::Error::other("nix command exited with non-zero status"),
                }
                .into());
            }

            Ok(stdout)
        }
    }
}
