mod command;
mod context;
mod types;

pub(crate) use command::NixCommand;
pub use context::SnowContext;
pub use types::{Host, HostMetadata, RebootPolicy, StorePath};

use crate::error::Result;

/// List all nixosConfigurations from the flake
///
/// If filter_expr is provided, it should be a Nix expression that takes a config and returns a boolean.
/// Example: "config: config.features.backup.enable or false"
///
/// Convenience function that uses SystemNixBackend under the hood.
pub fn list_hosts(ctx: &SnowContext, filter_expr: Option<&str>) -> Result<Vec<Host>> {
    use crate::backends::nix::{NixBackend, SystemNixBackend};
    SystemNixBackend::new().list_hosts(ctx, filter_expr)
}

/// Get metadata for a host from its NixOS configuration
///
/// Convenience function that uses SystemNixBackend under the hood.
pub fn get_host_metadata(ctx: &SnowContext, host: &str) -> Result<HostMetadata> {
    use crate::backends::nix::{NixBackend, SystemNixBackend};
    SystemNixBackend::new().get_host_metadata(ctx, host)
}

/// Build a host configuration and return the store path
///
/// Convenience function that uses SystemNixBackend under the hood.
pub fn build_host(ctx: &SnowContext, host: &str, quiet: bool) -> Result<StorePath> {
    use crate::backends::nix::{NixBackend, SystemNixBackend};
    SystemNixBackend::new().build_host(ctx, host, quiet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

        let result = build_host(&ctx, "hydrogen", false);
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
        let nitrogen_result = build_host(&ctx, "nitrogen", false);
        assert!(nitrogen_result.is_ok());
        let nitrogen_path = nitrogen_result.unwrap();
        assert!(nitrogen_path.as_str().starts_with("/nix/store/"));
        assert!(nitrogen_path.as_str().contains("test-nitrogen-toplevel"));

        let oxygen_result = build_host(&ctx, "oxygen", false);
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

        let result = build_host(&ctx, "nonexistent", false);
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Failed to build host configuration for 'nonexistent'"),
            "Error should mention build failure: {}",
            err_msg
        );
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
