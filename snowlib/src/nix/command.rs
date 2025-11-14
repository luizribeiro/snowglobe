//! Internal Nix command builder
//!
//! This module provides a builder pattern for constructing and executing Nix CLI
//! commands with proper error handling and output management.

use crate::error::{NixError, Result};
use crate::nix::Flake;
use crate::process::StreamingExecutor;
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
    flake: Flake,
    quiet: bool,
}

impl NixCommand {
    /// Create a new nix command builder with a flake (private)
    fn new(flake: &Flake) -> Self {
        Self {
            cmd: Command::new("nix"),
            flake: flake.clone(),
            quiet: true, // Default to quiet mode
        }
    }

    /// Start building a `nix eval` command for a flake attribute
    pub(crate) fn eval(flake: &Flake, attr: impl AsRef<str>) -> Self {
        let mut builder = Self::new(flake);
        builder.cmd.args(["eval", attr.as_ref()]);
        builder
    }

    /// Start building a `nix build` command
    pub(crate) fn build(flake: &Flake, target: impl AsRef<str>) -> Self {
        let mut builder = Self::new(flake);
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
        let result = StreamingExecutor::new(&mut self.cmd)
            .working_dir(self.flake.flake_root())
            .quiet(self.quiet)
            .execute()
            .map_err(|e| NixError::CommandFailed { source: e })?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr).to_string();
            return Err(NixError::EvalFailed {
                stderr,
                source: std::io::Error::other("nix command exited with non-zero status"),
            }
            .into());
        }

        Ok(result.stdout)
    }
}
