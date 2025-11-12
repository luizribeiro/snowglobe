use crate::deploy::reboot::RebootCheck;
use crate::deploy::{ActivationMode, DeployResult};
use crate::nix::{DeployTarget, HostMetadata, StorePath};

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

/// Result of pushing multiple hosts
#[derive(Debug)]
pub struct PushResults {
    pub results: Vec<(String, DeployResult)>, // (hostname, result)
}

impl PushResults {
    pub fn success_count(&self) -> usize {
        self.results.iter().filter(|(_, r)| r.success).count()
    }

    pub fn failure_count(&self) -> usize {
        self.results.iter().filter(|(_, r)| !r.success).count()
    }

    pub fn all_succeeded(&self) -> bool {
        self.results.iter().all(|(_, r)| r.success)
    }
}
