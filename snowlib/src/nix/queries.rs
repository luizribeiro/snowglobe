//! Query functions for discovering and filtering NixOS hosts

use super::{Host, NixCommand, SnowContext};
use crate::error::{NixError, Result};

/// Parse hostnames from nix JSON output
fn parse_hostnames(json: &[u8]) -> Result<Vec<Host>> {
    let hostnames: Vec<String> =
        serde_json::from_slice(json).map_err(|e| NixError::OutputParseFailed {
            output: String::from_utf8_lossy(json).to_string(),
            source: e,
        })?;

    let hosts = hostnames.into_iter().map(|name| Host { name }).collect();

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

    parse_hostnames(&stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_hostnames() {
        let json = br#"["hydrogen","nitrogen","oxygen"]"#;
        let hosts = parse_hostnames(json).unwrap();

        assert_eq!(hosts.len(), 3);
        assert_eq!(hosts[0].name, "hydrogen");
        assert_eq!(hosts[1].name, "nitrogen");
        assert_eq!(hosts[2].name, "oxygen");
    }

    #[test]
    fn test_parse_empty_hosts() {
        let json = br#"[]"#;
        let hosts = parse_hostnames(json).unwrap();
        assert_eq!(hosts.len(), 0);
    }

    #[test]
    fn test_parse_invalid_json() {
        let json = b"not valid json";
        let result = parse_hostnames(json);
        assert!(result.is_err());
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

        let hostnames: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
        assert!(hostnames.contains(&"hydrogen".to_string()));
        assert!(hostnames.contains(&"nitrogen".to_string()));
        assert!(hostnames.contains(&"oxygen".to_string()));
        assert!(hostnames.contains(&"helium".to_string()));
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

        let hostnames: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
        assert!(hostnames.contains(&"hydrogen".to_string()));
        assert!(hostnames.contains(&"nitrogen".to_string()));
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
}
