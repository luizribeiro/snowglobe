//! Mock provisioner for testing

use super::{InjectionResult, Provisioner, ProvisionerConfig};
use crate::bootstrap::HostKeys;
use crate::error::{ProvisionerError, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Mock provisioner for testing bootstrap workflows
pub struct MockProvisioner {
    name: String,
    state: Arc<Mutex<MockState>>,
}

#[derive(Default)]
struct MockState {
    fail_dependency_check: Option<String>,
    fail_injection: bool,
    return_manual_required: bool,
    injection_calls: Vec<InjectionCall>,
}

#[derive(Debug, Clone)]
pub struct InjectionCall {
    pub image_path: PathBuf,
    pub hostname: String,
}

impl MockProvisioner {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            state: Arc::new(Mutex::new(MockState::default())),
        }
    }

    pub fn with_dependency_failure(self, tool: &str) -> Self {
        self.state.lock().unwrap().fail_dependency_check = Some(tool.to_string());
        self
    }

    pub fn with_injection_failure(self) -> Self {
        self.state.lock().unwrap().fail_injection = true;
        self
    }

    pub fn with_manual_required(self) -> Self {
        self.state.lock().unwrap().return_manual_required = true;
        self
    }

    pub fn injection_calls(&self) -> Vec<InjectionCall> {
        self.state.lock().unwrap().injection_calls.clone()
    }
}

impl Provisioner for MockProvisioner {
    fn name(&self) -> &str {
        &self.name
    }

    fn check_dependencies(&self) -> Result<()> {
        let state = self.state.lock().unwrap();
        if let Some(tool) = &state.fail_dependency_check {
            return Err(ProvisionerError::MissingDependency {
                provisioner: self.name.clone(),
                tool: tool.clone(),
                install_hint: format!("Mock: install {}", tool),
            }
            .into());
        }
        Ok(())
    }

    fn inject_keys(
        &self,
        image_path: &Path,
        host_keys: &HostKeys,
        _config: &ProvisionerConfig,
    ) -> Result<InjectionResult> {
        let mut state = self.state.lock().unwrap();
        state.injection_calls.push(InjectionCall {
            image_path: image_path.to_path_buf(),
            hostname: host_keys.hostname.clone(),
        });

        if state.fail_injection {
            return Err(ProvisionerError::InjectionFailed {
                provisioner: self.name.clone(),
                reason: "Mock injection failed".to_string(),
            }
            .into());
        }

        if state.return_manual_required {
            Ok(InjectionResult::ManualRequired {
                instructions: format!("Mock manual instructions for {}", host_keys.hostname),
            })
        } else {
            Ok(InjectionResult::Success {
                image_path: image_path.to_path_buf(),
            })
        }
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
    fn test_mock_provisioner_default_success() {
        let provisioner = MockProvisioner::new("test");
        let keys = test_host_keys();
        let config = ProvisionerConfig::default();

        let result = provisioner
            .inject_keys(Path::new("/tmp/image.img"), &keys, &config)
            .unwrap();

        assert!(matches!(result, InjectionResult::Success { .. }));
    }

    #[test]
    fn test_mock_provisioner_dependency_failure() {
        let provisioner = MockProvisioner::new("test").with_dependency_failure("guestfish");

        let result = provisioner.check_dependencies();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("guestfish"));
        assert!(err_msg.contains("test"));
    }

    #[test]
    fn test_mock_provisioner_injection_failure() {
        let provisioner = MockProvisioner::new("test").with_injection_failure();
        let keys = test_host_keys();
        let config = ProvisionerConfig::default();

        let result = provisioner.inject_keys(Path::new("/tmp/image.img"), &keys, &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_provisioner_manual_required() {
        let provisioner = MockProvisioner::new("test").with_manual_required();
        let keys = test_host_keys();
        let config = ProvisionerConfig::default();

        let result = provisioner
            .inject_keys(Path::new("/tmp/image.img"), &keys, &config)
            .unwrap();

        match result {
            InjectionResult::ManualRequired { instructions } => {
                assert!(instructions.contains("testhost"));
            }
            _ => panic!("Expected ManualRequired"),
        }
    }

    #[test]
    fn test_mock_provisioner_tracks_calls() {
        let provisioner = MockProvisioner::new("test");
        let keys = test_host_keys();
        let config = ProvisionerConfig::default();

        provisioner
            .inject_keys(Path::new("/tmp/image1.img"), &keys, &config)
            .unwrap();
        provisioner
            .inject_keys(Path::new("/tmp/image2.img"), &keys, &config)
            .unwrap();

        let calls = provisioner.injection_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].image_path, PathBuf::from("/tmp/image1.img"));
        assert_eq!(calls[1].image_path, PathBuf::from("/tmp/image2.img"));
    }
}
