//! Context for push deployment operations
//!
//! This module defines the DeployContext struct which bundles together
//! all the state needed for a push deployment.

use crate::backends::deploy::DeployBackend;
use crate::backends::nix::NixBackend;
use crate::backends::reboot::RebootBackend;
use crate::deploy::config::DeployConfiguration;
use crate::deploy::hooks::HookExecutor;
use crate::nix::Flake;
use crate::observer::DeployObserver;

/// Context for push deployment operations
///
/// Bundles together the flake, backends, configuration, and observer needed for
/// push operations, reducing the number of parameters that need to be passed around.
pub struct DeployContext<'a> {
    pub flake: &'a Flake,
    pub nix_backend: &'a dyn NixBackend,
    pub deploy_backend: &'a dyn DeployBackend,
    pub reboot_backend: &'a dyn RebootBackend,
    pub config: &'a DeployConfiguration,
    pub observer: &'a dyn DeployObserver,
    pub hook_executor: Option<&'a HookExecutor>,
}

impl<'a> DeployContext<'a> {
    pub fn new(
        flake: &'a Flake,
        nix_backend: &'a dyn NixBackend,
        deploy_backend: &'a dyn DeployBackend,
        reboot_backend: &'a dyn RebootBackend,
        config: &'a DeployConfiguration,
        observer: &'a dyn DeployObserver,
        hook_executor: Option<&'a HookExecutor>,
    ) -> Self {
        Self {
            flake,
            nix_backend,
            deploy_backend,
            reboot_backend,
            config,
            observer,
            hook_executor,
        }
    }
}
