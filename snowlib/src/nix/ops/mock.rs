use crate::deploy::mock::{BuildFixture, DeploymentFixture, MetadataFixture};
use crate::error::Result;
use crate::nix::ops::NixOps;
use crate::nix::{HostMetadata, SnowContext, StorePath};
use std::thread;
use std::time::Duration;

/// Mock implementation of NixOps that uses fixtures instead of real Nix commands
///
/// This allows testing the entire deployment pipeline without requiring Nix infrastructure.
/// Fixtures can specify custom metadata, build successes/failures, timing, and output.
#[derive(Debug, Clone)]
pub struct MockNixOps {
    fixture: Option<DeploymentFixture>,
}

impl MockNixOps {
    /// Create a new MockNixOps with no fixture (uses defaults)
    pub fn new() -> Self {
        Self { fixture: None }
    }

    /// Create a MockNixOps from a deployment fixture
    pub fn from_fixture(fixture: DeploymentFixture) -> Self {
        Self {
            fixture: Some(fixture),
        }
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

impl Default for MockNixOps {
    fn default() -> Self {
        Self::new()
    }
}

impl NixOps for MockNixOps {
    fn get_host_metadata(&self, _ctx: &SnowContext, host: &str) -> Result<HostMetadata> {
        if let Some(meta_fix) = self.get_metadata_fixture(host) {
            Ok(HostMetadata {
                name: host.to_string(),
                phase: meta_fix.phase.clone(),
                tags: meta_fix.tags.clone(),
                fqdn: meta_fix.fqdn.clone(),
            })
        } else {
            // Default mock metadata
            Ok(HostMetadata {
                name: host.to_string(),
                phase: Some("production".to_string()),
                tags: vec![],
                fqdn: format!("{}.example.com", host),
            })
        }
    }

    fn build_host(&self, _ctx: &SnowContext, host: &str) -> Result<StorePath> {
        if let Some(build_fix) = self.get_build_fixture(host) {
            // Simulate build timing (only when not running tests)
            print!("Building {}... ", host);
            if !cfg!(test) {
                thread::sleep(Duration::from_millis(build_fix.duration_ms));
            }

            // Print output
            if !build_fix.stdout.is_empty() {
                print!("{}", build_fix.stdout);
            }
            if !build_fix.stderr.is_empty() {
                eprint!("{}", build_fix.stderr);
            }

            if build_fix.success {
                println!("✓");
                // Use store path from fixture or generate fake one
                let store_path = build_fix
                    .store_path
                    .clone()
                    .unwrap_or_else(|| format!("/nix/store/mock-{}", host));
                Ok(StorePath::new(store_path))
            } else {
                println!("✗ Build failed");
                Err(crate::error::NixError::BuildFailed {
                    host: host.to_string(),
                    stderr: build_fix.stderr.clone(),
                    source: std::io::Error::other("Build failed (from fixture)"),
                }
                .into())
            }
        } else {
            // No build fixture - just create a fake store path
            Ok(StorePath::new(format!("/nix/store/mock-{}", host)))
        }
    }
}
