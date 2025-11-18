use crate::error::{NixError, Result};
use crate::nix::{Flake, Host, HostMetadata, NixCommand, StorePath, SystemType};
use std::str::FromStr;

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

use crate::process::OutputMode;

/// Trait for Nix backend operations (evaluation, building)
///
/// Allows swapping between real Nix commands and mocked versions for testing.
pub trait NixBackend: Send + Sync {
    /// List all configurations from the flake (nixosConfigurations or darwinConfigurations)
    ///
    /// If filter_expr is provided, it should be a Nix expression that takes a config and returns a boolean.
    /// Example: "config: config.features.backup.enable or false"
    fn list_hosts(
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
    fn list_all_hosts(&self, flake: &Flake, filter_expr: Option<&str>) -> Result<Vec<Host>> {
        let mut all_hosts = Vec::new();
        let mut has_valid_result = false;

        match self.list_hosts(flake, SystemType::NixOS, filter_expr) {
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

        match self.list_hosts(flake, SystemType::Darwin, filter_expr) {
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
    fn get_host_metadata(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
    ) -> Result<HostMetadata>;

    /// Build a host's configuration and return the store path
    fn build_host(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
        output_mode: OutputMode,
    ) -> Result<StorePath>;
}

/// System implementation of NixBackend that executes real Nix commands
///
/// This is the production implementation that invokes `nix eval` and `nix build`
/// to interact with the actual Nix system.
#[derive(Debug, Clone)]
pub struct SystemNixBackend;

impl SystemNixBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SystemNixBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NixBackend for SystemNixBackend {
    fn list_hosts(
        &self,
        flake: &Flake,
        system_type: SystemType,
        filter_expr: Option<&str>,
    ) -> Result<Vec<Host>> {
        let apply_arg = if let Some(expr) = filter_expr {
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

        let flake_ref = format!(".#{}", system_type.flake_attr());
        let stdout = NixCommand::eval(flake, &flake_ref)
            .apply(&apply_arg)
            .json()
            .run()?;

        parse_hostnames(&stdout)
    }

    fn get_host_metadata(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
    ) -> Result<HostMetadata> {
        let flake_ref = format!(".#{}.{}", system_type.flake_attr(), host);

        let apply_expr = r#"config: {
          hostName = config.config.networking.hostName;
          domain = config.config.networking.domain or "";
          phase = config.config.core.pushPhase or null;
          tags = config.config.core.tags or [];
          rebootPolicy = config.config.core.rebootPolicy or "auto";
        }"#;

        let stdout = NixCommand::eval(flake, &flake_ref)
            .apply(apply_expr)
            .json()
            .run()
            .map_err(|e| match e {
                crate::error::SnowError::Nix(nix_err) => {
                    crate::error::SnowError::Nix(crate::error::NixError::MetadataFailed {
                        host: host.to_string(),
                        source: Box::new(nix_err),
                    })
                }
                other => other,
            })?;

        #[derive(serde::Deserialize)]
        struct MetadataJson {
            #[serde(rename = "hostName")]
            hostname: String,
            domain: String,
            phase: Option<String>,
            tags: Vec<String>,
            #[serde(rename = "rebootPolicy")]
            reboot_policy: String,
        }

        let meta: MetadataJson = serde_json::from_slice(&stdout).map_err(|e| {
            crate::error::NixError::OutputParseFailed {
                output: String::from_utf8_lossy(&stdout).to_string(),
                source: e,
            }
        })?;

        let fqdn = if meta.domain.is_empty() {
            meta.hostname.clone()
        } else {
            format!("{}.{}", meta.hostname, meta.domain)
        };

        let reboot_policy = crate::nix::RebootPolicy::from_str(&meta.reboot_policy)?;

        Ok(HostMetadata {
            name: host.to_string(),
            phase: meta.phase,
            tags: meta.tags,
            fqdn,
            reboot_policy,
            system_type,
        })
    }

    fn build_host(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
        output_mode: OutputMode,
    ) -> Result<StorePath> {
        let target = format!(
            ".#{}.{}.{}",
            system_type.flake_attr(),
            host,
            system_type.build_path_suffix()
        );

        let stdout = NixCommand::build(flake, &target)
            .no_link()
            .print_out_paths()
            .output_mode(output_mode)
            .run()
            .map_err(|e| match e {
                crate::error::SnowError::Nix(crate::error::NixError::EvalFailed {
                    stderr,
                    source,
                }) => crate::error::SnowError::Nix(crate::error::NixError::BuildFailed {
                    host: host.to_string(),
                    stderr,
                    source,
                }),
                other => other,
            })?;

        let path = String::from_utf8(stdout)
            .map_err(|e| crate::error::NixError::EvalFailed {
                stderr: format!("Non-UTF8 build path: {:?}", e.as_bytes()),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid UTF-8"),
            })?
            .trim()
            .to_string();

        Ok(StorePath::new(path))
    }
}
