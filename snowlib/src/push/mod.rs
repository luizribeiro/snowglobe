//! Push deployment orchestration
//!
//! This module provides the main `push()` function for deploying NixOS configurations
//! to remote hosts. The deployment uses a parallel-per-host execution model where
//! each host runs through all steps independently:
//!
//! 1. **Build**: Builds NixOS configuration for the host
//! 2. **Copy**: Copies closure to remote host
//! 3. **Planning**: Creates deployment plan with reboot detection
//! 4. **Activate**: Activates configuration on remote host
//! 5. **Reboot**: Reboots host if required
//!
//! The parallel execution is orchestrated by `execute_phase()` which uses Rayon to
//! run `execute_host_pipeline()` concurrently for each host. Dynamic step spans
//! (managed by the observer) provide visibility into which steps hosts are currently in.

mod config;
mod context;
mod pipeline;
mod planning;
mod results;

// Re-export public types
pub use config::{PushConfiguration, PushStrategy};
pub use context::PushContext;
pub use results::{
    BlockedHost, HostDeploymentResult, PhaseResult, PipelineStep, PushPlan, PushResults,
    SkippedPhase,
};

use crate::error::Result;
use crate::nix::HostMetadata;
use pipeline::execute_host_pipeline;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

/// Helper to create SkippedPhase entries for all remaining phases
fn skip_remaining_phases(
    phases: &[String],
    current_idx: usize,
    by_phase: &HashMap<String, Vec<HostMetadata>>,
) -> Vec<SkippedPhase> {
    phases[current_idx + 1..]
        .iter()
        .map(|phase| {
            let hostnames = by_phase
                .get(phase)
                .unwrap()
                .iter()
                .map(|h| h.name.clone())
                .collect();
            SkippedPhase {
                phase: phase.clone(),
                hostnames,
            }
        })
        .collect()
}

/// Main push orchestration function - handles complete deployment workflow
///
/// This is the primary entry point for deploying configurations. It orchestrates
/// the entire deployment process by phase:
///
/// For each phase (sequential):
/// 1. **Build** - Build NixOS configurations for all hosts in the phase
/// 2. **Copy** - Copy closures to remote hosts
/// 3. **Plan** - Create deployment plans with reboot detection
/// 4. **Activate** - Activate configurations (parallel within phase)
/// 5. **Reboot** - Reboot hosts that need it
///
/// If any step fails in a phase, deployment stops immediately.
///
/// # Arguments
///
/// - `push_ctx`: Push context containing flake, backends, configuration, and observer
/// - `hosts`: List of hosts to deploy
///
/// # Example
///
/// ```no_run
/// use snowlib::{push, PushConfiguration, PushContext, Flake, SystemNixBackend, SshDeployBackend, SystemRebootBackend};
/// use snowlib::observer::NoopObserver;
/// use snowlib::nix::HostMetadata;
///
/// # fn example(hosts: Vec<HostMetadata>) -> snowlib::Result<()> {
/// let flake = Flake::new(None)?;
/// let nix_backend = SystemNixBackend::new();
/// let deploy_backend = SshDeployBackend::new();
/// let reboot_backend = SystemRebootBackend::default();
/// let config = PushConfiguration::default();
/// let observer = NoopObserver;
/// let push_ctx = PushContext::new(&flake, &nix_backend, &deploy_backend, &reboot_backend, &config, &observer);
///
/// let results = push(&push_ctx, hosts)?;
/// # Ok(())
/// # }
/// ```
pub fn push(push_ctx: &PushContext, hosts: Vec<HostMetadata>) -> Result<PushResults> {
    let by_phase = group_by_phase(hosts, push_ctx.config.ignore_phases);
    let phases = sorted_phases(&by_phase);

    let mut executed_phases = Vec::new();
    let mut skipped_phases = Vec::new();

    for (idx, phase) in phases.iter().enumerate() {
        let phase_hosts = by_phase.get(phase).unwrap();

        let hostnames: Vec<String> = phase_hosts.iter().map(|h| h.name.clone()).collect();

        push_ctx.observer.on_phase_start(phase, &hostnames);

        // Execute phase and notify observer (even on failure)
        let phase_result = execute_phase(push_ctx, phase, phase_hosts);
        let phase_success = phase_result.as_ref().is_ok_and(|pr| pr.all_succeeded());
        push_ctx.observer.on_phase_complete(phase, phase_success);

        // Propagate infrastructure errors immediately
        let phase_result = phase_result?;

        // Always record this phase result
        executed_phases.push(phase_result);

        // If phase had failures, skip remaining phases and stop deployment
        if !phase_success {
            skipped_phases = skip_remaining_phases(&phases, idx, &by_phase);
            break;
        }
    }

    Ok(PushResults {
        phases: executed_phases,
        skipped_phases,
    })
}

/// Execute all deployment steps for a single phase
///
/// Uses parallel-per-host execution where each host independently runs through
/// all steps (build -> copy -> activate -> reboot). This allows hosts to progress
/// at different rates - one host can be activating while another is still building.
fn execute_phase(
    push_ctx: &PushContext,
    phase: &str,
    phase_hosts: &[HostMetadata],
) -> Result<PhaseResult> {
    // Sort hosts lexicographically for consistent rendering order
    let mut sorted_hosts = phase_hosts.to_vec();
    sorted_hosts.sort_by(|a, b| a.name.cmp(&b.name));

    // Wrap push_ctx for parallel access
    // Note: We're wrapping references in Arc which clippy warns about,
    // but this is necessary here because:
    // 1. We only have &T not T (T contains lifetimes and can't be cloned)
    // 2. We need to share across threads (Arc)
    // 3. Rayon's par_iter requires 'static closure
    #[allow(clippy::redundant_allocation)]
    let push_ctx = Arc::new(push_ctx);

    // Create thread pool with concurrency limit
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(push_ctx.config.concurrency)
        .build()
        .map_err(|e| {
            crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
                host: "thread pool".to_string(),
                source: std::io::Error::other(e.to_string()),
            })
        })?;

    // Execute hosts in parallel - each host runs through all steps independently
    let results: Vec<Result<HostDeploymentResult>> = pool.install(|| {
        sorted_hosts
            .par_iter()
            .map(|host_metadata| execute_host_pipeline(Arc::clone(&push_ctx), host_metadata))
            .collect()
    });

    // Collect results - note that pipeline now returns Ok(HostDeploymentResult) even on failure
    let mut host_results = Vec::new();

    for result in results {
        match result {
            Ok(host_result) => {
                host_results.push(host_result);
            }
            Err(e) => {
                // This should be rare now - pipeline converts most errors to HostDeploymentResult
                // But we still need to handle catastrophic errors (e.g., observer errors)
                return Err(e);
            }
        }
    }

    Ok(PhaseResult {
        phase: phase.to_string(),
        hosts: host_results,
    })
}

fn group_by_phase(
    hosts: Vec<HostMetadata>,
    ignore_phases: bool,
) -> HashMap<String, Vec<HostMetadata>> {
    let mut by_phase: HashMap<String, Vec<HostMetadata>> = HashMap::new();

    for host in hosts {
        let phase = if ignore_phases {
            "_all".to_string()
        } else {
            host.phase.clone().unwrap_or_else(|| "default".to_string())
        };
        by_phase.entry(phase).or_default().push(host);
    }

    by_phase
}

fn sorted_phases(by_phase: &HashMap<String, Vec<HostMetadata>>) -> Vec<String> {
    let mut phases: Vec<String> = by_phase.keys().cloned().collect();
    phases.sort();
    phases
}

#[cfg(test)]
mod tests;
