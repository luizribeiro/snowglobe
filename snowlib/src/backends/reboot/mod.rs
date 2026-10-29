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

impl std::fmt::Display for RebootReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RebootReason::KernelChanged { old, new } => {
                writeln!(f, "Kernel changed:")?;
                writeln!(f, "  Current: {}", old)?;
                write!(f, "  New:     {}", new)
            }
            RebootReason::InitrdChanged { old, new } => {
                writeln!(f, "Initrd changed:")?;
                writeln!(f, "  Current: {}", old)?;
                write!(f, "  New:     {}", new)
            }
            RebootReason::CriticalService(service) => {
                write!(f, "Critical service will restart: {}", service)
            }
            RebootReason::Policy => write!(f, "Host reboot policy requires reboot"),
        }
    }
}

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
            reboot_max_wait: 600,
            reboot_retry_interval: 5,
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

    #[test]
    fn test_reboot_reason_display() {
        let kernel_msg = RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        }
        .to_string();
        assert!(kernel_msg.contains("Kernel changed"));
        assert!(kernel_msg.contains("/nix/store/old-kernel"));
        assert!(kernel_msg.contains("/nix/store/new-kernel"));

        let initrd_msg = RebootReason::InitrdChanged {
            old: "/nix/store/old-initrd".to_string(),
            new: "/nix/store/new-initrd".to_string(),
        }
        .to_string();
        assert!(initrd_msg.contains("Initrd changed"));

        assert_eq!(
            RebootReason::CriticalService("audit.service".to_string()).to_string(),
            "Critical service will restart: audit.service"
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
