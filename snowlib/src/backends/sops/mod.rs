//! SOPS integration backend for managing secrets configuration
//!
//! Provides functionality to update .sops.yaml with new host age keys
//! and run `sops updatekeys` on affected secret files.

#[cfg(any(debug_assertions, test, feature = "test-utils"))]
mod mock;
mod system;

#[cfg(any(debug_assertions, test, feature = "test-utils"))]
pub use mock::MockSopsBackend;
pub use system::SystemSopsBackend;

use crate::error::Result;
use std::path::Path;

/// Result of adding a key to .sops.yaml
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SopsUpdateResult {
    /// The .sops.yaml file that was modified
    pub sops_yaml_path: std::path::PathBuf,
    /// Number of secret files that were re-encrypted
    pub files_updated: usize,
}

/// Backend for SOPS operations
///
/// This trait abstracts SOPS operations to enable testing without
/// actually running sops commands or modifying files.
pub trait SopsBackend: Send + Sync {
    /// Check if SOPS is available in PATH
    fn check_dependencies(&self) -> Result<()>;

    /// Check if .sops.yaml exists in the given directory
    fn sops_yaml_exists(&self, flake_dir: &Path) -> bool;

    /// Check if a host anchor already exists in .sops.yaml
    fn host_exists(&self, flake_dir: &Path, hostname: &str) -> Result<bool>;

    /// Add a new host's age key to the keys section of .sops.yaml
    ///
    /// Adds a line like `  - &hostname age1...` to the keys section.
    /// Uses line-based modification to preserve YAML anchors and formatting.
    fn add_host_key(&self, flake_dir: &Path, hostname: &str, age_key: &str) -> Result<()>;

    /// Run `sops updatekeys` on secret files that should include this host
    ///
    /// Returns the number of files updated. This is a best-effort operation -
    /// it will attempt to update files but won't fail if sops updatekeys
    /// encounters issues with individual files.
    fn update_keys(&self, flake_dir: &Path) -> Result<usize>;

    /// Perform the full SOPS update workflow: add key and update files
    fn update_sops(
        &self,
        flake_dir: &Path,
        hostname: &str,
        age_key: &str,
    ) -> Result<SopsUpdateResult> {
        self.check_dependencies()?;

        if !self.sops_yaml_exists(flake_dir) {
            return Err(crate::error::BootstrapError::SopsYamlNotFound {
                path: flake_dir.display().to_string(),
            }
            .into());
        }

        if self.host_exists(flake_dir, hostname)? {
            return Err(crate::error::BootstrapError::SopsHostExists {
                hostname: hostname.to_string(),
            }
            .into());
        }

        self.add_host_key(flake_dir, hostname, age_key)?;
        let files_updated = self.update_keys(flake_dir)?;

        Ok(SopsUpdateResult {
            sops_yaml_path: flake_dir.join(".sops.yaml"),
            files_updated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sops_update_result() {
        let result = SopsUpdateResult {
            sops_yaml_path: std::path::PathBuf::from("/tmp/.sops.yaml"),
            files_updated: 5,
        };
        assert_eq!(result.sops_yaml_path.to_string_lossy(), "/tmp/.sops.yaml");
        assert_eq!(result.files_updated, 5);
    }
}
