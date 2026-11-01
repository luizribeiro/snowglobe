//! Internal Nix command builder
//!
//! This module provides a builder pattern for constructing and executing Nix CLI
//! commands with proper error handling and output management.

use std::path::{Path, PathBuf};
use std::time::Duration;

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
    program: &'static str,
    args: Vec<String>,
    working_dir: Option<PathBuf>,
    output_mode: OutputMode,
    timeout: Option<Duration>,
}

impl NixCommand {
    fn new(flake: &Flake) -> Self {
        Self::tool("nix", Vec::<String>::new()).in_flake(flake)
    }

    /// Any nix-family program (`nix`, `nix-eval-jobs`), run outside a flake unless `in_flake` is used.
    pub(crate) fn tool(
        program: &'static str,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program,
            args: args.into_iter().map(Into::into).collect(),
            working_dir: None,
            output_mode: OutputMode::Capture,
            timeout: None,
        }
    }

    pub(crate) fn in_flake(mut self, flake: &Flake) -> Self {
        self.working_dir = Some(flake.flake_root().to_path_buf());
        self
    }

    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
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

    pub(crate) fn out_link(mut self, link: impl AsRef<Path>) -> Self {
        self.args.push("--out-link".to_string());
        self.args.push(link.as_ref().display().to_string());
        self
    }

    #[cfg(test)]
    pub(crate) fn args(&self) -> &[String] {
        &self.args
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

        let result = run_command(
            self.program,
            &args,
            self.output_mode,
            self.timeout,
            self.working_dir.as_deref(),
            &[],
        )
        .await
        .map_err(|e| NixError::CommandFailed { source: e })?;

        if result.timed_out || !result.status.success() {
            let mut stderr = String::from_utf8_lossy(&result.stderr).to_string();
            if result.timed_out {
                stderr.push_str(&format!("\n{} timed out", self.program));
            }
            return Err(NixError::EvalFailed {
                stderr,
                source: std::io::Error::other("nix command exited with non-zero status"),
            }
            .into());
        }

        Ok(result.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_out_link_roots_the_built_path() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("radon-hooks");

        let stdout = NixCommand::tool(
            "nix",
            [
                "build",
                "--expr",
                r#"builtins.toFile "snowglobe-hooks.json" "[]""#,
                "--print-out-paths",
            ],
        )
        .out_link(&link)
        .run()
        .await
        .unwrap();

        let built = String::from_utf8(stdout).unwrap();
        assert_eq!(
            std::fs::read_link(&link).unwrap().display().to_string(),
            built.trim()
        );
    }

    #[tokio::test]
    async fn test_tool_runs_without_a_flake() {
        let stdout = NixCommand::tool("nix", ["--version"]).run().await.unwrap();

        assert!(String::from_utf8_lossy(&stdout).starts_with("nix (Nix)"));
    }

    #[tokio::test]
    async fn test_failure_carries_stderr() {
        let err = NixCommand::tool("nix", ["derivation", "show", "/nix/store/not-a-drv"])
            .run()
            .await
            .unwrap_err();

        match err {
            crate::error::SnowError::Nix(NixError::EvalFailed { stderr, .. }) => {
                assert!(stderr.contains("not-a-drv"), "unexpected stderr: {stderr}")
            }
            other => panic!("expected EvalFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_timeout_is_reported() {
        let err = NixCommand::tool("sleep", ["5"])
            .timeout(Duration::from_millis(100))
            .run()
            .await
            .unwrap_err();

        match err {
            crate::error::SnowError::Nix(NixError::EvalFailed { stderr, .. }) => {
                assert!(
                    stderr.ends_with("sleep timed out"),
                    "unexpected stderr: {stderr}"
                )
            }
            other => panic!("expected EvalFailed, got {other:?}"),
        }
    }
}
