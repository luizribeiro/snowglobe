mod common;
mod failing_units;
mod local;
#[cfg(any(debug_assertions, test, feature = "test-utils"))]
mod mock;
mod ssh;
mod target;

pub use common::*;
pub use failing_units::{activate_reporting_failed_units, units_failed, UnitsFailed};
pub use local::LocalDeployBackend;
#[cfg(any(debug_assertions, test, feature = "test-utils"))]
pub use mock::{
    BuildFixture, DeploymentFixture, FailureSpec, HostFixture, MetadataFixture, MockDeployBackend,
    OperationFixture,
};
pub use ssh::SshDeployBackend;
pub use target::DeployTarget;

use crate::error::{DeployError, Result};
use crate::nix::{ActivationMode, StorePath};
use async_trait::async_trait;

const CURRENT_SYSTEM_LINK: &str = "/run/current-system";
const SYSTEM_PROFILE_LINK: &str = "/nix/var/nix/profiles/system";

/// The system a host is running and the one it will boot next
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSystems {
    pub running: StorePath,
    pub next_boot: StorePath,
}

impl HostSystems {
    pub fn both(system: StorePath) -> Self {
        Self {
            running: system.clone(),
            next_boot: system,
        }
    }

    /// Whether the host runs `system` and will keep running it after a reboot
    pub fn are(&self, system: &StorePath) -> bool {
        &self.running == system && &self.next_boot == system
    }
}

/// Validate the resolved target of `link` reported by a host
fn store_path_from_resolved(host: &str, link: &str, resolved: &str) -> Result<StorePath> {
    let resolved = resolved.trim();
    let is_store_path = resolved
        .strip_prefix("/nix/store/")
        .is_some_and(|name| !name.is_empty() && !name.contains(char::is_whitespace));
    if !is_store_path {
        return Err(DeployError::Failed {
            host: host.to_string(),
            source: std::io::Error::other(format!(
                "{} resolved to '{}', not a Nix store path",
                link, resolved
            )),
        }
        .into());
    }
    Ok(StorePath::new(resolved.to_string()))
}

/// Trait for deployment backend implementations
///
/// Implementations must be Send + Sync to support parallel deployments
#[async_trait]
pub trait DeployBackend: Send + Sync {
    /// Copy a nix closure to the target host
    async fn copy_closure(&self, target: &DeployTarget, store_path: &StorePath) -> Result<()>;

    /// The systems behind /run/current-system and /nix/var/nix/profiles/system
    async fn host_systems(&self, target: &DeployTarget) -> Result<HostSystems>;

    /// Checks that can fail the whole push before any host is touched
    async fn preflight(&self) -> Result<()> {
        Ok(())
    }

    /// Runs `program` with `args` as the deploy user, to read the host
    /// without changing it, and returns what it prints
    async fn inspect(
        &self,
        target: &DeployTarget,
        program: &str,
        args: &[String],
    ) -> Result<String>;

    /// The units failing on a NixOS host, as switch-to-configuration
    /// counts them
    async fn failing_units(&self, target: &DeployTarget) -> Result<Vec<String>> {
        let show = failing_units::FAILING_UNITS_ARGS.map(String::from);
        let output = self.inspect(target, "systemctl", &show).await?;
        Ok(failing_units::parse_failing_units(&output))
    }

    /// Activate a configuration on the target host
    async fn activate(
        &self,
        target: &DeployTarget,
        store_path: &StorePath,
        mode: ActivationMode,
    ) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> StorePath {
        StorePath::new(s.to_string())
    }

    #[test]
    fn test_store_path_from_resolved_store_path() {
        let resolved = store_path_from_resolved(
            "hydrogen",
            CURRENT_SYSTEM_LINK,
            "/nix/store/abc123-nixos-system-hydrogen-25.11\n",
        )
        .unwrap();

        assert_eq!(
            resolved,
            path("/nix/store/abc123-nixos-system-hydrogen-25.11")
        );
    }

    #[test]
    fn test_store_path_from_resolved_rejects_non_store_path() {
        for resolved in ["/run/current-system\n", "", "/nix/store/"] {
            assert!(store_path_from_resolved("hydrogen", CURRENT_SYSTEM_LINK, resolved).is_err());
        }
    }

    #[test]
    fn test_store_path_from_resolved_rejects_extra_output() {
        let with_banner = "/nix/store/abc123-nixos-system-hydrogen\nWelcome to hydrogen!\n";

        assert!(store_path_from_resolved("hydrogen", CURRENT_SYSTEM_LINK, with_banner).is_err());
    }

    #[test]
    fn test_host_systems_are_target_only_when_running_and_booting_it() {
        let target = path("/nix/store/new-nixos-system-hydrogen");
        let old = path("/nix/store/old-nixos-system-hydrogen");

        assert!(HostSystems::both(target.clone()).are(&target));
        assert!(!HostSystems::both(old.clone()).are(&target));
        assert!(!HostSystems {
            running: target.clone(),
            next_boot: old.clone(),
        }
        .are(&target));
        assert!(!HostSystems {
            running: old,
            next_boot: target.clone(),
        }
        .are(&target));
    }
}
