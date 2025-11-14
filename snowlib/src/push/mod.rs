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
pub use config::{DeploymentMode, PushConfiguration};
pub use context::PushContext;
pub use results::{BlockedHost, PushPlan, PushResults};

use crate::error::Result;
use crate::nix::HostMetadata;
use pipeline::execute_host_pipeline;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

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
    // Group hosts by phase
    let by_phase = group_by_phase(hosts, push_ctx.config.ignore_phases);
    let phases = sorted_phases(&by_phase);

    let mut all_results = Vec::new();

    // Execute each phase sequentially
    for phase in phases {
        let phase_hosts = by_phase.get(&phase).unwrap();

        // Collect hostnames for phase lifecycle notification
        let hostnames: Vec<String> = phase_hosts.iter().map(|h| h.name.clone()).collect();

        // Notify observer of phase start
        push_ctx.observer.on_phase_start(&phase, &hostnames)?;

        // Execute phase pipeline
        let phase_result = execute_phase(push_ctx, &phase, phase_hosts);

        // Notify observer of phase completion (always, even on failure)
        let phase_success = phase_result
            .as_ref()
            .map(|results| results.iter().all(|(_, r)| r.success))
            .unwrap_or(false);
        push_ctx.observer.on_phase_complete(&phase, phase_success)?;

        // Handle phase result
        match phase_result {
            Ok(phase_results) => {
                all_results.extend(phase_results);

                // If this phase failed, stop deployment
                if !phase_success {
                    return Ok(PushResults {
                        results: all_results,
                    });
                }
            }
            Err(e) => {
                // Phase failed with error - stop deployment
                return Err(e);
            }
        }
    }

    Ok(PushResults {
        results: all_results,
    })
}

/// Execute all deployment steps for a single phase
///
/// Uses parallel-per-host execution where each host independently runs through
/// all steps (build -> copy -> activate -> reboot). This allows hosts to progress
/// at different rates - one host can be activating while another is still building.
fn execute_phase(
    push_ctx: &PushContext,
    _phase: &str,
    phase_hosts: &[HostMetadata],
) -> Result<Vec<(String, crate::backends::deploy::DeployResult)>> {
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
    let results: Vec<Result<crate::backends::deploy::DeployResult>> = pool.install(|| {
        sorted_hosts
            .par_iter()
            .map(|host_metadata| execute_host_pipeline(Arc::clone(&push_ctx), host_metadata))
            .collect()
    });

    // Collect successful results and handle errors, pairing with hostnames
    let mut phase_results = Vec::new();
    let mut first_error = None;

    for (host_metadata, result) in sorted_hosts.iter().zip(results) {
        match result {
            Ok(deploy_result) => {
                phase_results.push((host_metadata.name.clone(), deploy_result));
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    // Return first error if any
    if let Some(err) = first_error {
        return Err(err);
    }

    Ok(phase_results)
}

/// Group hosts by phase
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

/// Get sorted list of phases
fn sorted_phases(by_phase: &HashMap<String, Vec<HostMetadata>>) -> Vec<String> {
    let mut phases: Vec<String> = by_phase.keys().cloned().collect();
    phases.sort();
    phases
}

#[cfg(test)]
mod tests;
