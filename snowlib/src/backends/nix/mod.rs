#[cfg(any(debug_assertions, test))]
mod mock;
mod system;

#[cfg(any(debug_assertions, test))]
pub use mock::MockNixBackend;
pub use system::SystemNixBackend;

use async_trait::async_trait;

use crate::bootstrap::BootstrapConfig;
use crate::error::{NixError, Result};
use crate::host::{Host, HostMetadata};
use crate::nix::{Flake, StorePath, SystemType};
use crate::process::OutputMode;

/// Parse hostnames from nix JSON output
pub(crate) fn parse_hostnames(json: &[u8]) -> Result<Vec<Host>> {
    let hostnames: Vec<String> =
        serde_json::from_slice(json).map_err(|e| NixError::OutputParseFailed {
            output: String::from_utf8_lossy(json).to_string(),
            source: e,
        })?;

    let hosts = hostnames.into_iter().map(|name| Host { name }).collect();

    Ok(hosts)
}

/// Trait for Nix backend operations (evaluation, building)
///
/// Allows swapping between real Nix commands and mocked versions for testing.
#[async_trait]
pub trait NixBackend: Send + Sync {
    /// List all configurations from the flake (nixosConfigurations or darwinConfigurations)
    ///
    /// If filter_expr is provided, it should be a Nix expression that takes a config and returns a boolean.
    /// Example: "config: config.features.backup.enable or false"
    async fn list_hosts(
        &self,
        flake: &Flake,
        system_type: SystemType,
        filter_expr: Option<&str>,
    ) -> Result<Vec<Host>>;

    /// List all configurations from both nixosConfigurations and darwinConfigurations
    ///
    /// Tries both configuration types and combines the results. If one type doesn't exist,
    /// ignores it and returns hosts from the other type. However, actual errors (like invalid
    /// filter syntax) are propagated.
    async fn list_all_hosts(&self, flake: &Flake, filter_expr: Option<&str>) -> Result<Vec<Host>> {
        let mut all_hosts = Vec::new();
        let mut has_valid_result = false;

        match self.list_hosts(flake, SystemType::NixOS, filter_expr).await {
            Ok(nixos_hosts) => {
                all_hosts.extend(nixos_hosts);
                has_valid_result = true;
            }
            Err(e) => {
                // Only ignore if the configuration type doesn't exist
                let err_str = e.to_string();
                if !err_str.contains("does not provide attribute") {
                    return Err(e);
                }
            }
        }

        match self
            .list_hosts(flake, SystemType::Darwin, filter_expr)
            .await
        {
            Ok(darwin_hosts) => {
                all_hosts.extend(darwin_hosts);
                has_valid_result = true;
            }
            Err(e) => {
                let err_str = e.to_string();
                if !err_str.contains("does not provide attribute") {
                    return Err(e);
                }
            }
        }

        if !has_valid_result {
            return Err(crate::error::FlakeError::NotFound {
                start_dir: flake.flake_root().to_path_buf(),
            }
            .into());
        }

        Ok(all_hosts)
    }

    /// Get metadata for a host from its configuration
    async fn get_host_metadata(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
    ) -> Result<HostMetadata>;

    /// Build a host's configuration and return the store path
    async fn build_host(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
        output_mode: OutputMode,
    ) -> Result<StorePath>;

    /// Get bootstrap configuration for a host
    ///
    /// Evaluates the `snowglobe.bootstrap` attribute from the host's NixOS configuration.
    /// Returns a default config (manual provisioner) if the attribute is not set.
    async fn get_bootstrap_config(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
    ) -> Result<BootstrapConfig>;

    /// Build a host's image and return the store path
    ///
    /// Unlike `build_host` which builds the system toplevel, this builds a custom
    /// attribute specified by `image_attr` (e.g., "config.system.build.sdImage").
    async fn build_image(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
        image_attr: &str,
        output_mode: OutputMode,
    ) -> Result<StorePath>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hostnames_valid() {
        let json = br#"["host1", "host2", "host3"]"#;
        let hosts = parse_hostnames(json).unwrap();

        assert_eq!(hosts.len(), 3);
        assert_eq!(hosts[0].name, "host1");
        assert_eq!(hosts[1].name, "host2");
        assert_eq!(hosts[2].name, "host3");
    }

    #[test]
    fn test_parse_hostnames_empty() {
        let json = br#"[]"#;
        let hosts = parse_hostnames(json).unwrap();
        assert_eq!(hosts.len(), 0);
    }

    #[test]
    fn test_parse_hostnames_invalid_json() {
        let json = b"not json";
        let result = parse_hostnames(json);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::error::SnowError::Nix(NixError::OutputParseFailed { .. })
        ));
    }

    #[tokio::test]
    async fn test_mock_build_image_returns_store_path() {
        let backend = MockNixBackend::new();
        let flake = Flake::test_context();
        let result = backend
            .build_image(
                &flake,
                "testhost",
                SystemType::NixOS,
                "config.system.build.sdImage",
                OutputMode::Capture,
            )
            .await
            .unwrap();

        assert!(result.as_str().contains("testhost"));
        assert!(result.as_str().contains("sdImage"));
    }

    #[tokio::test]
    async fn test_mock_build_image_extracts_last_attr_component() {
        let backend = MockNixBackend::new();
        let flake = Flake::test_context();

        let result = backend
            .build_image(
                &flake,
                "myhost",
                SystemType::NixOS,
                "config.system.build.rawImage",
                OutputMode::Capture,
            )
            .await
            .unwrap();

        assert_eq!(result.as_str(), "/nix/store/mock-myhost-rawImage");
    }

    #[tokio::test]
    async fn test_mock_build_image_respects_build_failures() {
        let backend = MockNixBackend::new().with_build_failures(vec!["failhost".to_string()]);
        let flake = Flake::test_context();

        let result = backend
            .build_image(
                &flake,
                "failhost",
                SystemType::NixOS,
                "config.system.build.sdImage",
                OutputMode::Capture,
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            crate::error::SnowError::Provisioner(
                crate::error::ProvisionerError::ImageBuildFailed { .. }
            )
        ));
    }

    #[tokio::test]
    async fn test_mock_build_image_success_for_non_failed_host() {
        let backend = MockNixBackend::new().with_build_failures(vec!["failhost".to_string()]);
        let flake = Flake::test_context();

        let result = backend
            .build_image(
                &flake,
                "successhost",
                SystemType::NixOS,
                "config.system.build.sdImage",
                OutputMode::Capture,
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_mock_get_bootstrap_config() {
        let backend = MockNixBackend::new();
        let flake = Flake::test_context();

        let config = backend
            .get_bootstrap_config(&flake, "testhost", SystemType::NixOS)
            .await
            .unwrap();

        assert_eq!(config.provisioner_name(), "mock");
        assert_eq!(config.ssh_key_path(), "/etc/ssh");
        assert!(config.has_image_attr());
    }
}
