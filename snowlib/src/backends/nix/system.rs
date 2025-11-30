use super::{parse_hostnames, NixBackend};
use async_trait::async_trait;

use crate::bootstrap::{BootstrapConfig, BootstrapConfigJson};
use crate::error::Result;
use crate::hooks::{HookConfig, HookLocation, HookStage};
use crate::host::{Host, HostMetadata};
use crate::nix::{Flake, NixCommand, StorePath, SystemType};
use crate::process::OutputMode;
use crate::reboot::RebootPolicy;
use std::str::FromStr;

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

#[async_trait]
impl NixBackend for SystemNixBackend {
    async fn list_hosts(
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
            .run()
            .await?;

        parse_hostnames(&stdout)
    }

    async fn get_host_metadata(
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
          hooks = config.config.snowglobe.hooks or {};
        }"#;

        let stdout = NixCommand::eval(flake, &flake_ref)
            .apply(apply_expr)
            .json()
            .run()
            .await
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
        struct HookConfigWithoutStage {
            location: HookLocation,
            command: String,
            #[serde(default)]
            environment: std::collections::HashMap<String, String>,
            #[serde(default, rename = "environmentFiles")]
            environment_files: Vec<std::path::PathBuf>,
            #[serde(default = "default_hook_timeout")]
            timeout: u64,
            #[serde(default, rename = "continueOnFailure")]
            continue_on_failure: bool,
            #[serde(default = "default_hook_priority")]
            priority: i32,
            #[serde(default)]
            retries: u32,
            #[serde(default = "default_retry_delay", rename = "retryDelay")]
            retry_delay: u64,
        }

        fn default_hook_timeout() -> u64 {
            300
        }

        fn default_hook_priority() -> i32 {
            100
        }

        fn default_retry_delay() -> u64 {
            5
        }

        #[derive(serde::Deserialize)]
        struct MetadataJson {
            #[serde(rename = "hostName")]
            hostname: String,
            // Domain can be null for hosts that connect to different networks (like laptops)
            // or when not configured in the NixOS/nix-darwin configuration
            domain: Option<String>,
            phase: Option<String>,
            tags: Vec<String>,
            #[serde(rename = "rebootPolicy")]
            reboot_policy: String,
            #[serde(default)]
            hooks: std::collections::HashMap<
                String,
                std::collections::HashMap<String, HookConfigWithoutStage>,
            >,
        }

        let meta: MetadataJson = serde_json::from_slice(&stdout).map_err(|e| {
            crate::error::NixError::OutputParseFailed {
                output: String::from_utf8_lossy(&stdout).to_string(),
                source: e,
            }
        })?;

        let fqdn = match meta.domain {
            Some(domain) if !domain.is_empty() => format!("{}.{}", meta.hostname, domain),
            _ => meta.hostname.clone(),
        };

        let reboot_policy = RebootPolicy::from_str(&meta.reboot_policy)?;

        // Convert hooks from stage-grouped HashMap to flat Vec
        let mut hooks = Vec::new();
        for (stage_str, stage_hooks) in meta.hooks {
            let stage = HookStage::from_str(&stage_str)?;
            for (hook_name, hook) in stage_hooks {
                hooks.push(HookConfig {
                    name: hook_name,
                    stage,
                    location: hook.location,
                    command: hook.command,
                    environment: hook.environment,
                    environment_files: hook.environment_files,
                    timeout: hook.timeout,
                    continue_on_failure: hook.continue_on_failure,
                    priority: hook.priority,
                    retries: hook.retries,
                    retry_delay: hook.retry_delay,
                });
            }
        }

        Ok(HostMetadata {
            name: host.to_string(),
            phase: meta.phase,
            tags: meta.tags,
            fqdn,
            reboot_policy,
            system_type,
            hooks,
        })
    }

    async fn build_host(
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
            .await
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

    async fn get_bootstrap_config(
        &self,
        flake: &Flake,
        host: &str,
        system_type: SystemType,
    ) -> Result<BootstrapConfig> {
        let flake_ref = format!(".#{}.{}", system_type.flake_attr(), host);

        let apply_expr = r#"config: {
          provisioner = config.config.snowglobe.bootstrap.provisioner or null;
          imageAttr = config.config.snowglobe.bootstrap.imageAttr or null;
          sshKeyPath = config.config.snowglobe.bootstrap.sshKeyPath or null;
        }"#;

        let stdout = NixCommand::eval(flake, &flake_ref)
            .apply(apply_expr)
            .json()
            .run()
            .await
            .map_err(|e| match e {
                crate::error::SnowError::Nix(nix_err) => {
                    crate::error::SnowError::Nix(crate::error::NixError::MetadataFailed {
                        host: host.to_string(),
                        source: Box::new(nix_err),
                    })
                }
                other => other,
            })?;

        let json_config: BootstrapConfigJson = serde_json::from_slice(&stdout).map_err(|e| {
            crate::error::NixError::OutputParseFailed {
                output: String::from_utf8_lossy(&stdout).to_string(),
                source: e,
            }
        })?;

        Ok(json_config.into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_metadata_json_with_null_domain() {
        let json = r#"{
            "hostName": "macbook",
            "domain": null,
            "phase": null,
            "tags": [],
            "rebootPolicy": "auto"
        }"#;

        #[derive(serde::Deserialize)]
        struct MetadataJson {
            #[serde(rename = "hostName")]
            hostname: String,
            domain: Option<String>,
            phase: Option<String>,
            tags: Vec<String>,
            #[serde(rename = "rebootPolicy")]
            reboot_policy: String,
        }

        let meta: MetadataJson = serde_json::from_str(json).unwrap();
        assert_eq!(meta.hostname, "macbook");
        assert_eq!(meta.domain, None);
        assert_eq!(meta.phase, None);
        assert_eq!(meta.tags, Vec::<String>::new());
        assert_eq!(meta.reboot_policy, "auto");

        let fqdn = match meta.domain {
            Some(domain) if !domain.is_empty() => format!("{}.{}", meta.hostname, domain),
            _ => meta.hostname.clone(),
        };
        assert_eq!(fqdn, "macbook");
    }

    #[test]
    fn test_metadata_json_with_empty_domain() {
        let json = r#"{
            "hostName": "server",
            "domain": "",
            "phase": "p1",
            "tags": ["web"],
            "rebootPolicy": "never"
        }"#;

        #[derive(serde::Deserialize)]
        struct MetadataJson {
            #[serde(rename = "hostName")]
            hostname: String,
            domain: Option<String>,
            phase: Option<String>,
            tags: Vec<String>,
            #[serde(rename = "rebootPolicy")]
            reboot_policy: String,
        }

        let meta: MetadataJson = serde_json::from_str(json).unwrap();
        assert_eq!(meta.hostname, "server");
        assert_eq!(meta.domain, Some("".to_string()));
        assert_eq!(meta.phase, Some("p1".to_string()));
        assert_eq!(meta.tags, vec!["web"]);
        assert_eq!(meta.reboot_policy, "never");

        let fqdn = match meta.domain {
            Some(domain) if !domain.is_empty() => format!("{}.{}", meta.hostname, domain),
            _ => meta.hostname.clone(),
        };
        assert_eq!(fqdn, "server");
    }

    #[test]
    fn test_metadata_json_with_valid_domain() {
        let json = r#"{
            "hostName": "web01",
            "domain": "example.com",
            "phase": "p2",
            "tags": ["mysql", "ldap"],
            "rebootPolicy": "always"
        }"#;

        #[derive(serde::Deserialize)]
        struct MetadataJson {
            #[serde(rename = "hostName")]
            hostname: String,
            domain: Option<String>,
            phase: Option<String>,
            tags: Vec<String>,
            #[serde(rename = "rebootPolicy")]
            reboot_policy: String,
        }

        let meta: MetadataJson = serde_json::from_str(json).unwrap();
        assert_eq!(meta.hostname, "web01");
        assert_eq!(meta.domain, Some("example.com".to_string()));
        assert_eq!(meta.phase, Some("p2".to_string()));
        assert_eq!(meta.tags, vec!["mysql", "ldap"]);
        assert_eq!(meta.reboot_policy, "always");

        let fqdn = match meta.domain {
            Some(domain) if !domain.is_empty() => format!("{}.{}", meta.hostname, domain),
            _ => meta.hostname.clone(),
        };
        assert_eq!(fqdn, "web01.example.com");
    }
}
