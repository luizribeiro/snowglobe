mod local;
#[cfg(any(debug_assertions, test, feature = "test-utils"))]
mod mock;
mod system;

pub use local::LocalRebootBackend;
#[cfg(any(debug_assertions, test, feature = "test-utils"))]
pub use mock::MockRebootBackend;
pub use system::SystemRebootBackend;

use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::nix::StorePath;
use async_trait::async_trait;

/// Trait for reboot backend operations (detection and execution)
///
/// This trait allows for testability by enabling mock implementations
/// that don't require actual SSH connections.
#[async_trait]
pub trait RebootBackend: Send + Sync {
    /// Check if a deployment requires a reboot
    async fn check_reboot_required(
        &self,
        target: &DeployTarget,
        new_system: &StorePath,
    ) -> Result<RebootCheck>;

    /// Reboot a host and wait for it to come back online
    async fn reboot_and_wait(&self, target: &DeployTarget) -> Result<()>;
}

/// Default critical services that require a reboot when restarted
pub const DEFAULT_CRITICAL_SERVICES: &[&str] = &[
    "audit.service",
    "kmod-static-nodes.service",
    "network-local-commands.service",
    "network-setup.service",
    "syslog.socket",
    "systemd-modules-load.service",
    "systemd-oomd.socket",
    "systemd-sysctl.service",
    "systemd-udevd-control.socket",
    "systemd-udevd-kernel.socket",
];

/// Result of a reboot detection check
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebootCheck {
    /// Whether a reboot is required
    pub required: bool,
    /// Reason for reboot (if required)
    pub reason: Option<RebootReason>,
}

impl RebootCheck {
    /// Create a check indicating no reboot is required
    pub fn not_required() -> Self {
        Self {
            required: false,
            reason: None,
        }
    }

    /// Create a check indicating a reboot is required
    pub fn required(reason: RebootReason) -> Self {
        Self {
            required: true,
            reason: Some(reason),
        }
    }
}

/// Reason why a reboot is required
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebootReason {
    /// Kernel changed
    KernelChanged { old: String, new: String },
    /// Initrd changed
    InitrdChanged { old: String, new: String },
    /// A critical service would be restarted
    CriticalService(String),
    /// Reboot required by host's reboot policy
    Policy,
}

/// One short line, e.g. "kernel linux-6.12.58 → linux-6.12.60": the store
/// paths themselves only differ in their hashes
impl std::fmt::Display for RebootReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RebootReason::KernelChanged { old, new } => {
                let (old, new) = (package_name(old), package_name(new));
                if old == new {
                    write!(f, "kernel rebuilt ({new})")
                } else {
                    write!(f, "kernel {old} → {new}")
                }
            }
            RebootReason::InitrdChanged { .. } => write!(f, "initrd changed"),
            RebootReason::CriticalService(service) => write!(f, "{service} would restart"),
            RebootReason::Policy => write!(f, "the host's reboot policy asks for one"),
        }
    }
}

/// "linux-6.12.58" from "/nix/store/<hash>-linux-6.12.58/bzImage"
fn package_name(path: &str) -> &str {
    path.strip_prefix("/nix/store/")
        .and_then(|rest| rest.split('/').next())
        .and_then(|entry| entry.split_once('-'))
        .map_or(path, |(_, name)| name)
}

/// dry-activate builds the switch plan for the whole system, which takes
/// far longer than a probe on big systems
const DEFAULT_DRY_ACTIVATE_TIMEOUT_SECS: u32 = 300;
pub const DEFAULT_REBOOT_MAX_WAIT_SECS: u32 = 600;

/// Configuration for reboot detection
#[derive(Debug, Clone)]
pub struct RebootDetectionConfig {
    /// List of services that require a reboot when restarted
    pub critical_services: Vec<String>,
    /// Whether to check kernel changes
    pub check_kernel: bool,
    /// Whether to check initrd changes
    pub check_initrd: bool,
    /// SSH connection timeout in seconds
    pub ssh_connect_timeout: u32,
    /// Maximum time to wait for host to come back online after reboot (seconds)
    pub reboot_max_wait: u32,
    /// Interval between retry attempts when waiting for reboot (seconds)
    pub reboot_retry_interval: u32,
    /// Wall-clock limit on every ssh command except dry-activate, auth
    /// included, on top of `ssh_connect_timeout` (seconds)
    pub probe_timeout: u32,
    /// Longest `switch-to-configuration dry-activate` may take (seconds)
    pub dry_activate_timeout: u32,
}

impl Default for RebootDetectionConfig {
    fn default() -> Self {
        Self {
            critical_services: DEFAULT_CRITICAL_SERVICES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            check_kernel: true,
            check_initrd: true,
            ssh_connect_timeout: 5,
            reboot_max_wait: DEFAULT_REBOOT_MAX_WAIT_SECS,
            reboot_retry_interval: 5,
            probe_timeout: crate::ssh_options::PROBE_TIMEOUT.as_secs() as u32,
            dry_activate_timeout: DEFAULT_DRY_ACTIVATE_TIMEOUT_SECS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reboot_check_equality() {
        let check1 = RebootCheck {
            required: true,
            reason: Some(RebootReason::KernelChanged {
                old: "/nix/store/old-kernel".to_string(),
                new: "/nix/store/new-kernel".to_string(),
            }),
        };
        let check2 = RebootCheck {
            required: true,
            reason: Some(RebootReason::KernelChanged {
                old: "/nix/store/old-kernel".to_string(),
                new: "/nix/store/new-kernel".to_string(),
            }),
        };
        assert_eq!(check1, check2);
    }

    #[test]
    fn test_reboot_check_not_required() {
        let check = RebootCheck::not_required();
        assert!(!check.required);
        assert!(check.reason.is_none());
    }

    #[test]
    fn test_reboot_check_required() {
        let check = RebootCheck::required(RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        });
        assert!(check.required);
        assert!(matches!(
            check.reason,
            Some(RebootReason::KernelChanged { .. })
        ));
    }

    fn kernel(old: &str, new: &str) -> RebootReason {
        RebootReason::KernelChanged {
            old: old.to_string(),
            new: new.to_string(),
        }
    }

    #[test]
    fn test_kernel_reason_names_the_versions() {
        let upgrade = kernel(
            "/nix/store/0n6kcz8q4lw1-linux-6.12.58/bzImage",
            "/nix/store/9s2mf0yh7l3a-linux-6.12.60/bzImage",
        );
        let rebuild = kernel(
            "/nix/store/0n6kcz8q4lw1-linux-6.12.58/bzImage",
            "/nix/store/4rq81mnd2cxz-linux-6.12.58/bzImage",
        );

        assert_eq!(upgrade.to_string(), "kernel linux-6.12.58 → linux-6.12.60");
        assert_eq!(rebuild.to_string(), "kernel rebuilt (linux-6.12.58)");
    }

    #[test]
    fn test_kernel_reason_outside_the_store_shows_the_paths() {
        assert_eq!(
            kernel("/boot/vmlinuz-old", "/boot/vmlinuz-new").to_string(),
            "kernel /boot/vmlinuz-old → /boot/vmlinuz-new"
        );
    }

    #[test]
    fn test_other_reasons_are_one_short_line() {
        let initrd = RebootReason::InitrdChanged {
            old: "/nix/store/b5h2sjpj-initrd-linux-6.12.58/initrd".to_string(),
            new: "/nix/store/96wpyhsq-initrd-linux-6.12.58/initrd".to_string(),
        };

        assert_eq!(initrd.to_string(), "initrd changed");
        assert_eq!(
            RebootReason::CriticalService("audit.service".to_string()).to_string(),
            "audit.service would restart"
        );
        assert_eq!(
            RebootReason::Policy.to_string(),
            "the host's reboot policy asks for one"
        );
    }

    #[test]
    fn test_default_config() {
        let config = RebootDetectionConfig::default();
        assert!(config.check_kernel);
        assert!(config.check_initrd);
        assert!(!config.critical_services.is_empty());
        assert!(config
            .critical_services
            .contains(&"audit.service".to_string()));
    }
}
