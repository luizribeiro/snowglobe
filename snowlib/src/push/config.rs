use crate::nix::ActivationMode;
use crate::process::OutputMode;

/// Configuration for push operations
#[derive(Debug, Clone)]
pub struct PushConfiguration {
    /// Push strategy (auto, switch, boot, test, dry-run)
    pub strategy: PushStrategy,
    /// Force deployment even when reboot is detected in switch mode
    pub force: bool,
    /// Maximum number of parallel deployments within a phase
    pub concurrency: usize,
    /// Ignore push phases and deploy all hosts in parallel
    pub ignore_phases: bool,
    /// Output mode for build and deploy operations
    pub output_mode: OutputMode,
    /// SSH user for deployment (if None, uses SSH config default)
    pub ssh_user: Option<String>,
    /// Allow automatic reboots (false = never reboot, overrides policy/detection)
    pub allow_reboot: bool,
    /// Deploy hosts even when they already run and would boot the built system
    pub redeploy: bool,
}

impl Default for PushConfiguration {
    fn default() -> Self {
        Self {
            strategy: PushStrategy::Auto,
            force: false,
            concurrency: 3,
            ignore_phases: false,
            output_mode: OutputMode::Display,
            ssh_user: None,
            allow_reboot: true,
            redeploy: false,
        }
    }
}

/// Push deployment strategy requested by the user
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushStrategy {
    /// Auto mode - detect reboot need and choose appropriate activation mode
    Auto,
    /// Switch now, blocking hosts that need a reboot instead of rebooting them; force switches them anyway and ignores reboot policies
    Switch,
    /// Set for next boot only
    Boot,
    /// Test activation (non-persistent)
    Test,
    /// Dry run - preview only
    DryRun,
}

impl PushStrategy {
    /// The mode every host activates in, or None when it depends on the host
    pub fn fixed_activation_mode(self) -> Option<ActivationMode> {
        match self {
            Self::DryRun => Some(ActivationMode::DryActivate),
            Self::Test => Some(ActivationMode::Test),
            // Like `nixos-rebuild boot`: the new system waits for the next boot
            Self::Boot => Some(ActivationMode::Boot),
            Self::Auto | Self::Switch => None,
        }
    }
}
