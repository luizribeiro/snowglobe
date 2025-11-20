//! Backend factory functions for creating backend implementations
//!
//! This module centralizes the logic for creating appropriate backend implementations
//! based on command-line arguments and build configuration.

use snowlib::{
    DeployBackend, HookBackend, NixBackend, OutputMode, Result, SshDeployBackend,
    SystemHookBackend, SystemRebootBackend,
};

#[cfg(debug_assertions)]
use snowlib::MockDeployBackend;

/// Create the appropriate NixBackend implementation
///
/// In debug builds with a mock fixture path, returns a MockNixBackend.
/// Otherwise, returns the production SystemNixBackend.
pub fn create_nix_backend(
    #[cfg(debug_assertions)] mock_fixture: &Option<String>,
) -> Result<Box<dyn NixBackend>> {
    #[cfg(debug_assertions)]
    {
        if let Some(fixture_path) = mock_fixture {
            println!("⚠ Using mock deployer");
            let path = std::path::Path::new(fixture_path);
            let fixture = snowlib::DeploymentFixture::from_file(path)?;
            return Ok(Box::new(snowlib::MockNixBackend::from_fixture(fixture)));
        }
    }

    Ok(Box::new(snowlib::SystemNixBackend::new()))
}

/// Create the appropriate DeployBackend implementation
///
/// In debug builds with a mock fixture path, returns a MockDeployBackend.
/// Otherwise, returns the production SshDeployBackend with appropriate configuration.
pub fn create_deploy_backend(
    #[cfg(debug_assertions)] mock_fixture: &Option<String>,
    output_mode: OutputMode,
) -> Result<Box<dyn DeployBackend>> {
    #[cfg(debug_assertions)]
    {
        if let Some(fixture_path) = mock_fixture {
            let path = std::path::Path::new(fixture_path);
            return Ok(Box::new(MockDeployBackend::from_file(path)?));
        }
    }

    Ok(Box::new(
        SshDeployBackend::new().with_output_mode(output_mode),
    ))
}

/// Create the RebootBackend with appropriate configuration
///
/// Returns a SystemRebootBackend configured with the specified SSH user (if any).
pub fn create_reboot_backend(ssh_user: Option<&str>) -> Result<SystemRebootBackend> {
    let mut backend = SystemRebootBackend::default();

    if let Some(user) = ssh_user {
        backend = backend.with_ssh_user(user);
    }

    Ok(backend)
}

/// Create the HookBackend with appropriate configuration
///
/// Returns a SystemHookBackend configured with SSH user and output mode.
pub fn create_hook_backend(
    ssh_user: Option<String>,
    output_mode: OutputMode,
) -> std::sync::Arc<dyn HookBackend> {
    std::sync::Arc::new(SystemHookBackend::new(ssh_user, output_mode))
}
