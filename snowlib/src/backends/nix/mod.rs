#[cfg(any(debug_assertions, test))]
mod mock;
mod system;

#[cfg(any(debug_assertions, test))]
pub use mock::MockNixBackend;
pub use system::SystemNixBackend;

use async_trait::async_trait;
use std::collections::HashSet;

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
        let nixos_hosts = self
            .list_hosts_if_present(flake, SystemType::NixOS, filter_expr)
            .await?;
        let darwin_hosts = self
            .list_hosts_if_present(flake, SystemType::Darwin, filter_expr)
            .await?;

        if nixos_hosts.is_none() && darwin_hosts.is_none() {
            return Err(crate::error::FlakeError::NotFound {
                start_dir: flake.flake_root().to_path_buf(),
            }
            .into());
        }

        Ok(nixos_hosts
            .into_iter()
            .chain(darwin_hosts)
            .flatten()
            .collect())
    }

    /// Like `list_hosts`, but `None` when the flake has no such configurations
    async fn list_hosts_if_present(
        &self,
        flake: &Flake,
        system_type: SystemType,
        filter_expr: Option<&str>,
    ) -> Result<Option<Vec<Host>>> {
        match self.list_hosts(flake, system_type, filter_expr).await {
            Ok(hosts) => Ok(Some(hosts)),
            Err(e) if e.to_string().contains("does not provide attribute") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Hosts declared in nixosConfigurations load as NixOS, all others as
    /// nix-darwin. Errors propagate.
    async fn load_host_metadata(
        &self,
        flake: &Flake,
        hostnames: &[String],
    ) -> Result<Vec<HostMetadata>> {
        let nixos_hosts: HashSet<String> = self
            .list_hosts_if_present(flake, SystemType::NixOS, None)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(|host| host.name)
            .collect();

        let mut metadata = Vec::new();
        for name in hostnames {
            let system_type = if nixos_hosts.contains(name) {
                SystemType::NixOS
            } else {
                SystemType::Darwin
            };
            metadata.push(self.get_host_metadata(flake, name, system_type).await?);
        }
        Ok(metadata)
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

    /// A flake with the given NixOS and nix-darwin hosts. Loading a host as
    /// NixOS fails for hosts in `broken_as_nixos`; `listing_fails` makes every
    /// host listing fail with an unrelated eval error.
    #[derive(Default)]
    struct FakeFlake {
        nixos: Option<Vec<&'static str>>,
        darwin: Option<Vec<&'static str>>,
        broken_as_nixos: Vec<&'static str>,
        listing_fails: bool,
    }

    fn missing_attr(system_type: SystemType) -> crate::error::SnowError {
        NixError::EvalFailed {
            stderr: format!(
                "error: flake does not provide attribute '{}'",
                system_type.flake_attr()
            ),
            source: std::io::Error::other("eval failed"),
        }
        .into()
    }

    #[async_trait]
    impl NixBackend for FakeFlake {
        async fn list_hosts(
            &self,
            _flake: &Flake,
            system_type: SystemType,
            _filter_expr: Option<&str>,
        ) -> Result<Vec<Host>> {
            if self.listing_fails {
                return Err(NixError::EvalFailed {
                    stderr: "error: undefined variable 'pkgz'".to_string(),
                    source: std::io::Error::other("eval failed"),
                }
                .into());
            }
            let hosts = match system_type {
                SystemType::NixOS => &self.nixos,
                SystemType::Darwin => &self.darwin,
            };
            hosts
                .as_ref()
                .map(|names| {
                    names
                        .iter()
                        .map(|n| Host {
                            name: n.to_string(),
                        })
                        .collect()
                })
                .ok_or_else(|| missing_attr(system_type))
        }

        async fn get_host_metadata(
            &self,
            _flake: &Flake,
            host: &str,
            system_type: SystemType,
        ) -> Result<HostMetadata> {
            if system_type == SystemType::NixOS && self.broken_as_nixos.contains(&host) {
                return Err(NixError::BuildFailed {
                    host: host.to_string(),
                    stderr: "hooks file failed to build".to_string(),
                    source: std::io::Error::other("build failed"),
                }
                .into());
            }
            Ok(HostMetadata {
                name: host.to_string(),
                phase: None,
                tags: vec![],
                fqdn: host.to_string(),
                reboot_policy: crate::reboot::RebootPolicy::Auto,
                system_type,
                hooks: vec![],
                serial_group: None,
            })
        }

        async fn build_host(
            &self,
            _: &Flake,
            _: &str,
            _: SystemType,
            _: OutputMode,
        ) -> Result<StorePath> {
            unimplemented!()
        }

        async fn get_bootstrap_config(
            &self,
            _: &Flake,
            _: &str,
            _: SystemType,
        ) -> Result<BootstrapConfig> {
            unimplemented!()
        }

        async fn build_image(
            &self,
            _: &Flake,
            _: &str,
            _: SystemType,
            _: &str,
            _: OutputMode,
        ) -> Result<StorePath> {
            unimplemented!()
        }
    }

    fn names(hosts: &[&str]) -> Vec<String> {
        hosts.iter().map(|h| h.to_string()).collect()
    }

    #[tokio::test]
    async fn test_load_host_metadata_picks_system_type_from_flake() {
        let flake = FakeFlake {
            nixos: Some(vec!["praseodymium", "curium"]),
            darwin: Some(vec!["ferrum"]),
            ..FakeFlake::default()
        };

        let metadata = flake
            .load_host_metadata(
                &Flake::test_context(),
                &names(&["praseodymium", "ferrum", "curium"]),
            )
            .await
            .unwrap();

        let types: Vec<_> = metadata
            .iter()
            .map(|m| (m.name.as_str(), m.system_type))
            .collect();
        assert_eq!(
            types,
            vec![
                ("praseodymium", SystemType::NixOS),
                ("ferrum", SystemType::Darwin),
                ("curium", SystemType::NixOS),
            ]
        );
    }

    #[tokio::test]
    async fn test_load_host_metadata_reports_errors_instead_of_trying_darwin() {
        let flake = FakeFlake {
            nixos: Some(vec!["praseodymium"]),
            darwin: Some(vec!["ferrum"]),
            broken_as_nixos: vec!["praseodymium"],
            ..FakeFlake::default()
        };

        let err = flake
            .load_host_metadata(&Flake::test_context(), &names(&["praseodymium"]))
            .await
            .unwrap_err();

        assert!(
            format!("{:?}", err).contains("hooks file failed to build"),
            "{:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_load_host_metadata_without_nixos_configurations() {
        let flake = FakeFlake {
            darwin: Some(vec!["ferrum"]),
            ..FakeFlake::default()
        };

        let metadata = flake
            .load_host_metadata(&Flake::test_context(), &names(&["ferrum"]))
            .await
            .unwrap();

        assert_eq!(metadata[0].system_type, SystemType::Darwin);
    }

    #[tokio::test]
    async fn test_unrelated_listing_errors_propagate() {
        let flake = FakeFlake {
            listing_fails: true,
            ..FakeFlake::default()
        };
        let context = Flake::test_context();

        assert!(flake
            .load_host_metadata(&context, &names(&["ferrum"]))
            .await
            .is_err());
        assert!(flake.list_all_hosts(&context, None).await.is_err());
    }

    #[tokio::test]
    async fn test_list_all_hosts_combines_present_configuration_sets() {
        let both = FakeFlake {
            nixos: Some(vec!["praseodymium"]),
            darwin: Some(vec!["ferrum"]),
            ..FakeFlake::default()
        };
        let darwin_only = FakeFlake {
            darwin: Some(vec!["ferrum"]),
            ..FakeFlake::default()
        };
        let flake = Flake::test_context();

        let names_of = |hosts: Vec<Host>| hosts.into_iter().map(|h| h.name).collect::<Vec<_>>();
        assert_eq!(
            names_of(both.list_all_hosts(&flake, None).await.unwrap()),
            ["praseodymium", "ferrum"]
        );
        assert_eq!(
            names_of(darwin_only.list_all_hosts(&flake, None).await.unwrap()),
            ["ferrum"]
        );
        assert!(matches!(
            FakeFlake::default().list_all_hosts(&flake, None).await,
            Err(crate::error::SnowError::Flake(
                crate::error::FlakeError::NotFound { .. }
            ))
        ));
    }

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
