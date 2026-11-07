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
//! The parallel execution is orchestrated by `execute_phase()` which uses tokio to
//! run `execute_host_pipeline()` concurrently for each host. Each host reports
//! its steps to the observer as host events.

mod config;
mod context;
mod hooks;
mod phase;
mod pipeline;
mod planning;
mod preflight;
mod results;
mod serial;

// Re-export public types
pub use config::{PushConfiguration, PushStrategy};
pub use context::PushContext;
pub use hooks::HookExecutor;
pub(crate) use phase::check_phase;
pub use phase::{is_name_char, ALL_HOSTS, NEVER, UNPHASED};
pub use results::{
    BlockedHost, HostDeploymentResult, PhaseResult, PipelineStep, PushPlan, PushResults,
    SkippedPhase,
};

use crate::error::Result;
use crate::host::HostMetadata;
use futures::stream::{FuturesUnordered, StreamExt};
use pipeline::execute_host_pipeline;
use serial::PhaseScheduler;
use std::collections::HashMap;

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
/// use snowlib::{push, PushConfiguration, PushContext, Flake, SystemNixBackend, SshDeployBackend, SystemRebootBackend, HostMetadata};
/// use snowlib::observer::NoopObserver;
///
/// # async fn example(hosts: Vec<HostMetadata>) -> snowlib::Result<()> {
/// let flake = Flake::new(None)?;
/// let nix_backend = SystemNixBackend::new();
/// let deploy_backend = SshDeployBackend::new();
/// let reboot_backend = SystemRebootBackend::default();
/// let config = PushConfiguration::default();
/// let observer = NoopObserver;
/// let push_ctx = PushContext::new(&flake, &nix_backend, &deploy_backend, &reboot_backend, &config, &observer, None);
///
/// let results = push(&push_ctx, hosts).await?;
/// # Ok(())
/// # }
/// ```
pub async fn push(push_ctx: &PushContext<'_>, hosts: Vec<HostMetadata>) -> Result<PushResults> {
    preflight::check_local_hooks(push_ctx, &hosts)?;
    preflight::check_ssh_agent(push_ctx).await?;

    let by_phase = group_by_phase(hosts, push_ctx.config.ignore_phases);
    let phases = sorted_phases(&by_phase);

    let mut executed_phases = Vec::new();
    let mut skipped_phases = Vec::new();

    for (idx, phase) in phases.iter().enumerate() {
        let phase_hosts = by_phase.get(phase).unwrap();

        let hostnames: Vec<String> = phase_hosts.iter().map(|h| h.name.clone()).collect();

        push_ctx.observer.on_phase_start(phase, &hostnames);

        // Execute phase and notify observer (even on failure)
        let phase_result = execute_phase(push_ctx, phase, phase_hosts).await;
        let phase_passed = phase_result
            .as_ref()
            .is_ok_and(|pr| !pr.has_blocking_failure());
        push_ctx
            .observer
            .on_phase_complete(phase, phase_result.as_ref().ok());

        // Propagate infrastructure errors immediately
        let phase_result = phase_result?;

        // Always record this phase result
        executed_phases.push(phase_result);

        // If phase had failures, skip remaining phases and stop deployment
        if !phase_passed {
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
async fn execute_phase(
    push_ctx: &PushContext<'_>,
    phase: &str,
    phase_hosts: &[HostMetadata],
) -> Result<PhaseResult> {
    let started = std::time::Instant::now();
    // Sort hosts lexicographically for consistent rendering order
    let mut sorted_hosts = phase_hosts.to_vec();
    sorted_hosts.sort_by(|a, b| a.name.cmp(&b.name));

    let scheduler = PhaseScheduler::new(push_ctx.config.concurrency, &sorted_hosts);
    let scheduler = &scheduler;

    let mut futures = FuturesUnordered::new();

    for host_metadata in sorted_hosts {
        futures.push(async move {
            let result = execute_host_pipeline(push_ctx, &host_metadata, scheduler).await;
            // Backstop for errors that bypass the pipeline's failure handling
            if !result.as_ref().is_ok_and(|r| r.success) {
                scheduler.halt_group_of(&host_metadata);
            }
            result
        });
    }

    // Collect results - note that pipeline now returns Ok(HostDeploymentResult) even on failure
    let mut host_results = Vec::new();

    while let Some(result) = futures.next().await {
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
        duration: started.elapsed(),
    })
}

/// The phases a push of `hosts` runs, in order, each with its hosts' names
pub fn phases(hosts: &[HostMetadata], ignore_phases: bool) -> Vec<(String, Vec<String>)> {
    let by_phase = group_by_phase(hosts.to_vec(), ignore_phases);
    sorted_phases(&by_phase)
        .into_iter()
        .map(|phase| {
            let mut names: Vec<String> = by_phase[&phase].iter().map(|h| h.name.clone()).collect();
            names.sort();
            (phase, names)
        })
        .collect()
}

fn group_by_phase(
    hosts: Vec<HostMetadata>,
    ignore_phases: bool,
) -> HashMap<String, Vec<HostMetadata>> {
    let mut by_phase: HashMap<String, Vec<HostMetadata>> = HashMap::new();

    for host in hosts {
        let phase = if ignore_phases {
            ALL_HOSTS.to_string()
        } else {
            host.phase.clone().unwrap_or_else(|| UNPHASED.to_string())
        };
        by_phase.entry(phase).or_default().push(host);
    }

    by_phase
}

fn sorted_phases(by_phase: &HashMap<String, Vec<HostMetadata>>) -> Vec<String> {
    let mut phases: Vec<String> = by_phase.keys().cloned().collect();
    phases.sort_by(|a, b| phase::run_order(a, b));
    phases
}

#[cfg(test)]
mod tests;
