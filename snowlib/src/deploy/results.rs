use crate::backends::deploy::DeployTarget;
use crate::backends::reboot::RebootCheck;
use crate::error::DeployError;
use crate::host::HostMetadata;
use crate::nix::{ActivationMode, StorePath};
use std::fmt;
use std::time::Duration;

/// A deployment plan for a single host
#[derive(Debug, Clone)]
pub struct DeployPlan {
    pub metadata: HostMetadata,
    pub target: DeployTarget,
    pub store_path: StorePath,
    pub activation_mode: ActivationMode,
    pub reboot_check: RebootCheck,
}

/// A host that was blocked from deployment
#[derive(Debug, Clone)]
pub struct BlockedHost {
    pub hostname: String,
    pub reason: String,
}

/// Pipeline step where deployment can succeed or fail
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineStep {
    Build,
    /// Reading which system the host runs, to skip unchanged hosts
    Check,
    Copy,
    Plan,
    PreDeploy,
    Activate,
    Reboot,
    PostDeploy,
    /// Never started because its serial group halted
    SerialGroup,
}

impl fmt::Display for PipelineStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineStep::Build => write!(f, "build"),
            PipelineStep::Check => write!(f, "check"),
            PipelineStep::Copy => write!(f, "copy"),
            PipelineStep::Plan => write!(f, "plan"),
            PipelineStep::PreDeploy => write!(f, "pre-deploy"),
            PipelineStep::Activate => write!(f, "activate"),
            PipelineStep::Reboot => write!(f, "reboot"),
            PipelineStep::PostDeploy => write!(f, "post-deploy"),
            PipelineStep::SerialGroup => write!(f, "serial group"),
        }
    }
}

/// Result of deploying a single host
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostDeploymentResult {
    pub hostname: String,
    pub success: bool,
    pub failed_at_step: Option<PipelineStep>, // Which step failed (if any)
    pub activation_mode: Option<ActivationMode>, // How it was activated (if successful)
    pub rebooted: bool,
    /// Host already ran and would boot the built system, so nothing was deployed
    pub unchanged: bool,
    pub error_message: Option<String>,
    /// From the host's metadata: its failure doesn't fail its phase or the deployment
    pub optional: bool,
    /// Units failing after activation that the host's metadata ignores
    pub ignored_failing: Vec<String>,
}

impl HostDeploymentResult {
    /// The hostname with the step it failed at, e.g. "lanthanum (plan)"
    pub fn failure_label(&self) -> String {
        match self.failed_at_step {
            Some(step) => format!("{} ({step})", self.hostname),
            None => self.hostname.clone(),
        }
    }

    pub fn skipped_by_serial_group(
        hostname: impl Into<String>,
        group: &str,
        failed_host: &str,
    ) -> Self {
        Self {
            hostname: hostname.into(),
            success: false,
            failed_at_step: Some(PipelineStep::SerialGroup),
            activation_mode: None,
            rebooted: false,
            unchanged: false,
            optional: false,
            ignored_failing: Vec::new(),
            error_message: Some(format!(
                "serial group '{group}' halted after {failed_host} failed"
            )),
        }
    }

    /// Failed, and not optional, so its phase fails
    pub fn fails_phase(&self) -> bool {
        !self.success && !self.optional
    }

    /// Never started because its serial group halted
    pub fn is_skipped(&self) -> bool {
        self.failed_at_step == Some(PipelineStep::SerialGroup)
    }

    pub fn unchanged(hostname: impl Into<String>) -> Self {
        Self {
            hostname: hostname.into(),
            success: true,
            failed_at_step: None,
            activation_mode: None,
            rebooted: false,
            unchanged: true,
            optional: false,
            ignored_failing: Vec::new(),
            error_message: None,
        }
    }
}

/// Result of deploying all hosts in a single phase
#[derive(Debug, Clone)]
pub struct PhaseResult {
    pub phase: String,
    pub hosts: Vec<HostDeploymentResult>,
    /// From the phase's start until its last host was done
    pub duration: Duration,
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

    /// Whether a host failed that wasn't optional, so no later phase runs
    pub fn has_blocking_failure(&self) -> bool {
        self.hosts.iter().any(HostDeploymentResult::fails_phase)
    }
}

/// Skipped phase information
#[derive(Debug, Clone)]
pub struct SkippedPhase {
    pub phase: String,
    pub hostnames: Vec<String>,
}

/// Result of deploying multiple hosts across multiple phases
#[derive(Debug)]
pub struct DeployResults {
    pub phases: Vec<PhaseResult>,
    pub skipped_phases: Vec<SkippedPhase>,
}

impl DeployResults {
    pub fn success_count(&self) -> usize {
        self.phases.iter().map(|p| p.success_count()).sum()
    }

    pub fn failure_count(&self) -> usize {
        self.phases.iter().map(|p| p.failure_count()).sum()
    }

    /// The part of `failure_count` that optional hosts make up
    pub fn optional_failure_count(&self) -> usize {
        self.phases
            .iter()
            .flat_map(|p| &p.hosts)
            .filter(|h| !h.success && h.optional)
            .count()
    }

    pub fn skipped_count(&self) -> usize {
        self.skipped_phases.iter().map(|p| p.hostnames.len()).sum()
    }

    pub fn all_succeeded(&self) -> bool {
        self.phases.iter().all(|p| p.all_succeeded()) && self.skipped_phases.is_empty()
    }

    pub fn total_count(&self) -> usize {
        self.success_count() + self.failure_count() + self.skipped_count()
    }

    /// `None` when no host failed, or only optional ones did
    pub fn failure_error(&self) -> Option<DeployError> {
        let failed: Vec<String> = self
            .phases
            .iter()
            .flat_map(|p| &p.hosts)
            .filter(|h| h.fails_phase())
            .map(HostDeploymentResult::failure_label)
            .collect();
        if failed.is_empty() {
            return None;
        }
        Some(DeployError::HostsFailed {
            total: self.total_count(),
            failed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(name: &str, failed_at_step: Option<PipelineStep>) -> HostDeploymentResult {
        HostDeploymentResult {
            hostname: name.to_string(),
            success: failed_at_step.is_none(),
            failed_at_step,
            activation_mode: failed_at_step.is_none().then_some(ActivationMode::Switch),
            rebooted: false,
            unchanged: false,
            optional: false,
            ignored_failing: Vec::new(),
            error_message: failed_at_step.map(|_| "error".to_string()),
        }
    }

    fn p3_results(hosts: Vec<HostDeploymentResult>, skipped: Vec<SkippedPhase>) -> DeployResults {
        DeployResults {
            phases: vec![PhaseResult {
                phase: "p3".to_string(),
                duration: Duration::ZERO,
                hosts,
            }],
            skipped_phases: skipped,
        }
    }

    #[test]
    fn test_failure_error_names_failed_hosts_and_steps() {
        let results = p3_results(
            vec![
                host("boron", None),
                host("lanthanum", Some(PipelineStep::Plan)),
                host("samarium", Some(PipelineStep::Plan)),
                HostDeploymentResult::skipped_by_serial_group("vanadium", "k8s", "lanthanum"),
            ],
            vec![SkippedPhase {
                phase: "p4".to_string(),
                hostnames: vec!["cerium".to_string(), "neodymium".to_string()],
            }],
        );

        assert_eq!(
            results.failure_error().unwrap().to_string(),
            "3 of 6 hosts failed: lanthanum (plan), samarium (plan), vanadium (serial group)"
        );
    }

    #[test]
    fn test_failure_error_for_host_without_step() {
        let mut radon = host("radon", None);
        radon.success = false;

        let error = p3_results(vec![radon], vec![]).failure_error().unwrap();

        assert_eq!(error.to_string(), "1 of 1 host failed: radon");
    }

    #[test]
    fn test_optional_failures_neither_block_the_phase_nor_fail_the_deploy() {
        let curium = HostDeploymentResult {
            optional: true,
            ..host("curium", Some(PipelineStep::Copy))
        };
        let results = p3_results(vec![host("boron", None), curium], vec![]);

        assert!(!results.phases[0].all_succeeded());
        assert!(!results.phases[0].has_blocking_failure());
        assert_eq!(results.failure_count(), 1);
        assert_eq!(results.optional_failure_count(), 1);
        assert!(results.failure_error().is_none());
    }

    #[test]
    fn test_failure_error_names_only_hosts_that_are_not_optional() {
        let curium = HostDeploymentResult {
            optional: true,
            ..host("curium", Some(PipelineStep::Copy))
        };
        let results = p3_results(
            vec![curium, host("radon", Some(PipelineStep::Build))],
            vec![],
        );

        assert!(results.phases[0].has_blocking_failure());
        assert_eq!(
            results.failure_error().unwrap().to_string(),
            "1 of 2 hosts failed: radon (build)"
        );
    }

    #[test]
    fn test_failure_error_when_all_succeeded() {
        let results = p3_results(vec![host("boron", None), host("radon", None)], vec![]);

        assert!(results.failure_error().is_none());
    }
}
