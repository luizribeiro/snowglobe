use crate::backends::deploy::DeployTarget;
use crate::backends::reboot::RebootCheck;
use crate::host::HostMetadata;
use crate::nix::{ActivationMode, StorePath};
use std::fmt;

/// A push deployment plan for a single host
#[derive(Debug, Clone)]
pub struct PushPlan {
    pub metadata: HostMetadata,
    pub target: DeployTarget,
    pub store_path: StorePath,
    pub activation_mode: ActivationMode,
    pub reboot_check: Option<RebootCheck>,
}

/// A host that was blocked from deployment
#[derive(Debug, Clone)]
pub struct BlockedHost {
    pub hostname: String,
    pub reason: String,
}

/// Pipeline step where deployment can succeed or fail
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineStep {
    Build,
    Copy,
    Activate,
    Reboot,
}

impl fmt::Display for PipelineStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineStep::Build => write!(f, "build"),
            PipelineStep::Copy => write!(f, "copy"),
            PipelineStep::Activate => write!(f, "activate"),
            PipelineStep::Reboot => write!(f, "reboot"),
        }
    }
}

/// Result of deploying a single host
#[derive(Debug, Clone)]
pub struct HostDeploymentResult {
    pub hostname: String,
    pub success: bool,
    pub failed_at_step: Option<PipelineStep>, // Which step failed (if any)
    pub activation_mode: Option<ActivationMode>, // How it was activated (if successful)
    pub rebooted: bool,
    pub error_message: Option<String>,
}

/// Result of deploying all hosts in a single phase
#[derive(Debug, Clone)]
pub struct PhaseResult {
    pub phase: String,
    pub hosts: Vec<HostDeploymentResult>,
}

impl PhaseResult {
    pub fn success_count(&self) -> usize {
        self.hosts.iter().filter(|h| h.success).count()
    }

    pub fn failure_count(&self) -> usize {
        self.hosts.iter().filter(|h| !h.success).count()
    }

    pub fn all_succeeded(&self) -> bool {
        self.hosts.iter().all(|h| h.success)
    }
}

/// Skipped phase information
#[derive(Debug, Clone)]
pub struct SkippedPhase {
    pub phase: String,
    pub hostnames: Vec<String>,
}

/// Result of pushing multiple hosts across multiple phases
#[derive(Debug)]
pub struct PushResults {
    pub phases: Vec<PhaseResult>,
    pub skipped_phases: Vec<SkippedPhase>,
}

impl PushResults {
    pub fn success_count(&self) -> usize {
        self.phases.iter().map(|p| p.success_count()).sum()
    }

    pub fn failure_count(&self) -> usize {
        self.phases.iter().map(|p| p.failure_count()).sum()
    }

    pub fn skipped_count(&self) -> usize {
        self.skipped_phases.iter().map(|p| p.hostnames.len()).sum()
    }

    pub fn all_succeeded(&self) -> bool {
        self.phases.iter().all(|p| p.all_succeeded()) && self.skipped_phases.is_empty()
    }
}
