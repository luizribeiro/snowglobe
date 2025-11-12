mod command;
mod context;
mod queries;
mod types;

pub(crate) use command::NixCommand;
pub use context::SnowContext;
pub use queries::list_hosts;
pub use types::{Host, HostMetadata, RebootPolicy, StorePath};

use crate::error::Result;

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
}
