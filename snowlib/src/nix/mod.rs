pub mod ops;

use crate::error::{FlakeError, IoError, NixError, Result};
use serde::Deserialize;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Immutable context for snowglobe operations
///
/// Holds the flake root directory and can be extended with additional
/// state in the future (e.g., configuration, cached data, etc.)
#[derive(Debug, Clone)]
pub struct SnowContext {
    flake_root: PathBuf,
}

impl SnowContext {
    /// Create a new context from an optional flake path
    ///
    /// If flake_path is provided, validates and uses it. Otherwise searches
    /// up from the current directory to find flake.nix.
    pub fn new(flake_path: Option<&str>) -> Result<Self> {
        let flake_root = if let Some(path) = flake_path {
            validate_flake_path(path)?
        } else {
            find_flake_root()?
        };

        Ok(Self { flake_root })
    }

    /// Get the flake root directory
    pub fn flake_root(&self) -> &Path {
        &self.flake_root
    }
}

/// Builder for constructing and executing Nix commands
///
/// Provides a fluent API for building nix command invocations with
/// proper error handling. Initialized with a SnowContext and allows
/// chaining operations.
struct NixCommand {
    cmd: Command,
    ctx: SnowContext,
}

impl NixCommand {
    /// Create a new nix command builder with a context (private)
    fn new(ctx: &SnowContext) -> Self {
        Self {
            cmd: Command::new("nix"),
            ctx: ctx.clone(),
        }
    }

    /// Start building a `nix eval` command for a flake attribute
    fn eval(ctx: &SnowContext, attr: impl AsRef<str>) -> Self {
        let mut builder = Self::new(ctx);
        builder.cmd.args(["eval", attr.as_ref()]);
        builder
    }

    /// Start building a `nix build` command
    fn build(ctx: &SnowContext, target: impl AsRef<str>) -> Self {
        let mut builder = Self::new(ctx);
        builder.cmd.args(["build", target.as_ref()]);
        builder
    }

    /// Add --json flag to output JSON
    fn json(mut self) -> Self {
        self.cmd.arg("--json");
        self
    }

    /// Add --apply flag with an expression
    fn apply(mut self, expr: impl AsRef<str>) -> Self {
        self.cmd.args(["--apply", expr.as_ref()]);
        self
    }

    /// Add --no-link flag (for build commands)
    fn no_link(mut self) -> Self {
        self.cmd.arg("--no-link");
        self
    }

    /// Add --print-out-paths flag (for build commands)
    fn print_out_paths(mut self) -> Self {
        self.cmd.arg("--print-out-paths");
        self
    }

    /// Execute the command and return stdout on success
    fn run(mut self) -> Result<Vec<u8>> {
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
    }
}

/// A NixOS host from the flake
#[derive(Debug, Clone, Deserialize)]
pub struct Host {
    pub name: String,
}

/// Metadata about a host for deployment
#[derive(Debug, Clone)]
pub struct HostMetadata {
    pub name: String,
    pub phase: Option<String>,
    pub tags: Vec<String>,
    pub fqdn: String,
}

/// Nix store path for a built configuration
#[derive(Debug, Clone)]
pub struct StorePath(String);

impl StorePath {
    pub fn new(path: String) -> Self {
        Self(path)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// SSH deployment target
#[derive(Debug, Clone)]
pub struct DeployTarget {
    pub user: String,
    pub hostname: String,
    pub port: Option<u16>,
}

impl DeployTarget {
    pub fn ssh_destination(&self) -> String {
        format!("{}@{}", self.user, self.hostname)
    }

    pub fn ssh_url(&self) -> String {
        if let Some(port) = self.port {
            format!("ssh://{}@{}:{}", self.user, self.hostname, port)
        } else {
            format!("ssh://{}@{}", self.user, self.hostname)
        }
    }
}

/// Validate and resolve a flake path provided by the user
///
/// The path can be either:
/// - A directory containing flake.nix
/// - A direct path to flake.nix file
///
/// Supports shell expansions like ~ and environment variables.
fn validate_flake_path(path: &str) -> Result<PathBuf> {
    let expanded = shellexpand::full(path).map_err(|_| FlakeError::InvalidPath {
        path: PathBuf::from(path),
    })?;
    let path_buf = PathBuf::from(expanded.as_ref());

    if !path_buf.exists() {
        return Err(FlakeError::PathNotFound { path: path_buf }.into());
    }

    // If path points to flake.nix file, use its parent directory
    let flake_dir =
        if path_buf.is_file() && path_buf.file_name() == Some(std::ffi::OsStr::new("flake.nix")) {
            path_buf
                .parent()
                .ok_or_else(|| FlakeError::InvalidPath {
                    path: path_buf.clone(),
                })?
                .to_path_buf()
        } else if path_buf.is_dir() {
            // If it's a directory, verify flake.nix exists in it
            let flake_file = path_buf.join("flake.nix");
            if !flake_file.exists() {
                return Err(FlakeError::MissingInDir { path: path_buf }.into());
            }
            path_buf
        } else {
            return Err(FlakeError::InvalidPath { path: path_buf }.into());
        };

    Ok(flake_dir)
}

/// Find the flake root by searching up from the current directory
/// Returns the path to the directory containing flake.nix
fn find_flake_root() -> Result<PathBuf> {
    let current_dir = env::current_dir().map_err(|e| IoError::CurrentDir { source: e })?;

    find_flake_root_from(&current_dir)
}

/// Find the flake root by searching up from a given directory
fn find_flake_root_from(start: &Path) -> Result<PathBuf> {
    let mut current = start;

    loop {
        let flake_path = current.join("flake.nix");
        if flake_path.exists() {
            return Ok(current.to_path_buf());
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => {
                return Err(FlakeError::NotFound {
                    start_dir: start.to_path_buf(),
                }
                .into())
            }
        }
    }
}

/// Parse host names from nix JSON output
fn parse_host_names(json: &[u8]) -> Result<Vec<Host>> {
    let host_names: Vec<String> =
        serde_json::from_slice(json).map_err(|e| NixError::OutputParseFailed {
            output: String::from_utf8_lossy(json).to_string(),
            source: e,
        })?;

    let hosts = host_names.into_iter().map(|name| Host { name }).collect();

    Ok(hosts)
}

/// List all nixosConfigurations from the flake
/// If filter_expr is provided, it should be a Nix expression that takes a config and returns a boolean
/// Example: "config: config.features.backup.enable or false"
pub fn list_hosts(ctx: &SnowContext, filter_expr: Option<&str>) -> Result<Vec<Host>> {
    let apply_arg = if let Some(expr) = filter_expr {
        // Build a filter expression similar to find-hosts in justfile
        format!(
            r#"configs:
  let
    inherit (builtins) filter attrNames;
    predicate = (
      {}
    );
    matchingHosts = filter (hostname: predicate configs.${{hostname}}.config) (attrNames configs);
  in matchingHosts"#,
            expr
        )
    } else {
        "builtins.attrNames".to_string()
    };

    let stdout = NixCommand::eval(ctx, ".#nixosConfigurations")
        .apply(&apply_arg)
        .json()
        .run()?;

    parse_host_names(&stdout)
}

/// Get metadata for a host from its NixOS configuration
///
/// Convenience function that uses SystemNixOps under the hood.
pub fn get_host_metadata(ctx: &SnowContext, host: &str) -> Result<HostMetadata> {
    use ops::{NixOps, SystemNixOps};
    SystemNixOps::new().get_host_metadata(ctx, host)
}

/// Build a host configuration and return the store path
///
/// Convenience function that uses SystemNixOps under the hood.
pub fn build_host(ctx: &SnowContext, host: &str) -> Result<StorePath> {
    use ops::{NixOps, SystemNixOps};
    SystemNixOps::new().build_host(ctx, host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_host_names() {
        let json = br#"["hydrogen","nitrogen","oxygen"]"#;
        let hosts = parse_host_names(json).unwrap();

        assert_eq!(hosts.len(), 3);
        assert_eq!(hosts[0].name, "hydrogen");
        assert_eq!(hosts[1].name, "nitrogen");
        assert_eq!(hosts[2].name, "oxygen");
    }

    #[test]
    fn test_parse_empty_hosts() {
        let json = br#"[]"#;
        let hosts = parse_host_names(json).unwrap();
        assert_eq!(hosts.len(), 0);
    }

    #[test]
    fn test_parse_invalid_json() {
        let json = b"not valid json";
        let result = parse_host_names(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_deploy_target_ssh_url_with_port() {
        let target = DeployTarget {
            user: "deploy".to_string(),
            hostname: "hydrogen.example.com".to_string(),
            port: Some(2222),
        };
        assert_eq!(target.ssh_url(), "ssh://deploy@hydrogen.example.com:2222");
    }

    #[test]
    fn test_find_flake_root_in_current_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_path = temp_dir.path().join("flake.nix");
        fs::write(&flake_path, "# test flake").unwrap();

        let result = find_flake_root_from(temp_dir.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_find_flake_root_in_parent_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_path = temp_dir.path().join("flake.nix");
        fs::write(&flake_path, "# test flake").unwrap();

        let subdir = temp_dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();

        let result = find_flake_root_from(&subdir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_find_flake_root_not_found() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = find_flake_root_from(temp_dir.path());
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Could not find flake.nix"));
    }

    #[test]
    fn test_find_flake_root_multiple_levels() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_path = temp_dir.path().join("flake.nix");
        fs::write(&flake_path, "# test flake").unwrap();

        let deep_dir = temp_dir.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep_dir).unwrap();

        let result = find_flake_root_from(&deep_dir);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), temp_dir.path());
    }

    #[test]
    fn test_snow_context_with_valid_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_file = temp_dir.path().join("flake.nix");
        fs::write(&flake_file, "# test flake").unwrap();

        let path_str = temp_dir.path().to_str().unwrap();
        let result = SnowContext::new(Some(path_str));

        assert!(result.is_ok());
        assert_eq!(result.unwrap().flake_root(), temp_dir.path());
    }

    #[test]
    fn test_snow_context_with_nonexistent_path() {
        let result = SnowContext::new(Some("/nonexistent/path"));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("does not exist"));
    }

    #[test]
    fn test_snow_context_with_path_missing_flake() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path_str = temp_dir.path().to_str().unwrap();

        let result = SnowContext::new(Some(path_str));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("No flake.nix found"));
    }

    #[test]
    fn test_snow_context_with_flake_nix_file_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let flake_file = temp_dir.path().join("flake.nix");
        fs::write(&flake_file, "# test flake").unwrap();

        let file_path_str = flake_file.to_str().unwrap();
        let result = SnowContext::new(Some(file_path_str));

        assert!(result.is_ok());
        assert_eq!(result.unwrap().flake_root(), temp_dir.path());
    }

    #[test]
    fn test_list_hosts_without_filter() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");

        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();
        let result = list_hosts(&ctx, None);
        assert!(
            result.is_ok(),
            "list_hosts should succeed: {:?}",
            result.err()
        );

        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 4);

        let host_names: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
        assert!(host_names.contains(&"hydrogen".to_string()));
        assert!(host_names.contains(&"nitrogen".to_string()));
        assert!(host_names.contains(&"oxygen".to_string()));
        assert!(host_names.contains(&"helium".to_string()));
    }

    #[test]
    fn test_list_hosts_with_postgresql_filter() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        let result = list_hosts(
            &ctx,
            Some("config: config.services.postgresql.enable or false"),
        );
        assert!(
            result.is_ok(),
            "list_hosts with postgresql filter should succeed: {:?}",
            result.err()
        );

        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 2);

        let host_names: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
        assert!(host_names.contains(&"hydrogen".to_string()));
        assert!(host_names.contains(&"oxygen".to_string()));
    }

    #[test]
    fn test_list_hosts_with_nginx_filter() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        let result = list_hosts(&ctx, Some("config: config.services.nginx.enable or false"));
        assert!(
            result.is_ok(),
            "list_hosts with nginx filter should succeed: {:?}",
            result.err()
        );

        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 2);

        let host_names: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
        assert!(host_names.contains(&"hydrogen".to_string()));
        assert!(host_names.contains(&"nitrogen".to_string()));
    }

    #[test]
    fn test_list_hosts_with_architecture_filter() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        let result = list_hosts(
            &ctx,
            Some("config: (config.nixpkgs.hostPlatform or \"\") == \"x86_64-linux\""),
        );
        assert!(
            result.is_ok(),
            "list_hosts with architecture filter should succeed: {:?}",
            result.err()
        );

        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 2);

        let host_names: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
        assert!(host_names.contains(&"hydrogen".to_string()));
        assert!(host_names.contains(&"oxygen".to_string()));
    }

    #[test]
    fn test_list_hosts_with_no_matches_filter() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        // Filter that matches no hosts
        let result = list_hosts(&ctx, Some("config: false"));
        assert!(
            result.is_ok(),
            "list_hosts with false filter should succeed: {:?}",
            result.err()
        );

        let hosts = result.unwrap();
        assert_eq!(hosts.len(), 0);
    }

    #[test]
    fn test_list_hosts_with_invalid_syntax() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        // Invalid Nix syntax - missing closing brace
        let result = list_hosts(&ctx, Some("config: { invalid syntax"));
        assert!(
            result.is_err(),
            "list_hosts with invalid syntax should fail"
        );

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Failed to evaluate Nix expression"),
            "Error should mention Nix eval failure: {}",
            err_msg
        );
    }

    #[test]
    fn test_list_hosts_with_invalid_attribute_access() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        // Accessing non-existent attribute without 'or' default - should fail
        let result = list_hosts(
            &ctx,
            Some("config: config.nonexistent.deeply.nested.attribute"),
        );
        assert!(
            result.is_err(),
            "list_hosts with invalid attribute access should fail"
        );

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Failed to evaluate Nix expression"),
            "Error should mention Nix eval failure: {}",
            err_msg
        );
    }

    #[test]
    fn test_list_hosts_with_malformed_expression() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        // Expression that doesn't return a boolean
        let result = list_hosts(&ctx, Some("config: \"not a boolean\""));
        assert!(
            result.is_err(),
            "list_hosts with non-boolean expression should fail"
        );
    }

    #[test]
    fn test_get_host_metadata_with_full_config() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        let result = get_host_metadata(&ctx, "hydrogen");
        assert!(
            result.is_ok(),
            "get_host_metadata should succeed: {:?}",
            result.err()
        );

        let meta = result.unwrap();
        assert_eq!(meta.name, "hydrogen");
        assert_eq!(meta.fqdn, "hydrogen.example.com");
        assert_eq!(meta.phase, Some("production".to_string()));
        assert_eq!(meta.tags, vec!["web", "database"]);
    }

    #[test]
    fn test_get_host_metadata_with_empty_domain() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        let result = get_host_metadata(&ctx, "oxygen");
        assert!(result.is_ok());

        let meta = result.unwrap();
        assert_eq!(meta.name, "oxygen");
        assert_eq!(meta.fqdn, "oxygen"); // No domain, so FQDN = hostname
        assert_eq!(meta.phase, Some("production".to_string()));
        assert_eq!(meta.tags, vec!["database"]);
    }

    #[test]
    fn test_get_host_metadata_with_defaults() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        let result = get_host_metadata(&ctx, "helium");
        assert!(result.is_ok());

        let meta = result.unwrap();
        assert_eq!(meta.name, "helium");
        assert_eq!(meta.fqdn, "helium.local");
        assert_eq!(meta.phase, None); // No pushPhase defined
        assert_eq!(meta.tags.len(), 0); // No tags defined
    }

    #[test]
    fn test_get_host_metadata_nonexistent_host() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        let result = get_host_metadata(&ctx, "nonexistent");
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Failed to get metadata for host"),
            "Error should mention metadata failure: {}",
            err_msg
        );
    }

    #[test]
    fn test_build_host_success() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        let result = build_host(&ctx, "hydrogen");
        assert!(
            result.is_ok(),
            "build_host should succeed: {:?}",
            result.err()
        );

        let store_path = result.unwrap();
        // Should return a valid Nix store path containing the derivation name
        assert!(store_path.as_str().starts_with("/nix/store/"));
        assert!(store_path.as_str().contains("test-hydrogen-toplevel"));
    }

    #[test]
    fn test_build_host_different_hosts() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        // Test multiple hosts to ensure the path formatting works correctly
        let nitrogen_result = build_host(&ctx, "nitrogen");
        assert!(nitrogen_result.is_ok());
        let nitrogen_path = nitrogen_result.unwrap();
        assert!(nitrogen_path.as_str().starts_with("/nix/store/"));
        assert!(nitrogen_path.as_str().contains("test-nitrogen-toplevel"));

        let oxygen_result = build_host(&ctx, "oxygen");
        assert!(oxygen_result.is_ok());
        let oxygen_path = oxygen_result.unwrap();
        assert!(oxygen_path.as_str().starts_with("/nix/store/"));
        assert!(oxygen_path.as_str().contains("test-oxygen-toplevel"));
    }

    #[test]
    fn test_build_host_nonexistent() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path = PathBuf::from(manifest_dir).join("tests/fixtures");
        let ctx = SnowContext::new(Some(fixture_path.to_str().unwrap())).unwrap();

        let result = build_host(&ctx, "nonexistent");
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Failed to build host configuration for 'nonexistent'"),
            "Error should mention build failure: {}",
            err_msg
        );
    }
}
