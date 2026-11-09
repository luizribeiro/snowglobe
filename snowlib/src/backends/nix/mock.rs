use super::NixBackend;
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Duration;

use crate::backends::deploy::{BuildFixture, DeploymentFixture, MetadataFixture};
use crate::bootstrap::BootstrapConfig;
use crate::error::Result;
use crate::host::{Host, HostMetadata};
use crate::nix::{Flake, StorePath, SystemType};
use crate::process::OutputMode;
use crate::reboot::RebootPolicy;

/// Mock implementation of NixBackend that uses fixtures instead of real Nix commands
///
/// This allows testing the entire deployment pipeline without requiring Nix infrastructure.
/// Fixtures can specify custom metadata, build successes/failures, timing, and output.
#[derive(Debug, Clone)]
pub struct MockNixBackend {
    fixture: Option<DeploymentFixture>,
    build_failures: Vec<String>,
    /// Extra build time per host, to control the order hosts become ready in tests
    build_delays: HashMap<String, Duration>,
}

impl MockNixBackend {
    /// Create a new MockNixBackend with no fixture (uses defaults)
    pub fn new() -> Self {
        Self {
            fixture: None,
            build_failures: Vec::new(),
            build_delays: HashMap::new(),
        }
    }

    /// Create a MockNixBackend from a deployment fixture
    pub fn from_fixture(fixture: DeploymentFixture) -> Self {
        Self {
            fixture: Some(fixture),
            build_failures: Vec::new(),
            build_delays: HashMap::new(),
        }
    }

    /// Configure hosts that should fail during build
    pub fn with_build_failures(mut self, failures: Vec<String>) -> Self {
        self.build_failures = failures;
        self
    }

    pub fn with_build_delays(mut self, delays: HashMap<String, Duration>) -> Self {
        self.build_delays = delays;
        self
    }

    /// Get metadata fixture for a host
    fn get_metadata_fixture(&self, host: &str) -> Option<&MetadataFixture> {
        self.fixture.as_ref()?.get_metadata(host)
    }

    /// Get build fixture for a host
    fn get_build_fixture(&self, host: &str) -> Option<&BuildFixture> {
        self.fixture.as_ref()?.get_build(host)
    }
}

impl Default for MockNixBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NixBackend for MockNixBackend {
    async fn list_hosts(
        &self,
        _ctx: &Flake,
        _system_type: SystemType,
        _filter_expr: Option<&str>,
    ) -> Result<Vec<Host>> {
        if let Some(fixture) = &self.fixture {
            let hosts = fixture
                .hosts
                .iter()
                .map(|h| Host {
                    name: h.hostname.clone(),
                })
                .collect();
            Ok(hosts)
        } else {
            Ok(vec![])
        }
    }

    async fn get_host_metadata(
        &self,
        _flake: &Flake,
        host: &str,
        system_type: SystemType,
    ) -> Result<HostMetadata> {
        if let Some(meta_fix) = self.get_metadata_fixture(host) {
            Ok(HostMetadata {
                name: host.to_string(),
                phase: meta_fix.phase.clone(),
                tags: meta_fix.tags.clone(),
                fqdn: meta_fix.fqdn.clone(),
                reboot_policy: RebootPolicy::Auto,
                system_type,
                system: None,
                serial_group: None,
                ssh_port: None,
                optional: false,
                ignored_failed_units: Vec::new(),
                hooks: Vec::new(),
            })
        } else {
            Ok(HostMetadata {
                name: host.to_string(),
                phase: Some("production".to_string()),
                tags: vec![],
                fqdn: format!("{}.example.com", host),
                reboot_policy: RebootPolicy::Auto,
                system_type,
                system: None,
                serial_group: None,
                ssh_port: None,
                optional: false,
                ignored_failed_units: Vec::new(),
                hooks: Vec::new(),
            })
        }
    }

    async fn build_host(
        &self,
        _flake: &Flake,
        host: &str,
        _system_type: SystemType,
        _output_mode: OutputMode,
    ) -> Result<StorePath> {
        if let Some(delay) = self.build_delays.get(host) {
            tokio::time::sleep(*delay).await;
        }

        // Check if this host should fail
        if self.build_failures.contains(&host.to_string()) {
            return Err(crate::error::NixError::BuildFailed {
                host: host.to_string(),
                stderr: "Mock build failure".to_string(),
            }
            .into());
        }

        if let Some(build_fix) = self.get_build_fixture(host) {
            if !cfg!(debug_assertions) {
                tokio::time::sleep(Duration::from_millis(build_fix.duration_ms)).await;
            }

            // nix build prints only the built path on stdout; what it says
            // along the way goes to stderr
            eprint!("{}{}", build_fix.stdout, build_fix.stderr);

            if build_fix.success {
                // Use store path from fixture or generate fake one
                let store_path = build_fix
                    .store_path
                    .clone()
                    .unwrap_or_else(|| format!("/nix/store/mock-{}", host));
                Ok(StorePath::new(store_path))
            } else {
                Err(crate::error::NixError::BuildFailed {
                    host: host.to_string(),
                    stderr: build_fix.stderr.clone(),
                }
                .into())
            }
        } else {
            // No build fixture - just create a fake store path
            Ok(StorePath::new(format!("/nix/store/mock-{}", host)))
        }
    }

    async fn get_bootstrap_config(
        &self,
        _flake: &Flake,
        _host: &str,
        _system_type: SystemType,
    ) -> Result<BootstrapConfig> {
        Ok(BootstrapConfig {
            provisioner: Some("mock".to_string()),
            image_attr: Some("config.system.build.image".to_string()),
            ssh_key_path: None,
            root_fs_device: Some("/dev/disk/by-label/nixos".to_string()),
        })
    }

    async fn build_image(
        &self,
        _flake: &Flake,
        host: &str,
        _system_type: SystemType,
        image_attr: &str,
        _output_mode: OutputMode,
    ) -> Result<StorePath> {
        // Check if this host should fail during build
        if self.build_failures.contains(&host.to_string()) {
            return Err(crate::error::ProvisionerError::ImageBuildFailed {
                host: host.to_string(),
                stderr: "Mock image build failure".to_string(),
            }
            .into());
        }

        // Return a fake store path based on the image attribute
        let attr_suffix = image_attr.split('.').next_back().unwrap_or("image");
        Ok(StorePath::new(format!(
            "/nix/store/mock-{}-{}",
            host, attr_suffix
        )))
    }
}
