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
mod planning;
mod results;

// Re-export public types
pub use config::{DeploymentMode, PushConfiguration};
pub use context::PushContext;
pub use results::{BlockedHost, PushPlan, PushResults};

use crate::error::Result;
use crate::nix::{Flake, HostMetadata, StorePath};
use crate::observer::PushObserver;
use planning::create_deployment_plans;
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
/// - `ctx`: Nix context (flake root)
/// - `nix_backend`: Nix backend implementation (for building)
/// - `deploy_backend`: Deployment backend implementation (for copying and activation)
/// - `reboot_backend`: Reboot backend implementation (detection and execution)
/// - `hosts`: List of hosts to deploy
/// - `config`: Push configuration (mode, concurrency, etc.)
/// - `observer`: Observer to receive lifecycle events
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
/// let push_ctx = PushContext::new(&nix_backend, &deploy_backend, &reboot_backend, &config);
/// let observer = NoopObserver;
///
/// let results = push(&flake, &push_ctx, hosts, &observer)?;
/// # Ok(())
/// # }
/// ```
pub fn push(
    flake: &Flake,
    push_ctx: &PushContext,
    hosts: Vec<HostMetadata>,
    observer: &dyn PushObserver,
) -> Result<PushResults> {
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
        observer.on_phase_start(&phase, &hostnames)?;

        // Execute phase pipeline
        let phase_result = execute_phase(&phase, phase_hosts, flake, push_ctx, observer);

        // Notify observer of phase completion (always, even on failure)
        let phase_success = phase_result
            .as_ref()
            .map(|results| results.iter().all(|(_, r)| r.success))
            .unwrap_or(false);
        observer.on_phase_complete(&phase, phase_success)?;

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
    _phase: &str,
    phase_hosts: &[HostMetadata],
    flake: &Flake,
    push_ctx: &PushContext,
    observer: &dyn PushObserver,
) -> Result<Vec<(String, crate::backends::deploy::DeployResult)>> {
    // Sort hosts lexicographically for consistent rendering order
    let mut sorted_hosts = phase_hosts.to_vec();
    sorted_hosts.sort_by(|a, b| a.name.cmp(&b.name));

    // Wrap resources for parallel access
    // Note: We're wrapping references in Arc which clippy warns about,
    // but this is necessary here because:
    // 1. We only have &T not T (T contains lifetimes and can't be cloned)
    // 2. We need to share across threads (Arc)
    // 3. Rayon's par_iter requires 'static closure
    #[allow(clippy::redundant_allocation)]
    let flake = Arc::new(flake);
    #[allow(clippy::redundant_allocation)]
    let push_ctx = Arc::new(push_ctx);
    #[allow(clippy::redundant_allocation)]
    let observer = Arc::new(observer);

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
    let results: Vec<Result<(String, crate::backends::deploy::DeployResult)>> =
        pool.install(|| {
            sorted_hosts
                .par_iter()
                .map(|host_metadata| {
                    execute_host_pipeline(
                        host_metadata,
                        Arc::clone(&flake),
                        Arc::clone(&push_ctx),
                        Arc::clone(&observer),
                    )
                })
                .collect()
        });

    // Collect successful results and handle errors
    let mut phase_results = Vec::new();
    let mut first_error = None;

    for result in results {
        match result {
            Ok((hostname, deploy_result)) => {
                phase_results.push((hostname, deploy_result));
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

/// Intermediate data structure for passing build results through the pipeline
#[derive(Debug, Clone)]
struct BuildOutput {
    metadata: HostMetadata,
    target: crate::backends::deploy::DeployTarget,
    store_path: StorePath,
}

/// Intermediate data structure for passing copy results through the pipeline
#[derive(Debug, Clone)]
struct CopyOutput {
    metadata: HostMetadata,
    target: crate::backends::deploy::DeployTarget,
    store_path: StorePath,
}

/// Execute the full deployment pipeline for a single host
///
/// Runs: Build → Copy → Plan → Activate → Reboot
///
/// This function is called in parallel for each host in a phase. Observer callbacks
/// are used to track when hosts enter/leave steps, enabling dynamic step span visibility.
///
/// Note: Uses Arc<&T> pattern which clippy warns about, but is necessary because:
/// - We only have &T not T (types contain lifetimes and can't be easily cloned)
/// - We need to share across threads (Arc)
/// - Rayon's par_iter requires 'static closure
#[allow(clippy::redundant_allocation)]
fn execute_host_pipeline(
    host_metadata: &HostMetadata,
    flake: Arc<&Flake>,
    push_ctx: Arc<&PushContext>,
    observer: Arc<&dyn PushObserver>,
) -> Result<(String, crate::backends::deploy::DeployResult)> {
    let hostname = &host_metadata.name;

    // Deref Arc to get the actual references
    let flake_ref = flake.as_ref();
    let push_ctx_ref = push_ctx.as_ref();
    let observer_ref = observer.as_ref();

    // Step 1: Build
    observer_ref.on_build_start(hostname)?;
    let build_output =
        match push_ctx_ref
            .nix_backend
            .build_host(flake_ref, hostname, push_ctx_ref.config.quiet)
        {
            Ok(store_path) => {
                // Create deployment target
                let target = crate::backends::deploy::DeployTarget {
                    user: push_ctx_ref.config.ssh_user.clone(),
                    hostname: host_metadata.fqdn.clone(),
                    port: None,
                };

                BuildOutput {
                    metadata: host_metadata.clone(),
                    target,
                    store_path,
                }
            }
            Err(e) => {
                observer_ref.on_build_complete(hostname, false)?;
                return Err(e);
            }
        };
    observer_ref.on_build_complete(hostname, true)?;

    // Step 2: Copy
    observer_ref.on_copy_start(hostname)?;
    let copy_output = match push_ctx_ref
        .deploy_backend
        .copy_closure(&build_output.target, &build_output.store_path)
    {
        Ok(()) => CopyOutput {
            metadata: build_output.metadata,
            target: build_output.target,
            store_path: build_output.store_path,
        },
        Err(e) => {
            observer_ref.on_copy_complete(hostname, false)?;
            return Err(e);
        }
    };
    observer_ref.on_copy_complete(hostname, true)?;

    // Step 3: Activate (includes planning + activation)
    // Start the activate observer span before planning so planning work is visible
    observer_ref.on_activation_start(hostname)?;

    // Planning: Determine activation strategy
    let (plans, blocked) = create_deployment_plans(
        vec![(
            copy_output.metadata.clone(),
            copy_output.target.clone(),
            copy_output.store_path.clone(),
        )],
        push_ctx_ref.reboot_backend,
        push_ctx_ref.config.mode,
        push_ctx_ref.config.force,
    )?;

    // Handle blocked hosts
    if !blocked.is_empty() {
        return Err(crate::error::SnowError::Deploy(
            crate::error::DeployError::Failed {
                host: blocked[0].hostname.clone(),
                source: std::io::Error::other(blocked[0].reason.clone()),
            },
        ));
    }

    // Should only have one plan
    let plan = plans.into_iter().next().ok_or_else(|| {
        crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
            host: hostname.clone(),
            source: std::io::Error::other("No deployment plan created"),
        })
    })?;

    // Execute activation
    let activate_result = push_ctx_ref
        .deploy_backend
        .deploy(&plan.target, &plan.store_path, plan.activation_mode)
        .unwrap_or_else(|e| {
            // Build comprehensive error message with error chain
            let mut error_msg = e.to_string();
            let mut source = std::error::Error::source(&e);
            while let Some(err) = source {
                error_msg.push_str(&format!("\n  Caused by: {}", err));
                source = std::error::Error::source(err);
            }
            crate::backends::deploy::DeployResult::failure(error_msg)
        });
    observer_ref.on_activation_complete(hostname, &activate_result)?;

    if !activate_result.success {
        return Err(crate::error::SnowError::Deploy(
            crate::error::DeployError::Failed {
                host: hostname.clone(),
                source: std::io::Error::other(activate_result.message.clone()),
            },
        ));
    }

    // Step 4: Reboot (if needed)
    // Reboot if:
    // 1. allow_reboot is true, AND
    // 2. activation_mode is Boot, AND
    // 3. Either no reboot check OR reboot is required
    let should_reboot = push_ctx_ref.config.allow_reboot
        && plan.activation_mode == crate::backends::deploy::ActivationMode::Boot
        && (plan.reboot_check.is_none()
            || plan
                .reboot_check
                .as_ref()
                .map(|c| c.required)
                .unwrap_or(false));

    let final_result = if should_reboot {
        observer_ref.on_reboot_start(hostname)?;

        match push_ctx_ref.reboot_backend.reboot_and_wait(&plan.target) {
            Ok(()) => {
                observer_ref.on_reboot_complete(hostname, true)?;
                activate_result
            }
            Err(e) => {
                observer_ref.on_reboot_complete(hostname, false)?;
                crate::backends::deploy::DeployResult::failure(format!(
                    "Deployment succeeded but reboot failed: {}",
                    e
                ))
            }
        }
    } else {
        activate_result
    };

    Ok((hostname.clone(), final_result))
}

/// Helper function to execute a step and handle failures (DEPRECATED)
///
/// This function is no longer used after parallel-per-host refactor.
/// Kept temporarily for reference.
#[allow(dead_code)]
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
