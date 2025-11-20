use crate::backends::deploy::DeployTarget;
use crate::backends::reboot::RebootCheck;
use crate::nix::{ActivationMode, HostMetadata, StorePath};
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

    /// Print a formatted summary of the push results
    pub fn print_summary(&self) {
        println!("\nPush Summary:\n");

        // Print results for each phase that was executed
        for phase_result in &self.phases {
            let total = phase_result.hosts.len();
            let failed = phase_result.failure_count();

            // Phase header
            if phase_result.all_succeeded() {
                println!(
                    "Phase {} ({} hosts): ✓ All succeeded",
                    phase_result.phase, total
                );
            } else {
                println!(
                    "Phase {} ({} hosts): ✗ {}/{} failed",
                    phase_result.phase, total, failed, total
                );
            }

            // Print each host result
            for host in &phase_result.hosts {
                if host.success {
                    // Build status message
                    let mut parts = Vec::new();

                    // Activation mode
                    if let Some(mode) = host.activation_mode {
                        match mode {
                            ActivationMode::Switch => parts.push("switch".to_string()),
                            ActivationMode::Boot => parts.push("boot".to_string()),
                            ActivationMode::Test => parts.push("test".to_string()),
                            ActivationMode::DryActivate => parts.push("dry-run".to_string()),
                        }
                    }

                    // Reboot status
                    if host.rebooted {
                        parts.push("rebooted".to_string());
                    }

                    let status = if parts.is_empty() {
                        "deployed".to_string()
                    } else {
                        format!("deployed ({})", parts.join(", "))
                    };

                    println!("  ✓ {} - {}", host.hostname, status);
                } else {
                    let step_msg = if let Some(step) = host.failed_at_step {
                        format!("failed at {}", step)
                    } else {
                        "failed".to_string()
                    };
                    // Inline summary: show first line only for quick scanning
                    let error_msg = host
                        .error_message
                        .as_ref()
                        .map(|e| e.lines().next().unwrap_or(""))
                        .unwrap_or("");
                    println!("  ✗ {} - {}: {}", host.hostname, step_msg, error_msg);
                }
            }

            println!();
        }

        // Print skipped phases
        for skipped in &self.skipped_phases {
            let hostnames = skipped.hostnames.join(", ");
            println!(
                "Phase {} ({} hosts): - Skipped (previous phase failed)",
                skipped.phase,
                skipped.hostnames.len()
            );
            println!("  - {}", hostnames);
            println!();
        }

        // Overall summary
        let total_hosts = self.success_count() + self.failure_count() + self.skipped_count();
        let succeeded = self.success_count();
        let failed = self.failure_count();
        let skipped = self.skipped_count();

        if skipped > 0 {
            println!(
                "Result: {}/{} hosts deployed, {} not attempted",
                succeeded, total_hosts, skipped
            );
        } else if failed > 0 {
            println!("Result: {}/{} hosts deployed", succeeded, total_hosts);
        } else {
            println!("Result: All {} hosts deployed successfully", succeeded);
        }

        // Print detailed errors if any
        let errors: Vec<_> = self
            .phases
            .iter()
            .flat_map(|p| &p.hosts)
            .filter(|h| !h.success && h.error_message.is_some())
            .collect();

        if !errors.is_empty() {
            println!("\nErrors:");
            for host in errors {
                if let Some(error_msg) = &host.error_message {
                    let step_label = host
                        .failed_at_step
                        .map(|s| format!(" ({})", s))
                        .unwrap_or_default();

                    let lines: Vec<&str> = error_msg.lines().collect();

                    if lines.len() <= 1 {
                        println!("  {}{}: {}", host.hostname, step_label, error_msg);
                    } else {
                        println!("  {}{}:", host.hostname, step_label);

                        // Detailed errors: show up to 30 lines for debugging
                        let max_lines = 30;
                        let truncated = lines.len() > max_lines;
                        let lines_to_show = if truncated {
                            &lines[..max_lines]
                        } else {
                            &lines[..]
                        };

                        for line in lines_to_show {
                            println!("    {}", line);
                        }

                        if truncated {
                            println!("    ... ({} more lines omitted)", lines.len() - max_lines);
                        }
                    }
                }
            }
        }
    }
}
