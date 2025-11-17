//! Internal Nix command builder
//!
//! This module provides a builder pattern for constructing and executing Nix CLI
//! commands with proper error handling and output management.

use crate::error::{NixError, Result};
use crate::nix::Flake;
use crate::process::{OutputMode, StreamingExecutor};
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
    output_mode: OutputMode,
}

impl NixCommand {
    fn new(flake: &Flake) -> Self {
        Self {
            cmd: Command::new("nix"),
            flake: flake.clone(),
            output_mode: OutputMode::Capture,
        }
    }

    pub(crate) fn eval(flake: &Flake, attr: impl AsRef<str>) -> Self {
        let mut builder = Self::new(flake);
        builder.cmd.args(["eval", attr.as_ref()]);
        builder
    }

    pub(crate) fn build(flake: &Flake, target: impl AsRef<str>) -> Self {
        let mut builder = Self::new(flake);
        builder.cmd.args(["build", target.as_ref()]);
        builder
    }

    pub(crate) fn json(mut self) -> Self {
        self.cmd.arg("--json");
        self
    }

    pub(crate) fn apply(mut self, expr: impl AsRef<str>) -> Self {
        self.cmd.args(["--apply", expr.as_ref()]);
        self
    }

    pub(crate) fn no_link(mut self) -> Self {
        self.cmd.arg("--no-link");
        self
    }

    pub(crate) fn print_out_paths(mut self) -> Self {
        self.cmd.arg("--print-out-paths");
        self
    }

    pub(crate) fn output_mode(mut self, mode: OutputMode) -> Self {
        self.output_mode = mode;
        self
    }

    pub(crate) fn run(mut self) -> Result<Vec<u8>> {
        let result = StreamingExecutor::new(&mut self.cmd)
            .working_dir(self.flake.flake_root())
            .output_mode(self.output_mode)
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
