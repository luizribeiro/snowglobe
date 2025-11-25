//! Internal Nix command builder
//!
//! This module provides a builder pattern for constructing and executing Nix CLI
//! commands with proper error handling and output management.

use crate::error::{NixError, Result};
use crate::nix::Flake;
use crate::process::r#async::run_command;
use crate::process::OutputMode;

/// Builder for constructing and executing Nix commands
///
/// Provides a fluent API for building nix command invocations with
/// proper error handling. Initialized with a Flake and allows
/// chaining operations.
///
/// This is an internal implementation detail and should not be used outside
/// the nix module. Use the public functions like `list_hosts()` instead.
pub(crate) struct NixCommand {
    args: Vec<String>,
    flake: Flake,
    output_mode: OutputMode,
}

impl NixCommand {
    fn new(flake: &Flake) -> Self {
        Self {
            args: Vec::new(),
            flake: flake.clone(),
            output_mode: OutputMode::Capture,
        }
    }

    pub(crate) fn eval(flake: &Flake, attr: impl AsRef<str>) -> Self {
        let mut builder = Self::new(flake);
        builder.args.push("eval".to_string());
        builder.args.push(attr.as_ref().to_string());
        builder
    }

    pub(crate) fn build(flake: &Flake, target: impl AsRef<str>) -> Self {
        let mut builder = Self::new(flake);
        builder.args.push("build".to_string());
        builder.args.push(target.as_ref().to_string());
        builder
    }

    pub(crate) fn json(mut self) -> Self {
        self.args.push("--json".to_string());
        self
    }

    pub(crate) fn apply(mut self, expr: impl AsRef<str>) -> Self {
        self.args.push("--apply".to_string());
        self.args.push(expr.as_ref().to_string());
        self
    }

    pub(crate) fn no_link(mut self) -> Self {
        self.args.push("--no-link".to_string());
        self
    }

    pub(crate) fn print_out_paths(mut self) -> Self {
        self.args.push("--print-out-paths".to_string());
        self
    }

    pub(crate) fn output_mode(mut self, mode: OutputMode) -> Self {
        self.output_mode = mode;
        self
    }

    pub(crate) async fn run(self) -> Result<Vec<u8>> {
        let args: Vec<&str> = self.args.iter().map(|s| s.as_str()).collect();
        let working_dir = self.flake.flake_root();

        let result = run_command("nix", &args, self.output_mode, None, Some(working_dir), &[])
            .await
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
