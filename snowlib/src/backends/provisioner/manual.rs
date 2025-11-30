//! Manual provisioner for user-handled key injection

use super::{InjectionResult, Provisioner, ProvisionerConfig};
use crate::bootstrap::HostKeys;
use crate::error::Result;
use std::path::Path;

/// Provisioner that outputs instructions for manual key injection
///
/// Use this when:
/// - The image format is not directly supported
/// - You want full control over the injection process
/// - Testing the bootstrap workflow without actual image modification
pub struct ManualProvisioner;

impl ManualProvisioner {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ManualProvisioner {
    fn default() -> Self {
        Self::new()
    }
}

impl Provisioner for ManualProvisioner {
    fn name(&self) -> &str {
        "manual"
    }

    fn check_dependencies(&self) -> Result<()> {
        Ok(())
    }

    fn inject_keys(
        &self,
        _image_path: &Path,
        host_keys: &HostKeys,
        config: &ProvisionerConfig,
    ) -> Result<InjectionResult> {
        let ssh_path = config.ssh_key_path();
        let instructions = format!(
            r#"Keys generated for {}. Manual injection required.

Copy these files to the target's {}/:
  {} -> {}/ssh_host_ed25519_key
  {} -> {}/ssh_host_ed25519_key.pub

Ensure correct permissions:
  chmod 600 {}/ssh_host_ed25519_key
  chmod 644 {}/ssh_host_ed25519_key.pub"#,
            host_keys.hostname,
            ssh_path,
            host_keys.ssh_private_key.display(),
            ssh_path,
            host_keys.ssh_public_key.display(),
            ssh_path,
            ssh_path,
            ssh_path,
        );

        Ok(InjectionResult::ManualRequired { instructions })
    }

    fn deployment_instructions(&self, _output_path: &Path, hostname: &str) -> String {
        format!(
            r#"Manual deployment required for {}.

After copying keys to the target:
  1. Boot the target system
  2. Verify SSH connectivity: ssh root@{}
  3. Run: snow push {}"#,
            hostname, hostname, hostname,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_host_keys() -> HostKeys {
        HostKeys::new(
            "testhost".to_string(),
            PathBuf::from("/tmp/snow/ssh_host_ed25519_key"),
            PathBuf::from("/tmp/snow/ssh_host_ed25519_key.pub"),
            "age1testkey".to_string(),
            PathBuf::from("/tmp/snow"),
        )
    }

    #[test]
    fn test_manual_provisioner_name() {
        let provisioner = ManualProvisioner::new();
        assert_eq!(provisioner.name(), "manual");
    }

    #[test]
    fn test_manual_provisioner_check_dependencies() {
        let provisioner = ManualProvisioner::new();
        assert!(provisioner.check_dependencies().is_ok());
    }

    #[test]
    fn test_manual_provisioner_inject_keys_returns_manual_required() {
        let provisioner = ManualProvisioner::new();
        let keys = test_host_keys();
        let config = ProvisionerConfig::new();

        let result = provisioner
            .inject_keys(Path::new("/tmp/image.img"), &keys, &config)
            .unwrap();

        match result {
            InjectionResult::ManualRequired { instructions } => {
                assert!(instructions.contains("testhost"));
                assert!(instructions.contains("/etc/ssh"));
                assert!(instructions.contains("chmod 600"));
            }
            _ => panic!("Expected ManualRequired result"),
        }
    }

    #[test]
    fn test_manual_provisioner_inject_keys_with_custom_path() {
        let provisioner = ManualProvisioner::new();
        let keys = test_host_keys();
        let config = ProvisionerConfig::new().with_ssh_key_path("/custom/path");

        let result = provisioner
            .inject_keys(Path::new("/tmp/image.img"), &keys, &config)
            .unwrap();

        match result {
            InjectionResult::ManualRequired { instructions } => {
                assert!(instructions.contains("/custom/path"));
            }
            _ => panic!("Expected ManualRequired result"),
        }
    }

    #[test]
    fn test_manual_provisioner_deployment_instructions() {
        let provisioner = ManualProvisioner::new();
        let instructions =
            provisioner.deployment_instructions(Path::new("/tmp/image.img"), "myhost");

        assert!(instructions.contains("myhost"));
        assert!(instructions.contains("snow push"));
    }
}
