use crate::error::Result;
use crate::nix::{HostMetadata, NixCommand, SnowContext, StorePath};

#[cfg(debug_assertions)]
pub mod mock;

/// Trait for Nix operations (build, metadata retrieval)
///
/// Allows swapping between real Nix commands and mocked versions for testing.
pub trait NixOps: Send + Sync {
    /// Get metadata for a host from its NixOS configuration
    fn get_host_metadata(&self, ctx: &SnowContext, host: &str) -> Result<HostMetadata>;

    /// Build a host's NixOS configuration and return the store path
    fn build_host(&self, ctx: &SnowContext, host: &str) -> Result<StorePath>;
}

/// System implementation of NixOps that executes real Nix commands
///
/// This is the production implementation that invokes `nix eval` and `nix build`
/// to interact with the actual Nix system.
#[derive(Debug, Clone)]
pub struct SystemNixOps;

impl SystemNixOps {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SystemNixOps {
    fn default() -> Self {
        Self::new()
    }
}

impl NixOps for SystemNixOps {
    fn get_host_metadata(&self, ctx: &SnowContext, host: &str) -> Result<HostMetadata> {
        let config_path = format!(".#nixosConfigurations.{}.config", host);

        // Build a Nix expression that evaluates all metadata at once
        let eval_expr = format!(
            r#"let config = ({}).config; in {{
          hostName = config.networking.hostName;
          domain = config.networking.domain or "";
          phase = config.core.pushPhase or null;
          tags = config.core.tags or [];
        }}"#,
            config_path
        );

        let stdout = NixCommand::eval_expr(ctx, &eval_expr)
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

        Ok(HostMetadata {
            name: host.to_string(),
            phase: meta.phase,
            tags: meta.tags,
            fqdn,
        })
    }

    fn build_host(&self, ctx: &SnowContext, host: &str) -> Result<StorePath> {
        let target = format!(
            ".#nixosConfigurations.{}.config.system.build.toplevel",
            host
        );

        let stdout = NixCommand::build(ctx, &target)
            .no_link()
            .print_out_paths()
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
