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
    nix_backend_or_mock(
        #[cfg(debug_assertions)]
        mock_fixture,
        snowlib::SystemNixBackend::new(),
    )
}

/// Like `create_nix_backend`, but roots what it builds in `gc_roots`
pub fn create_deploy_nix_backend(
    #[cfg(debug_assertions)] mock_fixture: &Option<String>,
    gc_roots: &std::path::Path,
) -> Result<Box<dyn NixBackend>> {
    nix_backend_or_mock(
        #[cfg(debug_assertions)]
        mock_fixture,
        snowlib::SystemNixBackend::new().with_gc_roots(gc_roots),
    )
}

/// A Nix backend whose host metadata carries no hooks, so reading it builds nothing
pub fn create_hookless_nix_backend(
    #[cfg(debug_assertions)] mock_fixture: &Option<String>,
) -> Result<Box<dyn NixBackend>> {
    nix_backend_or_mock(
        #[cfg(debug_assertions)]
        mock_fixture,
        snowlib::SystemNixBackend::new().without_hooks(),
    )
}

fn nix_backend_or_mock(
    #[cfg(debug_assertions)] mock_fixture: &Option<String>,
    system: snowlib::SystemNixBackend,
) -> Result<Box<dyn NixBackend>> {
    #[cfg(debug_assertions)]
    {
        if let Some(fixture_path) = mock_fixture {
            tracing::warn!("Using the mock deployer from {fixture_path}");
            let path = std::path::Path::new(fixture_path);
            let fixture = snowlib::DeploymentFixture::from_file(path)?;
            return Ok(Box::new(snowlib::MockNixBackend::from_fixture(fixture)));
        }
    }

    Ok(Box::new(system))
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
pub fn create_reboot_backend(
    ssh_user: Option<&str>,
    reboot_timeout_secs: u32,
) -> Result<SystemRebootBackend> {
    let mut backend = SystemRebootBackend::default().with_reboot_max_wait(reboot_timeout_secs);

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
